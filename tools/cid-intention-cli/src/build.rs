//! Assemble and offline-sign a CKB-only Intention create tx.

use ckb_cinnabar_calculator::{
    instruction::Instruction,
    operation::basic::{
        AddCellDep, AddInputCellByOutPoint, AddOutputCell, AddSecp256k1SighashSignatures,
    },
    re_exports::{
        ckb_hash::blake2b_256,
        ckb_jsonrpc_types,
        ckb_types::{
            core::{Capacity, ScriptHashType},
            packed::{OutPoint, Script},
            prelude::*,
            H256,
        },
        secp256k1::{PublicKey, Secp256k1, SecretKey},
    },
    rpc::Network,
    simulation::FakeRpcClient,
    skeleton::{CellOutputEx, ChangeReceiver, ScriptEx, TransactionSkeleton},
    types::{h256_to_hash, DepType},
    TransactionCalculator, TYPE_ID_CODE_HASH,
};
use ckb_common_intention_dispatcher_calculator::{
    cid_lock, encode_cid_args, Payload, CONTRACT_NAME,
};
use serde_json::Value;
use thiserror::Error;

use crate::hexutil::{decode_hash32, decode_hex, encode_hex};
use crate::request::{CreateRequest, NamedDep, ScriptJson};

/// Environment variable holding a 32-byte hex secp256k1 private key.
pub const PRIVKEY_ENV: &str = "CID_INTENTION_PRIVKEY";

fn hex_digit_count(s: &str) -> usize {
    s.chars().filter(|c| c.is_ascii_hexdigit()).count()
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("invalid hex: {0}")]
    InvalidHex(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("missing or invalid {PRIVKEY_ENV}")]
    PrivkeyEnv,
    #[error("funding lock does not match key")]
    LockKeyMismatch,
    #[error("insufficient funding capacity")]
    InsufficientFunds,
    #[error("duplicate outpoint in request")]
    DuplicateOutpoint,
    #[error("calculator: {0}")]
    Calculator(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("rpc: {0}")]
    Rpc(String),
    /// Short policy / fail-closed messages (must not carry raw hex dumps).
    #[error("network guard: {0}")]
    NetworkGuard(String),
    /// Public chain identifiers — safe to surface via [`CliError::safe_message`].
    #[error("genesis mismatch: node={node} expected={expected}")]
    GenesisMismatch { node: String, expected: String },
}

impl CliError {
    /// Operator-facing text: actionable genesis diagnostics stay visible; raw
    /// config/RPC-controlled hex floods (possible key material) are omitted.
    pub fn safe_message(&self) -> String {
        match self {
            CliError::GenesisMismatch { node, expected } => {
                format!("genesis mismatch: node={node} expected={expected}")
            }
            CliError::NetworkGuard(msg) => {
                // Do not blanket-trust config-controlled strings that may embed secrets.
                if hex_digit_count(msg) > 40 || msg.len() > 160 {
                    "network guard: details omitted".into()
                } else {
                    format!("network guard: {msg}")
                }
            }
            CliError::Rpc(msg) => {
                if hex_digit_count(msg) > 60 || msg.len() > 200 {
                    "rpc: details omitted".into()
                } else {
                    format!("rpc: {msg}")
                }
            }
            other => {
                let raw = other.to_string();
                if raw.len() > 8 && hex_digit_count(&raw) > 60 {
                    "error (details omitted to avoid leaking hex material)".into()
                } else {
                    raw
                }
            }
        }
    }
}

/// Read `CID_INTENTION_PRIVKEY` without echoing it.
pub fn load_privkey_from_env() -> Result<SecretKey, CliError> {
    let raw = std::env::var(PRIVKEY_ENV).map_err(|_| CliError::PrivkeyEnv)?;
    parse_privkey(&raw)
}

pub fn parse_privkey(raw: &str) -> Result<SecretKey, CliError> {
    let bytes = decode_hex(raw).map_err(|_| CliError::PrivkeyEnv)?;
    if bytes.len() != 32 {
        return Err(CliError::PrivkeyEnv);
    }
    SecretKey::from_slice(&bytes).map_err(|_| CliError::PrivkeyEnv)
}

