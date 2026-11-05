//! Process example integration tests with real SSRI invocation.
//!
//! Task 8: helpers and constants only. Digest cases land in Task 9+.

use std::path::Path;

use ckb_cinnabar_calculator::{
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::Script,
        prelude::{Builder, Pack},
    },
};
use cid_process_example_calculator::PROCESS_CONTRACT_NAME;

const CID_CONTRACT_NAME: &str = "ckb_common_intention_dispatcher";
const CONTRACT_BIN: &str = "../build/release";
#[allow(dead_code)]
const MAX_CYCLES: u64 = 50_000_000;

#[allow(dead_code)]
fn require_binaries() {
    let cid_path = format!("{CONTRACT_BIN}/{CID_CONTRACT_NAME}");
    let process_path = format!("{CONTRACT_BIN}/{PROCESS_CONTRACT_NAME}");

    assert!(
        Path::new(&cid_path).exists(),
        "CID binary missing; run 'make build' in contracts/ckb_common_intention_dispatcher ({cid_path})"
    );
    assert!(
        Path::new(&process_path).exists(),
        "Process binary missing; run 'make build' in contracts/cid_process_example ({process_path})"
    );
}

#[allow(dead_code)]
fn process_type_script() -> Script {
    Script::new_builder()
        .code_hash(
            ckb_cinnabar_calculator::re_exports::ckb_hash::blake2b_256(
                std::fs::read(format!("{CONTRACT_BIN}/{PROCESS_CONTRACT_NAME}")).unwrap(),
            )
            .pack(),
        )
        .hash_type(ScriptHashType::Data1)
        .args(b"process-type-id-v0".to_vec().pack())
        .build()
}
