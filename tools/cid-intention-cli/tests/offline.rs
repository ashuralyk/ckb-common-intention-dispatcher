//! Offline Intention create tests (synthetic keys only; no RPC).

mod common;

use std::sync::{Mutex, OnceLock};

use cid_intention_cli::{
    create_signed_intention, create_signed_intention_bundle,
    create_signed_intention_with_same_lock_decoy, parse_privkey, process_type_hash_of, CliError,
    CreateRequest, ScriptJson, PRIVKEY_ENV,
};
use ckb_cinnabar_calculator::{
    re_exports::ckb_types::{core::ScriptHashType, packed::Script, prelude::*},
    simulation::TransactionSimulator,
};
use serde_json::json;

use common::{fixture_request, key};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn process_type_hash_is_full_script_hash_not_code_hash() {
    let process_type = ScriptJson {
        code_hash: format!("0x{}", hex::encode([0x11; 32])),
        hash_type: "data1".into(),
        args: "0xaabb".into(),
    };
    let h = process_type_hash_of(&process_type).unwrap();
    assert_ne!(h, [0x11; 32]);
    let packed = Script::new_builder()
        .code_hash([0x11; 32].pack())
        .hash_type(ScriptHashType::Data1)
        .args(hex::decode("aabb").unwrap().pack())
        .build();
    assert_eq!(&h, packed.calc_script_hash().as_slice());
}

