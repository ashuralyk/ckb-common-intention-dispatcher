//! Assemble and offline-sign a Process state cell create tx.

use cid_process_example_calculator::{
    create_process_state_by_outpoint, CREATE_FEE_RATE, DEFAULT_PROCESS_STATE_CAPACITY,
    PROCESS_CONTRACT_NAME,
};
use ckb_cinnabar_calculator::{
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
    TransactionCalculator, TYPE_ID_CODE_HASH,
};
use ckb_common_intention_dispatcher_calculator::CONTRACT_NAME;
use serde_json::{json, Value};

use crate::build::CliError;
use crate::fee::{
    apply_fee_by_shrinking_change, assert_fee_covers_rate, install_sighash_witness_placeholders,
    min_fee_for_skeleton,
};
use crate::hexutil::{decode_hash32, decode_hex, encode_hex};
use crate::request::{CreateProcessRequest, NamedDep, ScriptJson};

// Positions in the signed transaction produced by create-process.
pub(crate) const CID_CODE_DEP_INDEX: usize = 0;
pub(crate) const PROCESS_CODE_DEP_INDEX: usize = 1;
pub(crate) const PROCESS_STATE_OUTPUT_INDEX: usize = 0;

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

fn script_json_from_packed(script: &Script) -> ScriptJson {
    let ht: u8 = script.hash_type().into();
    ScriptJson {
        code_hash: encode_hex(script.code_hash().as_slice()),
        hash_type: match ht {
            0 => "data".into(),
            1 => "type".into(),
            2 => "data1".into(),
            4 => "data2".into(),
            _ => "type".into(),
        },
        args: encode_hex(&script.args().raw_data()),
    }
}

