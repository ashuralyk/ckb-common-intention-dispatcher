//! Local HTTP JSON-RPC mock tests for `send` (no public network).

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use cid_intention_cli::{
    create_signed_intention, create_signed_process_state, send_signed_transaction, CliError,
    ScriptJson, SendOptions,
};
use ckb_cinnabar_calculator::re_exports::ckb_jsonrpc_types;
use serde_json::{json, Value};

use common::{fixture_process_request, fixture_request, key};

fn live_typed_cell(type_script: &ScriptJson) -> Value {
    json!({
        "status": "live",
        "cell": {
            "output": {
                "capacity": "0x1",
                "lock": {"code_hash": h256(0x00), "hash_type": "data", "args": "0x"},
                "type": {
                    "code_hash": type_script.code_hash,
                    "hash_type": type_script.hash_type,
                    "args": type_script.args
                }
            }
        }
    })
}

fn live_untyped_cell() -> Value {
    json!({
        "status": "live",
        "cell": {
            "output": {
                "capacity": "0x1",
                "lock": {"code_hash": h256(0x00), "hash_type": "data", "args": "0x"},
                "type": null
            }
        }
    })
}

type MockHandler = Box<dyn FnMut(&str, &Value) -> Value + Send>;

fn spawn_mock(handler: Arc<Mutex<MockHandler>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            // Read until headers end then Content-Length body.
            loop {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                    let headers = String::from_utf8_lossy(&buf[..header_end]);
                    let mut content_len = 0usize;
                    for line in headers.lines() {
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    while buf.len() < header_end + content_len {
                        let n = stream.read(&mut tmp).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let body = &buf[header_end..header_end + content_len];
                    let req: Value = serde_json::from_slice(body).unwrap_or(json!({}));
                    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
                    let params = req.get("params").cloned().unwrap_or(json!([]));
                    let result = {
                        let mut h = handler.lock().unwrap();
                        h(method, &params)
                    };
                    let id = req.get("id").cloned().unwrap_or(json!(1));
                    let resp = if result.get("error").is_some() {
                        json!({"jsonrpc":"2.0","id": id, "error": result["error"].clone()})
                    } else {
                        json!({"jsonrpc":"2.0","id": id, "result": result})
                    };
                    let body = serde_json::to_vec(&resp).unwrap();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(&body);
                    break;
                }
            }
        }
    });
    format!("http://{addr}")
}

fn h256(byte: u8) -> String {
    format!("0x{}", hex::encode([byte; 32]))
}

fn sample_tx() -> Value {
    json!({
        "version": "0x0",
        "cell_deps": [],
        "header_deps": [],
        "inputs": [{
            "since":"0x0",
            "previous_output":{"tx_hash": h256(0x11), "index":"0x0"}
        }],
        "outputs": [{
            "capacity":"0x1",
            "lock":{"code_hash": h256(0x00), "hash_type":"data", "args":"0x"},
            "type":null
        }],
        "outputs_data": ["0x"],
        "witnesses": ["0x"]
    })
}

/// Standard testnet-shaped secp256k1/blake160 lock (`hash_type=type`) + sighash
/// `dep_group` (group cell is not a TYPE_ID code cell). Must not trip the CID
/// live Type ID guard.
fn secp_type_dep_group_tx() -> Value {
    // Genesis secp type hash (RFC 0024); args are 20-byte blake160, not CidArgs.
    let secp_type_hash = "0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8";
    json!({
        "version": "0x0",
        "cell_deps": [{
            "out_point": {"tx_hash": h256(0x71), "index": "0x1"},
            "dep_type": "dep_group"
        }],
        "header_deps": [],
        "inputs": [{
            "since":"0x0",
            "previous_output":{"tx_hash": h256(0x11), "index":"0x0"}
        }],
        "outputs": [{
            "capacity":"0x1",
            "lock":{
                "code_hash": secp_type_hash,
                "hash_type":"type",
                "args": format!("0x{}", hex::encode([0xABu8; 20]))
            },
            "type":null
        }],
        "outputs_data": ["0x"],
        "witnesses": ["0x"]
    })
}

