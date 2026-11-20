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
        core::{Capacity, ScriptHashType},
        packed::Script,
        prelude::{Builder, Entity, Pack, Unpack},
        H256,
    },
    simulation::{
        always_success_script, AddFakeAlwaysSuccessCelldep, AddFakeInputCell, FakeRpcClient,
        TransactionSimulator,
    },
    CalculatorError, TransactionCalculator,
};
use ckb_common_intention_dispatcher_calculator::{
    cid_lock, create_intention, digest_incompatible, digest_ok, encode_cid_args, timeout_burn,
    Payload, ERROR_ARGS_INVALID, ERROR_HEADER_MISSING, ERROR_MIXED_CID_LOCK, ERROR_PARTIAL_DIGEST,
    ERROR_PROCESS_MISMATCH, ERROR_REFUND_ASSET_MISMATCH, ERROR_REFUND_MISSING,
    ERROR_TIMEOUT_NOT_REACHED, ERROR_UNKNOWN_BRANCH, ERROR_UNSUPPORTED_TYPE, ERROR_ZERO_ASSET,
    MAX_REFUND_FEE, TIMEOUT_BLOCKS, XUDT_CODE_HASH,
};

use crate::faker::{
    cid_args, dest_lock, dest_script, outpoint_ids, prepared_skeleton, process_type_hash,
    script_to_cid, seed_fakenet_xudt_cell, seed_header, seed_intention, seed_minimal_ckb_intention,
    seed_process_cell, seed_typed_dummy_cell, seed_typed_intention, seed_user_cell, user_address,
    user_lock, user_script, xudt_type_script, CKB_CAPACITY, CONTRACT_BIN, CONTRACT_NAME,
    PACK_BLOCK, USER_CELL_CAPACITY, XUDT_AMOUNT,
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
    // A CKB Intention must carry capacity above the occupied bytes.
    let ix = create_intention::<FakeRpcClient>(user_address(), args.clone(), 1, None);
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .expect("assemble create");

    assert_eq!(skeleton.outputs.len(), 1, "one Intention output");
    let out = &skeleton.outputs[0];
    assert!(out.data.is_empty(), "CKB Intention data is empty");
    assert!(out.type_script().is_none(), "CKB Intention has no type");
    let occupied = out
        .output
        .occupied_capacity(Capacity::bytes(out.data.len()).unwrap())
        .unwrap()
        .as_u64();
    assert_eq!(out.capacity_shannons(), occupied + 1);
    let expected = cid_lock(args).to_script(&skeleton).expect("cid lock");
    assert_eq!(out.lock_script().as_slice(), expected.as_slice());

    let created_output = out.clone();
    verify(&rpc, skeleton, vec![])
        .await
        .expect("create does not run CID; payer lock succeeds");

    let intention = ckb_cinnabar_calculator::simulation::fake_outpoint();
    rpc.insert_fake_cell(intention.clone(), created_output, None);
    let refund = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_lock(),
        occupied + 1,
        None,
    );
    verify(&rpc, prepared_skeleton(&rpc), vec![refund])
        .await
        .expect("a minimum-sized CKB Intention can be refunded");
}

#[tokio::test]
async fn create_xudt_intention_skeleton() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, CKB_CAPACITY);
    let mut skeleton = prepared_skeleton(&rpc);
    let args = cid_args(process_type_hash(), &[0x42; 512]);
    let xudt = xudt_type_script();
    rpc.insert_fake_cell(
        ckb_cinnabar_calculator::simulation::fake_outpoint(),
        ckb_cinnabar_calculator::skeleton::CellOutputEx::new_from_scripts(
            user_script(),
            Some(xudt.clone().to_script(&skeleton).unwrap()),
            XUDT_AMOUNT.to_le_bytes().to_vec(),
            Some(CKB_CAPACITY),
        )
        .unwrap(),
        None,
    );
    let ix = create_intention::<FakeRpcClient>(
        user_address(),
        args,
        1, // below occupied; create bumps to occupied only
        Some((xudt.clone(), XUDT_AMOUNT)),
    );
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .expect("assemble xudt create");

    assert_eq!(skeleton.outputs.len(), 1);
    let out = &skeleton.outputs[0];
    assert_eq!(out.data, XUDT_AMOUNT.to_le_bytes().to_vec());
    let occupied = out
        .output
        .occupied_capacity(Capacity::bytes(out.data.len()).unwrap())
        .unwrap()
        .as_u64();
    assert!(occupied > 1);
    assert_eq!(out.capacity_shannons(), occupied);
    let typ = out.type_script().expect("xUDT type");
    assert_eq!(typ.code_hash().as_slice(), XUDT_CODE_HASH.as_slice());
    let ht: u8 = typ.hash_type().into();
    assert_eq!(ht, 2, "xUDT hash_type is Data1");

    verify(&rpc, skeleton, vec![])
        .await
        .expect("xUDT transfer + payer lock succeed; CID lock Create does not run");
}

