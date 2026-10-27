//! Off-chain CID v0.1 recipes. Each function is one transition-table row.
//!
//! Calculate assembles; it does not re-implement Verify predicates.

use ckb_cinnabar_calculator::{
    address::Address,
    instruction::Instruction,
    operation::{
        basic::{
            AddCellDep, AddHeaderDepByBlockNumber, AddHeaderDepByInputIndex, AddInputCellByAddress,
            AddInputCellByOutPoint, AddOutputCell,
        },
        udt::AddXudtCelldep,
        Operation,
    },
    rpc::RPC,
    skeleton::ScriptEx,
    types::{DepType, Hash256},
};
use cid_protocol::{CidArgs, Payload, Script as CidScript, XUDT_AMOUNT_LEN};

/// FakeRpc / skeleton name of this lock binary (`build/release/<name>`).
pub const CONTRACT_NAME: &str = "ckb_common_intention_dispatcher";

/// Build a CID lock that resolves against the named contract celldep.
pub fn cid_lock(args: Vec<u8>) -> ScriptEx {
    ScriptEx::Reference(CONTRACT_NAME.into(), args)
}

/// Encode lock args: Process Type Hash prefix + molecule(`CidEntry`).
pub fn encode_cid_args(
    process_type_hash: [u8; 32],
    user_lock: CidScript,
    payload: Payload,
) -> Vec<u8> {
    CidArgs::new(process_type_hash, user_lock, payload)
        .encode()
        .expect("cid args encode")
}

fn xudt_data(amount: u128) -> Vec<u8> {
    amount.to_le_bytes()[..XUDT_AMOUNT_LEN].to_vec()
}

/// Create an Intention cell. Lock Create does **not** run CID Verify.
///
/// `xudt`: when set, attach the official xUDT type + 16-byte amount and the
/// xUDT celldep. CKB-only path uses empty data and `type_script = None`.
pub fn create_intention<T: RPC>(
    payer: Address,
    args: Vec<u8>,
    capacity: u64,
    xudt: Option<(ScriptEx, u128)>,
) -> Instruction<T> {
    let mut ops: Vec<Box<dyn Operation<T>>> = vec![Box::new(AddInputCellByAddress { address: payer })];
    let (type_script, data) = match xudt {
        Some((type_script, amount)) => {
            ops.push(Box::new(AddXudtCelldep {}));
            (Some(type_script), xudt_data(amount))
        }
        None => (None, Vec::new()),
    };
    ops.push(Box::new(AddOutputCell {
        lock_script: cid_lock(args),
        type_script,
        data,
        capacity,
        absolute_capacity: true,
        type_id: false,
    }));
    Instruction::new(ops)
}

/// Destroy the Intention against a matching Process CellDep.
/// CID does not force a User Lock refund; `asset_output` is optional Process destination.
pub fn digest_ok<T: RPC>(
    intention: (Hash256, u32),
    process: (Hash256, u32),
    asset_output: Option<AddOutputCell>,
) -> Instruction<T> {
    let mut ops: Vec<Box<dyn Operation<T>>> = vec![
        Box::new(AddInputCellByOutPoint {
            tx_hash: intention.0,
            index: intention.1,
            since: None,
        }),
        Box::new(AddCellDep {
            name: "process".into(),
            tx_hash: process.0,
            index: process.1,
            dep_type: DepType::Code,
            with_data: true,
        }),
    ];
    if let Some(output) = &asset_output {
        if output.type_script.is_some() {
            ops.push(Box::new(AddXudtCelldep {}));
        }
    }
    if let Some(output) = asset_output {
        ops.push(Box::new(output));
    }
    Instruction::new(ops)
}

/// Incompatible digest: destroy + refund assets to User Lock (user not in inputs).
pub fn digest_incompatible<T: RPC>(
    intention: (Hash256, u32),
    user_lock: ScriptEx,
    capacity: u64,
    xudt: Option<(ScriptEx, u128)>,
) -> Instruction<T> {
    refund_burn(intention, user_lock, capacity, xudt, None, false)
}

/// Timeout reclaim: user lock in inputs + pack/judge headers + User Lock refund.
pub fn timeout_burn<T: RPC>(
    intention: (Hash256, u32),
    user: Address,
    user_lock: ScriptEx,
    capacity: u64,
    xudt: Option<(ScriptEx, u128)>,
    judge_block_number: u64,
) -> Instruction<T> {
    refund_burn(
        intention,
        user_lock,
        capacity,
        xudt,
        Some((user, judge_block_number)),
        true,
    )
}

fn refund_burn<T: RPC>(
    intention: (Hash256, u32),
    user_lock: ScriptEx,
    capacity: u64,
    xudt: Option<(ScriptEx, u128)>,
    timeout: Option<(Address, u64)>,
    add_pack_header: bool,
) -> Instruction<T> {
    let mut ops: Vec<Box<dyn Operation<T>>> = vec![Box::new(AddInputCellByOutPoint {
        tx_hash: intention.0,
        index: intention.1,
        since: None,
    })];
    if let Some((user, judge)) = timeout {
        ops.push(Box::new(AddInputCellByAddress { address: user }));
        if add_pack_header {
            ops.push(Box::new(AddHeaderDepByInputIndex { input_index: 0 }));
        }
        ops.push(Box::new(AddHeaderDepByBlockNumber {
            block_number: judge,
        }));
    }
    let (type_script, data) = match xudt {
        Some((type_script, amount)) => {
            ops.push(Box::new(AddXudtCelldep {}));
            (Some(type_script), xudt_data(amount))
        }
        None => (None, Vec::new()),
    };
    ops.push(Box::new(AddOutputCell {
        lock_script: user_lock,
        type_script,
        data,
        capacity,
        absolute_capacity: true,
        type_id: false,
    }));
    Instruction::new(ops)
}

pub use cid_protocol::{
    CidArgs, CidEntry, Payload, Script as CidScript, CID_VERSION, ERROR_ARGS_INVALID,
    ERROR_HEADER_MISSING, ERROR_PARTIAL_DIGEST, ERROR_PROCESS_MISMATCH, ERROR_REFUND_ASSET_MISMATCH,
    ERROR_REFUND_MISSING, ERROR_TIMEOUT_NOT_REACHED, ERROR_UNKNOWN_BRANCH, ERROR_UNSUPPORTED_TYPE,
    ERROR_ZERO_ASSET, HOP_DIGEST_INCOMPATIBLE, HOP_DIGEST_OK, HOP_TIMEOUT_BURN, TIMEOUT_BLOCKS,
    XUDT_CODE_HASH,
};
