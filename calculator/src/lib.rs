//! Off-chain CID v0.1 recipes. Each function is one transition-table row.
//!
//! Calculate assembles; it does not re-implement Verify predicates.

use cid_protocol::XUDT_AMOUNT_LEN;
use ckb_cinnabar_calculator::{
    address::Address,
    indexer::{CellQueryOptions, GetCellsIter, SearchMode},
    instruction::Instruction,
    operation::{
        basic::{
            AddHeaderDepByBlockNumber, AddHeaderDepByInputIndex, AddInputCellByAddress,
            AddInputCellByOutPoint, AddOutputCell,
        },
        udt::AddXudtCelldep,
        Log, Operation,
    },
    re_exports::ckb_types::prelude::Unpack,
    rpc::RPC,
    skeleton::{CellInputEx, CellOutputEx, ScriptEx, TransactionSkeleton},
    types::Hash256,
    CalculatorError, Result,
};

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
    cid_protocol::CidArgs::new(process_type_hash, user_lock, payload)
        .encode()
        .expect("cid args encode")
}

fn xudt_data(amount: u128) -> Vec<u8> {
    amount.to_le_bytes()[..XUDT_AMOUNT_LEN].to_vec()
}

// Collect by the full type script, not by issuer authority. Never mint tokens.
struct CollectXudt {
    payer: ScriptEx,
    type_script: ScriptEx,
    amount: u128,
}

impl<T: RPC> Operation<T> for CollectXudt {
    fn run(
        self: Box<Self>,
        rpc: &T,
        skeleton: &mut TransactionSkeleton,
        _: &mut Log,
    ) -> Result<()> {
        let typ = self.type_script.to_script(skeleton)?;
        let code_hash: [u8; 32] = typ.code_hash().unpack();
        if self.amount == 0 || !cid_protocol::is_standard_xudt(&code_hash, typ.hash_type().into()) {
            return Err(CalculatorError::Other(
                "expected nonzero standard xUDT transfer".into(),
            ));
        }
        let payer = self.payer.to_script(skeleton)?;
        let mut query = CellQueryOptions::new_lock(payer.clone());
        query.with_data = Some(true);
        query.script_search_mode = Some(SearchMode::Exact);
        query.secondary_script = Some(typ.clone());
        let mut cells = GetCellsIter::new(rpc, query.into());
        let mut collected = 0u128;
        let mut inputs = Vec::new();
        while let Some(cell) = cells.next()? {
            if skeleton
                .inputs
                .iter()
                .any(|i| i.input.previous_output() == cell.out_point)
                || inputs
                    .iter()
                    .any(|i: &CellInputEx| i.input.previous_output() == cell.out_point)
            {
                continue;
            }
            // Recheck indexer results locally, including exact type/lock equality.
            if cell.output.lock() != payer || cell.output.type_().to_opt() != Some(typ.clone()) {
                continue;
            }
            let bytes: [u8; 16] = cell
                .output_data
                .as_slice()
                .try_into()
                .map_err(|_| CalculatorError::Other("expected 16-byte xUDT amount".into()))?;
            collected = collected
                .checked_add(u128::from_le_bytes(bytes))
                .ok_or_else(|| CalculatorError::Other("xUDT amount overflow".into()))?;
            inputs.push(CellInputEx::new_from_live_cell(cell, None));
            if collected >= self.amount {
                break;
            }
        }
        if collected < self.amount {
            return Err(CalculatorError::NoAvailableCells(format!(
                "insufficient xUDT balance: need {}, collected {}",
                self.amount, collected
            )));
        }
        let change = if collected > self.amount {
            Some(CellOutputEx::new_from_scripts(
                payer,
                Some(typ),
                xudt_data(collected - self.amount),
                None,
            )?)
        } else {
            None
        };
        for input in inputs {
            skeleton.input(input)?.witness(Default::default());
        }
        if let Some(change) = change {
            skeleton.output(change);
        }
        Ok(())
    }
}

// Input assets need their type code even when the recipe emits no output.
struct AddInputAssetDeps;
impl<T: RPC> Operation<T> for AddInputAssetDeps {
    fn run(
        self: Box<Self>,
        rpc: &T,
        skeleton: &mut TransactionSkeleton,
        log: &mut Log,
    ) -> Result<()> {
        let has_xudt = skeleton.inputs.iter().any(|input| {
            input.output.type_script().is_some_and(|typ| {
                let hash: [u8; 32] = typ.code_hash().unpack();
                cid_protocol::is_standard_xudt(&hash, typ.hash_type().into())
            })
        });
        if has_xudt {
            Box::new(AddXudtCelldep {}).run(rpc, skeleton, log)?;
        }
        Ok(())
    }
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
    let mut ops: Vec<Box<dyn Operation<T>>> = vec![Box::new(AddInputCellByAddress {
        address: payer.clone(),
    })];
    let (type_script, data) = match xudt {
        Some((type_script, amount)) => {
            ops.push(Box::new(AddXudtCelldep {}));
            ops.push(Box::new(CollectXudt {
                payer: payer.payload().into(),
                type_script: type_script.clone(),
                amount,
            }));
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

// Consume the Process cell and recreate its state, rather than referencing it.
struct ConsumeProcess {
    process: (Hash256, u32),
}

impl<T: RPC> Operation<T> for ConsumeProcess {
    fn run(
        self: Box<Self>,
        rpc: &T,
        skeleton: &mut TransactionSkeleton,
        _: &mut Log,
    ) -> Result<()> {
        let input = CellInputEx::new_from_outpoint(rpc, self.process.0, self.process.1, None, true)?;
        if input.output.type_script().is_none() {
            return Err(CalculatorError::Other("Process cell must have a type script".into()));
        }
        let output = input.output.clone();
        skeleton.input(input)?.witness(Default::default());
        skeleton.output(output);
        Ok(())
    }
}

/// Consume the Intention and Process, and recreate the Process unchanged.
/// No input or signature from the Intention's User Lock is required.
/// The caller supplies code deps and witnesses for the Process lock/type scripts,
/// and may update the recreated Process state as those scripts permit.
/// `asset_output` may include partial user proceeds. Process code/data deps
/// remain ordinary CKB requirements; the live Process state is not a CellDep.
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
        Box::new(ConsumeProcess { process }),
    ];
    ops.push(Box::new(AddInputAssetDeps));
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
    ERROR_HEADER_MISSING, ERROR_PARTIAL_DIGEST, ERROR_PROCESS_MISMATCH,
    ERROR_REFUND_ASSET_MISMATCH, ERROR_REFUND_MISSING, ERROR_TIMEOUT_NOT_REACHED,
    ERROR_UNKNOWN_BRANCH, ERROR_UNSUPPORTED_TYPE, ERROR_ZERO_ASSET, HOP_DIGEST_INCOMPATIBLE,
    HOP_DIGEST_OK, HOP_TIMEOUT_BURN, TIMEOUT_BLOCKS, XUDT_CODE_HASH,
};