#[tokio::test]
async fn rejects_process_celldep_only_ckb_theft() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let mut skeleton = prepared_skeleton(&rpc);
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
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    // Undo the recipe's Process transfer and reproduce the old CellDep-only spend.
    skeleton
        .inputs
        .retain(|i| i.input.previous_output() != process);
    skeleton.witnesses.truncate(skeleton.inputs.len());
    skeleton.outputs.remove(0);
    let (tx_hash, index) = outpoint_ids(&process);
    let dep = Instruction::new(vec![Box::new(
        ckb_cinnabar_calculator::operation::basic::AddCellDep {
            name: "process".into(),
            tx_hash,
            index,
            dep_type: ckb_cinnabar_calculator::types::DepType::Code,
            with_data: true,
        },
    )]);
    let err = verify(&rpc, skeleton, vec![dep])
        .await
        .expect_err("CellDep cannot authorize CKB theft");
    assert_eq!(exit(err), ERROR_PROCESS_MISMATCH);
}

#[tokio::test]
async fn rejects_process_celldep_only_xudt_theft() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let mut skeleton = prepared_skeleton(&rpc);
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
    TransactionCalculator::default()
        .instruction(ix)
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    // Undo the recipe's Process transfer and reproduce the old CellDep-only spend.
    skeleton
        .inputs
        .retain(|i| i.input.previous_output() != process);
    skeleton.witnesses.truncate(skeleton.inputs.len());
    skeleton.outputs.remove(0);
    let (tx_hash, index) = outpoint_ids(&process);
    let dep = Instruction::new(vec![Box::new(
        ckb_cinnabar_calculator::operation::basic::AddCellDep {
            name: "process".into(),
            tx_hash,
            index,
            dep_type: ckb_cinnabar_calculator::types::DepType::Code,
            with_data: true,
        },
    )]);
    let err = verify(&rpc, skeleton, vec![dep])
        .await
        .expect_err("CellDep cannot authorize xUDT theft");
    assert_eq!(exit(err), ERROR_PROCESS_MISMATCH);
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
async fn digest_incompatible_refund_fee_within_cap() {
    require_binary();
    for (label, fee, xudt) in [
        ("ckb-exact", 0u64, None),
        ("ckb-half-cap", MAX_REFUND_FEE / 2, None),
        ("ckb-full-cap", MAX_REFUND_FEE, None),
        (
            "xudt-full-cap",
            MAX_REFUND_FEE,
            Some((xudt_type_script(), XUDT_AMOUNT)),
        ),
    ] {
        let mut rpc = FakeRpcClient::default();
        seed_fakenet_xudt_cell(&mut rpc);
        let skeleton = prepared_skeleton(&rpc);
        let intention = seed_intention(
            &mut rpc,
            &skeleton,
            cid_args(process_type_hash(), label.as_bytes()),
            CKB_CAPACITY,
            xudt.clone(),
            PACK_BLOCK,
        );
        // Pure incompatible haircut: no Process cells; shortfall == miner fee.
        let ix = digest_incompatible::<FakeRpcClient>(
            outpoint_ids(&intention),
            user_lock(),
            CKB_CAPACITY - fee,
            xudt,
        );
        verify(&rpc, skeleton, vec![ix])
            .await
            .unwrap_or_else(|e| panic!("refund fee within cap ({label}): {e:?}"));
    }
}