fn network_of(s: &str) -> Result<Network, CliError> {
    match s.to_ascii_lowercase().as_str() {
        "testnet" => Ok(Network::Testnet),
        "fake" => Ok(Network::Fake),
        "mainnet" => Err(CliError::InvalidRequest(
            "mainnet is not supported in v1".into(),
        )),
        other => Err(CliError::InvalidRequest(format!(
            "unsupported network `{other}`"
        ))),
    }
}

fn hash_type_core(b: u8) -> Result<ScriptHashType, CliError> {
    match b {
        0 => Ok(ScriptHashType::Data),
        1 => Ok(ScriptHashType::Type),
        2 => Ok(ScriptHashType::Data1),
        4 => Ok(ScriptHashType::Data2),
        _ => Err(CliError::InvalidRequest("bad hash_type byte".into())),
    }
}

fn packed_script(s: &ScriptJson) -> Result<Script, CliError> {
    let cid = s.to_cid_script()?;
    Ok(Script::new_builder()
        .code_hash(cid.code_hash.pack())
        .hash_type(hash_type_core(cid.hash_type)?)
        .args(cid.args.pack())
        .build())
}

fn script_hash32(script: &Script) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(script.calc_script_hash().as_slice());
    out
}

/// Fail-closed: Intention CID locks must reference the code cell by Type ID.
///
/// Without a type script on the dep cell, `ScriptEx::Reference` would silently
/// fall back to `hash_type=data1` + data hash — which strands Intentions across
/// Type-ID upgrades. No data-hash opt-in in v1.
/// Process state **type** must be built-in TYPE_ID. Process lock Create does not
/// run on-chain (output-only), so creation tools must validate TYPE_ID shape here.
fn require_process_type_id_script(script: &ScriptJson) -> Result<Script, CliError> {
    let ht = script.parse_hash_type()?;
    if ht != 1 {
        return Err(CliError::InvalidRequest(format!(
            "process_type_script.hash_type must be `type` (got `{}`); Process Type ID uses hash_type=type",
            script.hash_type
        )));
    }
    let type_script = packed_script(script)?;
    let code: [u8; 32] = type_script.code_hash().unpack();
    if code != TYPE_ID_CODE_HASH {
        return Err(CliError::InvalidRequest(
            "process_type_script.code_hash must be the TYPE_ID code hash".into(),
        ));
    }
    let args_len = type_script.args().raw_data().len();
    if args_len != 32 {
        return Err(CliError::InvalidRequest(format!(
            "process_type_script.args must be exactly 32 bytes (got {args_len})"
        )));
    }
    Ok(type_script)
}

fn require_cid_lock_type_script(dep: &NamedDep) -> Result<Script, CliError> {
    let Some(type_json) = &dep.type_script else {
        return Err(CliError::InvalidRequest(
            "cid_lock_dep.type_script is required (Type ID); refusing data-hash CID lock reference"
                .into(),
        ));
    };
    let ht = type_json.parse_hash_type()?;
    if ht != 1 {
        return Err(CliError::InvalidRequest(format!(
            "cid_lock_dep.type_script.hash_type must be `type` (got `{}`); Type ID cells use hash_type=type",
            type_json.hash_type
        )));
    }
    let type_script = packed_script(type_json)?;
    let code: [u8; 32] = type_script.code_hash().unpack();
    if code != TYPE_ID_CODE_HASH {
        return Err(CliError::InvalidRequest(
            "cid_lock_dep.type_script.code_hash must be the TYPE_ID code hash".into(),
        ));
    }
    let args_len = type_script.args().raw_data().len();
    if args_len != 32 {
        return Err(CliError::InvalidRequest(format!(
            "cid_lock_dep.type_script.args must be exactly 32 bytes (got {args_len})"
        )));
    }
    Ok(type_script)
}

fn lock_for_key(secret: &SecretKey, hash_type: ScriptHashType, code_hash: [u8; 32]) -> Script {
    let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), secret).serialize();
    let args = blake2b_256(pubkey)[..20].to_vec();
    Script::new_builder()
        .code_hash(code_hash.pack())
        .hash_type(hash_type)
        .args(args.pack())
        .build()
}

