//! Process example integration tests with real SSRI invocation.

use std::path::Path;

use ckb_cinnabar_calculator::{
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::Script,
        prelude::{Builder, Entity, Pack},
    },
    simulation::{fake_outpoint, FakeRpcClient},
};
use cid_process_example_calculator::{process_digest_ok, IntentionCell, PROCESS_CONTRACT_NAME};

const CID_CONTRACT_NAME: &str = "ckb_common_intention_dispatcher";
const CONTRACT_BIN: &str = "../build/release";
#[allow(dead_code)]
const MAX_CYCLES: u64 = 50_000_000;

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

fn mock_intention(i: u32) -> IntentionCell {
    IntentionCell {
        out_point: fake_outpoint(),
        lock_args: {
            let mut args = vec![0u8; 32];
            args.extend_from_slice(format!("mock_payload_{i}").as_bytes());
            args
        },
        capacity: 100_000_000_000,
        data: vec![],
    }
}

fn assert_digest_counts(n: usize) {
    require_binaries();
    let intentions: Vec<IntentionCell> = (0..n as u32).map(mock_intention).collect();
    let rpc = FakeRpcClient::default();
    let (skeleton, _instructions) = process_digest_ok(&rpc, intentions, CONTRACT_BIN)
        .unwrap_or_else(|e| panic!("should assemble skeleton for {n} intention(s): {e}"));
    assert_eq!(
        skeleton.inputs.len(),
        n,
        "skeleton inputs should match intention count"
    );
    assert_eq!(
        skeleton.outputs.len(),
        n,
        "skeleton outputs should match intention count (beneficiaries)"
    );
    println!("digest n={n} with real SSRI: SUCCESS (inputs={}, outputs={})",
        skeleton.inputs.len(),
        skeleton.outputs.len()
    );
}

#[tokio::test]
async fn single_intention_digest_real_ssri() {
    assert_digest_counts(1);
}

#[tokio::test]
async fn multi_intention_digest_n2_real_ssri() {
    assert_digest_counts(2);
}

#[tokio::test]
async fn multi_intention_digest_n4_real_ssri() {
    assert_digest_counts(4);
}