#[tokio::test]
async fn digest_incompatible_refund_fee_beyond_cap_rejected() {
    require_binary();
    for xudt in [None, Some((xudt_type_script(), XUDT_AMOUNT))] {
        let mut rpc = FakeRpcClient::default();
        seed_fakenet_xudt_cell(&mut rpc);
        let skeleton = prepared_skeleton(&rpc);
        let intention = seed_intention(
            &mut rpc,
            &skeleton,
            cid_args(process_type_hash(), b"fee-over"),
            CKB_CAPACITY,
            xudt.clone(),
            PACK_BLOCK,
        );
        let ix = digest_incompatible::<FakeRpcClient>(
            outpoint_ids(&intention),
            user_lock(),
            CKB_CAPACITY - MAX_REFUND_FEE - 1,
            xudt,
        );
        let err = verify(&rpc, skeleton, vec![ix])
            .await
            .expect_err("fee beyond MAX_REFUND_FEE");
        assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
    }
}

#[tokio::test]
async fn digest_incompatible_wrong_lock_rejected() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"wrong-lock"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    // Refund to a different lock: no User Lock output → not DigestIncompatible.
    let ix = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention),
        dest_lock(),
        CKB_CAPACITY - MAX_REFUND_FEE,
        None,
    );
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("wrong refund lock");
    assert_eq!(exit(err), ERROR_UNKNOWN_BRANCH);
}

#[tokio::test]
async fn digest_incompatible_token_amount_change_rejected() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let xudt = xudt_type_script();
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"token-mut"),
        CKB_CAPACITY,
        Some((xudt.clone(), XUDT_AMOUNT)),
        PACK_BLOCK,
    );
    let ix = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_lock(),
        CKB_CAPACITY - MAX_REFUND_FEE,
        Some((xudt, XUDT_AMOUNT - 1)),
    );
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("token amount must stay exact");
    assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
}

#[tokio::test]
async fn digest_incompatible_batch_two_users_fee_covers_aggregate() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);

    let user_a = user_lock();
    let user_b = dest_lock();
    let args_a = cid_args(process_type_hash(), b"batch-a");
    let args_b = encode_cid_args(
        process_type_hash(),
        script_to_cid(&dest_script()),
        Payload {
            data: b"batch-b".to_vec(),
        },
    );
    let intention_a = seed_intention(&mut rpc, &skeleton, args_a, CKB_CAPACITY, None, PACK_BLOCK);
    let intention_b = seed_intention(&mut rpc, &skeleton, args_b, CKB_CAPACITY, None, PACK_BLOCK);

    // Aggregate shortfall = MAX + MAX/2 equals miner fee when only these cells exist.
    let refund_a = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention_a),
        user_a,
        CKB_CAPACITY - MAX_REFUND_FEE,
        None,
    );
    let refund_b = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&intention_b),
        user_b,
        CKB_CAPACITY - MAX_REFUND_FEE / 2,
        None,
    );
    verify(&rpc, skeleton, vec![refund_a, refund_b])
        .await
        .expect("multi-user batch when fee covers aggregate shortfall");
}

#[tokio::test]
async fn digest_incompatible_rejects_third_party_siphon_of_shortfall() {
    require_binary();
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"siphon"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    // Funding input lets the siphon cell stay above occupied while absorbing the
    // shaved shortfall so miner fee drops to 0.
    let fund = {
        let out = fake_outpoint();
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(dest_script(), None, vec![], Some(USER_CELL_CAPACITY))
                .unwrap(),
            None,
        );
        out
    };
    let shortfall = MAX_REFUND_FEE;
    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention).0,
                index: outpoint_ids(&intention).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&fund).0,
                index: outpoint_ids(&fund).1,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: USER_CELL_CAPACITY + shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("shortfall must not fund third-party outputs");
    assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
}

#[tokio::test]
async fn digest_incompatible_rejects_fee_reuse_across_users() {
    require_binary();
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let args_a = cid_args(process_type_hash(), b"reuse-a");
    let args_b = encode_cid_args(
        process_type_hash(),
        script_to_cid(&dest_script()),
        Payload {
            data: b"reuse-b".to_vec(),
        },
    );
    let intention_a = seed_intention(&mut rpc, &skeleton, args_a, CKB_CAPACITY, None, PACK_BLOCK);
    let intention_b = seed_intention(&mut rpc, &skeleton, args_b, CKB_CAPACITY, None, PACK_BLOCK);
    let fund = {
        let out = fake_outpoint();
        // Third lock so refund lookups for A/B stay unique.
        let other = always_success_script(vec![0xDD]);
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(other.clone(), None, vec![], Some(USER_CELL_CAPACITY))
                .unwrap(),
            None,
        );
        (out, other)
    };

    // Each shaves MAX (aggregate 2×MAX); fund change absorbs both shortfalls → fee 0.
    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention_a).0,
                index: outpoint_ids(&intention_a).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention_b).0,
                index: outpoint_ids(&intention_b).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&fund.0).0,
                index: outpoint_ids(&fund.0).1,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - MAX_REFUND_FEE,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - MAX_REFUND_FEE,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: fund.1.into(),
            type_script: None,
            data: vec![],
            capacity: USER_CELL_CAPACITY + MAX_REFUND_FEE * 2,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("fee must cover aggregate shortfall, not one MAX reused twice");
    assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
}

