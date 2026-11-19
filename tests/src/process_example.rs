//! Process example integration tests with real SSRI invocation and on-chain verify.

use std::path::Path;

use cid_process_example_calculator::{
    process_digest_ok, process_digest_ok_with_process, IntentionCell, ProcessStateCell,
    PROCESS_CONTRACT_NAME,
};
use ckb_cinnabar_calculator::{
    address::{Address, AddressPayload},
    instruction::Instruction,
    operation::basic::AddOutputCell,
    re_exports::ckb_hash::blake2b_256,
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::{OutPoint, Script},
        prelude::{Builder, Entity, Pack, Unpack},
    },
    rpc::Network,
    simulation::{
        always_success_script, fake_header_view, fake_outpoint, AddFakeAlwaysSuccessCelldep,
        AddFakeContractCelldepByName, AddFakeInputCell, FakeRpcClient, TransactionSimulator,
    },
    skeleton::{CellOutputEx, ScriptEx, TransactionSkeleton},
    CalculatorError, TransactionCalculator, TYPE_ID_CODE_HASH,
};
use ckb_common_intention_dispatcher_calculator::{
    cid_lock, digest_incompatible, digest_ok, encode_cid_args, timeout_burn, CidScript, Payload,
    TIMEOUT_BLOCKS,
};

use crate::faker::{
    dest_lock, dest_script, outpoint_ids, script_to_cid, seed_fakenet_xudt_cell, seed_header,
    seed_intention, user_lock, user_lock_cid, xudt_type_script, CKB_CAPACITY, CONTRACT_BIN,
    CONTRACT_NAME, PACK_BLOCK, USER_CELL_CAPACITY, XUDT_AMOUNT,
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

/// Process example binary as the state-cell **lock**.
fn process_example_lock_script() -> Script {
    process_example_lock_script_with(ScriptHashType::Data1, Vec::new())
}

fn process_example_lock_script_with(hash_type: ScriptHashType, args: Vec<u8>) -> Script {
    Script::new_builder()
        .code_hash(blake2b_256(process_example_binary()).pack())
        .hash_type(hash_type)
        .args(args.pack())
        .build()
}

fn process_example_lock_ex() -> ScriptEx {
    ScriptEx::new_code(blake2b_256(process_example_binary()), vec![])
}

/// Built-in TYPE_ID type of the Process state cell (Process Type ID).
fn process_example_type_script(tag: Vec<u8>) -> Script {
    let mut args = [0u8; 32];
    let n = tag.len().min(32);
    args[..n].copy_from_slice(&tag[..n]);
    Script::new_builder()
        .code_hash(TYPE_ID_CODE_HASH.pack())
        .hash_type(ScriptHashType::Type)
        .args(args.to_vec().pack())
        .build()
}

fn process_example_type_hash(script: &Script) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(script.calc_script_hash().as_slice());
    hash
}

fn beneficiary_args(process_type_hash: [u8; 32], beneficiary: &Script) -> Vec<u8> {
    encode_cid_args(
        process_type_hash,
        user_lock_cid(),
        Payload::from_script(&script_to_cid(beneficiary)).expect("encode beneficiary script"),
    )
}

fn mock_process_type_id() -> Script {
    process_example_type_script(b"process-type-id-v0".to_vec())
}

fn mock_intention(i: u32, process_type_hash: [u8; 32]) -> IntentionCell {
    let _ = i;
    let lock_args = encode_cid_args(process_type_hash, CidScript::default(), Payload::default());
    IntentionCell {
        out_point: fake_outpoint(),
        lock_args,
        capacity: 100_000_000_000,
        data: vec![],
        type_script: None,
    }
}

fn mock_process_state() -> ProcessStateCell {
    ProcessStateCell {
        out_point: fake_outpoint(),
        lock: process_example_lock_script(),
        type_script: mock_process_type_id(),
        capacity: CKB_CAPACITY,
        data: vec![],
    }
}

fn dest_script_data2() -> Script {
    dest_script()
        .as_builder()
        .hash_type(ScriptHashType::Data2)
        .build()
}

fn prepared_process_skeleton(rpc: &FakeRpcClient) -> TransactionSkeleton {
    prepared_process_skeleton_with_cid_type_id(rpc, None)
}

