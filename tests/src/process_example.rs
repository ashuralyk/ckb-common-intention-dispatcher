//! Process example integration tests with real SSRI invocation and on-chain verify.

use std::path::Path;

use cid_process_example_calculator::{process_digest_ok, IntentionCell, PROCESS_CONTRACT_NAME};
use ckb_cinnabar_calculator::{
    instruction::Instruction,
    operation::basic::AddOutputCell,
    re_exports::ckb_hash::blake2b_256,
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::{OutPoint, Script},
        prelude::{Builder, Entity, Pack},
    },
    simulation::{
        always_success_script, fake_header_view, fake_outpoint, AddFakeAlwaysSuccessCelldep,
        AddFakeContractCelldepByName, AddFakeInputCell, FakeRpcClient, TransactionSimulator,
    },
    skeleton::{CellOutputEx, ScriptEx, TransactionSkeleton},
    CalculatorError, TransactionCalculator,
};
use ckb_common_intention_dispatcher_calculator::{digest_ok, encode_cid_args, Payload};

use crate::faker::{
    dest_lock, dest_script, outpoint_ids, script_to_cid, seed_intention, user_lock, user_lock_cid,
    CKB_CAPACITY, CONTRACT_BIN, CONTRACT_NAME, PACK_BLOCK,
};

const CID_CONTRACT_NAME: &str = "ckb_common_intention_dispatcher";
const MAX_CYCLES: u64 = 50_000_000;

const ERROR_BENEFICIARY_MISSING: i8 = 32;
const ERROR_BENEFICIARY_AMOUNT_MISMATCH: i8 = 33;
const ERROR_TYPE_ID_INVALID: i8 = 35;

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

fn process_example_binary() -> Vec<u8> {
    std::fs::read(format!("{CONTRACT_BIN}/{PROCESS_CONTRACT_NAME}")).unwrap()
}

fn process_example_type_script(args: Vec<u8>) -> Script {
    Script::new_builder()
        .code_hash(blake2b_256(process_example_binary()).pack())
        .hash_type(ScriptHashType::Data1)
        .args(args.pack())
        .build()
}

fn process_example_type_hash(script: &Script) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(script.calc_script_hash().as_slice());
    hash
}

fn process_example_type_ex(args: Vec<u8>) -> ScriptEx {
    ScriptEx::new_code(blake2b_256(process_example_binary()), args)
}

