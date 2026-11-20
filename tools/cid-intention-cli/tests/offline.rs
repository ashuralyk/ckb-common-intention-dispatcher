//! Offline Intention create tests (synthetic keys only; no RPC).

mod common;

use std::sync::{Mutex, OnceLock};

use cid_intention_cli::{
    assert_fee_covers_rate, create_signed_intention, create_signed_intention_bundle,
    create_signed_intention_with_same_lock_decoy, create_signed_process_state,
    create_signed_process_state_bundle, fee_at_rate, parse_privkey, process_type_hash_of,
    serialized_size_in_block, CliError, CreateProcessRequest, CreateRequest, ScriptJson,
    MIN_FEE_RATE, PRIVKEY_ENV,
};
use ckb_cinnabar_calculator::{
    re_exports::ckb_types::{core::ScriptHashType, packed::Script, prelude::*},
    simulation::TransactionSimulator,
    TYPE_ID_CODE_HASH,
};
use serde_json::json;

use common::{fixture_process_request, fixture_request, key};

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
    assert_eq!(
        cap,
        cid_process_example_calculator::MIN_DEPOSIT_CAPACITY,
        "CLI default Intention capacity is the example Process deposit min"
    );

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
fn create_intention_accepts_tight_funding_intention_plus_fee_plus_change_dust() {
    // Regression: early check must not require FEE_HEADROOM=1_000_000 when
    // fee_shannons=10000 already covers rate-1000 min. Documented budget
    // intention + fee + 61 CKB change must succeed.
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    assert_eq!(req.fee_shannons, 10_000);
    const MIN_CHANGE: u64 = 61_0000_0000;
    req.funding.capacity = req
        .intention_capacity
        .checked_add(req.fee_shannons)
        .unwrap()
        .checked_add(MIN_CHANGE)
        .unwrap();
    let bundle = create_signed_intention_bundle(&req, &secret).unwrap();
    let (paid, need) = assert_fee_covers_rate(&bundle.skeleton, MIN_FEE_RATE).unwrap();
    assert!(paid >= need);
    assert_eq!(paid, req.fee_shannons.max(need));
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

#[test]
fn create_process_fee_covers_rate_1000_serialized_size_in_block() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let bundle = create_signed_process_state_bundle(&req, &secret).unwrap();
    let (paid, need) = assert_fee_covers_rate(&bundle.skeleton, MIN_FEE_RATE).unwrap();
    let size = serialized_size_in_block(&bundle.skeleton);
    assert_eq!(need, fee_at_rate(size, MIN_FEE_RATE));
    assert!(
        paid >= 641 || paid >= need,
        "regression: underpay vs rate-1000 size (paid={paid} need={need} size={size})"
    );
    assert!(paid >= need);
    assert_eq!(bundle.tx_json["fee_shannons"].as_u64().unwrap(), paid);
}

#[test]
fn create_intention_fee_covers_rate_1000_serialized_size_in_block() {
    let secret = key(0x42);
    let req = fixture_request(&secret);
    let bundle = create_signed_intention_bundle(&req, &secret).unwrap();
    let (paid, need) = assert_fee_covers_rate(&bundle.skeleton, MIN_FEE_RATE).unwrap();
    let size = serialized_size_in_block(&bundle.skeleton);
    assert_eq!(need, fee_at_rate(size, MIN_FEE_RATE));
    assert!(paid >= need, "paid={paid} need={need} size={size}");
    // Requested fee_shannons=10000 should win over min when larger.
    assert_eq!(paid, req.fee_shannons.max(need));
}

#[test]
fn create_rejects_intention_below_example_deposit_minimum() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    assert!(req.enforce_example_process_deposit_minimum);
    let minimum = cid_process_example_calculator::MIN_DEPOSIT_CAPACITY;

    for capacity in [minimum / 2, minimum - 1] {
        req.intention_capacity = capacity;
        let err = create_signed_intention(&req, &secret).unwrap_err();
        assert!(
            matches!(err, CliError::InvalidRequest(ref message)
                if message.contains("intention_capacity") && message.contains(&minimum.to_string())),
            "unexpected error for requested capacity {capacity}: {err}"
        );
    }
}

#[test]
fn create_allows_sub_example_min_when_example_deposit_policy_disabled() {
    // Non-example Process: no protocol floor — only occupied capacity applies.
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.enforce_example_process_deposit_minimum = false;
    let minimum = cid_process_example_calculator::MIN_DEPOSIT_CAPACITY;
    // Below example 600 CKB, but well above typical occupied (~61 CKB + args).
    req.intention_capacity = minimum / 2; // 300 CKB
    assert!(req.intention_capacity < minimum);
    let bundle = create_signed_intention_bundle(&req, &secret).expect(
        "non-example Process must accept deposit below example 600 CKB when above occupied",
    );
    let effective = bundle.skeleton.outputs[0].capacity_shannons();
    assert!(effective >= req.intention_capacity);
    assert!(effective < minimum);
}