#[test]
fn send_success_returns_hash_after_guards() {
    let genesis = h256(0xAB);
    let tx_hash = h256(0xCD);
    let genesis_c = genesis.clone();
    let tx_hash_c = tx_hash.clone();
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => {
                assert_eq!(params, &json!(["0x0"]));
                json!(genesis_c.clone())
            }
            "send_transaction" => {
                let tx = &params.as_array().unwrap()[0];
                assert!(tx.get("inputs").is_some());
                assert!(tx.get("hash").is_none());
                assert!(tx.get("process_type_hash").is_none());
                serde_json::from_value::<ckb_jsonrpc_types::Transaction>(tx.clone())
                    .expect("payload must deserialize as Transaction");
                json!(tx_hash_c.clone())
            }
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));

    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let hash = send_signed_transaction(&opts, &sample_tx()).unwrap();
    assert_eq!(hash, tx_hash);
}

#[test]
fn create_output_sends_after_stripping_view_metadata() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let created = create_signed_intention(&req, &secret).unwrap();
    assert!(created.get("hash").is_some() || created.get("process_type_hash").is_some());
    // Create output must not parse as Transaction until metadata is stripped.
    assert!(serde_json::from_value::<ckb_jsonrpc_types::Transaction>(created.clone()).is_err());

    let genesis = h256(0xAB);
    let tx_hash = h256(0xCD);
    let genesis_c = genesis.clone();
    let tx_hash_c = tx_hash.clone();
    let cid_tx = req.cid_lock_dep.tx_hash.clone();
    let cid_type = req.cid_lock_dep.type_script.clone().unwrap();
    let saw_send = Arc::new(Mutex::new(false));
    let saw_send_c = Arc::clone(&saw_send);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&cid_tx) {
                    live_typed_cell(&cid_type)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => {
                let tx = &params.as_array().unwrap()[0];
                assert!(tx.get("hash").is_none(), "hash must be stripped");
                assert!(
                    tx.get("process_type_hash").is_none(),
                    "process_type_hash must be stripped"
                );
                serde_json::from_value::<ckb_jsonrpc_types::Transaction>(tx.clone())
                    .expect("create→send payload must be a Transaction");
                *saw_send_c.lock().unwrap() = true;
                json!(tx_hash_c.clone())
            }
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));

    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let hash = send_signed_transaction(&opts, &created).unwrap();
    assert_eq!(hash, tx_hash);
    assert!(*saw_send.lock().unwrap());
}

#[test]
fn send_allows_secp_type_lock_with_dep_group() {
    let genesis = h256(0xAB);
    let tx_hash = h256(0xCD);
    let genesis_c = genesis.clone();
    let tx_hash_c = tx_hash.clone();
    let saw_send = Arc::new(Mutex::new(false));
    let saw_send_c = Arc::clone(&saw_send);
    let saw_live = Arc::new(Mutex::new(false));
    let saw_live_c = Arc::clone(&saw_live);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                *saw_live_c.lock().unwrap() = true;
                // Group cell itself has no TYPE_ID matching the secp type hash.
                live_untyped_cell()
            }
            "send_transaction" => {
                *saw_send_c.lock().unwrap() = true;
                json!(tx_hash_c.clone())
            }
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));

    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let hash = send_signed_transaction(&opts, &secp_type_dep_group_tx()).unwrap();
    assert_eq!(hash, tx_hash);
    assert!(*saw_send.lock().unwrap(), "send_transaction must run");
    assert!(
        !*saw_live.lock().unwrap(),
        "CID Type ID live-cell probe must not run for non-CID type locks"
    );
}

