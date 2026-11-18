//! Assemble and offline-sign a CKB-only Intention create tx.

use ckb_cinnabar_calculator::{
    address::{Address, AddressPayload},
    instruction::Instruction,
    operation::basic::{AddCellDep, AddSecp256k1SighashSignatures},
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
    TransactionCalculator,
};
use ckb_common_intention_dispatcher_calculator::{
    create_intention, encode_cid_args, Payload, CONTRACT_NAME,
};
use serde_json::Value;
use thiserror::Error;

use crate::hexutil::{decode_hash32, decode_hex, encode_hex};
use crate::request::{CreateRequest, NamedDep, ScriptJson};

/// Environment variable holding a 32-byte hex secp256k1 private key.
pub const PRIVKEY_ENV: &str = "CID_INTENTION_PRIVKEY";

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
    #[error("network guard: {0}")]
    NetworkGuard(String),
}

impl CliError {
    /// Redact anything that might look like key material in Display paths we control.
    pub fn safe_message(&self) -> String {
        let raw = self.to_string();
        if raw.len() > 8 && raw.chars().filter(|c| c.is_ascii_hexdigit()).count() > 60 {
            "error (details omitted to avoid leaking hex material)".into()
        } else {
            raw
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

/// Build and offline-sign an Intention create transaction.
///
/// `secret` must never be logged or written into the returned JSON.
pub fn create_signed_intention(req: &CreateRequest, secret: &SecretKey) -> Result<Value, CliError> {
    assert_unique_outpoints(req)?;
    let network = network_of(&req.network)?;

    if req.cid_lock_dep.name != CONTRACT_NAME {
        return Err(CliError::InvalidRequest(format!(
            "cid_lock_dep.name must be `{CONTRACT_NAME}`"
        )));
    }
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

    let process_type = packed_script(&req.process_type_script)?;
    let process_type_hash = script_hash32(&process_type);
    let beneficiary = req.beneficiary_lock.to_cid_script()?;
    let payload = Payload::from_script(&beneficiary)
        .map_err(|_| CliError::InvalidRequest("beneficiary_lock encode failed".into()))?;
    let user_lock = req.funding.lock.to_cid_script()?;
    let args = encode_cid_args(process_type_hash, user_lock, payload);

    let mut rpc = FakeRpcClient::default();
    insert_cell(
        &mut rpc,
        &req.funding.tx_hash,
        req.funding.index,
        funding_lock.clone(),
        None,
        decode_hex(&req.funding.data)?,
        Some(req.funding.capacity),
    )?;
    insert_cell(
        &mut rpc,
        &req.cid_lock_dep.tx_hash,
        req.cid_lock_dep.index,
        Script::default(),
        None,
        decode_hex(&req.cid_lock_dep.data)?,
        None,
    )?;
    for dep in &req.sighash_deps {
        insert_cell(
            &mut rpc,
            &dep.tx_hash,
            dep.index,
            Script::default(),
            None,
            decode_hex(&dep.data)?,
            None,
        )?;
    }

    let mut skeleton = TransactionSkeleton::default();
    add_named_dep(&rpc, &mut skeleton, &req.cid_lock_dep)?;
    for dep in &req.sighash_deps {
        add_named_dep(&rpc, &mut skeleton, dep)?;
    }

    let address = Address::new(network, AddressPayload::from(funding_lock.clone()));
    let create =
        create_intention::<FakeRpcClient>(address, args.clone(), req.intention_capacity, None);
    TransactionCalculator::default()
        .instruction(create)
        .apply_skeleton(&rpc, &mut skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    skeleton
        .balance(
            &rpc,
            Capacity::shannons(req.fee_shannons),
            funding_lock.clone().into(),
            ChangeReceiver::Script(funding_lock.clone().into()),
        )
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    let sign = AddSecp256k1SighashSignatures {
        user_lock_scripts: vec![ScriptEx::from(funding_lock.clone())],
        user_private_keys: vec![*secret],
    };
    TransactionCalculator::default()
        .instruction(Instruction::new(vec![Box::new(sign)]))
        .apply_skeleton(&rpc, &mut skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    // Structural checks (no network).
    if skeleton.inputs.len() != 1 {
        return Err(CliError::Calculator(format!(
            "expected 1 input, got {}",
            skeleton.inputs.len()
        )));
    }
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

    let view = skeleton.into_transaction_view();
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
    Ok(value)
}

/// Expose process type hash derivation for tests.
pub fn process_type_hash_of(script: &ScriptJson) -> Result<[u8; 32], CliError> {
    Ok(script_hash32(&packed_script(script)?))
}