#[tokio::test]
async fn digest_incompatible_sponsor_change_ok_when_fee_covers() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"sponsor-ok"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    // Sponsor must not use the Intention user_lock (that would select TimeoutBurn).
    let sponsor = {
        use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
        let out = fake_outpoint();
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(dest_script(), None, vec![], Some(USER_CELL_CAPACITY))
                .unwrap(),
            None,
        );
        out
    };
    let shortfall = MAX_REFUND_FEE;
    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention).0,
                index: outpoint_ids(&intention).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&sponsor).0,
                index: outpoint_ids(&sponsor).1,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
        // Sponsor change breaks even; miner fee == shortfall.
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: USER_CELL_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    verify(&rpc, skeleton, vec![ix])
        .await
        .expect("sponsor change ok when fee equals shortfall");
}

#[tokio::test]
async fn digest_incompatible_sponsor_cannot_inflate_siphon() {
    require_binary();
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"sponsor-siphon"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let sponsor = {
        let out = fake_outpoint();
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(dest_script(), None, vec![], Some(USER_CELL_CAPACITY))
                .unwrap(),
            None,
        );
        out
    };
    let shortfall = MAX_REFUND_FEE;
    // Sponsor "change" absorbs the shortfall → fee 0 < shortfall.
    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention).0,
                index: outpoint_ids(&intention).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&sponsor).0,
                index: outpoint_ids(&sponsor).1,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: USER_CELL_CAPACITY + shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("sponsor must not underwrite a siphon from the shortfall");
    assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
}

#[tokio::test]
async fn digest_incompatible_rejects_refund_below_occupied() {
    require_binary();
    use ckb_cinnabar_calculator::skeleton::CellOutputEx;
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let occupied = CellOutputEx::new_from_scripts(user_script(), None, vec![], None)
        .unwrap()
        .capacity_shannons();
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"occupied"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    // Below-occupied outputs cannot be assembled (consensus/occupied floor). The
    // lock predicate is covered by protocol::refund_capacity_window; here we prove
    // the tx cannot be built with a sub-occupied refund cell.
    let mut skel = skeleton;
    let err = TransactionCalculator::default()
        .instruction(digest_incompatible::<FakeRpcClient>(
            outpoint_ids(&intention),
            user_lock(),
            occupied - 1,
            None,
        ))
        .apply_skeleton(&rpc, &mut skel)
        .expect_err("sub-occupied refund must fail assembly");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("InsufficientCapacity") || msg.contains("capacity"),
        "unexpected assembly error: {msg}"
    );
}

#[tokio::test]
async fn timeout_burn_rejects_shaved_refund() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"timeout-shave"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let judge = PACK_BLOCK + TIMEOUT_BLOCKS;
    seed_header(&mut rpc, judge);
    let ix = timeout_burn::<FakeRpcClient>(
        outpoint_ids(&intention),
        user_address(),
        user_lock(),
        CKB_CAPACITY - MAX_REFUND_FEE,
        None,
        judge,
    );
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("TimeoutBurn requires exact capacity");
    assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
}