#[test]
fn create_process_output_sends_after_stripping_metadata() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let created = create_signed_process_state(&req, &secret).unwrap();
    assert!(created.get("registry_post").is_some());
    assert!(created.get("identity_type_id").is_some());
    assert!(serde_json::from_value::<ckb_jsonrpc_types::Transaction>(created.clone()).is_err());

    let genesis = h256(0xAB);
    let tx_hash = h256(0xCD);
    let genesis_c = genesis.clone();
    let tx_hash_c = tx_hash.clone();
    let cid_tx = req.cid_lock_dep.tx_hash.clone();
    let cid_type = req.cid_lock_dep.type_script.clone().unwrap();
    let process_tx = req.process_code_dep.tx_hash.clone();
    let process_type = req.process_code_dep.type_script.clone().unwrap();
    let saw_send = Arc::new(Mutex::new(false));
    let saw_send_c = Arc::clone(&saw_send);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&cid_tx) {
                    live_typed_cell(&cid_type)
                } else if txh.eq_ignore_ascii_case(&process_tx) {
                    live_typed_cell(&process_type)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => {
                let tx = &params.as_array().unwrap()[0];
                assert!(tx.get("registry_post").is_none());
                assert!(tx.get("identity_type_id").is_none());
                assert!(tx.get("state_outpoint").is_none());
                assert!(tx.get("process_type_script").is_none());
                assert!(tx.get("cid_code_type_id").is_none());
                assert!(tx.get("cid_code_type_id_args").is_none());
                serde_json::from_value::<ckb_jsonrpc_types::Transaction>(tx.clone())
                    .expect("process create→send payload must be a Transaction");
                *saw_send_c.lock().unwrap() = true;
                json!(tx_hash_c.clone())
            }
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));

    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    for signed in [
        created.clone(),
        json!({"transaction": created.clone()}),
        json!({"inner": {"transaction": created.clone()}}),
    ] {
        let hash = send_signed_transaction(&opts, &signed).unwrap();
        assert_eq!(hash, tx_hash);
    }
    assert!(*saw_send.lock().unwrap());
    *saw_send.lock().unwrap() = false;

    for field in [
        "state_outpoint",
        "identity_type_id",
        "identity_type_id_args",
        "process_type_script",
        "registry_post",
        "cid_code_type_id",
        "cid_code_type_id_args",
    ] {
        let mut incomplete = created.clone();
        incomplete.as_object_mut().unwrap().remove(field);
        let err = send_signed_transaction(&opts, &incomplete).unwrap_err();
        assert!(matches!(err, CliError::NetworkGuard(_)), "{field}: {err}");
    }

    for fields in [
        ["state_outpoint", "tx_hash"],
        ["process_type_script", "args"],
        ["registry_post", "identity_type_id"],
        ["registry_post", "identity_type_id_args"],
        ["registry_post", "code_type_id"],
        ["registry_post", "code_type_id_args"],
    ] {
        let mut inconsistent = created.clone();
        inconsistent[fields[0]][fields[1]] = json!(h256(0xFE));
        let err = send_signed_transaction(&opts, &inconsistent).unwrap_err();
        assert!(
            matches!(err, CliError::NetworkGuard(_)),
            "{fields:?}: {err}"
        );
    }

    let mut duplicate = created.clone();
    duplicate["cell_deps"][1]["out_point"] = duplicate["cell_deps"][0]["out_point"].clone();
    let err = send_signed_transaction(&opts, &duplicate).unwrap_err();
    assert!(matches!(err, CliError::NetworkGuard(msg) if msg.contains("distinct outpoints")));
    assert!(!*saw_send.lock().unwrap());
}

#[test]
fn send_rejects_dead_celldep_fail_closed() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let created = create_signed_process_state(&req, &secret).unwrap();
    let genesis = h256(0xAB);
    let genesis_c = genesis.clone();
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => json!({
                "status": "unknown",
                "cell": null
            }),
            "send_transaction" => json!(h256(0xFF)),
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &created).unwrap_err();
    match err {
        CliError::NetworkGuard(msg) => assert!(msg.contains("not live"), "{msg}"),
        other => panic!("expected NetworkGuard, got {other:?}"),
    }
}

#[test]
fn send_rejects_process_create_without_live_cid_type_id_dep() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let created = create_signed_process_state(&req, &secret).unwrap();
    let genesis = h256(0xAB);
    let genesis_c = genesis.clone();
    let process_tx = req.process_code_dep.tx_hash.clone();
    let process_type = req.process_code_dep.type_script.clone().unwrap();
    // Only Process code is TYPE_ID-live; CID dep is live but untyped → no CID Type ID.
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&process_tx) {
                    live_typed_cell(&process_type)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => json!(h256(0xFF)),
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &created).unwrap_err();
    match err {
        CliError::NetworkGuard(msg) => assert!(
            msg.contains("CID code Type ID") || msg.contains("cid_code_type_id"),
            "{msg}"
        ),
        other => panic!("expected NetworkGuard, got {other:?}"),
    }
}