fn outpoint(tx_hash: &str, index: u32) -> Result<OutPoint, CliError> {
    let h = decode_hash32(tx_hash)?;
    Ok(OutPoint::new(H256(h).pack(), index))
}

fn insert_cell(
    rpc: &mut FakeRpcClient,
    tx_hash: &str,
    index: u32,
    lock: Script,
    type_script: Option<Script>,
    data: Vec<u8>,
    capacity: Option<u64>,
) -> Result<(), CliError> {
    let cell = CellOutputEx::new_from_scripts(lock, type_script, data, capacity)
        .map_err(|e| CliError::Calculator(e.to_string()))?;
    rpc.insert_fake_cell(outpoint(tx_hash, index)?, cell, None);
    Ok(())
}

fn assert_unique_outpoints(req: &CreateRequest) -> Result<(), CliError> {
    let mut keys = Vec::new();
    keys.push((req.funding.tx_hash.to_ascii_lowercase(), req.funding.index));
    keys.push((
        req.cid_lock_dep.tx_hash.to_ascii_lowercase(),
        req.cid_lock_dep.index,
    ));
    for d in &req.sighash_deps {
        keys.push((d.tx_hash.to_ascii_lowercase(), d.index));
    }
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != keys.len() {
        return Err(CliError::DuplicateOutpoint);
    }
    Ok(())
}