#[test]
fn create_signed_tx_shape_fee_witness_and_args() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let pth = process_type_hash_of(&req.process_type_script).unwrap();
    let tx = create_signed_intention(&req, &secret).unwrap();
    let dumped = serde_json::to_string(&tx).unwrap();
    assert!(!dumped.contains(&hex::encode(secret.secret_bytes())));
    assert!(!dumped
        .to_ascii_lowercase()
        .contains(&format!("0x{}", hex::encode(secret.secret_bytes()))));

    let inner = tx
        .get("inner")
        .or_else(|| tx.get("transaction"))
        .unwrap_or(&tx);
    // ckb_jsonrpc_types::TransactionView serializes as { hash, transaction? } depending on version —
    // accept either wrapper or raw transaction object.
    let transaction = if inner.get("inputs").is_some() {
        inner
    } else {
        inner.get("transaction").unwrap_or(inner)
    };
    let inputs = transaction["inputs"].as_array().unwrap();
    let outputs = transaction["outputs"].as_array().unwrap();
    let witnesses = transaction["witnesses"].as_array().unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0]["previous_output"]["tx_hash"].as_str().unwrap(),
        req.funding.tx_hash
    );
    assert_eq!(
        u32::from_str_radix(
            inputs[0]["previous_output"]["index"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
            16
        )
        .unwrap(),
        req.funding.index
    );
    assert!(outputs.len() >= 2);
    assert!(!witnesses.is_empty());
    assert_ne!(witnesses[0], json!("0x"));

    let cap = u64::from_str_radix(
        transaction["outputs"][0]["capacity"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(cap, 200_000_000_000);

    let args = transaction["outputs"][0]["lock"]["args"].as_str().unwrap();
    let args_bytes = hex::decode(args.trim_start_matches("0x")).unwrap();
    assert_eq!(&args_bytes[..32], &pth);

    let lock = &transaction["outputs"][0]["lock"];
    assert_eq!(
        lock["hash_type"].as_str().unwrap(),
        "type",
        "CID Intention lock must use Type ID reference"
    );
    let type_script = req.cid_lock_dep.type_script.as_ref().unwrap();
    let expected_code = process_type_hash_of(type_script).unwrap();
    assert_eq!(
        lock["code_hash"].as_str().unwrap(),
        format!("0x{}", hex::encode(expected_code)),
        "code_hash must be the Type ID type-script hash"
    );

    assert_eq!(
        tx["process_type_hash"].as_str().unwrap(),
        format!("0x{}", hex::encode(pth))
    );
}

#[test]
fn cid_lock_resolves_to_type_id_script_hash() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let type_script = req.cid_lock_dep.type_script.as_ref().unwrap();
    let expected = process_type_hash_of(type_script).unwrap();
    let bundle = create_signed_intention_bundle(&req, &secret).unwrap();
    let lock = bundle.skeleton.outputs[0].lock_script();
    let code: [u8; 32] = lock.code_hash().unpack();
    let ht: u8 = lock.hash_type().into();
    assert_eq!(ht, 1, "hash_type must be type");
    assert_eq!(code, expected);
}

#[test]
fn missing_cid_lock_type_script_rejected() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.cid_lock_dep.type_script = None;
    let err = create_signed_intention(&req, &secret).unwrap_err();
    match err {
        CliError::InvalidRequest(msg) => {
            assert!(
                msg.contains("type_script") && msg.contains("required"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[test]
fn cid_lock_type_script_must_be_type_id() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    let mut bad = req.cid_lock_dep.type_script.clone().unwrap();
    bad.hash_type = "data1".into();
    req.cid_lock_dep.type_script = Some(bad);
    let err = create_signed_intention(&req, &secret).unwrap_err();
    assert!(matches!(err, CliError::InvalidRequest(_)));
}

#[test]
fn cid_lock_type_script_args_short_rejected() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    let mut bad = req.cid_lock_dep.type_script.clone().unwrap();
    bad.args = format!("0x{}", hex::encode([0xABu8; 16]));
    req.cid_lock_dep.type_script = Some(bad);
    let err = create_signed_intention(&req, &secret).unwrap_err();
    match err {
        CliError::InvalidRequest(msg) => {
            assert!(
                msg.contains("exactly 32 bytes") && msg.contains("got 16"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[test]
fn cid_lock_type_script_args_long_rejected() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    let mut bad = req.cid_lock_dep.type_script.clone().unwrap();
    bad.args = format!("0x{}", hex::encode([0xABu8; 33]));
    req.cid_lock_dep.type_script = Some(bad);
    let err = create_signed_intention(&req, &secret).unwrap_err();
    match err {
        CliError::InvalidRequest(msg) => {
            assert!(
                msg.contains("exactly 32 bytes") && msg.contains("got 33"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[test]
fn wrong_key_rejected() {
    let req = fixture_request(&key(0x42));
    let err = create_signed_intention(&req, &key(0x43)).unwrap_err();
    assert!(matches!(err, CliError::LockKeyMismatch));
}

#[test]
fn insufficient_funds_rejected() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.funding.capacity = req.intention_capacity + req.fee_shannons; // no room for change
    let err = create_signed_intention(&req, &secret).unwrap_err();
    assert!(matches!(err, CliError::InsufficientFunds));
}

#[test]
fn duplicate_outpoints_rejected() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.cid_lock_dep.tx_hash = req.funding.tx_hash.clone();
    req.cid_lock_dep.index = req.funding.index;
    let err = create_signed_intention(&req, &secret).unwrap_err();
    assert!(matches!(err, CliError::DuplicateOutpoint));
}

#[test]
fn malformed_cell_hex_rejected() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.funding.data = "0xzz".into();
    assert!(create_signed_intention(&req, &secret).is_err());
}

#[test]
fn missing_beneficiary_field_fails_parse() {
    let secret = key(0x42);
    let mut v = serde_json::to_value(fixture_request(&secret)).unwrap();
    v.as_object_mut().unwrap().remove("beneficiary_lock");
    assert!(serde_json::from_value::<CreateRequest>(v).is_err());
}

#[test]
fn cli_reads_env_and_writes_output_without_leaking_key() {
    let _guard = env_lock().lock().unwrap();
    let secret = key(0x55);
    let req = fixture_request(&secret);
    let dir = tempfile::tempdir().unwrap();
    let req_path = dir.path().join("req.json");
    let out_path = dir.path().join("tx.json");
    std::fs::write(&req_path, serde_json::to_vec_pretty(&req).unwrap()).unwrap();

    std::env::set_var(PRIVKEY_ENV, hex::encode(secret.secret_bytes()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_cid-intention-cli"))
        .args([
            "create",
            "--request",
            req_path.to_str().unwrap(),
            "--output",
            out_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    std::env::remove_var(PRIVKEY_ENV);

    assert!(
        status.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&status.stderr)
    );
    let stderr = String::from_utf8_lossy(&status.stderr);
    let stdout = String::from_utf8_lossy(&status.stdout);
    let key_hex = hex::encode(secret.secret_bytes());
    assert!(!stderr.contains(&key_hex));
    assert!(!stdout.contains(&key_hex));
    let out = std::fs::read_to_string(&out_path).unwrap();
    assert!(!out.contains(&key_hex));
    assert!(out.contains("process_type_hash"));
}

#[test]
fn parse_privkey_rejects_bad_lengths() {
    assert!(matches!(parse_privkey("0x11"), Err(CliError::PrivkeyEnv)));
}

#[test]
fn funding_outpoint_pinned_despite_alternate_same_lock_cell() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let decoy = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let bundle =
        create_signed_intention_with_same_lock_decoy(&req, &secret, decoy, 0, 900_000_000_000)
            .unwrap();
    assert_eq!(bundle.skeleton.inputs.len(), 1);
    let prev = bundle.skeleton.inputs[0].input.previous_output();
    let want_tx = format!("0x{}", hex::encode(prev.tx_hash().as_slice()));
    assert_eq!(want_tx, req.funding.tx_hash);
    let idx: u32 = prev.index().unpack();
    assert_eq!(idx, req.funding.index);
    assert_ne!(want_tx, decoy);

    let inner = bundle
        .tx_json
        .get("inner")
        .or_else(|| bundle.tx_json.get("transaction"))
        .unwrap_or(&bundle.tx_json);
    let transaction = if inner.get("inputs").is_some() {
        inner
    } else {
        inner.get("transaction").unwrap_or(inner)
    };
    assert_eq!(
        transaction["inputs"][0]["previous_output"]["tx_hash"]
            .as_str()
            .unwrap(),
        req.funding.tx_hash
    );
}

#[tokio::test]
async fn signed_intention_passes_sighash_vm_verification() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let bundle = create_signed_intention_bundle(&req, &secret).unwrap();
    assert!(
        !bundle.skeleton.witnesses.is_empty(),
        "witness must be present"
    );
    let ok = TransactionSimulator::default()
        .skeleton(bundle.skeleton.clone())
        .link_cell_to_header(bundle.rpc.get_outpoint_to_headers())
        .async_verify(&bundle.rpc, vec![], 50_000_000)
        .await
        .is_ok();
    assert!(ok, "signed create tx must pass secp sighash VM verify");

    // Tamper output → signature must fail (proves verify is meaningful).
    let mut bad = bundle.skeleton;
    bad.outputs[0].output = bad.outputs[0]
        .output
        .clone()
        .as_builder()
        .capacity(199_000_000_000u64)
        .build();
    let bad_ok = TransactionSimulator::default()
        .skeleton(bad)
        .link_cell_to_header(bundle.rpc.get_outpoint_to_headers())
        .async_verify(&bundle.rpc, vec![], 50_000_000)
        .await
        .is_ok();
    assert!(!bad_ok, "tampered tx must fail sighash VM verify");
}