#[tokio::test]
async fn shaved_digest_incompatible_rejects_co_bundle_with_digest_ok() {
    require_binary();
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let process = seed_process_cell(&mut rpc);
    // Process fixture cells use dest_script as lock — refund user must differ.
    let refund_user = always_success_script(vec![0xBB]);
    let ok_intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"ok-bundle"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    // Unrelated process_type_hash so this Intention stays DigestIncompatible while
    // the Process pair is present for the DigestOk Intention.
    let refund_intention = seed_intention(
        &mut rpc,
        &skeleton,
        encode_cid_args(
            [0xAB; 32],
            script_to_cid(&refund_user),
            Payload {
                data: b"refund-bundle".to_vec(),
            },
        ),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let digest = digest_ok::<FakeRpcClient>(
        outpoint_ids(&ok_intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    let refund = digest_incompatible::<FakeRpcClient>(
        outpoint_ids(&refund_intention),
        refund_user.into(),
        CKB_CAPACITY - MAX_REFUND_FEE,
        None,
    );
    let err = verify(&rpc, skeleton, vec![digest, refund])
        .await
        .expect_err("shaved refund must not co-bundle with DigestOk");
    assert_eq!(exit(err), ERROR_PARTIAL_DIGEST);
}

/// Foreign lock with CID-shaped args in a DigestIncompatible tx must be rejected
/// (`MixedCidLock`) — cannot safely ignore spoofs without enabling cross-version
/// fee reuse.
#[tokio::test]
async fn digest_incompatible_rejects_foreign_cid_shaped_lock() {
    require_binary();
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"mixed-foreign"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let decoy_args = encode_cid_args(
        process_type_hash(),
        script_to_cid(&dest_script()),
        Payload {
            data: b"decoy-cid-args".to_vec(),
        },
    );
    let decoy_lock = always_success_script(decoy_args);
    let decoy = {
        let out = fake_outpoint();
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(
                decoy_lock.clone(),
                None,
                vec![],
                Some(USER_CELL_CAPACITY),
            )
            .unwrap(),
            None,
        );
        out
    };
    let shortfall = 1_000u64;
    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention).0,
                index: outpoint_ids(&intention).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&decoy).0,
                index: outpoint_ids(&decoy).1,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: decoy_lock.into(),
            type_script: None,
            data: vec![],
            capacity: USER_CELL_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("foreign CID-shaped lock must MixedCidLock");
    assert_eq!(exit(err), ERROR_MIXED_CID_LOCK);
}