fn add_named_dep(
    rpc: &FakeRpcClient,
    skeleton: &mut TransactionSkeleton,
    dep: &NamedDep,
) -> Result<(), CliError> {
    let tx = h256_to_hash(&H256(decode_hash32(&dep.tx_hash)?));
    let ix = Instruction::new(vec![Box::new(AddCellDep {
        name: dep.name.clone(),
        tx_hash: tx,
        index: dep.index,
        dep_type: DepType::Code,
        with_data: true,
    })]);
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(rpc, skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;
    Ok(())
}

/// Signed Intention plus FakeRpc/skeleton retained for offline VM verification.
pub struct SignedIntentionBundle {
    pub tx_json: Value,
    pub rpc: FakeRpcClient,
    pub skeleton: TransactionSkeleton,
}

/// Build and offline-sign an Intention create transaction.
///
/// `secret` must never be logged or written into the returned JSON.
pub fn create_signed_intention(req: &CreateRequest, secret: &SecretKey) -> Result<Value, CliError> {
    Ok(create_signed_intention_bundle(req, secret)?.tx_json)
}

/// Same as [`create_signed_intention`], but keeps FakeRpc + skeleton for tests
/// (e.g. TransactionSimulator sighash verification with synthetic keys).
pub fn create_signed_intention_bundle(
    req: &CreateRequest,
    secret: &SecretKey,
) -> Result<SignedIntentionBundle, CliError> {
    create_signed_intention_bundle_inner(req, secret, &[])
}

/// Like [`create_signed_intention_bundle`], but seeds additional same-lock live cells
/// into FakeRpc before outpoint-pinned selection (proves substitutes cannot win).
pub fn create_signed_intention_with_same_lock_decoy(
    req: &CreateRequest,
    secret: &SecretKey,
    decoy_tx_hash: &str,
    decoy_index: u32,
    decoy_capacity: u64,
) -> Result<SignedIntentionBundle, CliError> {
    create_signed_intention_bundle_inner(
        req,
        secret,
        &[(decoy_tx_hash, decoy_index, decoy_capacity)],
    )
}

fn create_signed_intention_bundle_inner(
    req: &CreateRequest,
    secret: &SecretKey,
    same_lock_decoys: &[(&str, u32, u64)],
) -> Result<SignedIntentionBundle, CliError> {
    assert_unique_outpoints(req)?;
    let _network = network_of(&req.network)?;

    if req.cid_lock_dep.name != CONTRACT_NAME {
        return Err(CliError::InvalidRequest(format!(
            "cid_lock_dep.name must be `{CONTRACT_NAME}`"
        )));
    }
    let cid_type_script = require_cid_lock_type_script(&req.cid_lock_dep)?;
    if req.sighash_deps.len() < 2 {
        return Err(CliError::InvalidRequest(
            "sighash_deps must include secp code and data cells".into(),
        ));
    }
    if req.intention_capacity == 0 || req.fee_shannons == 0 {
        return Err(CliError::InvalidRequest(
            "intention_capacity and fee_shannons must be > 0".into(),
        ));
    }

    let funding_lock = packed_script(&req.funding.lock)?;
    let funding_code: [u8; 32] = funding_lock.code_hash().unpack();
    let funding_ht = hash_type_core(u8::from(funding_lock.hash_type()))?;
    let expected_lock = lock_for_key(secret, funding_ht, funding_code);
    if expected_lock != funding_lock {
        return Err(CliError::LockKeyMismatch);
    }

    // Change needs a realistic minimum; require funding covers intention + fee + 61 CKB dust floor.
    const MIN_CHANGE: u64 = 61_0000_0000;
    let need = req
        .intention_capacity
        .checked_add(req.fee_shannons)
        .and_then(|v| v.checked_add(MIN_CHANGE))
        .ok_or(CliError::InsufficientFunds)?;
    if req.funding.capacity < need {
        return Err(CliError::InsufficientFunds);
    }

    let process_type = require_process_type_id_script(&req.process_type_script)?;
    let process_type_hash = script_hash32(&process_type);
    let beneficiary = req.beneficiary_lock.to_cid_script()?;
    let payload = Payload::from_script(&beneficiary)
        .map_err(|_| CliError::InvalidRequest("beneficiary_lock encode failed".into()))?;
    let user_lock = req.funding.lock.to_cid_script()?;
    let args = encode_cid_args(process_type_hash, user_lock, payload);

    let funding_out = outpoint(&req.funding.tx_hash, req.funding.index)?;
    let funding_tx_hash = h256_to_hash(&H256(decode_hash32(&req.funding.tx_hash)?));
    let expected_cid_code_hash = script_hash32(&cid_type_script);

    let mut rpc = FakeRpcClient::default();
    // Seed decoys first so address-ordered scans would prefer them if used.
    for (tx, idx, cap) in same_lock_decoys {
        insert_cell(
            &mut rpc,
            tx,
            *idx,
            funding_lock.clone(),
            None,
            Vec::new(),
            Some(*cap),
        )?;
    }
    insert_cell(
        &mut rpc,
        &req.funding.tx_hash,
        req.funding.index,
        funding_lock.clone(),
        None,
        decode_hex(&req.funding.data)?,
        Some(req.funding.capacity),
    )?;
    // Seed the live CID code cell **with** its Type ID type script so
    // `ScriptEx::Reference` resolves to hash_type=type (never Data1).
    insert_cell(
        &mut rpc,
        &req.cid_lock_dep.tx_hash,
        req.cid_lock_dep.index,
        Script::default(),
        Some(cid_type_script.clone()),
        decode_hex(&req.cid_lock_dep.data)?,
        None,
    )?;
    for dep in &req.sighash_deps {
        let typ = match &dep.type_script {
            Some(s) => Some(packed_script(s)?),
            None => None,
        };
        insert_cell(
            &mut rpc,
            &dep.tx_hash,
            dep.index,
            Script::default(),
            typ,
            decode_hex(&dep.data)?,
            None,
        )?;
    }

    let mut skeleton = TransactionSkeleton::default();
    add_named_dep(&rpc, &mut skeleton, &req.cid_lock_dep)?;
    for dep in &req.sighash_deps {
        add_named_dep(&rpc, &mut skeleton, dep)?;
    }

    // Pin funding by explicit outpoint (CKB-only create path; no address scan).
    let create = Instruction::new(vec![
        Box::new(AddInputCellByOutPoint {
            tx_hash: funding_tx_hash,
            index: req.funding.index,
            since: None,
        }),
        Box::new(AddOutputCell {
            lock_script: cid_lock(args),
            type_script: None,
            data: Vec::new(),
            capacity: req.intention_capacity,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    TransactionCalculator::default()
        .instruction(create)
        .apply_skeleton(&rpc, &mut skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    // Enforce outpoint contract before fee selection can pull same-lock substitutes.
    if skeleton.inputs.len() != 1 {
        return Err(CliError::Calculator(format!(
            "expected 1 input after outpoint pin, got {}",
            skeleton.inputs.len()
        )));
    }
    if skeleton.inputs[0].input.previous_output() != funding_out {
        return Err(CliError::Calculator(
            "funding previous_output does not match requested outpoint".into(),
        ));
    }

    skeleton
        .balance(
            &rpc,
            Capacity::shannons(req.fee_shannons),
            funding_lock.clone().into(),
            ChangeReceiver::Script(funding_lock.clone().into()),
        )
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    // Re-assert after balance: still exactly the requested funding outpoint (no substitutes).
    if skeleton.inputs.len() != 1 {
        return Err(CliError::Calculator(format!(
            "expected 1 input after balance, got {}",
            skeleton.inputs.len()
        )));
    }
    if skeleton.inputs[0].input.previous_output() != funding_out {
        return Err(CliError::Calculator(
            "funding previous_output changed after balance".into(),
        ));
    }

    let sign = AddSecp256k1SighashSignatures {
        user_lock_scripts: vec![ScriptEx::from(funding_lock.clone())],
        user_private_keys: vec![*secret],
    };
    TransactionCalculator::default()
        .instruction(Instruction::new(vec![Box::new(sign)]))
        .apply_skeleton(&rpc, &mut skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    // Structural checks (no network).
    if skeleton.outputs.len() < 2 {
        return Err(CliError::Calculator(
            "expected intention + change outputs".into(),
        ));
    }
    let intention_out = &skeleton.outputs[0];
    if intention_out.capacity_shannons() != req.intention_capacity {
        return Err(CliError::Calculator("intention capacity mismatch".into()));
    }
    let lock = intention_out.lock_script();
    let lock_code: [u8; 32] = lock.code_hash().unpack();
    let lock_ht: u8 = lock.hash_type().into();
    if lock_ht != 1 || lock_code != expected_cid_code_hash {
        return Err(CliError::Calculator(format!(
            "intention lock must resolve to Type ID reference (hash_type=type, code_hash=type_script_hash); got hash_type={lock_ht}"
        )));
    }
    let lock_args = lock.args().raw_data();
    if lock_args.len() < 32 || lock_args[..32] != process_type_hash {
        return Err(CliError::Calculator(
            "intention lock args missing process_type_hash prefix".into(),
        ));
    }
    let total_in: u64 = skeleton
        .inputs
        .iter()
        .map(|i| i.output.capacity_shannons())
        .sum();
    let total_out: u64 = skeleton.outputs.iter().map(|o| o.capacity_shannons()).sum();
    if total_in.saturating_sub(total_out) != req.fee_shannons {
        return Err(CliError::Calculator("fee mismatch after balance".into()));
    }
    if skeleton.witnesses.is_empty() {
        return Err(CliError::Calculator("missing sighash witness".into()));
    }

    let view = skeleton.clone().into_transaction_view();
    let json_tx = ckb_jsonrpc_types::TransactionView::from(view);
    let mut value = serde_json::to_value(&json_tx)?;
    // Attach non-secret metadata useful for operators.
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "process_type_hash".into(),
            Value::String(encode_hex(&process_type_hash)),
        );
    }
    // Belt-and-suspenders: ensure privkey hex is not present.
    let dumped = serde_json::to_string(&value)?;
    let key_hex = encode_hex(&secret.secret_bytes());
    let key_raw = hex::encode(secret.secret_bytes());
    if dumped.contains(&key_hex) || dumped.contains(&key_raw) {
        return Err(CliError::Calculator(
            "refusing to emit output that contains key material".into(),
        ));
    }
    Ok(SignedIntentionBundle {
        tx_json: value,
        rpc,
        skeleton,
    })
}

/// Expose process type hash derivation for tests.
pub fn process_type_hash_of(script: &ScriptJson) -> Result<[u8; 32], CliError> {
    Ok(script_hash32(&packed_script(script)?))
}