#[test]
fn create_checks_funding_against_occupied_capacity() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.beneficiary_lock.args = format!("0x{}", "ab".repeat(512));
    let bundle = create_signed_intention_bundle(&req, &secret).unwrap();
    let output = &bundle.skeleton.outputs[0];
    assert_eq!(
        output.capacity_shannons(),
        output.occupied_capacity().as_u64() + 1
    );
    assert!(output.capacity_shannons() > req.intention_capacity);

    // Enough for the requested capacity, but not the same output at its
    // occupied capacity.
    req.funding.capacity = req.intention_capacity + req.fee_shannons + 61_0000_0000;
    let err = create_signed_intention(&req, &secret).unwrap_err();
    assert!(matches!(err, CliError::InsufficientFunds), "{err}");
}

#[test]
fn create_intention_bumps_sub_min_fee_shannons_to_rate_1000() {
    let secret = key(0x42);
    let mut req = fixture_request(&secret);
    req.fee_shannons = 1; // far below rate-1000 min for a create tx
                          // Early check only asks for fee_shannons; need_exact enforces real min fee.
                          // Pad so post-placeholder effective_fee still fits with change dust.
    req.funding.capacity = req.intention_capacity + 1_000_000 + 61_0000_0000 + 10_0000_0000;
    let bundle = create_signed_intention_bundle(&req, &secret).unwrap();
    let (paid, need) = assert_fee_covers_rate(&bundle.skeleton, MIN_FEE_RATE).unwrap();
    assert!(need > 1, "min fee must exceed the sub-min request");
    assert_eq!(
        paid, need,
        "sub-min fee_shannons must bump to rate-1000 min"
    );
    assert!(paid > req.fee_shannons);
}

#[test]
fn create_process_state_shape_type_id_registry_and_no_key_leak() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let tx = create_signed_process_state(&req, &secret).unwrap();
    let dumped = serde_json::to_string(&tx).unwrap();
    assert!(!dumped.contains(&hex::encode(secret.secret_bytes())));

    let process_code_type = req.process_code_dep.type_script.as_ref().unwrap();
    let expected_lock_code = process_type_hash_of(process_code_type).unwrap();

    let inner = tx
        .get("inner")
        .or_else(|| tx.get("transaction"))
        .unwrap_or(&tx);
    let transaction = if inner.get("inputs").is_some() {
        inner
    } else {
        inner.get("transaction").unwrap_or(inner)
    };
    let outputs = transaction["outputs"].as_array().unwrap();
    assert!(outputs.len() >= 2);

    let lock = &outputs[0]["lock"];
    assert_eq!(lock["hash_type"].as_str().unwrap(), "type");
    assert_eq!(
        lock["code_hash"].as_str().unwrap(),
        format!("0x{}", hex::encode(expected_lock_code))
    );
    let expected_args = tx["process_lock_args"].as_str().unwrap();
    assert_eq!(lock["args"].as_str().unwrap(), expected_args);
    let cid_type = req.cid_lock_dep.type_script.as_ref().unwrap();
    let expected_cid = process_type_hash_of(cid_type).unwrap();
    let mut want = expected_cid.to_vec();
    want.push(1);
    want.push(0x01);
    assert_eq!(expected_args, format!("0x{}", hex::encode(want)));

    let typ = &outputs[0]["type"];
    assert_eq!(
        typ["code_hash"].as_str().unwrap(),
        format!("0x{}", hex::encode(TYPE_ID_CODE_HASH))
    );
    assert_eq!(typ["hash_type"].as_str().unwrap(), "type");
    let typ_args = hex::decode(typ["args"].as_str().unwrap().trim_start_matches("0x")).unwrap();
    assert_eq!(typ_args.len(), 32);

    let identity = tx["identity_type_id"].as_str().unwrap();
    assert_eq!(identity.len(), 66);
    let registry = &tx["registry_post"];
    assert_eq!(
        registry["identity_type_id_args"].as_str().unwrap(),
        typ["args"].as_str().unwrap()
    );
    assert_eq!(registry["identity_type_id"].as_str().unwrap(), identity);
    assert_eq!(
        registry["code_type_id_args"].as_str().unwrap(),
        process_code_type.args
    );
    assert_eq!(
        registry["code_type_id"].as_str().unwrap(),
        format!("0x{}", hex::encode(expected_lock_code))
    );
    assert!(tx["state_outpoint"]["tx_hash"].as_str().is_some());
    assert_eq!(tx["state_outpoint"]["index"].as_u64().unwrap(), 0);
    assert_eq!(tx["fee_rate"].as_u64().unwrap(), 1000);
    assert_eq!(
        tx["cid_code_type_id"].as_str().unwrap(),
        format!("0x{}", hex::encode(expected_cid))
    );
    assert_eq!(tx["cid_code_type_id_args"].as_str().unwrap(), cid_type.args);

    // Intention CLI can target this identity via process_type_script.
    let pts = &tx["process_type_script"];
    assert_eq!(
        pts["code_hash"].as_str().unwrap(),
        typ["code_hash"].as_str().unwrap()
    );
    assert_eq!(pts["args"].as_str().unwrap(), typ["args"].as_str().unwrap());
    let pth = process_type_hash_of(&serde_json::from_value(pts.clone()).unwrap()).unwrap();
    assert_eq!(format!("0x{}", hex::encode(pth)), identity);
}

