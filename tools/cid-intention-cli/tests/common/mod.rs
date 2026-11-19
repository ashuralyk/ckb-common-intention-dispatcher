//! Shared synthetic-key fixtures for CLI integration tests (no network).

use cid_intention_cli::{CreateProcessRequest, CreateRequest, ScriptJson};
use ckb_cinnabar_calculator::{
    re_exports::{
        ckb_hash::blake2b_256,
        ckb_types::{core::ScriptHashType, packed::Script, prelude::*},
        secp256k1::{PublicKey, Secp256k1, SecretKey},
    },
    TYPE_ID_CODE_HASH,
};
use serde_json::json;

pub fn key(byte: u8) -> SecretKey {
    SecretKey::from_slice(&[byte; 32]).unwrap()
}

pub fn secp_data_lock(secret: &SecretKey) -> Script {
    let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), secret).serialize();
    Script::new_builder()
        .code_hash(ckb_system_scripts::CODE_HASH_SECP256K1_BLAKE160_SIGHASH_ALL.pack())
        .hash_type(ScriptHashType::Data)
        .args(blake2b_256(pubkey)[..20].to_vec().pack())
        .build()
}

pub fn script_json(script: &Script) -> ScriptJson {
    ScriptJson {
        code_hash: format!("0x{}", hex::encode(script.code_hash().as_slice())),
        hash_type: match u8::from(script.hash_type()) {
            0 => "data".into(),
            1 => "type".into(),
            2 => "data1".into(),
            4 => "data2".into(),
            _ => panic!("ht"),
        },
        args: format!("0x{}", hex::encode(script.args().raw_data())),
    }
}

fn bundled(name: &str) -> Vec<u8> {
    ckb_system_scripts::BUNDLED_CELL
        .get(&format!("specs/cells/{name}"))
        .unwrap()
        .into_owned()
}

pub fn fixture_request(secret: &SecretKey) -> CreateRequest {
    let lock = secp_data_lock(secret);
    // Process Type ID = built-in TYPE_ID type script of the Process state cell
    // (args must be exactly 32 bytes — creation tools validate this).
    let mut process_args = [0u8; 32];
    process_args[0] = 0xaa;
    process_args[1] = 0xbb;
    let process_type = Script::new_builder()
        .code_hash(TYPE_ID_CODE_HASH.pack())
        .hash_type(ScriptHashType::Type)
        .args(process_args.to_vec().pack())
        .build();
    let beneficiary = Script::new_builder()
        .code_hash([0x22; 32].pack())
        .hash_type(ScriptHashType::Data1)
        .args([0xEEu8].pack())
        .build();
    let cid_bin = vec![0xC1u8; 64];
    let cid_type = Script::new_builder()
        .code_hash(TYPE_ID_CODE_HASH.pack())
        .hash_type(ScriptHashType::Type)
        .args([0xABu8; 32].to_vec().pack())
        .build();
    serde_json::from_value(json!({
        "network": "fake",
        "fee_shannons": 10000,
        "intention_capacity": 200_000_000_000u64,
        "funding": {
            "tx_hash": "0x1111111111111111111111111111111111111111111111111111111111111111",
            "index": 0,
            "capacity": 500_000_000_000u64,
            "lock": script_json(&lock),
            "data": "0x"
        },
        "cid_lock_dep": {
            "name": "ckb_common_intention_dispatcher",
            "tx_hash": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "index": 0,
            "data": format!("0x{}", hex::encode(&cid_bin)),
            "type_script": script_json(&cid_type)
        },
        "sighash_deps": [
            {
                "name": "secp256k1_blake160_sighash_all",
                "tx_hash": "0x3333333333333333333333333333333333333333333333333333333333333333",
                "index": 1,
                "data": format!("0x{}", hex::encode(bundled("secp256k1_blake160_sighash_all")))
            },
            {
                "name": "secp256k1_data",
                "tx_hash": "0x3333333333333333333333333333333333333333333333333333333333333333",
                "index": 3,
                "data": format!("0x{}", hex::encode(bundled("secp256k1_data")))
            }
        ],
        "process_type_script": script_json(&process_type),
        "beneficiary_lock": script_json(&beneficiary)
    }))
    .unwrap()
}

pub fn fixture_process_request(secret: &SecretKey) -> CreateProcessRequest {
    let lock = secp_data_lock(secret);
    let cid_bin = vec![0xC1u8; 64];
    let process_bin = vec![0xDEu8; 64];
    let cid_type = Script::new_builder()
        .code_hash(TYPE_ID_CODE_HASH.pack())
        .hash_type(ScriptHashType::Type)
        .args([0xABu8; 32].to_vec().pack())
        .build();
    let process_code_type = Script::new_builder()
        .code_hash(TYPE_ID_CODE_HASH.pack())
        .hash_type(ScriptHashType::Type)
        .args([0xCDu8; 32].to_vec().pack())
        .build();
    serde_json::from_value(json!({
        "network": "fake",
        "process_capacity": 300_000_000_000u64,
        "process_lock_args": "0x01",
        "funding": {
            "tx_hash": "0x1111111111111111111111111111111111111111111111111111111111111111",
            "index": 0,
            "capacity": 500_000_000_000u64,
            "lock": script_json(&lock),
            "data": "0x"
        },
        "cid_lock_dep": {
            "name": "ckb_common_intention_dispatcher",
            "tx_hash": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "index": 0,
            "data": format!("0x{}", hex::encode(&cid_bin)),
            "type_script": script_json(&cid_type)
        },
        "process_code_dep": {
            "name": "cid_process_example",
            "tx_hash": "0x4444444444444444444444444444444444444444444444444444444444444444",
            "index": 0,
            "data": format!("0x{}", hex::encode(&process_bin)),
            "type_script": script_json(&process_code_type)
        },
        "sighash_deps": [
            {
                "name": "secp256k1_blake160_sighash_all",
                "tx_hash": "0x3333333333333333333333333333333333333333333333333333333333333333",
                "index": 1,
                "data": format!("0x{}", hex::encode(bundled("secp256k1_blake160_sighash_all")))
            },
            {
                "name": "secp256k1_data",
                "tx_hash": "0x3333333333333333333333333333333333333333333333333333333333333333",
                "index": 3,
                "data": format!("0x{}", hex::encode(bundled("secp256k1_data")))
            }
        ]
    }))
    .unwrap()
}