/// Two CID identities (Data1 vs Data2) each shaving; F = max(S_i) would pass
/// broken per-group checks while S_A+S_B > F funds a third-party siphon.
#[tokio::test]
async fn digest_incompatible_rejects_cross_identity_fee_reuse() {
    require_binary();
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let shortfall = 1_000u64;
    let intention_a = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"id-a"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    // Second Intention: same binary code_hash, Data2 hash_type → distinct identity.
    let args_b = encode_cid_args(
        process_type_hash(),
        script_to_cid(&dest_script()),
        Payload {
            data: b"id-b".to_vec(),
        },
    );
    let lock_b = cid_lock(args_b)
        .to_script(&skeleton)
        .unwrap()
        .as_builder()
        .hash_type(ScriptHashType::Data2)
        .build();
    let intention_b = {
        let out = fake_outpoint();
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(lock_b, None, vec![], Some(CKB_CAPACITY)).unwrap(),
            Some(ckb_cinnabar_calculator::simulation::fake_header_view(
                PACK_BLOCK, PACK_BLOCK, PACK_BLOCK,
            )),
        );
        out
    };
    // Funder lets the siphon cell stay above occupied while absorbing one shortfall
    // so F = shortfall = max(S_A, S_B) < S_A+S_B (the broken per-group accept case).
    let siphon_lock = always_success_script(vec![0xDD]);
    let funder = {
        let out = fake_outpoint();
        rpc.insert_fake_cell(
            out.clone(),
            CellOutputEx::new_from_scripts(
                siphon_lock.clone(),
                None,
                vec![],
                Some(USER_CELL_CAPACITY),
            )
            .unwrap(),
            None,
        );
        out
    };
    let ix = Instruction::<FakeRpcClient>::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention_a).0,
                index: outpoint_ids(&intention_a).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&intention_b).0,
                index: outpoint_ids(&intention_b).1,
                since: None,
            },
        ),
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash: outpoint_ids(&funder).0,
                index: outpoint_ids(&funder).1,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY - shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
        Box::new(AddOutputCell {
            lock_script: siphon_lock.into(),
            type_script: None,
            data: vec![],
            capacity: USER_CELL_CAPACITY + shortfall,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let err = verify(&rpc, skeleton, vec![ix])
        .await
        .expect_err("mixed CID identities must not share DigestIncompatible tx");
    assert_eq!(exit(err), ERROR_MIXED_CID_LOCK);
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
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        args.clone(),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
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
        .hash_type(ScriptHashType::Data1)
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

#[tokio::test]
async fn rejects_xudt_refund_capacity_theft() {
    for timeout in [false, true] {
        let mut rpc = FakeRpcClient::default();
        seed_fakenet_xudt_cell(&mut rpc);
        let skeleton = prepared_skeleton(&rpc);
        let xudt = xudt_type_script();
        let intention = seed_intention(
            &mut rpc,
            &skeleton,
            cid_args(process_type_hash(), b"capacity-theft"),
            CKB_CAPACITY,
            Some((xudt.clone(), XUDT_AMOUNT)),
            PACK_BLOCK,
        );
        let ix = if timeout {
            seed_user_cell(&mut rpc, USER_CELL_CAPACITY);
            seed_header(&mut rpc, PACK_BLOCK + TIMEOUT_BLOCKS);
            timeout_burn(
                outpoint_ids(&intention),
                user_address(),
                user_lock(),
                CKB_CAPACITY - 100_000_000,
                Some((xudt, XUDT_AMOUNT)),
                PACK_BLOCK + TIMEOUT_BLOCKS,
            )
        } else {
            digest_incompatible(
                outpoint_ids(&intention),
                user_lock(),
                CKB_CAPACITY - 100_000_000,
                Some((xudt, XUDT_AMOUNT)),
            )
        };
        let err = verify(&rpc, skeleton, vec![ix])
            .await
            .expect_err("must refund CKB as well as tokens");
        assert_eq!(exit(err), ERROR_REFUND_ASSET_MISMATCH);
    }
}

#[tokio::test]
async fn create_from_non_issuer_collects_tokens_and_returns_change() {
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    for balance in [1500u128, 999] {
        let mut rpc = FakeRpcClient::default();
        seed_fakenet_xudt_cell(&mut rpc);
        seed_user_cell(&mut rpc, CKB_CAPACITY);
        let mut skeleton = prepared_skeleton(&rpc);
        // Issuer differs from payer; this cannot exercise xUDT mint mode.
        let typ = ckb_cinnabar_calculator::skeleton::ScriptEx::new_code(
            XUDT_CODE_HASH,
            crate::faker::dest_script()
                .calc_script_hash()
                .as_slice()
                .to_vec(),
        );
        let token_input = fake_outpoint();
        rpc.insert_fake_cell(
            token_input.clone(),
            CellOutputEx::new_from_scripts(
                user_script(),
                Some(typ.clone().to_script(&skeleton).unwrap()),
                balance.to_le_bytes().to_vec(),
                Some(CKB_CAPACITY),
            )
            .unwrap(),
            None,
        );
        let result = TransactionCalculator::default()
            .instruction(create_intention(
                user_address(),
                cid_args(process_type_hash(), b"transfer"),
                CKB_CAPACITY,
                Some((typ.clone(), 1000)),
            ))
            .apply_skeleton(&rpc, &mut skeleton);
        if balance < 1000 {
            assert!(
                result.is_err(),
                "insufficient tokens must fail during assembly"
            );
            continue;
        }
        result.unwrap();
        assert!(skeleton
            .inputs
            .iter()
            .any(|i| i.input.previous_output() == token_input));
        assert!(skeleton
            .outputs
            .iter()
            .any(|o| o.lock_script() == user_script()
                && o.type_script() == Some(typ.clone().to_script(&skeleton).unwrap())
                && o.data == 500u128.to_le_bytes()));
        verify(&rpc, skeleton, vec![])
            .await
            .expect("ordinary token transfer verifies");
    }
}

#[test]
fn digest_adds_xudt_dependency_from_input_without_output() {
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let mut skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"input-dep"),
        CKB_CAPACITY,
        Some((xudt_type_script(), XUDT_AMOUNT)),
        PACK_BLOCK,
    );
    let process = seed_process_cell(&mut rpc);
    skeleton.celldeps.retain(|d| d.name != "xudt");
    TransactionCalculator::default()
        .instruction(digest_ok(
            outpoint_ids(&intention),
            outpoint_ids(&process),
            None,
        ))
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    assert!(skeleton.celldeps.iter().any(|d| d.name == "xudt"));
}