#[test]
fn send_rejects_process_create_with_mismatched_cid_code_type_id_metadata() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let mut created = create_signed_process_state(&req, &secret).unwrap();
    // Corrupt metadata so send demands a TYPE_ID that is not live on any celldep.
    created.as_object_mut().unwrap().insert(
        "cid_code_type_id".into(),
        json!(format!("0x{}", hex::encode([0xEEu8; 32]))),
    );
    let genesis = h256(0xAB);
    let genesis_c = genesis.clone();
    let cid_tx = req.cid_lock_dep.tx_hash.clone();
    let cid_type = req.cid_lock_dep.type_script.clone().unwrap();
    let process_tx = req.process_code_dep.tx_hash.clone();
    let process_type = req.process_code_dep.type_script.clone().unwrap();
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&cid_tx) {
                    live_typed_cell(&cid_type)
                } else if txh.eq_ignore_ascii_case(&process_tx) {
                    live_typed_cell(&process_type)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => json!(h256(0xFF)),
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &created).unwrap_err();
    match err {
        CliError::NetworkGuard(msg) => assert!(msg.contains("CID code Type ID"), "{msg}"),
        other => panic!("expected NetworkGuard, got {other:?}"),
    }
}

#[test]
fn send_rejects_process_with_mismatched_code_type_id() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let created = create_signed_process_state(&req, &secret).unwrap();
    let genesis = h256(0xAB);
    let genesis_c = genesis.clone();
    let cid_tx = req.cid_lock_dep.tx_hash.clone();
    let cid_type = req.cid_lock_dep.type_script.clone().unwrap();
    // Live Process code cell reports different Type ID args than the offline request.
    let mut wrong = req.process_code_dep.type_script.clone().unwrap();
    wrong.args = format!("0x{}", hex::encode([0xEEu8; 32]));
    let process_tx = req.process_code_dep.tx_hash.clone();
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&cid_tx) {
                    live_typed_cell(&cid_type)
                } else if txh.eq_ignore_ascii_case(&process_tx) {
                    live_typed_cell(&wrong)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => json!(h256(0xFF)),
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &created).unwrap_err();
    match err {
        CliError::NetworkGuard(msg) => assert!(msg.contains("Process lock Type ID"), "{msg}"),
        other => panic!("expected NetworkGuard, got {other:?}"),
    }
}