#[test]
fn create_process_rejects_empty_lock_args() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    req.process_lock_salt = None;
    req.process_lock_args = Some("0x".into());
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    match err {
        CliError::InvalidRequest(msg) => assert!(
            msg.contains("process_lock_args invalid") || msg.contains("TooShort"),
            "{msg}"
        ),
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
}

#[test]
fn create_process_rejects_unsupported_cid_hash_type_in_lock_args() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    req.process_lock_salt = None;
    let cid = process_type_hash_of(req.cid_lock_dep.type_script.as_ref().unwrap()).unwrap();
    let mut b = cid.to_vec();
    b.push(3); // unsupported ScriptHashType
    b.push(0x01);
    req.process_lock_args = Some(format!("0x{}", hex::encode(b)));
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    assert!(
        matches!(err, CliError::InvalidRequest(ref msg) if msg.contains("process_lock_args invalid")),
        "{err}"
    );
}

fn process_create_output_lock_args(tx: &serde_json::Value) -> &str {
    let inner = tx
        .get("inner")
        .or_else(|| tx.get("transaction"))
        .unwrap_or(tx);
    let transaction = if inner.get("outputs").is_some() {
        inner
    } else {
        inner.get("transaction").unwrap_or(inner)
    };
    transaction["outputs"][0]["lock"]["args"].as_str().unwrap()
}

#[test]
fn create_process_autofills_lock_args_without_salt() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    req.process_lock_args = None;
    req.process_lock_salt = None;
    let tx = create_signed_process_state(&req, &secret).unwrap();
    let cid = process_type_hash_of(req.cid_lock_dep.type_script.as_ref().unwrap()).unwrap();
    let mut want = cid.to_vec();
    want.push(1);
    let expected = format!("0x{}", hex::encode(&want));
    assert_eq!(tx["process_lock_args"].as_str().unwrap(), expected);
    assert_eq!(process_create_output_lock_args(&tx), expected);
}

#[test]
fn create_process_autofills_lock_args_with_salt() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    req.process_lock_args = None;
    req.process_lock_salt = Some("0x0102".into());
    let tx = create_signed_process_state(&req, &secret).unwrap();
    let cid = process_type_hash_of(req.cid_lock_dep.type_script.as_ref().unwrap()).unwrap();
    let mut want = cid.to_vec();
    want.push(1);
    want.extend_from_slice(&[0x01, 0x02]);
    let expected = format!("0x{}", hex::encode(&want));
    assert_eq!(tx["process_lock_args"].as_str().unwrap(), expected);
    assert_eq!(process_create_output_lock_args(&tx), expected);
}

#[test]
fn create_process_accepts_explicit_lock_args_matching_cid() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    let cid = process_type_hash_of(req.cid_lock_dep.type_script.as_ref().unwrap()).unwrap();
    let mut args = cid.to_vec();
    args.push(1);
    args.push(0xAB);
    req.process_lock_salt = None;
    req.process_lock_args = Some(format!("0x{}", hex::encode(&args)));
    let tx = create_signed_process_state(&req, &secret).unwrap();
    assert_eq!(
        tx["process_lock_args"].as_str().unwrap(),
        format!("0x{}", hex::encode(&args))
    );
}

#[test]
fn create_process_rejects_explicit_lock_args_wrong_cid_code_hash() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    let mut args = vec![0xEEu8; 32];
    args.push(1);
    args.push(0x01);
    req.process_lock_salt = None;
    req.process_lock_args = Some(format!("0x{}", hex::encode(args)));
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    assert!(
        matches!(
            err,
            CliError::InvalidRequest(ref msg) if msg.contains("CID identity must match cid_lock_dep")
        ),
        "{err}"
    );
}