#[tokio::test]
async fn process_digest_accepts_partial_user_proceeds_without_user_input() {
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let intention = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"partial-proceeds"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let process = seed_process_cell(&mut rpc);
    let digest = digest_ok(
        outpoint_ids(&intention),
        outpoint_ids(&process),
        Some(AddOutputCell {
            lock_script: user_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY / 2,
            absolute_capacity: true,
            type_id: false,
        }),
    );
    verify(&rpc, skeleton, vec![digest])
        .await
        .expect("Process-authorized digest without a user input");
}

#[tokio::test]
async fn rejects_shared_refund_across_distinct_intentions() {
    use ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint;
    for xudt in [None, Some((xudt_type_script(), XUDT_AMOUNT))] {
        let mut rpc = FakeRpcClient::default();
        seed_fakenet_xudt_cell(&mut rpc);
        let skeleton = prepared_skeleton(&rpc);
        let first = seed_intention(
            &mut rpc,
            &skeleton,
            cid_args(process_type_hash(), b"first"),
            CKB_CAPACITY,
            xudt.clone(),
            PACK_BLOCK,
        );
        let second = seed_intention(
            &mut rpc,
            &skeleton,
            cid_args(process_type_hash(), b"second"),
            CKB_CAPACITY,
            xudt.clone(),
            PACK_BLOCK,
        );
        let (tx_hash, index) = outpoint_ids(&second);
        let steal = Instruction::new(vec![
            Box::new(AddInputCellByOutPoint {
                tx_hash,
                index,
                since: None,
            }),
            Box::new(AddOutputCell {
                lock_script: dest_lock(),
                type_script: xudt.as_ref().map(|(t, _)| t.clone()),
                data: xudt
                    .as_ref()
                    .map(|(_, a)| a.to_le_bytes().to_vec())
                    .unwrap_or_default(),
                capacity: CKB_CAPACITY,
                absolute_capacity: true,
                type_id: false,
            }),
        ]);
        let refund = digest_incompatible(outpoint_ids(&first), user_lock(), CKB_CAPACITY, xudt);
        let err = verify(&rpc, skeleton, vec![steal, refund])
            .await
            .expect_err("one refund must not release two intentions");
        assert_eq!(exit(err), ERROR_PARTIAL_DIGEST);
    }
}

#[tokio::test]
async fn rejects_shared_refund_across_cid_hash_type_aliases() {
    use ckb_cinnabar_calculator::{simulation::fake_outpoint, skeleton::CellOutputEx};
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let skeleton = prepared_skeleton(&rpc);
    let lock = cid_lock(cid_args(process_type_hash(), b"data2-alias"))
        .to_script(&skeleton)
        .unwrap()
        .as_builder()
        .hash_type(ScriptHashType::Data2)
        .build();
    let intention = fake_outpoint();
    rpc.insert_fake_cell(
        intention.clone(),
        CellOutputEx::new_from_scripts(lock, None, vec![], Some(CKB_CAPACITY)).unwrap(),
        None,
    );
    let first = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"data1-alias"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let (tx_hash, index) = outpoint_ids(&intention);
    let steal = Instruction::new(vec![
        Box::new(
            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                tx_hash,
                index,
                since: None,
            },
        ),
        Box::new(AddOutputCell {
            lock_script: dest_lock(),
            type_script: None,
            data: vec![],
            capacity: CKB_CAPACITY,
            absolute_capacity: true,
            type_id: false,
        }),
    ]);
    let refund = digest_incompatible(outpoint_ids(&first), user_lock(), CKB_CAPACITY, None);
    let err = verify(&rpc, skeleton, vec![steal, refund])
        .await
        .expect_err("aliases must not share refunds");
    assert_eq!(exit(err), ERROR_PARTIAL_DIGEST);
}