#[test]
fn send_rejects_live_outpoint_with_mismatched_type_id_args() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let created = create_signed_intention(&req, &secret).unwrap();

    let mut live_type = req.cid_lock_dep.type_script.clone().unwrap();
    // Same outpoint / TYPE_ID shape, different 32-byte args → different type_id.
    live_type.args = format!("0x{}", hex::encode([0xCDu8; 32]));
    assert_ne!(
        live_type.args,
        req.cid_lock_dep.type_script.as_ref().unwrap().args
    );

    let genesis = h256(0xAB);
    let genesis_c = genesis.clone();
    let cid_tx = req.cid_lock_dep.tx_hash.clone();
    let saw_send = Arc::new(Mutex::new(false));
    let saw_send_c = Arc::clone(&saw_send);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&cid_tx) {
                    live_typed_cell(&live_type)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => {
                *saw_send_c.lock().unwrap() = true;
                json!(h256(0xCD))
            }
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));

    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &created).unwrap_err();
    match err {
        CliError::NetworkGuard(msg) => {
            assert!(
                msg.contains("Type ID") && msg.contains("refusing send"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected NetworkGuard, got {other:?}"),
    }
    assert!(
        !*saw_send.lock().unwrap(),
        "send_transaction must not run when live Type ID mismatches"
    );
}

#[test]
fn send_rejects_mainnet_network_flag() {
    let opts = SendOptions {
        rpc_url: "http://127.0.0.1:1".into(),
        network: "mainnet".into(),
        genesis_hash: h256(0x01),
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    assert!(matches!(err, CliError::NetworkGuard(_)));
}

#[test]
fn send_rejects_mainnet_chain_from_node() {
    let genesis = h256(0x11);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb"}),
            "get_block_hash" => json!(h256(0x11)),
            _ => json!(null),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    assert!(matches!(err, CliError::NetworkGuard(_)));
}

#[test]
fn send_rejects_empty_or_missing_chain() {
    for chain_info in [json!({}), json!({"chain": ""}), json!({"chain": "   "})] {
        let saw_send = Arc::new(Mutex::new(false));
        let saw_send_c = Arc::clone(&saw_send);
        let chain_info_c = chain_info.clone();
        let url = spawn_mock(Arc::new(Mutex::new(Box::new(
            move |method, _| match method {
                "get_blockchain_info" => chain_info_c.clone(),
                "get_block_hash" => json!(h256(0x11)),
                "send_transaction" => {
                    *saw_send_c.lock().unwrap() = true;
                    json!(h256(0xCD))
                }
                _ => json!(null),
            },
        ))));
        let opts = SendOptions {
            rpc_url: url,
            network: "testnet".into(),
            genesis_hash: h256(0x11),
        };
        let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
        assert!(
            matches!(err, CliError::NetworkGuard(_)),
            "expected NetworkGuard for {chain_info}, got {err:?}"
        );
        assert!(
            !*saw_send.lock().unwrap(),
            "send_transaction must not run for {chain_info}"
        );
    }
}

#[test]
fn send_rejects_unknown_chain_identity() {
    let saw_send = Arc::new(Mutex::new(false));
    let saw_send_c = Arc::clone(&saw_send);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb_dev"}),
            "get_block_hash" => json!(h256(0x11)),
            "send_transaction" => {
                *saw_send_c.lock().unwrap() = true;
                json!(h256(0xCD))
            }
            _ => json!(null),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: h256(0x11),
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    assert!(matches!(err, CliError::NetworkGuard(_)));
    assert!(!*saw_send.lock().unwrap());
}

#[test]
fn send_rejects_genesis_mismatch() {
    let node = h256(0xAA);
    let expected = h256(0xBB);
    let node_c = node.clone();
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(node_c.clone()),
            _ => json!(null),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: expected.clone(),
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    assert!(matches!(err, CliError::GenesisMismatch { .. }));
    let msg = err.safe_message();
    assert!(msg.contains(&node), "safe_message missing node hash: {msg}");
    assert!(
        msg.contains(&expected),
        "safe_message missing expected hash: {msg}"
    );
    assert!(
        !msg.contains("details omitted"),
        "genesis mismatch must not be blanketed"
    );
}

#[test]
fn safe_message_keeps_genesis_hashes_but_omits_hex_flood_guards() {
    let node = h256(0x11);
    let expected = h256(0x22);
    let msg = CliError::GenesisMismatch {
        node: node.clone(),
        expected: expected.clone(),
    }
    .safe_message();
    assert!(msg.contains(&node));
    assert!(msg.contains(&expected));

    // Synthetic secret-shaped payload must not pass through NetworkGuard safely.
    let secretish = format!("0x{}", hex::encode([0x42u8; 32]));
    let flooded = CliError::NetworkGuard(format!("injected config dump {secretish}"));
    let omitted = flooded.safe_message();
    assert!(!omitted.contains(&secretish));
    assert!(omitted.contains("omitted"));
}

#[test]
fn send_surfaces_rpc_error_without_url_credentials() {
    let genesis = h256(0x01);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(h256(0x01)),
            "send_transaction" => {
                json!({"error": {"code": -3, "message": "TransactionFailedToResolve"}})
            }
            _ => json!(null),
        },
    ))));
    // Embed fake credentials; errors must redact them.
    let opts = SendOptions {
        rpc_url: url.replace("http://", "http://user:s3cret@"),
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    let msg = err.safe_message();
    assert!(!msg.contains("s3cret"));
    assert!(matches!(err, CliError::Rpc(_)));
}