/// When `cid_type_id_args` is set, the CID code celldep carries a Type ID type
/// script so `ScriptEx::Reference` / Intention locks resolve as `hash_type=type`.
fn prepared_process_skeleton_with_cid_type_id(
    rpc: &FakeRpcClient,
    cid_type_id_args: Option<[u8; 32]>,
) -> TransactionSkeleton {
    let prepare = Instruction::<FakeRpcClient>::new(vec![
        Box::new(AddFakeAlwaysSuccessCelldep {}),
        Box::new(AddFakeContractCelldepByName {
            contract: CONTRACT_NAME.into(),
            type_id_args: cid_type_id_args.map(Into::into),
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

fn seed_process_example_cell(
    rpc: &mut FakeRpcClient,
    lock_script: Script,
    type_script: Script,
) -> OutPoint {
    let cell =
        CellOutputEx::new_from_scripts(lock_script, Some(type_script), vec![], Some(CKB_CAPACITY))
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
    let process = mock_process_state();
    let pth = process_example_type_hash(&process.type_script);
    let intentions: Vec<IntentionCell> = (0..n as u32).map(|i| mock_intention(i, pth)).collect();
    let rpc = FakeRpcClient::default();
    let (skeleton, _instructions) =
        process_digest_ok_with_process(&rpc, intentions, Some(process), CONTRACT_BIN)
            .unwrap_or_else(|e| panic!("should assemble skeleton for {n} intention(s): {e}"));
    // Process cell + n intentions
    assert_eq!(
        skeleton.inputs.len(),
        n + 1,
        "skeleton inputs = process + intentions"
    );
    // Process recreate + n beneficiaries
    assert_eq!(
        skeleton.outputs.len(),
        n + 1,
        "skeleton outputs = process recreate + beneficiaries"
    );
    println!(
        "digest n={n} with real SSRI full-tx: SUCCESS (inputs={}, outputs={})",
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
    apply_transfer_to(rpc, beneficiary, &dest_script(), capacity, data)
}

fn apply_transfer_to(
    rpc: &mut FakeRpcClient,
    output_lock: ScriptEx,
    payload_beneficiary: &Script,
    capacity: u64,
    data: Vec<u8>,
) -> TransactionSkeleton {
    let mut skeleton = prepared_process_skeleton(rpc);
    let type_script = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_hash = process_example_type_hash(&type_script);
    let process = seed_process_example_cell(rpc, process_example_lock_script(), type_script);
    let args = beneficiary_args(process_hash, payload_beneficiary);
    let intention = seed_intention(rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);

    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: output_lock,
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

/// Intention locked via Type ID (`hash_type=type`, `code_hash` = Type ID script
/// hash) must still DigestOk through the CID lock + Process example.
#[tokio::test]
async fn type_id_referenced_intention_digest_ok() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let cid_type_id_args = [0x71u8; 32];
    let mut skeleton = prepared_process_skeleton_with_cid_type_id(&rpc, Some(cid_type_id_args));

    let resolved = cid_lock(vec![]).to_script(&skeleton).expect("cid lock");
    let ht: u8 = resolved.hash_type().into();
    assert_eq!(
        ht, 1,
        "CID Reference must resolve to hash_type=type when dep has Type ID"
    );
    let code: [u8; 32] = resolved.code_hash().unpack();
    let data_hash = blake2b_256(std::fs::read(format!("{CONTRACT_BIN}/{CONTRACT_NAME}")).unwrap());
    assert_ne!(
        code, data_hash,
        "Type ID code_hash must not be the binary data hash"
    );

    let type_script = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_hash = process_example_type_hash(&type_script);
    let process = seed_process_example_cell(&mut rpc, process_example_lock_script(), type_script);
    let args = beneficiary_args(process_hash, &dest_script());
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);

    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .expect("assemble type-id DigestOk");

    let in_lock = skeleton.inputs[0].output.lock_script();
    let in_ht: u8 = in_lock.hash_type().into();
    assert_eq!(in_ht, 1, "seeded Intention must use hash_type=type");
    let in_code: [u8; 32] = in_lock.code_hash().unpack();
    assert_eq!(in_code, code);

    verify(&rpc, skeleton, vec![])
        .await
        .expect("Type-ID-referenced Intention DigestOk must pass CID + Process VM");
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
    let process = mock_process_state();
    let pth = process_example_type_hash(&process.type_script);
    let intentions: Vec<IntentionCell> = (0..5).map(|i| mock_intention(i, pth)).collect();
    let rpc = FakeRpcClient::default();
    let err = process_digest_ok_with_process(&rpc, intentions, Some(process), CONTRACT_BIN)
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
            // Process as lock; built-in TYPE_ID as type (type_id: true fills args).
            lock_script: process_example_lock_ex(),
            type_script: None,
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
        .expect("Create with Process lock + TYPE_ID type must pass");
}

#[tokio::test]
async fn transfer_rejects_non_type_id_type() {
    // Lock scripts do not run on Create-only; TYPE_ID presence is enforced on Transfer.
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let bad_type = dest_script();
    let bad_hash = process_example_type_hash(&bad_type);
    let process = seed_process_example_cell(&mut rpc, process_example_lock_script(), bad_type);
    // Intention matches the non-TYPE_ID type hash; Process Transfer rejects it.
    let args = beneficiary_args(bad_hash, &dest_script());
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("Transfer without TYPE_ID type must fail");
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

#[tokio::test]
async fn reject_two_processes_sharing_one_beneficiary() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let type_b = process_example_type_script(b"process-type-id-v1".to_vec());
    // Distinct lock args → distinct Process lock groups (1→1 each).
    let lock_a = process_example_lock_script_with(ScriptHashType::Data1, vec![0xA1]);
    let lock_b = process_example_lock_script_with(ScriptHashType::Data1, vec![0xB2]);
    let process_a = seed_process_example_cell(&mut rpc, lock_a, type_a.clone());
    let process_b = seed_process_example_cell(&mut rpc, lock_b, type_b.clone());
    let int_a = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_a), &dest_script()),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let int_b = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_b), &dest_script()),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    let first = digest_ok::<FakeRpcClient>(
        outpoint_ids(&int_a),
        outpoint_ids(&process_a),
        Some(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    let second = digest_ok::<FakeRpcClient>(outpoint_ids(&int_b), outpoint_ids(&process_b), None);
    TransactionCalculator::default()
        .instruction(first)
        .instruction(second)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();

    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("two Processes must not share one beneficiary output");
    assert_eq!(exit(err), ERROR_BENEFICIARY_MISSING);
}

#[tokio::test]
async fn digest_ok_plus_unrelated_refund_succeeds() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_a =
        seed_process_example_cell(&mut rpc, process_example_lock_script(), type_a.clone());

    let int_a = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_a), &dest_script()),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    let user_b = always_success_script(vec![0xBB]);
    let payload_v = always_success_script(vec![0xCC]);
    let int_b = seed_intention(
        &mut rpc,
        &skeleton,
        encode_cid_args(
            [0xAB; 32],
            script_to_cid(&user_b),
            Payload::from_script(&script_to_cid(&payload_v)).expect("payload V"),
        ),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    let digest = digest_ok::<FakeRpcClient>(
        outpoint_ids(&int_a),
        outpoint_ids(&process_a),
        Some(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    let refund = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&int_b),
        user_b.into(),
        CKB_CAPACITY,
        None,
    );
    TransactionCalculator::default()
        .instruction(digest)
        .instruction(refund)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();

    verify(&rpc, skeleton, vec![])
        .await
        .expect("Process A DigestOk plus unrelated DigestIncompatible refund must succeed");
}

/// A's payload beneficiary equals B's refund user_lock. One shared C-capacity
/// output cannot settle both the DigestOk payment and the DigestIncompatible refund.
#[tokio::test]
async fn reject_digest_beneficiary_shared_with_refund() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_a =
        seed_process_example_cell(&mut rpc, process_example_lock_script(), type_a.clone());
    let user_b = always_success_script(vec![0xBB]);
    let payload_v = always_success_script(vec![0xCC]);

    let int_a = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_a), &user_b),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let int_b = seed_intention(
        &mut rpc,
        &skeleton,
        encode_cid_args(
            [0xAB; 32],
            script_to_cid(&user_b),
            Payload::from_script(&script_to_cid(&payload_v)).expect("payload V"),
        ),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    let digest = digest_ok::<FakeRpcClient>(outpoint_ids(&int_a), outpoint_ids(&process_a), None);
    let refund = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&int_b),
        user_b.into(),
        CKB_CAPACITY,
        None,
    );
    TransactionCalculator::default()
        .instruction(digest)
        .instruction(refund)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();

    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("payment and refund must not share one output");
    assert_eq!(exit(err), ERROR_BENEFICIARY_MISSING);
}