#[test]
fn create_process_rejects_explicit_lock_args_wrong_hash_type() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    let cid = process_type_hash_of(req.cid_lock_dep.type_script.as_ref().unwrap()).unwrap();
    let mut args = cid.to_vec();
    args.push(2); // Data1, not type
    args.push(0x01);
    req.process_lock_salt = None;
    req.process_lock_args = Some(format!("0x{}", hex::encode(args)));
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    assert!(
        matches!(
            err,
            CliError::InvalidRequest(ref msg) if msg.contains("CID identity must match cid_lock_dep")
        ),
        "{err}"
    );
}

#[test]
fn create_process_rejects_both_lock_args_and_salt() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    let cid = process_type_hash_of(req.cid_lock_dep.type_script.as_ref().unwrap()).unwrap();
    let mut args = cid.to_vec();
    args.push(1);
    req.process_lock_args = Some(format!("0x{}", hex::encode(args)));
    req.process_lock_salt = Some("0x01".into());
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    assert!(
        matches!(
            err,
            CliError::InvalidRequest(ref msg)
                if msg.contains("process_lock_args") && msg.contains("process_lock_salt")
        ),
        "{err}"
    );
}

#[test]
fn create_process_rejects_missing_process_code_type_script() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    req.process_code_dep.type_script = None;
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    assert!(matches!(err, CliError::InvalidRequest(_)));
}

#[test]
fn create_process_cli_writes_output_without_leaking_key() {
    let _guard = env_lock().lock().unwrap();
    let secret = key(0x66);
    let req = fixture_process_request(&secret);
    let dir = tempfile::tempdir().unwrap();
    let req_path = dir.path().join("req.json");
    let out_path = dir.path().join("tx.json");
    std::fs::write(&req_path, serde_json::to_vec_pretty(&req).unwrap()).unwrap();

    std::env::set_var(PRIVKEY_ENV, hex::encode(secret.secret_bytes()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_cid-intention-cli"))
        .args([
            "create-process",
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
    let out = std::fs::read_to_string(&out_path).unwrap();
    let key_hex = hex::encode(secret.secret_bytes());
    assert!(!out.contains(&key_hex));
    assert!(out.contains("registry_post"));
    assert!(out.contains("identity_type_id"));
    let _ = serde_json::from_str::<CreateProcessRequest>(&serde_json::to_string(&req).unwrap());
}

#[tokio::test]
async fn signed_process_passes_sighash_vm_verification() {
    let secret = key(0x42);
    let req = fixture_process_request(&secret);
    let bundle = create_signed_process_state_bundle(&req, &secret).unwrap();
    assert!(!bundle.skeleton.witnesses.is_empty());
    let ok = TransactionSimulator::default()
        .skeleton(bundle.skeleton.clone())
        .link_cell_to_header(bundle.rpc.get_outpoint_to_headers())
        .async_verify(&bundle.rpc, vec![], 50_000_000)
        .await
        .is_ok();
    assert!(
        ok,
        "signed process create tx must pass secp sighash VM verify"
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

#[test]
fn process_lock_args_cannot_use_intention_format() {
    let secret = key(0x42);
    let mut process = fixture_process_request(&secret);
    let intention = create_signed_intention(&fixture_request(&secret), &secret).unwrap();
    process.process_lock_salt = None;
    process.process_lock_args = Some(
        intention["outputs"][0]["lock"]["args"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    assert!(create_signed_process_state(&process, &secret).is_err());
}

#[test]
fn process_funding_accepts_measured_fee_and_minimum_change() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    let normal = create_signed_process_state(&req, &secret).unwrap();
    let fee = normal["fee_shannons"].as_u64().unwrap();
    req.funding.capacity = req.process_capacity + 61_0000_0000 + fee;
    let exact = create_signed_process_state_bundle(&req, &secret)
        .expect("measured fee and 61 CKB change fit");
    assert_eq!(exact.tx_json["fee_shannons"], fee);
    assert_eq!(exact.skeleton.outputs[1].capacity_shannons(), 61_0000_0000);
}

#[test]
fn process_requires_capacity_above_occupied_floor_for_guest_fee() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    let template = create_signed_process_state_bundle(&req, &secret).unwrap();
    req.process_capacity = template.skeleton.outputs[0].occupied_capacity().as_u64();
    assert!(
        create_signed_process_state(&req, &secret).is_err(),
        "process minted at its occupied floor: {} shannons",
        req.process_capacity
    );
}

#[test]
fn create_process_rejects_duplicate_code_type_ids() {
    let secret = key(0x42);
    let mut req = fixture_process_request(&secret);
    req.cid_lock_dep.type_script = req.process_code_dep.type_script.clone();
    let err = create_signed_process_state(&req, &secret).unwrap_err();
    assert!(matches!(err, CliError::InvalidRequest(msg) if msg.contains("distinct Type IDs")));
}