/// Fail-closed: Process / CID code deps must carry built-in TYPE_ID type scripts.
fn require_code_type_id_script(dep: &NamedDep, label: &str) -> Result<Script, CliError> {
    let Some(type_json) = &dep.type_script else {
        return Err(CliError::InvalidRequest(format!(
            "{label}.type_script is required (Type ID); refusing data-hash code reference"
        )));
    };
    let ht = type_json.parse_hash_type()?;
    if ht != 1 {
        return Err(CliError::InvalidRequest(format!(
            "{label}.type_script.hash_type must be `type` (got `{}`); Type ID cells use hash_type=type",
            type_json.hash_type
        )));
    }
    let type_script = packed_script(type_json)?;
    let code: [u8; 32] = type_script.code_hash().unpack();
    if code != TYPE_ID_CODE_HASH {
        return Err(CliError::InvalidRequest(format!(
            "{label}.type_script.code_hash must be the TYPE_ID code hash"
        )));
    }
    let args_len = type_script.args().raw_data().len();
    if args_len != 32 {
        return Err(CliError::InvalidRequest(format!(
            "{label}.type_script.args must be exactly 32 bytes (got {args_len})"
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

fn assert_unique_outpoints(req: &CreateProcessRequest) -> Result<(), CliError> {
    let mut keys = Vec::new();
    keys.push((req.funding.tx_hash.to_ascii_lowercase(), req.funding.index));
    keys.push((
        req.cid_lock_dep.tx_hash.to_ascii_lowercase(),
        req.cid_lock_dep.index,
    ));
    keys.push((
        req.process_code_dep.tx_hash.to_ascii_lowercase(),
        req.process_code_dep.index,
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

/// Signed Process state create plus FakeRpc/skeleton for offline checks.
pub struct SignedProcessBundle {
    pub tx_json: Value,
    pub rpc: FakeRpcClient,
    pub skeleton: TransactionSkeleton,
}

/// Build and offline-sign a Process state create transaction.
///
/// Fee is computed at [`CREATE_FEE_RATE`] (1000). `secret` must never be logged
/// or written into the returned JSON.
pub fn create_signed_process_state(
    req: &CreateProcessRequest,
    secret: &SecretKey,
) -> Result<Value, CliError> {
    Ok(create_signed_process_state_bundle(req, secret)?.tx_json)
}

pub fn create_signed_process_state_bundle(
    req: &CreateProcessRequest,
    secret: &SecretKey,
) -> Result<SignedProcessBundle, CliError> {
    assert_unique_outpoints(req)?;
    let _network = network_of(&req.network)?;

    if req.cid_lock_dep.name != CONTRACT_NAME {
        return Err(CliError::InvalidRequest(format!(
            "cid_lock_dep.name must be `{CONTRACT_NAME}`"
        )));
    }
    if req.process_code_dep.name != PROCESS_CONTRACT_NAME {
        return Err(CliError::InvalidRequest(format!(
            "process_code_dep.name must be `{PROCESS_CONTRACT_NAME}`"
        )));
    }
    let cid_type_script = require_code_type_id_script(&req.cid_lock_dep, "cid_lock_dep")?;
    let cid_code_type_id = script_hash32(&cid_type_script);
    let cid_code_type_id_args = {
        let raw = cid_type_script.args().raw_data();
        let mut a = [0u8; 32];
        a.copy_from_slice(&raw);
        a
    };
    let process_code_type = require_code_type_id_script(&req.process_code_dep, "process_code_dep")?;
    let process_code_type_hash = script_hash32(&process_code_type);
    if cid_code_type_id == process_code_type_hash {
        return Err(CliError::InvalidRequest(
            "CID and Process code deps must have distinct Type IDs".into(),
        ));
    }
    let process_code_type_id_args = {
        let raw = process_code_type.args().raw_data();
        let mut a = [0u8; 32];
        a.copy_from_slice(&raw);
        a
    };

    if req.sighash_deps.len() < 2 {
        return Err(CliError::InvalidRequest(
            "sighash_deps must include secp code and data cells".into(),
        ));
    }

    let capacity = if req.process_capacity == 0 {
        DEFAULT_PROCESS_STATE_CAPACITY
    } else {
        req.process_capacity
    };
    let lock_args = decode_hex(&req.process_lock_args)?;
    if lock_args.is_empty() || cid_protocol::CidArgs::decode(&lock_args).is_ok() {
        return Err(CliError::InvalidRequest(
            "process_lock_args must be non-empty and distinct from CID Intention encoding".into(),
        ));
    }

    let funding_lock = packed_script(&req.funding.lock)?;
    let funding_code: [u8; 32] = funding_lock.code_hash().unpack();
    let funding_ht = hash_type_core(u8::from(funding_lock.hash_type()))?;
    let expected_lock = lock_for_key(secret, funding_ht, funding_code);
    if expected_lock != funding_lock {
        return Err(CliError::LockKeyMismatch);
    }

    // The exact fee is calculated after installing the sighash placeholder.
    const MIN_CHANGE: u64 = 61_0000_0000;
    let need = capacity
        .checked_add(MIN_CHANGE)
        .ok_or(CliError::InsufficientFunds)?;
    if req.funding.capacity < need {
        return Err(CliError::InsufficientFunds);
    }

    let funding_out = outpoint(&req.funding.tx_hash, req.funding.index)?;
    let funding_tx_hash = h256_to_hash(&H256(decode_hash32(&req.funding.tx_hash)?));

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
        Some(cid_type_script),
        decode_hex(&req.cid_lock_dep.data)?,
        None,
    )?;
    insert_cell(
        &mut rpc,
        &req.process_code_dep.tx_hash,
        req.process_code_dep.index,
        Script::default(),
        Some(process_code_type.clone()),
        decode_hex(&req.process_code_dep.data)?,
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
    add_named_dep(&rpc, &mut skeleton, &req.process_code_dep)?;
    for dep in &req.sighash_deps {
        add_named_dep(&rpc, &mut skeleton, dep)?;
    }

    let create = create_process_state_by_outpoint(
        (funding_tx_hash, req.funding.index),
        lock_args.clone(),
        capacity,
    );
    TransactionCalculator::default()
        .instruction(create)
        .apply_skeleton(&rpc, &mut skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;

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

    // Materialize change first (fee=0), then size with sighash placeholders and
    // apply FeeRate(1000) on serialized_size_in_block (= molecule + 4).
    skeleton
        .balance(
            &rpc,
            Capacity::shannons(0),
            funding_lock.clone().into(),
            ChangeReceiver::Script(funding_lock.clone().into()),
        )
        .map_err(|e| CliError::Calculator(e.to_string()))?;

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
    if skeleton.outputs.len() < 2 {
        return Err(CliError::Calculator(
            "expected process state + change outputs".into(),
        ));
    }
    let change_index = skeleton.outputs.len() - 1;
    install_sighash_witness_placeholders(&mut skeleton);
    let required_fee = min_fee_for_skeleton(&skeleton, CREATE_FEE_RATE);
    apply_fee_by_shrinking_change(&mut skeleton, change_index, required_fee)?;

    let sign = AddSecp256k1SighashSignatures {
        user_lock_scripts: vec![ScriptEx::from(funding_lock.clone())],
        user_private_keys: vec![*secret],
    };
    TransactionCalculator::default()
        .instruction(Instruction::new(vec![Box::new(sign)]))
        .apply_skeleton(&rpc, &mut skeleton)
        .map_err(|e| CliError::Calculator(e.to_string()))?;

    let state_out = &skeleton.outputs[PROCESS_STATE_OUTPUT_INDEX];
    if state_out.capacity_shannons() != capacity {
        return Err(CliError::Calculator("process capacity mismatch".into()));
    }
    if !state_out.data.is_empty() {
        return Err(CliError::Calculator(
            "process state data must be empty".into(),
        ));
    }

    // Off-chain Process lock validation (Create does not run on-chain).
    let lock = state_out.lock_script();
    let lock_code: [u8; 32] = lock.code_hash().unpack();
    let lock_ht: u8 = lock.hash_type().into();
    if lock_ht != 1 || lock_code != process_code_type_hash {
        return Err(CliError::Calculator(format!(
            "process lock must resolve to Process code Type ID (hash_type=type); got hash_type={lock_ht}"
        )));
    }
    if lock.args().raw_data().as_ref() != lock_args.as_slice() {
        return Err(CliError::Calculator(
            "process lock args mismatch after assemble".into(),
        ));
    }

    // Off-chain TYPE_ID validation.
    let typ = state_out
        .type_script()
        .ok_or_else(|| CliError::Calculator("process state missing TYPE_ID type".into()))?;
    let typ_code: [u8; 32] = typ.code_hash().unpack();
    let typ_ht: u8 = typ.hash_type().into();
    if typ_ht != 1 || typ_code != TYPE_ID_CODE_HASH {
        return Err(CliError::Calculator(
            "process type must be built-in TYPE_ID with hash_type=type".into(),
        ));
    }
    if typ.args().raw_data().len() != 32 {
        return Err(CliError::Calculator(
            "process TYPE_ID args must be exactly 32 bytes".into(),
        ));
    }
    let expected_type_id_args = skeleton
        .calc_type_id(PROCESS_STATE_OUTPUT_INDEX)
        .map_err(|e| CliError::Calculator(e.to_string()))?;
    let mut identity_type_id_args = [0u8; 32];
    identity_type_id_args.copy_from_slice(&typ.args().raw_data());
    if identity_type_id_args != expected_type_id_args {
        return Err(CliError::Calculator(
            "process TYPE_ID args do not match first_input+out_index formula".into(),
        ));
    }
    let identity_type_id = script_hash32(&typ);
    let process_type_script = script_json_from_packed(&typ);

    if skeleton.witnesses.is_empty() {
        return Err(CliError::Calculator("missing sighash witness".into()));
    }
    let (paid_fee, _) = assert_fee_covers_rate(&skeleton, CREATE_FEE_RATE)?;

    for (index, dep) in [
        (CID_CODE_DEP_INDEX, &req.cid_lock_dep),
        (PROCESS_CODE_DEP_INDEX, &req.process_code_dep),
    ] {
        let expected = outpoint(&dep.tx_hash, dep.index)?;
        if skeleton.celldeps.get(index).map(|d| d.celldep.out_point()) != Some(expected) {
            return Err(CliError::Calculator(format!(
                "unexpected {} position in Process create cell_deps",
                dep.name
            )));
        }
    }

    let view = skeleton.clone().into_transaction_view();
    let tx_hash = view.hash();
    let json_tx = ckb_jsonrpc_types::TransactionView::from(view);
    let mut value = serde_json::to_value(&json_tx)?;

    let state_outpoint = json!({
        "tx_hash": encode_hex(tx_hash.as_slice()),
        "index": PROCESS_STATE_OUTPUT_INDEX as u32,
    });
    let registry_post = json!({
        "identity_type_id_args": encode_hex(&identity_type_id_args),
        "identity_type_id": encode_hex(&identity_type_id),
        "code_type_id_args": encode_hex(&process_code_type_id_args),
        "code_type_id": encode_hex(&process_code_type_hash),
    });

    if let Some(obj) = value.as_object_mut() {
        obj.insert("state_outpoint".into(), state_outpoint);
        obj.insert(
            "identity_type_id".into(),
            Value::String(encode_hex(&identity_type_id)),
        );
        obj.insert(
            "identity_type_id_args".into(),
            Value::String(encode_hex(&identity_type_id_args)),
        );
        obj.insert(
            "process_type_script".into(),
            serde_json::to_value(&process_type_script)?,
        );
        obj.insert("registry_post".into(), registry_post);
        // Bind send-time CID live Type ID check to the create request (not any
        // other live TYPE_ID celldep).
        obj.insert(
            "cid_code_type_id".into(),
            Value::String(encode_hex(&cid_code_type_id)),
        );
        obj.insert(
            "cid_code_type_id_args".into(),
            Value::String(encode_hex(&cid_code_type_id_args)),
        );
        obj.insert(
            "fee_shannons".into(),
            Value::Number(serde_json::Number::from(paid_fee)),
        );
        obj.insert(
            "fee_rate".into(),
            Value::Number(serde_json::Number::from(CREATE_FEE_RATE)),
        );
    }

    let dumped = serde_json::to_string(&value)?;
    let key_hex = encode_hex(&secret.secret_bytes());
    let key_raw = hex::encode(secret.secret_bytes());
    if dumped.contains(&key_hex) || dumped.contains(&key_raw) {
        return Err(CliError::Calculator(
            "refusing to emit output that contains key material".into(),
        ));
    }

    Ok(SignedProcessBundle {
        tx_json: value,
        rpc,
        skeleton,
    })
}