#[test]
fn cli_send_genesis_mismatch_prints_hashes_not_credentials() {
    let node = h256(0xAA);
    let expected = h256(0xBB);
    let node_c = node.clone();
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(node_c.clone()),
            _ => json!(null),
        },
    ))));
    let credentialed = url.replace("http://", "http://user:s3cret@");
    let dir = tempfile::tempdir().unwrap();
    let tx_path = dir.path().join("tx.json");
    std::fs::write(&tx_path, serde_json::to_vec(&sample_tx()).unwrap()).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cid-intention-cli"))
        .args([
            "send",
            "--tx",
            tx_path.to_str().unwrap(),
            "--rpc-url",
            &credentialed,
            "--network",
            "testnet",
            "--genesis-hash",
            &expected,
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stderr.contains(&node),
        "stderr missing node genesis: {stderr}"
    );
    assert!(
        stderr.contains(&expected),
        "stderr missing expected genesis: {stderr}"
    );
    assert!(!stderr.contains("s3cret"), "credential leaked: {stderr}");
    assert!(!stdout.contains("s3cret"));
    assert!(
        !stderr.contains("details omitted"),
        "process path must not blank genesis diagnostics: {stderr}"
    );
}

#[test]
fn raw_type_id_asset_with_secp_dep_group_sends() {
    let genesis = h256(0xAB);
    let tx_hash = h256(0xCD);
    let genesis_c = genesis.clone();
    let tx_hash_c = tx_hash.clone();
    let saw_send = Arc::new(Mutex::new(false));
    let saw_send_c = Arc::clone(&saw_send);
    let saw_live = Arc::new(Mutex::new(false));
    let saw_live_c = Arc::clone(&saw_live);
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                *saw_live_c.lock().unwrap() = true;
                // Group cell itself has no TYPE_ID matching the secp type hash.
                live_untyped_cell()
            }
            "send_transaction" => {
                *saw_send_c.lock().unwrap() = true;
                json!(tx_hash_c.clone())
            }
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));

    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let mut tx = secp_type_dep_group_tx();
    tx["outputs"][0]["capacity"] = json!("0x4a817c800");
    use ckb_cinnabar_calculator::re_exports::{
        ckb_hash::new_blake2b,
        ckb_types::{packed::Transaction, prelude::*},
    };
    let packed: Transaction = serde_json::from_value::<ckb_jsonrpc_types::Transaction>(tx.clone())
        .unwrap()
        .into();
    let mut hasher = new_blake2b();
    hasher.update(packed.raw().inputs().get(0).unwrap().as_slice());
    hasher.update(&0u64.to_le_bytes());
    let mut args = [0u8; 32];
    hasher.finalize(&mut args);
    tx["outputs"][0]["type"] = json!({
        "code_hash": "0x00000000000000000000000000000000000000000000000000545950455f4944",
        "hash_type": "type",
        "args": format!("0x{}", hex::encode(args)),
    });
    let hash = send_signed_transaction(&opts, &tx).unwrap();
    assert_eq!(hash, tx_hash);
    assert!(*saw_send.lock().unwrap(), "send_transaction must run");
    assert!(
        !*saw_live.lock().unwrap(),
        "CID Type ID live-cell probe must not run for non-CID type locks"
    );
}

#[test]
fn process_create_rejects_cid_type_id_at_wrong_dep() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let created = create_signed_process_state(&req, &secret).unwrap();
    let genesis = h256(0xAB);
    let genesis_c = genesis.clone();
    let unrelated_dep = req.sighash_deps[0].clone();
    let cid_type = req.cid_lock_dep.type_script.clone().unwrap();
    let process_tx = req.process_code_dep.tx_hash.clone();
    let process_type = req.process_code_dep.type_script.clone().unwrap();
    // The designated CID dep is untyped. Only another dep carries its expected Type ID.
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, params| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(genesis_c.clone()),
            "get_live_cell" => {
                let op = &params.as_array().unwrap()[0];
                let txh = op["tx_hash"].as_str().unwrap_or("");
                if txh.eq_ignore_ascii_case(&process_tx) {
                    live_typed_cell(&process_type)
                } else if txh == unrelated_dep.tx_hash
                    && op["index"] == format!("0x{:x}", unrelated_dep.index)
                {
                    live_typed_cell(&cid_type)
                } else {
                    live_untyped_cell()
                }
            }
            "send_transaction" => json!(h256(0xFF)),
            _ => json!({"error": {"code": -1, "message": "unknown"}}),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: genesis,
    };
    let err = send_signed_transaction(&opts, &created).unwrap_err();
    match err {
        CliError::NetworkGuard(msg) => assert!(
            msg.contains("CID code Type ID") || msg.contains("cid_code_type_id"),
            "{msg}"
        ),
        other => panic!("expected NetworkGuard, got {other:?}"),
    }
}
