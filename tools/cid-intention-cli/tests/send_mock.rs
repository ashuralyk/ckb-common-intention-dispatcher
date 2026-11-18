//! Local HTTP JSON-RPC mock tests for `send` (no public network).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use cid_intention_cli::{send_signed_transaction, CliError, SendOptions};
use serde_json::{json, Value};

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

fn sample_tx() -> Value {
    json!({
        "version": "0x0",
        "cell_deps": [],
        "header_deps": [],
        "inputs": [{"since":"0x0","previous_output":{"tx_hash":"0x11","index":"0x0"}}],
        "outputs": [{"capacity":"0x1","lock":{"code_hash":"0x00","hash_type":"data","args":"0x"},"type":null}],
        "outputs_data": ["0x"],
        "witnesses": ["0x"]
    })
}

#[test]
fn send_success_returns_hash_after_guards() {
    let genesis = format!("0x{}", hex::encode([0xAB; 32]));
    let tx_hash = format!("0x{}", hex::encode([0xCD; 32]));
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
                assert!(params.as_array().unwrap()[0].get("inputs").is_some());
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
fn send_rejects_mainnet_network_flag() {
    let opts = SendOptions {
        rpc_url: "http://127.0.0.1:1".into(),
        network: "mainnet".into(),
        genesis_hash: format!("0x{}", hex::encode([1u8; 32])),
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    assert!(matches!(err, CliError::NetworkGuard(_)));
}

#[test]
fn send_rejects_mainnet_chain_from_node() {
    let genesis = format!("0x{}", hex::encode([0x11; 32]));
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb"}),
            "get_block_hash" => json!(format!("0x{}", hex::encode([0x11; 32]))),
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
fn send_rejects_genesis_mismatch() {
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(format!("0x{}", hex::encode([0xAA; 32]))),
            _ => json!(null),
        },
    ))));
    let opts = SendOptions {
        rpc_url: url,
        network: "testnet".into(),
        genesis_hash: format!("0x{}", hex::encode([0xBB; 32])),
    };
    let err = send_signed_transaction(&opts, &sample_tx()).unwrap_err();
    assert!(matches!(err, CliError::NetworkGuard(_)));
}

#[test]
fn send_surfaces_rpc_error_without_url_credentials() {
    let genesis = format!("0x{}", hex::encode([0x01; 32]));
    let url = spawn_mock(Arc::new(Mutex::new(Box::new(
        move |method, _| match method {
            "get_blockchain_info" => json!({"chain": "ckb_testnet"}),
            "get_block_hash" => json!(format!("0x{}", hex::encode([0x01; 32]))),
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
