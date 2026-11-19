//! Explicit testnet broadcast of a previously signed tx JSON.
//!
//! Never used by `create`. Development/CI must point `--rpc-url` at a local mock,
//! not a public endpoint.

use ckb_cinnabar_calculator::{re_exports::ckb_jsonrpc_types, TYPE_ID_CODE_HASH};
use serde_json::{json, Value};

use crate::build::{process_type_hash_of, CliError};
use crate::hexutil::{decode_hash32, decode_hex, encode_hex};
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

fn extract_transaction(value: &Value) -> Result<Value, CliError> {
    // Accept:
    // - TransactionView-shaped object (flattened fields + hash) from `create`
    // - raw Transaction object with inputs/outputs
    // - wrappers: { transaction: ... } / { inner: ... }
    // Always strip create metadata before RPC submit (`Transaction` is deny_unknown_fields).
    let candidate = if value.get("inputs").is_some() && value.get("outputs").is_some() {
        value.clone()
    } else if let Some(tx) = value.get("transaction") {
        if tx.get("inputs").is_some() {
            tx.clone()
        } else {
            return Err(CliError::InvalidRequest(
                "signed JSON transaction wrapper missing inputs".into(),
            ));
        }
    } else if let Some(inner) = value.get("inner") {
        return extract_transaction(inner);
    } else {
        return Err(CliError::InvalidRequest(
            "signed JSON does not contain a transaction body".into(),
        ));
    };

    let cleaned = strip_tx_metadata(candidate);
    // Round-trip through the RPC Transaction type so unknown fields fail locally.
    let _: ckb_jsonrpc_types::Transaction =
        serde_json::from_value(cleaned.clone()).map_err(|e| {
            CliError::InvalidRequest(format!(
                "signed JSON is not a valid CKB Transaction after stripping metadata: {e}"
            ))
        })?;
    Ok(cleaned)
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

/// Probe every cell_dep: **fail closed** if any outpoint is not live. Collect
/// TYPE_ID script hashes from live typed code cells (non-TYPE_ID live cells are
/// ignored for the hash set — e.g. genesis secp data).
fn collect_live_type_id_script_hashes(url: &str, tx: &Value) -> Result<Vec<[u8; 32]>, CliError> {
    let cell_deps = tx
        .get("cell_deps")
        .and_then(|c| c.as_array())
        .ok_or_else(|| CliError::InvalidRequest("transaction missing cell_deps".into()))?;
    let mut live_type_ids = Vec::new();
    for dep in cell_deps {
        let out_point = dep
            .get("out_point")
            .ok_or_else(|| CliError::InvalidRequest("cell_dep missing out_point".into()))?;
        let live = rpc_call(url, "get_live_cell", json!([out_point, false]))?;
        let status = live.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if status != "live" {
            return Err(CliError::NetworkGuard(
                "cell_dep outpoint is not live; refusing send".into(),
            ));
        }
        let typ = live
            .get("cell")
            .and_then(|c| c.get("output"))
            .and_then(|o| o.get("type"));
        let Some(typ) = typ.filter(|t| !t.is_null()) else {
            continue;
        };
        let sj = script_json_from_rpc(typ)?;
        if sj.parse_hash_type()? != 1 {
            continue;
        }
        let code = decode_hash32(&sj.code_hash)?;
        if code != TYPE_ID_CODE_HASH {
            continue;
        }
        if decode_hex(&sj.args)?.len() != 32 {
            continue;
        }
        live_type_ids.push(process_type_hash_of(&sj)?);
    }
    Ok(live_type_ids)
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
        if !live_type_ids.iter().any(|got| got == want) {
            return Err(CliError::NetworkGuard(
                "Intention lock Type ID does not match any live celldep TYPE_ID script hash; refusing send"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// Process state outputs: lock is Process code Type ID (`hash_type=type`) and
/// type is built-in TYPE_ID. Collect those lock `code_hash` values.
fn process_state_lock_type_id_code_hashes(tx: &Value) -> Result<Vec<[u8; 32]>, CliError> {
    let outputs = tx
        .get("outputs")
        .and_then(|o| o.as_array())
        .ok_or_else(|| CliError::InvalidRequest("transaction missing outputs".into()))?;
    let mut out = Vec::new();
    for cell in outputs {
        let typ = cell.get("type").filter(|t| !t.is_null());
        let Some(typ) = typ else {
            continue;
        };
        let typ_ht = typ.get("hash_type").and_then(|h| h.as_str()).unwrap_or("");
        if !typ_ht.eq_ignore_ascii_case("type") {
            continue;
        }
        let typ_code = typ.get("code_hash").and_then(|c| c.as_str()).unwrap_or("");
        let typ_code = decode_hash32(typ_code)?;
        if typ_code != TYPE_ID_CODE_HASH {
            continue;
        }
        let typ_args = typ.get("args").and_then(|a| a.as_str()).unwrap_or("0x");
        if decode_hex(typ_args)?.len() != 32 {
            continue;
        }
        let lock = cell
            .get("lock")
            .ok_or_else(|| CliError::InvalidRequest("transaction output missing lock".into()))?;
        let ht = lock.get("hash_type").and_then(|h| h.as_str()).unwrap_or("");
        if !ht.eq_ignore_ascii_case("type") {
            continue;
        }
        // Skip CID Intention locks (CID args) — those are checked separately.
        let args_hex = lock.get("args").and_then(|a| a.as_str()).unwrap_or("0x");
        let args = decode_hex(args_hex)?;
        if cid_protocol::CidArgs::decode(&args).is_ok() {
            continue;
        }
        let code = lock
            .get("code_hash")
            .and_then(|c| c.as_str())
            .ok_or_else(|| CliError::InvalidRequest("Process lock missing code_hash".into()))?;
        out.push(decode_hash32(code)?);
    }
    Ok(out)
}

/// CID code Type ID recorded by `create-process` for send-time binding.
fn cid_code_type_id_from_metadata(signed: &Value) -> Result<Option<[u8; 32]>, CliError> {
    let Some(hex) = signed
        .get("cid_code_type_id")
        .and_then(|v| v.as_str())
        .or_else(|| {
            signed
                .get("inner")
                .and_then(|i| i.get("cid_code_type_id"))
                .and_then(|v| v.as_str())
        })
    else {
        return Ok(None);
    };
    Ok(Some(decode_hash32(hex)?))
}

/// Live-verify Process **and** the exact CID code Type ID for Process creates.
///
/// Intention lock checks are a no-op on Process create (no CID args outputs), so
/// this path requires `cid_code_type_id` metadata from create-process and a live
/// celldep whose TYPE_ID script hash equals that value.
fn assert_live_process_and_cid_code_type_ids(
    url: &str,
    tx: &Value,
    signed: &Value,
) -> Result<(), CliError> {
    let want_process = process_state_lock_type_id_code_hashes(tx)?;
    if want_process.is_empty() {
        return Ok(());
    }
    let cell_deps = tx.get("cell_deps").and_then(|c| c.as_array());
    if cell_deps.map(|c| c.is_empty()).unwrap_or(true) {
        return Err(CliError::NetworkGuard(
            "Process state Type ID lock present but cell_deps is empty".into(),
        ));
    }
    let want_cid = cid_code_type_id_from_metadata(signed)?.ok_or_else(|| {
        CliError::NetworkGuard(
            "Process create missing cid_code_type_id metadata; refusing send".into(),
        )
    })?;
    let live_type_ids = collect_live_type_id_script_hashes(url, tx)?;
    for want in &want_process {
        if !live_type_ids.iter().any(|got| got == want) {
            return Err(CliError::NetworkGuard(
                "Process lock Type ID does not match any live celldep TYPE_ID script hash; refusing send"
                    .into(),
            ));
        }
    }
    if !live_type_ids.iter().any(|got| got == &want_cid) {
        return Err(CliError::NetworkGuard(
            "Process create CID code Type ID does not match any live celldep TYPE_ID script hash; refusing send"
                .into(),
        ));
    }
    Ok(())
}

/// Submit a signed tx JSON to a CKB JSON-RPC endpoint after testnet/genesis checks.
pub fn send_signed_transaction(opts: &SendOptions, signed: &Value) -> Result<String, CliError> {
    assert_testnet_guards(opts)?;
    let tx = extract_transaction(signed)?;

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
    assert_live_process_and_cid_code_type_ids(&opts.rpc_url, &tx, signed)?;

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
