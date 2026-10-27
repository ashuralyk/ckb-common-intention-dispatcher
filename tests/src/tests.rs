//! FakeRpc + CKB-VM tests for CID v0.1.
//!
//! Happy paths: create (skeleton asserts; lock Create does not run CID) and
//! the three Burn hops. Failures use `CalculatorError::script_exit_code()`.

use std::path::Path;

use ckb_cinnabar_calculator::{
    assert_verify,
    instruction::Instruction,
    operation::basic::AddOutputCell,
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::Script,
        prelude::{Pack, Unpack},
        H256,
    },
    simulation::{
        always_success_script, AddFakeAlwaysSuccessCelldep, AddFakeInputCell, FakeRpcClient,
        TransactionSimulator,
    },
    CalculatorError, TransactionCalculator,
};
use ckb_common_intention_dispatcher_calculator::{
    create_intention, digest_incompatible, digest_ok, timeout_burn, cid_lock, ERROR_ARGS_INVALID,
    ERROR_HEADER_MISSING, ERROR_PARTIAL_DIGEST, ERROR_PROCESS_MISMATCH, ERROR_REFUND_ASSET_MISMATCH,
    ERROR_REFUND_MISSING, ERROR_TIMEOUT_NOT_REACHED, ERROR_UNKNOWN_BRANCH, ERROR_UNSUPPORTED_TYPE,
    ERROR_ZERO_ASSET, TIMEOUT_BLOCKS, XUDT_CODE_HASH,
};

use crate::faker::{
    cid_args, dest_lock, outpoint_ids, prepared_skeleton, process_type_hash, seed_fakenet_xudt_cell,
    seed_header, seed_intention, seed_minimal_ckb_intention, seed_process_cell,
    seed_typed_dummy_cell, seed_typed_intention, seed_user_cell, user_address, user_lock,
    user_script, xudt_type_script, CKB_CAPACITY, CONTRACT_BIN, CONTRACT_NAME, PACK_BLOCK,
    USER_CELL_CAPACITY, XUDT_AMOUNT,
};

const MAX_CYCLES: u64 = 50_000_000;

fn require_binary() {
    let path = format!("{CONTRACT_BIN}/{CONTRACT_NAME}");
    assert!(
        Path::new(&path).exists(),
        "contract binary missing; run `make build` first ({path})"
    );
    assert!(
        Path::new("./binaries/xudt").exists(),
        "xUDT RISC-V binary missing at tests/binaries/xudt"
    );
}

async fn verify(
    rpc: &FakeRpcClient,
    skeleton: ckb_cinnabar_calculator::skeleton::TransactionSkeleton,
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

/// Smoke: always-success lock verifies without CID.
#[tokio::test]
async fn always_success_simulates() {
    let rpc = FakeRpcClient::default();
    let lock = always_success_script(vec![]);
    let prepare = Instruction::new(vec![
        Box::new(AddFakeAlwaysSuccessCelldep {}),
        Box::new(AddFakeInputCell {
            lock_script: lock.clone().into(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
        }),
        Box::new(AddOutputCell {
            lock_script: lock.into(),
            type_script: None,
            data: vec![],
            capacity: 100_000_000_000,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    assert_verify!(&rpc, vec![prepare], 0).expect("always-success should verify");
}

#[tokio::test]
async fn create_ckb_intention_skeleton() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, CKB_CAPACITY);
    let mut skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"hello");
    let ix = create_intention::<FakeRpcClient>(user_address(), args.clone(), CKB_CAPACITY, None);
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .expect("assemble create");

    assert_eq!(skeleton.outputs.len(), 1, "one Intention output");
    let out = &skeleton.outputs[0];
    assert!(out.data.is_empty(), "CKB Intention data is empty");
    assert!(out.type_script().is_none(), "CKB Intention has no type");
    assert_eq!(out.capacity_shannons(), CKB_CAPACITY);
    let expected = cid_lock(args).to_script(&skeleton).expect("cid lock");
    assert_eq!(out.lock_script().as_slice(), expected.as_slice());

    verify(&rpc, skeleton, vec![])
        .await
        .expect("create does not run CID; payer lock succeeds");
}

#[tokio::test]
async fn create_xudt_intention_skeleton() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, CKB_CAPACITY);
    let mut skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"xudt");
    let xudt = xudt_type_script();
    let ix = create_intention::<FakeRpcClient>(
        user_address(),
        args,
        CKB_CAPACITY,
        Some((xudt.clone(), XUDT_AMOUNT)),
    );
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .expect("assemble xudt create");

    assert_eq!(skeleton.outputs.len(), 1);
    let out = &skeleton.outputs[0];
    assert_eq!(out.data, XUDT_AMOUNT.to_le_bytes().to_vec());
    let typ = out.type_script().expect("xUDT type");
    assert_eq!(typ.code_hash().as_slice(), XUDT_CODE_HASH.as_slice());
    let ht: u8 = typ.hash_type().into();
    assert_eq!(ht, 2, "xUDT hash_type is Data1");

    verify(&rpc, skeleton, vec![])
        .await
        .expect("xUDT mint + payer lock succeed; CID lock Create does not run");
}