#[tokio::test]
async fn process_pair_executes_without_user_and_rejects_invalid_transitions() {
    for xudt in [false, true] {
        for case in [
            "valid",
            "input_only",
            "output_only",
            "wrong_input",
            "wrong_output",
            "reject",
        ] {
            let mut rpc = FakeRpcClient::default();
            seed_fakenet_xudt_cell(&mut rpc);
            let mut skeleton = prepared_skeleton(&rpc);
            let typ = xudt.then(xudt_type_script);
            let intention = seed_intention(
                &mut rpc,
                &skeleton,
                cid_args(process_type_hash(), b"delegated"),
                CKB_CAPACITY,
                typ.clone().map(|t| (t, XUDT_AMOUNT)),
                PACK_BLOCK,
            );
            let process = seed_process_cell(&mut rpc);
            TransactionCalculator::default()
                .instruction(digest_ok(
                    outpoint_ids(&intention),
                    outpoint_ids(&process),
                    Some(AddOutputCell {
                        lock_script: dest_lock(),
                        type_script: typ,
                        data: if xudt {
                            XUDT_AMOUNT.to_le_bytes().to_vec()
                        } else {
                            vec![]
                        },
                        capacity: CKB_CAPACITY,
                        absolute_capacity: true,
                        type_id: false,
                    }),
                ))
                .apply_skeleton(&rpc, &mut skeleton)
                .unwrap();
            assert!(skeleton
                .inputs
                .iter()
                .all(|i| i.output.lock_script() != user_script()));
            assert!(skeleton
                .celldeps
                .iter()
                .all(|d| d.celldep.out_point() != process));
            assert_eq!(
                skeleton.outputs[0].type_script(),
                Some(crate::faker::process_type_script())
            );
            assert_eq!(skeleton.outputs[0].capacity_shannons(), CKB_CAPACITY);
            match case {
                "input_only" => {
                    skeleton.outputs.remove(0);
                }
                "output_only" => {
                    skeleton
                        .inputs
                        .retain(|i| i.input.previous_output() != process);
                    skeleton.witnesses.truncate(skeleton.inputs.len());
                }
                "wrong_input" => {
                    skeleton
                        .inputs
                        .retain(|i| i.input.previous_output() != process);
                    skeleton.witnesses.truncate(skeleton.inputs.len());
                    let dummy = seed_typed_dummy_cell(&mut rpc);
                    let (tx_hash, index) = outpoint_ids(&dummy);
                    TransactionCalculator::default()
                        .instruction(Instruction::new(vec![Box::new(
                            ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint {
                                tx_hash,
                                index,
                                since: None,
                            },
                        )]))
                        .apply_skeleton(&rpc, &mut skeleton)
                        .unwrap();
                }
                "wrong_output" => {
                    let wrong = crate::faker::process_type_script()
                        .as_builder()
                        .args(
                            {
                                let mut a = [0u8; 32];
                                a[..13].copy_from_slice(b"wrong-process");
                                a.to_vec()
                            }
                            .pack(),
                        )
                        .build();
                    skeleton.outputs[0].output = skeleton.outputs[0]
                        .output
                        .clone()
                        .as_builder()
                        .type_(Some(wrong).pack())
                        .build();
                }
                "reject" => skeleton.outputs[0].data = b"reject".to_vec(),
                _ => {}
            }
            let result = verify(&rpc, skeleton, vec![]).await;
            if case == "valid" {
                result.expect("matching executable Process pair needs no user input/signature");
            } else if case == "reject" {
                assert_eq!(
                    exit(result.expect_err("Process type script must execute")),
                    42
                );
            } else {
                assert_eq!(exit(result.expect_err(case)), ERROR_PROCESS_MISMATCH);
            }
        }
    }
}

#[tokio::test]
async fn process_digest_batches_distinct_intentions_for_same_user() {
    use ckb_cinnabar_calculator::operation::basic::AddInputCellByOutPoint;
    let mut rpc = FakeRpcClient::default();
    seed_fakenet_xudt_cell(&mut rpc);
    let mut skeleton = prepared_skeleton(&rpc);
    let first = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"batch-first"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let second = seed_intention(
        &mut rpc,
        &skeleton,
        cid_args(process_type_hash(), b"batch-second"),
        CKB_CAPACITY,
        None,
        PACK_BLOCK,
    );
    let process = seed_process_cell(&mut rpc);
    TransactionCalculator::default()
        .instruction(digest_ok(
            outpoint_ids(&first),
            outpoint_ids(&process),
            None,
        ))
        .apply_skeleton(&rpc, &mut skeleton)
        .unwrap();
    let (tx_hash, index) = outpoint_ids(&second);
    let extra = Instruction::new(vec![Box::new(AddInputCellByOutPoint {
        tx_hash,
        index,
        since: None,
    })]);
    verify(&rpc, skeleton, vec![extra])
        .await
        .expect("Process pair can authorize distinct Intention groups; refund guard is separate");
}
