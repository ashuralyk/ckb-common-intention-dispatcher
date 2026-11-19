//! Explicit testnet broadcast of a previously signed tx JSON.
//!
//! Never used by `create`. Development/CI must point `--rpc-url` at a local mock,
//! not a public endpoint.

use ckb_cinnabar_calculator::{
    re_exports::{
        ckb_jsonrpc_types,
        ckb_types::{packed, prelude::*},
    },
    TYPE_ID_CODE_HASH,
};
use serde_json::{json, Value};

use crate::build::{process_type_hash_of, CliError};
use crate::hexutil::{decode_hash32, decode_hex, encode_hex};
use crate::process_build::{
    CID_CODE_DEP_INDEX, PROCESS_CODE_DEP_INDEX, PROCESS_STATE_OUTPUT_INDEX,
};
use crate::request::ScriptJson;

/// Options for `send` (no private key).
#[derive(Debug, Clone)]
pub struct SendOptions {
    pub rpc_url: String,
    /// Must be `testnet` in v1.
    pub network: String,
    /// Expected genesis block hash (`get_block_hash 0x0`).
    pub genesis_hash: String,
}

/// Node `get_blockchain_info.chain` values accepted for `send` (fail-closed).
const ALLOWED_TESTNET_CHAINS: &[&str] = &["ckb_testnet"];

fn redact_url(url: &str) -> String {
    // Strip credentials if present: https://user:pass@host/...
    if let Some(idx) = url.find("://") {
        let (scheme, rest) = url.split_at(idx + 3);
        if let Some(at) = rest.find('@') {
            return format!("{scheme}***@{}", &rest[at + 1..]);
        }
    }
    url.to_string()
}

fn strip_tx_metadata(mut value: Value) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("hash");
        obj.remove("process_type_hash");
        // Process create metadata (not part of RPC Transaction).
        obj.remove("state_outpoint");
        obj.remove("identity_type_id");
        obj.remove("identity_type_id_args");
        obj.remove("process_type_script");
        obj.remove("registry_post");
        obj.remove("cid_code_type_id");
        obj.remove("cid_code_type_id_args");
        obj.remove("fee_shannons");
        obj.remove("fee_rate");
    }
    value
}

fn has_process_metadata(value: &Value) -> bool {
    [
        "state_outpoint",
        "identity_type_id",
        "identity_type_id_args",
        "process_type_script",
        "registry_post",
        "cid_code_type_id",
        "cid_code_type_id_args",
    ]
    .iter()
    .any(|field| value.get(*field).is_some())
}

fn extract_transaction(
    mut value: &Value,
) -> Result<(Value, ckb_jsonrpc_types::Transaction, Option<&Value>), CliError> {
    let mut process_metadata = None;
    loop {
        if has_process_metadata(value) && process_metadata.replace(value).is_some() {
            return Err(CliError::InvalidRequest(
                "Process create metadata must come from one transaction layer".into(),
            ));
        }
        if value.get("inputs").is_some() && value.get("outputs").is_some() {
            let cleaned = strip_tx_metadata(value.clone());
            // The RPC Transaction type rejects fields that do not belong on the wire.
            let parsed = serde_json::from_value(cleaned.clone()).map_err(|e| {
                CliError::InvalidRequest(format!(
                    "signed JSON is not a valid CKB Transaction after stripping metadata: {e}"
                ))
            })?;
            return Ok((cleaned, parsed, process_metadata));
        }
        value = value
            .get("transaction")
            .or_else(|| value.get("inner"))
            .ok_or_else(|| {
                CliError::InvalidRequest("signed JSON does not contain a transaction body".into())
            })?;
    }
}

fn rpc_call(url: &str, method: &str, params: Value) -> Result<Value, CliError> {
    let body = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| CliError::Rpc(format!("client build failed: {e}")))?;
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|_| CliError::Rpc(format!("request failed to {}", redact_url(url))))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|_| CliError::Rpc("failed to read RPC body".into()))?;
    if !status.is_success() {
        return Err(CliError::Rpc(format!(
            "HTTP {} from {}",
            status.as_u16(),
            redact_url(url)
        )));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| CliError::Rpc("invalid JSON-RPC response".into()))?;
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("rpc error");
        return Err(CliError::Rpc(msg.to_string()));
    }
    v.get("result")
        .cloned()
        .ok_or_else(|| CliError::Rpc("missing JSON-RPC result".into()))
}