#[tokio::test]
async fn digest_ok_ckb() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"ok");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let process = seed_process_cell(&mut rpc);

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
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("digest_ok CKB");
}

#[tokio::test]
async fn digest_ok_xudt() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"ok-xudt");
    let xudt = xudt_type_script();
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        args,
        CKB_CAPACITY,
        Some((xudt.clone(), XUDT_AMOUNT)),
        PACK_BLOCK,
    );
    let process = seed_process_cell(&mut rpc);

    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: dest_lock(),
            type_script: Some(xudt),
            data: XUDT_AMOUNT.to_le_bytes().to_vec(),
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("digest_ok xUDT");
}

#[tokio::test]
async fn digest_incompatible_ckb() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"inc");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);

    let ix = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_lock(),
        CKB_CAPACITY,
        None,
    );
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("digest_incompatible CKB");
}

#[tokio::test]
async fn digest_incompatible_xudt() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"inc-xudt");
    let xudt = xudt_type_script();
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        args,
        CKB_CAPACITY,
        Some((xudt.clone(), XUDT_AMOUNT)),
        PACK_BLOCK,
    );

    let ix = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_lock(),
        CKB_CAPACITY,
        Some((xudt, XUDT_AMOUNT)),
    );
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("digest_incompatible xUDT");
}

#[tokio::test]
async fn timeout_burn_at_threshold() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"timeout");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let judge = PACK_BLOCK + TIMEOUT_BLOCKS;
    seed_header(&mut rpc, judge);

    let ix = timeout_burn::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_address(),
        user_lock(),
        CKB_CAPACITY,
        None,
        judge,
    );
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("timeout_burn at exact TIMEOUT_BLOCKS");
}

#[tokio::test]
async fn timeout_burn_xudt() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"timeout-xudt");
    let xudt = xudt_type_script();
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        args,
        CKB_CAPACITY,
        Some((xudt.clone(), XUDT_AMOUNT)),
        PACK_BLOCK,
    );
    let judge = PACK_BLOCK + TIMEOUT_BLOCKS + 1;
    seed_header(&mut rpc, judge);

    let ix = timeout_burn::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_address(),
        user_lock(),
        CKB_CAPACITY,
        Some((xudt, XUDT_AMOUNT)),
        judge,
    );
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("timeout_burn xUDT");
}

#[tokio::test]
async fn fail_args_invalid() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        vec![0u8; 8],
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let process = seed_process_cell(&mut rpc);
    let ix = digest_ok::<FakeRpcClient>(outpoint_ids(&intention), outpoint_ids(&process), None);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("args");
    assert_eq!(exit(err), ERROR_ARGS_INVALID);
}