fn beneficiary_args(process_type_hash: [u8; 32], beneficiary: &Script) -> Vec<u8> {
    encode_cid_args(
        process_type_hash,
        user_lock_cid(),
        Payload::from_script(&script_to_cid(beneficiary)).expect("encode beneficiary script"),
    )
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

fn prepared_process_skeleton(rpc: &FakeRpcClient) -> TransactionSkeleton {
    let prepare = Instruction::<FakeRpcClient>::new(vec![
        Box::new(AddFakeAlwaysSuccessCelldep {}),
        Box::new(AddFakeContractCelldepByName {
            contract: CONTRACT_NAME.into(),
            type_id_args: None,
            contract_binary_path: CONTRACT_BIN.into(),
        }),
        Box::new(AddFakeContractCelldepByName {
            contract: PROCESS_CONTRACT_NAME.into(),
            type_id_args: None,
            contract_binary_path: CONTRACT_BIN.into(),
        }),
    ]);
    TransactionCalculator::default()
        .instruction(prepare)
        .new_skeleton(rpc)
        .expect("prepare process skeleton")
        .0
}

fn seed_process_example_cell(rpc: &mut FakeRpcClient, type_script: Script) -> OutPoint {
    let cell = CellOutputEx::new_from_scripts(
        dest_script(),
        Some(type_script),
        vec![],
        Some(CKB_CAPACITY),
    )
    .expect("process example cell");
    let out = fake_outpoint();
    rpc.insert_fake_cell(out.clone(), cell, Some(fake_header_view(2, 2, 2)));
    out
}

async fn verify(
    rpc: &FakeRpcClient,
    skeleton: TransactionSkeleton,
    instructions: Vec<Instruction<FakeRpcClient>>,
) -> Result<u64, CalculatorError> {
    TransactionSimulator::default()
        .skeleton(skeleton)
        .link_cell_to_header(rpc.get_outpoint_to_headers())
        .async_verify(rpc, instructions, MAX_CYCLES)
        .await
}

fn exit(err: CalculatorError) -> i8 {
    err.script_exit_code()
        .unwrap_or_else(|| panic!("expected script_exit_code, got {err:?}"))
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
    println!(
        "digest n={n} with real SSRI: SUCCESS (inputs={}, outputs={})",
        skeleton.inputs.len(),
        skeleton.outputs.len()
    );
}

fn apply_transfer(
    rpc: &mut FakeRpcClient,
    beneficiary: ScriptEx,
    capacity: u64,
    data: Vec<u8>,
) -> TransactionSkeleton {
    let mut skeleton = prepared_process_skeleton(rpc);
    let type_script = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_hash = process_example_type_hash(&type_script);
    let process = seed_process_example_cell(rpc, type_script);
    let args = beneficiary_args(process_hash, &dest_script());
    let intention = seed_intention(rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);

    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: beneficiary,
            type_script: None,
            data,
            capacity,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(rpc, &mut skeleton)
        .unwrap();
    skeleton
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

#[tokio::test]
async fn reject_empty_batch_real_ssri() {
    require_binaries();
    let rpc = FakeRpcClient::default();
    let err = process_digest_ok(&rpc, vec![], CONTRACT_BIN)
        .err()
        .expect("should reject empty batch");
    match err {
        ckb_cinnabar_calculator::CalculatorError::Other(msg) => {
            assert!(
                msg.contains("out of bounds") || msg.contains("Incompatible"),
                "error should mention bounds: {msg}"
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    println!("Empty batch rejected: SUCCESS");
}

#[tokio::test]
async fn reject_count_exceeds_max_real_ssri() {
    require_binaries();
    let intentions: Vec<IntentionCell> = (0..5).map(mock_intention).collect();
    let rpc = FakeRpcClient::default();
    let err = process_digest_ok(&rpc, intentions, CONTRACT_BIN)
        .err()
        .expect("should reject 5 intentions (> BATCH_MAX=4)");
    match err {
        ckb_cinnabar_calculator::CalculatorError::Other(msg) => {
            assert!(
                msg.contains("out of bounds") || msg.contains("Incompatible"),
                "error should mention bounds: {msg}"
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    println!("Count > BATCH_MAX rejected: SUCCESS");
}

#[tokio::test]
async fn create_process_accepts_type_id_init() {
    require_binaries();
    let rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let ix = Instruction::new(vec![
        Box::new(AddFakeInputCell {
            lock_script: always_success_script(vec![]).into(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
        }),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: Some(process_example_type_ex(vec![])),
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: true,
        }),
    ]);
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    verify(&rpc, skeleton, vec![])
        .await
        .expect("Create with calc_type_id args must pass");
}

#[tokio::test]
async fn create_process_rejects_wrong_type_id() {
    require_binaries();
    let rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let ix = Instruction::new(vec![
        Box::new(AddFakeInputCell {
            lock_script: always_success_script(vec![]).into(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
        }),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: Some(process_example_type_ex(vec![0xFF; 32])),
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("Create with non-calc_type_id args must fail");
    assert_eq!(exit(err), ERROR_TYPE_ID_INVALID);
}

#[tokio::test]
async fn reject_wrong_beneficiary() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let skeleton = apply_transfer(&mut rpc, user_lock(), CKB_CAPACITY, vec![]);
    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("wrong beneficiary lock must fail");
    assert_eq!(exit(err), ERROR_BENEFICIARY_MISSING);
}

#[tokio::test]
async fn reject_beneficiary_capacity_mismatch() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let skeleton = apply_transfer(&mut rpc, dest_lock(), CKB_CAPACITY - 1, vec![]);
    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("capacity mismatch must fail");
    assert_eq!(exit(err), ERROR_BENEFICIARY_AMOUNT_MISMATCH);
}

#[tokio::test]
async fn reject_beneficiary_data_mismatch() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let skeleton = apply_transfer(&mut rpc, dest_lock(), CKB_CAPACITY, vec![0xAA]);
    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("data mismatch must fail");
    assert_eq!(exit(err), ERROR_BENEFICIARY_AMOUNT_MISMATCH);
}

#[tokio::test]
async fn transfer_matching_beneficiary_succeeds() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let skeleton = apply_transfer(&mut rpc, dest_lock(), CKB_CAPACITY, vec![]);
    verify(&rpc, skeleton, vec![])
        .await
        .expect("matching beneficiary Script/capacity/data must pass");
}

// Still ignored: Process type script never runs if the Process cell is absent,
// so a "missing type-id pair" tx is a CID-lock PROCESS_MISMATCH, not ProcessPairInvalid.
#[tokio::test]
#[ignore]
async fn reject_missing_type_id_pair() {
    todo!("Process type does not execute when the Process cell is missing from the tx");
}