fn assert_testnet_guards(opts: &SendOptions) -> Result<(), CliError> {
    if !opts.network.eq_ignore_ascii_case("testnet") {
        return Err(CliError::NetworkGuard(
            "send only allows --network testnet".into(),
        ));
    }
    let _ = decode_hash32(&opts.genesis_hash)?;
    let url_l = opts.rpc_url.to_ascii_lowercase();
    // Fail-closed heuristics for accidental mainnet endpoints.
    if url_l.contains("mainnet")
        || url_l.contains("ckb.rpc") && !url_l.contains("testnet") && url_l.contains("main")
    {
        return Err(CliError::NetworkGuard(
            "rpc-url looks like mainnet; refusing to send".into(),
        ));
    }
    Ok(())
}

fn assert_allowed_testnet_chain(chain_info: &Value) -> Result<(), CliError> {
    let chain = chain_info
        .get("chain")
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            CliError::NetworkGuard("node get_blockchain_info missing non-empty chain".into())
        })?;
    let chain_l = chain.to_ascii_lowercase();
    if ALLOWED_TESTNET_CHAINS
        .iter()
        .any(|allowed| chain_l == *allowed)
    {
        return Ok(());
    }
    if chain_l == "ckb" || (chain_l.contains("main") && !chain_l.contains("test")) {
        return Err(CliError::NetworkGuard(format!(
            "node reports chain `{chain}`; mainnet forbidden"
        )));
    }
    Err(CliError::NetworkGuard(format!(
        "node chain `{chain}` is not an allowed testnet identity"
    )))
}

fn script_json_from_rpc(v: &Value) -> Result<ScriptJson, CliError> {
    let code_hash = v
        .get("code_hash")
        .and_then(|c| c.as_str())
        .ok_or_else(|| CliError::Rpc("live cell type missing code_hash".into()))?
        .to_string();
    let hash_type = v
        .get("hash_type")
        .and_then(|h| h.as_str())
        .ok_or_else(|| CliError::Rpc("live cell type missing hash_type".into()))?
        .to_string();
    let args = v
        .get("args")
        .and_then(|a| a.as_str())
        .ok_or_else(|| CliError::Rpc("live cell type missing args".into()))?
        .to_string();
    Ok(ScriptJson {
        code_hash,
        hash_type,
        args,
    })
}

/// Collect Type-ID `code_hash` values for **CID Intention** output locks only.
///
/// Must not treat every `hash_type=type` lock as CID: testnet secp256k1/blake160
/// also uses `hash_type=type`, often with a sighash `dep_group` whose group cell
/// is not a TYPE_ID code cell. Scope by successful `CidArgs` decode.
fn cid_intention_type_id_code_hashes(tx: &Value) -> Result<Vec<[u8; 32]>, CliError> {
    let outputs = tx
        .get("outputs")
        .and_then(|o| o.as_array())
        .ok_or_else(|| CliError::InvalidRequest("transaction missing outputs".into()))?;
    let mut out = Vec::new();
    for cell in outputs {
        let lock = cell
            .get("lock")
            .ok_or_else(|| CliError::InvalidRequest("transaction output missing lock".into()))?;
        let ht = lock.get("hash_type").and_then(|h| h.as_str()).unwrap_or("");
        if !ht.eq_ignore_ascii_case("type") {
            continue;
        }
        let args_hex = lock.get("args").and_then(|a| a.as_str()).unwrap_or("0x");
        let args = decode_hex(args_hex)?;
        if cid_protocol::CidArgs::decode(&args).is_err() {
            continue;
        }
        let code = lock
            .get("code_hash")
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                CliError::InvalidRequest("CID Intention lock missing code_hash".into())
            })?;
        out.push(decode_hash32(code)?);
    }
    Ok(out)
}

fn type_id_script_hash(script: &Value) -> Result<Option<[u8; 32]>, CliError> {
    let script = script_json_from_rpc(script)?;
    if script.parse_hash_type()? != 1
        || decode_hash32(&script.code_hash)? != TYPE_ID_CODE_HASH
        || decode_hex(&script.args)?.len() != 32
    {
        return Ok(None);
    }
    Ok(Some(process_type_hash_of(&script)?))
}