/// Same overlap as `reject_digest_beneficiary_shared_with_refund`, but two
/// independent C-capacity outputs to B's user_lock settle both obligations.
#[tokio::test]
async fn digest_beneficiary_and_refund_independent_outputs_succeeds() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_a =
        seed_process_example_cell(&mut rpc, process_example_lock_script(), type_a.clone());
    let user_b = always_success_script(vec![0xBB]);
    let payload_v = always_success_script(vec![0xCC]);

    let int_a = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_a), &user_b),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let int_b = seed_intention(
        &mut rpc,
        &skeleton,
        encode_cid_args(
            [0xAB; 32],
            script_to_cid(&user_b),
            Payload::from_script(&script_to_cid(&payload_v)).expect("payload V"),
        ),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    let digest = digest_ok::<FakeRpcClient>(
        outpoint_ids(&int_a),
        outpoint_ids(&process_a),
        Some(AddOutputCell {
            lock_script: user_b.clone().into(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    let refund = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&int_b),
        user_b.into(),
        CKB_CAPACITY,
        None,
    );
    TransactionCalculator::default()
        .instruction(digest)
        .instruction(refund)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();

    verify(&rpc, skeleton, vec![])
        .await
        .expect("independent payment and refund outputs to the same lock must succeed");
}

fn address_of(script: Script) -> Address {
    Address::new(Network::Fake, AddressPayload::from(script))
}

fn seed_user_lock_cell(rpc: &mut FakeRpcClient, lock: Script, capacity: u64) {
    let cell =
        CellOutputEx::new_from_scripts(lock, None, vec![], Some(capacity)).expect("user cell");
    rpc.insert_fake_cell(fake_outpoint(), cell, Some(fake_header_view(1, 1, 1)));
}

/// DigestOk A + TimeoutBurn B. B has user_lock in inputs, pack/judge headers at
/// TIMEOUT_BLOCKS, and a full user_lock refund. A's beneficiary is B.user_lock.
fn digest_ok_plus_timeout_burn(
    independent_payment_output: bool,
) -> (FakeRpcClient, TransactionSkeleton) {
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_a =
        seed_process_example_cell(&mut rpc, process_example_lock_script(), type_a.clone());
    let user_b = always_success_script(vec![0xBB]);
    let payload_v = always_success_script(vec![0xCC]);
    seed_user_lock_cell(&mut rpc, user_b.clone(), USER_CELL_CAPACITY);
    let judge = PACK_BLOCK + TIMEOUT_BLOCKS;
    seed_header(&mut rpc, judge);

    let int_a = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_a), &user_b),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let int_b = seed_intention(
        &mut rpc,
        &skeleton,
        encode_cid_args(
            [0xAB; 32],
            script_to_cid(&user_b),
            Payload::from_script(&script_to_cid(&payload_v)).expect("payload V"),
        ),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    // TimeoutBurn first so AddHeaderDepByInputIndex(0) is B's pack header.
    let timeout = timeout_burn::<FakeRpcClient>(
        outpoint_ids(&int_b),
        address_of(user_b.clone()),
        user_b.clone().into(),
        CKB_CAPACITY,
        None,
        judge,
    );
    let payment = if independent_payment_output {
        Some(AddOutputCell {
            lock_script: user_b.into(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        })
    } else {
        None
    };
    let digest =
        digest_ok::<FakeRpcClient>(outpoint_ids(&int_a), outpoint_ids(&process_a), payment);
    TransactionCalculator::default()
        .instruction(timeout)
        .instruction(digest)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    (rpc, skeleton)
}

/// A's payload beneficiary equals B's TimeoutBurn user_lock. One shared
/// C-capacity output cannot settle both the payment and the timeout refund.
#[tokio::test]
async fn reject_digest_beneficiary_shared_with_timeout_burn() {
    require_binaries();
    let (rpc, skeleton) = digest_ok_plus_timeout_burn(false);
    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("payment and TimeoutBurn refund must not share one output");
    assert_eq!(exit(err), ERROR_BENEFICIARY_MISSING);
}

/// Same overlap as `reject_digest_beneficiary_shared_with_timeout_burn`, but
/// two independent C-capacity outputs to B's user_lock settle both obligations.
#[tokio::test]
async fn digest_beneficiary_and_timeout_burn_independent_outputs_succeeds() {
    require_binaries();
    let (rpc, skeleton) = digest_ok_plus_timeout_burn(true);
    verify(&rpc, skeleton, vec![])
        .await
        .expect("independent payment and TimeoutBurn refund outputs must succeed");
}

/// Data1 and Data2 lock deployments of this binary still cannot unique-assign
/// the same beneficiary output (distinct Process Type IDs).
#[tokio::test]
async fn reject_data1_data2_processes_sharing_one_beneficiary() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    let mut skeleton = prepared_process_skeleton(&rpc);
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let type_b = process_example_type_script(b"process-type-id-v1".to_vec());
    let lock_a = process_example_lock_script_with(ScriptHashType::Data1, vec![0xA1]);
    let lock_b = process_example_lock_script_with(ScriptHashType::Data2, vec![0xB2]);
    let process_a = seed_process_example_cell(&mut rpc, lock_a, type_a.clone());
    let process_b = seed_process_example_cell(&mut rpc, lock_b, type_b.clone());
    let int_a = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_a), &dest_script()),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let int_b = seed_intention(
        &mut rpc,
        &skeleton,
        beneficiary_args(process_example_type_hash(&type_b), &dest_script()),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );

    let first = digest_ok::<FakeRpcClient>(
        outpoint_ids(&int_a),
        outpoint_ids(&process_a),
        Some(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    let second = digest_ok::<FakeRpcClient>(outpoint_ids(&int_b), outpoint_ids(&process_b), None);
    TransactionCalculator::default()
        .instruction(first)
        .instruction(second)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();

    let err = verify(&rpc, skeleton, vec![])
        .await
        .expect_err("Data1 and Data2 Processes must not share one beneficiary output");
    assert_eq!(exit(err), ERROR_BENEFICIARY_MISSING);
}

#[tokio::test]
async fn assemble_and_verify_data2_beneficiary() {
    require_binaries();
    let dest = dest_script_data2();
    let rpc = FakeRpcClient::default();
    let process = mock_process_state();
    let pth = process_example_type_hash(&process.type_script);
    let args = beneficiary_args(pth, &dest);
    let (assembled, _) = process_digest_ok_with_process(
        &rpc,
        vec![IntentionCell {
            out_point: fake_outpoint(),
            lock_args: args,
            capacity: CKB_CAPACITY,
            data: vec![],
            type_script: None,
        }],
        Some(process),
        CONTRACT_BIN,
    )
    .expect("assemble Data2 payload");
    // outputs[0] = Process recreate; outputs[1] = beneficiary
    let ht: u8 = assembled.outputs[1].lock_script().hash_type().into();
    assert_eq!(ht, Into::<u8>::into(ScriptHashType::Data2));

    let mut rpc = FakeRpcClient::default();
    let skeleton = apply_transfer_to(&mut rpc, dest.clone().into(), &dest, CKB_CAPACITY, vec![]);
    verify(&rpc, skeleton, vec![])
        .await
        .expect("on-chain verify must accept a Data2 beneficiary that assemble emitted");
}

#[tokio::test]
async fn assemble_and_verify_xudt_beneficiary() {
    require_binaries();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let mut skeleton = prepared_process_skeleton(&rpc);
    let xudt = xudt_type_script();
    let xudt_packed = xudt.clone().to_script(&skeleton).expect("xudt script");
    let type_a = process_example_type_script(b"process-type-id-v0".to_vec());
    let process_hash = process_example_type_hash(&type_a);
    let process = seed_process_example_cell(&mut rpc, process_example_lock_script(), type_a);
    let args = beneficiary_args(process_hash, &dest_script());
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        args.clone(),
        CKB_CAPACITY,
        Some((xudt.clone(), XUDT_AMOUNT)),
        PACK_BLOCK,
    );

    let process_state = ProcessStateCell {
        out_point: process.clone(),
        lock: process_example_lock_script(),
        type_script: process_example_type_script(b"process-type-id-v0".to_vec()),
        capacity: CKB_CAPACITY,
        data: vec![],
    };
    let (assembled, _) = process_digest_ok_with_process(
        &rpc,
        vec![IntentionCell {
            out_point: intention.clone(),
            lock_args: args,
            capacity: CKB_CAPACITY,
            data: XUDT_AMOUNT.to_le_bytes().to_vec(),
            type_script: Some(xudt_packed.as_slice().to_vec()),
        }],
        Some(process_state),
        CONTRACT_BIN,
    )
    .expect("assemble xUDT intention");
    // outputs[0] = Process recreate; outputs[1] = beneficiary
    let assembled_out = &assembled.outputs[1];
    assert_eq!(
        assembled_out
            .type_script()
            .expect("assembled beneficiary keeps xUDT type")
            .as_slice(),
        xudt_packed.as_slice()
    );
    assert_eq!(assembled_out.data, XUDT_AMOUNT.to_le_bytes().to_vec());

    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: assembled_out.lock_script().into(),
            type_script: assembled_out.type_script().map(Into::into),
            data: assembled_out.data.clone(),
            capacity: assembled_out.capacity_shannons(),
            absolute_capacity: true,
            type_id: false,
        }),
    );
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    verify(&rpc, skeleton, vec![])
        .await
        .expect("assembled xUDT beneficiary must verify on-chain");
}

// Still ignored: Process type script never runs if the Process cell is absent,
// so a "missing type-id pair" tx is a CID-lock PROCESS_MISMATCH, not ProcessPairInvalid.
#[tokio::test]
#[ignore]
async fn reject_missing_type_id_pair() {
    todo!("Process type does not execute when the Process cell is missing from the tx");
}