#[tokio::test]
async fn fail_unknown_branch() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"unknown");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let ix = Instruction::<FakeRpcClient>::new(vec![Box::new(
        ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
            tx_hash: outpoint_ids(&intention).0,
            index: outpoint_ids(&intention).1,
            since: None,
        },
    )]);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("unknown");
    assert_eq!(exit(err), ERROR_UNKNOWN_BRANCH);
}

#[tokio::test]
async fn fail_process_mismatch() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"mismatch");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let dummy = seed_typed_dummy_cell(&mut rpc);
    let ix = digest_ok::<FakeRpcClient>(outpoint_ids(&intention), outpoint_ids(&dummy), None);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("process");
    assert_eq!(exit(err), ERROR_PROCESS_MISMATCH);
}

#[tokio::test]
async fn fail_partial_digest() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"partial");
    let intention = seed_intention(&mut rpc, &skeleton, args.clone(), CKB_CAPACITY, None, PACK_BLOCK);
    let process = seed_process_cell(&mut rpc);
    let ix = digest_ok::<FakeRpcClient>(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: cid_lock(args),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("partial");
    assert_eq!(exit(err), ERROR_PARTIAL_DIGEST);
}

#[tokio::test]
async fn fail_refund_missing_on_timeout() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"no-refund");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let judge = PACK_BLOCK + TIMEOUT_BLOCKS;
    seed_header(&mut rpc, judge);

    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention).0,
                index: outpoint_ids(&intention).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByAddress {
                address: user_address(),
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddHeaderDepByInputIndex { input_index: 0 },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddHeaderDepByBlockNumber {
                block_number: judge,
            },
        ),
    ]);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("refund");
    assert_eq!(exit(err), ERROR_REFUND_MISSING);
}

#[tokio::test]
async fn fail_refund_asset_mismatch() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"bad-refund");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let ix = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_lock(),
        CKB_CAPACITY / 2,
        None,
    );
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("asset");
    assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
}

#[tokio::test]
async fn fail_timeout_not_reached() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"early");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);
    let judge = PACK_BLOCK + TIMEOUT_BLOCKS - 1;
    seed_header(&mut rpc, judge);

    let ix = timeout_burn::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_address(),
        user_lock(),
        CKB_CAPACITY,
        None,
        judge,
    );
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("early");
    assert_eq!(exit(err), ERROR_TIMEOUT_NOT_REACHED);
}

#[tokio::test]
async fn fail_header_missing() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"no-hdr");
    let intention = seed_intention(&mut rpc, &skeleton, args, CKB_CAPACITY, None, PACK_BLOCK);

    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention).0,
                index: outpoint_ids(&intention).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByAddress {
                address: user_address(),
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("header");
    assert_eq!(exit(err), ERROR_HEADER_MISSING);
}

#[tokio::test]
async fn fail_zero_asset() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"zero");
    let intention = seed_minimal_ckb_intention(&mut rpc, &skeleton, args);
    let process = seed_process_cell(&mut rpc);
    let ix = digest_ok::<FakeRpcClient>(outpoint_ids(&intention), outpoint_ids(&process), None);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("zero");
    assert_eq!(exit(err), ERROR_ZERO_ASSET);
}

#[tokio::test]
async fn fail_unsupported_type() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), b"bad-type");
    let foreign = Script::new_builder()
        .code_hash(H256(blake2b_always_success()).pack())
        .hash_type(ScriptHashType::Data1.into())
        .args(b"not-xudt".to_vec().pack())
        .build();
    let intention = seed_typed_intention(&mut rpc, &skeleton, args, foreign);
    let process = seed_process_cell(&mut rpc);
    let ix = digest_ok::<FakeRpcClient>(outpoint_ids(&intention), outpoint_ids(&process), None);
    let err = verify(&rpc, skeleton, vec![ix]).await.expect_err("type");
    assert_eq!(exit(err), ERROR_UNSUPPORTED_TYPE);
}

fn blake2b_always_success() -> [u8; 32] {
    user_script().code_hash().unpack()
}