/// Probe every declared dep for liveness and keep its Type ID hash at the same
/// index as the signed cell_deps vector.
fn collect_live_type_id_script_hashes(
    url: &str,
    tx: &Value,
) -> Result<Vec<Option<[u8; 32]>>, CliError> {
    let cell_deps = tx
        .get("cell_deps")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::InvalidRequest("transaction missing cell_deps".into()))?;
    cell_deps
        .iter()
        .map(|dep| {
            let out_point = dep
                .get("out_point")
                .ok_or_else(|| CliError::InvalidRequest("cell_dep missing out_point".into()))?;
            let live = rpc_call(url, "get_live_cell", json!([out_point, false]))?;
            if live.get("status").and_then(Value::as_str) != Some("live") {
                return Err(CliError::NetworkGuard(
                    "cell_dep outpoint is not live; refusing send".into(),
                ));
            }
            match live.pointer("/cell/output/type").filter(|t| !t.is_null()) {
                Some(typ) => type_id_script_hash(typ),
                None => Ok(None),
            }
        })
        .collect()
}

/// Fail-closed: every CID Intention lock (`hash_type=type` + CID args) in `tx`
/// must match a **live** celldep whose type script is TYPE_ID and whose script
/// hash equals the lock `code_hash`. Offline `create` only consistency-checks
/// the request against itself; a mistyped Type ID args with the correct
/// outpoint would otherwise broadcast a stranded Intention. Non-CID type locks
/// (e.g. genesis secp) are ignored so raw-transaction send with `dep_group`
/// remains usable.
fn assert_live_cid_type_id_matches_intention(url: &str, tx: &Value) -> Result<(), CliError> {
    let want_hashes = cid_intention_type_id_code_hashes(tx)?;
    if want_hashes.is_empty() {
        return Ok(());
    }
    let cell_deps = tx.get("cell_deps").and_then(|c| c.as_array());
    if cell_deps.map(|c| c.is_empty()).unwrap_or(true) {
        return Err(CliError::NetworkGuard(
            "Type ID Intention lock present but cell_deps is empty".into(),
        ));
    }

    let live_type_ids = collect_live_type_id_script_hashes(url, tx)?;
    for want in &want_hashes {
        if !live_type_ids.iter().flatten().any(|got| got == want) {
            return Err(CliError::NetworkGuard(
                "Intention lock Type ID does not match any live celldep TYPE_ID script hash; refusing send"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn metadata_hash(metadata: &Value, field: &str) -> Result<[u8; 32], CliError> {
    let value = metadata[field].as_str().unwrap_or_default();
    decode_hash32(value).map_err(|_| {
        CliError::NetworkGuard(format!(
            "Process create has missing or invalid {field} metadata"
        ))
    })
}

fn type_id_hash_from_args(args: [u8; 32]) -> Result<[u8; 32], CliError> {
    process_type_hash_of(&ScriptJson {
        code_hash: encode_hex(&TYPE_ID_CODE_HASH),
        hash_type: "type".into(),
        args: encode_hex(&args),
    })
}

fn validate_process_state_metadata(
    tx: &Value,
    tx_hash: &[u8],
    metadata: &Value,
) -> Result<[u8; 32], CliError> {
    let output = tx["outputs"]
        .get(PROCESS_STATE_OUTPUT_INDEX)
        .ok_or_else(|| CliError::NetworkGuard("missing Process state output".into()))?;
    let typ = &output["type"];
    let identity = if typ.is_null() {
        None
    } else {
        type_id_script_hash(typ)?
    };
    let identity = identity.ok_or_else(|| {
        CliError::NetworkGuard(
            "Process state output must have a built-in TYPE_ID type script".into(),
        )
    })?;
    let identity_args = decode_hash32(typ["args"].as_str().unwrap_or_default())?;
    let lock = &output["lock"];
    let args = decode_hex(lock["args"].as_str().unwrap_or_default())?;
    if lock["hash_type"].as_str() != Some("type")
        || args.is_empty()
        || cid_protocol::CidArgs::decode(&args).is_ok()
    {
        return Err(CliError::NetworkGuard("invalid Process state lock".into()));
    }
    let process_code = decode_hash32(lock["code_hash"].as_str().unwrap_or_default())?;
    let state_outpoint = &metadata["state_outpoint"];
    if state_outpoint["index"].as_u64() != Some(PROCESS_STATE_OUTPUT_INDEX as u64)
        || metadata_hash(state_outpoint, "tx_hash")? != tx_hash
    {
        return Err(CliError::NetworkGuard(
            "Process create state_outpoint metadata does not match the signed transaction".into(),
        ));
    }

    let registry = &metadata["registry_post"];
    let script = &metadata["process_type_script"];
    let script_identity = type_id_script_hash(script).ok().flatten();
    if metadata_hash(metadata, "identity_type_id")? != identity
        || metadata_hash(metadata, "identity_type_id_args")? != identity_args
        || script_identity != Some(identity)
        || metadata_hash(registry, "identity_type_id")? != identity
        || metadata_hash(registry, "identity_type_id_args")? != identity_args
        || metadata_hash(registry, "code_type_id")? != process_code
        || type_id_hash_from_args(metadata_hash(registry, "code_type_id_args")?)? != process_code
    {
        return Err(CliError::NetworkGuard(
            "Process create identity or registry metadata does not match the state output".into(),
        ));
    }
    Ok(process_code)
}

/// Only Process create artifacts request the two-role check. Both code roles
/// are tied to distinct outpoints in the signed cell_deps vector.
fn assert_live_process_and_cid_code_type_ids(
    url: &str,
    tx: &Value,
    parsed: ckb_jsonrpc_types::Transaction,
    metadata: &Value,
) -> Result<(), CliError> {
    for (role, index) in [
        ("CID code", CID_CODE_DEP_INDEX),
        ("Process code", PROCESS_CODE_DEP_INDEX),
    ] {
        if !parsed
            .cell_deps
            .get(index)
            .is_some_and(|dep| dep.dep_type == ckb_jsonrpc_types::DepType::Code)
        {
            return Err(CliError::NetworkGuard(format!(
                "Process create requires a direct {role} dep at cell_deps[{index}]"
            )));
        }
    }
    if parsed.cell_deps[CID_CODE_DEP_INDEX].out_point
        == parsed.cell_deps[PROCESS_CODE_DEP_INDEX].out_point
    {
        return Err(CliError::NetworkGuard(
            "CID and Process code deps must use distinct outpoints".into(),
        ));
    }

    let cid = metadata_hash(metadata, "cid_code_type_id")?;
    if type_id_hash_from_args(metadata_hash(metadata, "cid_code_type_id_args")?)? != cid {
        return Err(CliError::NetworkGuard(
            "Process create CID code Type ID metadata is inconsistent".into(),
        ));
    }
    let packed_tx: packed::Transaction = parsed.into();
    let tx_hash = packed_tx.into_view().hash();
    let process = validate_process_state_metadata(tx, tx_hash.as_slice(), metadata)?;
    if cid == process {
        return Err(CliError::NetworkGuard(
            "CID and Process code Type IDs must be distinct".into(),
        ));
    }
    let live = collect_live_type_id_script_hashes(url, tx)?;
    for (role, index, expected) in [
        ("CID code", CID_CODE_DEP_INDEX, cid),
        ("Process lock", PROCESS_CODE_DEP_INDEX, process),
    ] {
        if live[index] != Some(expected) {
            return Err(CliError::NetworkGuard(format!(
                "{role} Type ID does not match its live code celldep at index {index}; refusing send"
            )));
        }
    }
    Ok(())
}
/// Submit a signed tx JSON to a CKB JSON-RPC endpoint after testnet/genesis checks.
pub fn send_signed_transaction(opts: &SendOptions, signed: &Value) -> Result<String, CliError> {
    assert_testnet_guards(opts)?;
    let (tx, parsed, process_metadata) = extract_transaction(signed)?;

    let chain_info = rpc_call(&opts.rpc_url, "get_blockchain_info", json!([]))?;
    assert_allowed_testnet_chain(&chain_info)?;

    let genesis = rpc_call(&opts.rpc_url, "get_block_hash", json!(["0x0"]))?;
    let genesis_s = genesis
        .as_str()
        .ok_or_else(|| CliError::Rpc("get_block_hash returned non-string".into()))?;
    let got = decode_hash32(genesis_s)?;
    let want = decode_hash32(&opts.genesis_hash)?;
    if got != want {
        return Err(CliError::GenesisMismatch {
            node: encode_hex(&got),
            expected: encode_hex(&want),
        });
    }

    assert_live_cid_type_id_matches_intention(&opts.rpc_url, &tx)?;
    if let Some(metadata) = process_metadata {
        assert_live_process_and_cid_code_type_ids(&opts.rpc_url, &tx, parsed, metadata)?;
    }

    let result = rpc_call(
        &opts.rpc_url,
        "send_transaction",
        json!([tx, "passthrough"]),
    )?;
    let hash = result
        .as_str()
        .ok_or_else(|| CliError::Rpc("send_transaction returned non-string hash".into()))?
        .to_string();
    if decode_hash32(&hash).is_err() {
        return Err(CliError::Rpc(
            "send_transaction hash was not 32 bytes".into(),
        ));
    }
    Ok(hash)
}
