//! Process Calculator for CID Process example

use std::collections::HashMap;

use cid_process_executor::{AssembledTx, HostQuery, MockHost, ProcessExecutor};
use cid_protocol::{CidArgs, TYPE_ID_CODE_HASH};
use ckb_cinnabar_calculator::{
    address::Address,
    instruction::Instruction,
    operation::{
        basic::{AddInputCellByAddress, AddInputCellByOutPoint, AddOutputCell},
        Log, Operation,
    },
    re_exports::ckb_types::{
        core::{DepType, ScriptHashType},
        packed::{CellDep, CellInput, CellOutput, OutPoint, RawTransaction, Script, Transaction},
        prelude::{Builder, Entity, Pack, PackVec, Unpack},
    },
    rpc::RPC,
    skeleton::{CellDepEx, CellInputEx, CellOutputEx, ScriptEx, TransactionSkeleton, WitnessEx},
    types::Hash256,
    CalculatorError, Result,
};

pub const PROCESS_CONTRACT_NAME: &str = "cid_process_example";

/// Default Process state cell capacity: 300 CKB.
pub const DEFAULT_PROCESS_STATE_CAPACITY: u64 = 300_0000_0000;

/// Example Process minimum CKB deposit (shannons): **600 CKB**.
///
/// Process-owned (not protocol). Single source:
/// `crates/cid-process-example-policy`.
pub use cid_process_example_policy::{ckb_deposit_capacity_ok, MIN_DEPOSIT_CAPACITY};

pub use cid_intention_filter::{
    intention_belongs_to_process, intention_process_type_id, FilterCell, FilterRequest,
    IntentionRouteError, FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM, FILTER_MALFORMED_CELL,
    FILTER_MALFORMED_DIGEST, FILTER_RESERVED_2, FILTER_RESERVED_3, FILTER_UNSUPPORTED_ACTION,
    FILTER_UNSUPPORTED_ASSET,
};

/// Fee rate (shannons/KW) used when balancing Process state create txs.
/// Matches testnet `min_fee_rate` and the example Process assemble fee rate.
pub const CREATE_FEE_RATE: u64 = 1000;

/// Build a Process lock that resolves against the named Process code celldep
/// (`hash_type=type` when the dep carries a Type ID type script).
pub fn process_lock(args: Vec<u8>) -> ScriptEx {
    ScriptEx::Reference(PROCESS_CONTRACT_NAME.into(), args)
}

/// Create a Process state cell (address-scanned funding).
///
/// Layout: Process code Type ID lock (`hash_type=type`) + built-in TYPE_ID type
/// (args = blake2b(first_input ‖ le64(out_index))), empty data.
///
/// The instruction validates the Process code dependency and state lock before
/// adding funding inputs. The Process lock does not run on-chain for output-only mints.
pub fn create_process_state<T: RPC>(
    payer: Address,
    lock_args: Vec<u8>,
    capacity: u64,
) -> Instruction<T> {
    create_process_state_instruction(
        Box::new(AddInputCellByAddress { address: payer }),
        lock_args,
        capacity,
    )
}

/// Create a Process state cell funded by an explicit outpoint (CLI / offline).
///
/// Uses the same layout and off-chain checks as the address-scanned entry.
pub fn create_process_state_by_outpoint<T: RPC>(
    funding: (Hash256, u32),
    lock_args: Vec<u8>,
    capacity: u64,
) -> Instruction<T> {
    create_process_state_instruction(
        Box::new(AddInputCellByOutPoint {
            tx_hash: funding.0,
            index: funding.1,
            since: None,
        }),
        lock_args,
        capacity,
    )
}

fn create_process_state_instruction<T: RPC>(
    funding_input: Box<dyn Operation<T>>,
    lock_args: Vec<u8>,
    capacity: u64,
) -> Instruction<T> {
    let lock_script = process_lock(lock_args);
    Instruction::new(vec![
        Box::new(ValidateProcessCreate {
            lock_script: lock_script.clone(),
            capacity,
        }),
        funding_input,
        Box::new(AddOutputCell {
            lock_script,
            type_script: None,
            data: Vec::new(),
            capacity,
            absolute_capacity: true,
            type_id: true,
        }),
    ])
}

struct ValidateProcessCreate {
    lock_script: ScriptEx,
    capacity: u64,
}

impl<T: RPC> Operation<T> for ValidateProcessCreate {
    fn run(
        self: Box<Self>,
        _rpc: &T,
        skeleton: &mut TransactionSkeleton,
        _log: &mut Log,
    ) -> Result<()> {
        let args = self.lock_script.args();
        if args.is_empty() || CidArgs::decode(&args).is_ok() {
            return Err(CalculatorError::Other(
                "Process lock args must be non-empty and distinct from CID Intention encoding"
                    .into(),
            ));
        }

        let (_, dep) = skeleton
            .find_celldep_by_script(&self.lock_script)
            .ok_or_else(|| {
                CalculatorError::CellDepNotFound("Process code cell dep is required".into())
            })?;
        let dep_type: u8 = dep.celldep.dep_type().into();
        let type_script = dep.output.type_script().filter(|script| {
            let hash_type: u8 = script.hash_type().into();
            let code_hash: [u8; 32] = script.code_hash().unpack();
            hash_type == ScriptHashType::Type as u8
                && code_hash == TYPE_ID_CODE_HASH
                && script.args().raw_data().len() == 32
        });
        let Some(type_script) = type_script.filter(|_| dep_type == DepType::Code as u8) else {
            return Err(CalculatorError::Other(
                "Process code dep must be a direct code cell with a built-in TYPE_ID script".into(),
            ));
        };

        // The state TYPE_ID has the same occupied length as this code-cell TYPE_ID.
        let lock = self.lock_script.clone().to_script(skeleton)?;
        let occupied = CellOutputEx::new_from_scripts(lock, Some(type_script), vec![], None)?
            .occupied_capacity()
            .as_u64();
        // At rate 1000, reserve a baseline of 1000 shannons plus one shannon per
        // state lock arg byte. Larger future digest transactions can cost more.
        let fee_reserve = CREATE_FEE_RATE + args.len() as u64;
        if self.capacity.saturating_sub(occupied) < fee_reserve {
            return Err(CalculatorError::InsufficientCapacity(format!(
                "Process state needs at least {fee_reserve} shannons above occupied capacity for digest fees"
            )));
        }
        Ok(())
    }
}

type KnownCells = HashMap<Vec<u8>, (CellOutput, Vec<u8>)>;

/// Intention cell reference for digest
#[derive(Clone, Debug)]
pub struct IntentionCell {
    pub out_point: OutPoint,
    pub lock_args: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
    /// Packed molecule `Script` of the intention type, if any (official xUDT).
    pub type_script: Option<Vec<u8>>,
    /// Full CID lock script. Required for FakeRpc/VM verify of the assembled
    /// skeleton; SSRI-only assemble paths may leave this `None` (placeholder
    /// code_hash is used for host cell shape only).
    pub lock: Option<Script>,
}

/// Process state cell for full-tx assemble.
#[derive(Clone, Debug)]
pub struct ProcessStateCell {
    pub out_point: OutPoint,
    pub lock: Script,
    pub type_script: Script,
    pub capacity: u64,
    pub data: Vec<u8>,
}

/// Live cell payload for a draft cell_dep (out-point is in `cell_dep`).
///
/// Molecule Transactions only carry the `CellDep` out-point; FakeRpc / VM
/// verify of a packed-rebuilt skeleton still needs the dep cell bytes, which
/// the chain resolves from live state. Callers pass them here so the submit
/// skeleton can hydrate deps without rewriting packed bytes.
#[derive(Clone, Debug)]
pub struct DraftCellDep {
    pub cell_dep: CellDep,
    pub output: CellOutput,
    pub data: Vec<u8>,
}

/// Additional draft inputs and required code dependencies.
///
/// `cell_deps` must be non-empty on the submit path (CID lock code, Process
/// code, and any xUDT code). The example Process requires empty funding inputs.
#[derive(Clone, Debug, Default)]
pub struct AssembleExtras {
    pub funding_cells: Vec<ProcessStateCell>,
    pub cell_deps: Vec<DraftCellDep>,
}

fn require_submit_cell_deps(extras: &AssembleExtras) -> Result<()> {
    if extras.cell_deps.is_empty() {
        return Err(CalculatorError::Other(
            "cell_deps required for submit: pass CID, Process, and any xUDT code deps \
             via process_digest_ok_with_extras (or process_digest_ok_with_process)"
                .into(),
        ));
    }
    Ok(())
}

/// Load the Process binary from build/release/<name>
pub fn load_process_binary(bin_dir: &str) -> Result<Vec<u8>> {
    let path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
    std::fs::read(&path).map_err(|e| {
        CalculatorError::Other(format!("failed to load Process binary at {}: {}", path, e))
    })
}

/// Build a [`FilterCell`] from an [`IntentionCell`] (packed out_point + output + data).
pub fn intention_to_filter_cell(intention: &IntentionCell) -> Result<FilterCell> {
    let output = intention_output(intention)?;
    Ok(FilterCell {
        out_point: intention.out_point.as_slice().to_vec(),
        output: output.as_slice().to_vec(),
        data: intention.data.clone(),
    })
}

/// Call SSRI `Process.filter` for Intentions already routed to one Process.
///
/// Returns one reason code per input, same order. Judges Process **business**
/// compliance only — callers must route with [`intention_process_type_id`] /
/// [`intention_belongs_to_process`] first.
///
/// Sequencer flow: scan → route → this filter → assemble accepted cells.
pub fn process_filter(intentions: &[IntentionCell], bin_dir: &str) -> Result<Vec<i8>> {
    let mut cells = Vec::with_capacity(intentions.len());
    for intention in intentions {
        cells.push(intention_to_filter_cell(intention)?);
    }
    process_filter_cells(&cells, bin_dir)
}

/// Like [`process_filter`] but takes pre-encoded [`FilterCell`]s.
pub fn process_filter_cells(cells: &[FilterCell], bin_dir: &str) -> Result<Vec<i8>> {
    let binary = load_process_binary(bin_dir)?;
    let executor = ProcessExecutor::new(binary);
    let req = FilterRequest {
        cells: cells.to_vec(),
    };
    let bytes = cid_intention_filter::encode_filter_request(&req)
        .map_err(|e| CalculatorError::Other(format!("encode filter request: {:?}", e)))?;
    executor
        .filter(&bytes)
        .map_err(|e| CalculatorError::Other(format!("Process.filter: {e}")))
}

fn intention_output(intention: &IntentionCell) -> Result<CellOutput> {
    let lock = intention.lock.clone().unwrap_or_else(|| {
        Script::new_builder()
            .code_hash([0xC1; 32].pack())
            .hash_type(ScriptHashType::Type)
            .args(intention.lock_args.clone().pack())
            .build()
    });
    let mut builder = CellOutput::new_builder()
        .capacity(intention.capacity)
        .lock(lock);
    if let Some(bytes) = &intention.type_script {
        let typ = Script::from_slice(bytes)
            .map_err(|e| CalculatorError::Other(format!("type script: {:?}", e)))?;
        builder = builder.type_(Some(typ).pack());
    }
    Ok(builder.build())
}

fn process_output(process: &ProcessStateCell) -> CellOutput {
    CellOutput::new_builder()
        .capacity(process.capacity)
        .lock(process.lock.clone())
        .type_(Some(process.type_script.clone()).pack())
        .build()
}

/// Build draft molecule bytes + known cell payloads for skeleton hydration.
fn build_draft_and_known(
    intentions: &[IntentionCell],
    process: &ProcessStateCell,
    extras: &AssembleExtras,
) -> Result<(Vec<u8>, KnownCells)> {
    if !extras.funding_cells.is_empty() {
        return Err(CalculatorError::Other(
            "the example Process does not support extra funding inputs".into(),
        ));
    }
    for intention in intentions {
        let has_type = intention.type_script.is_some();
        if !ckb_deposit_capacity_ok(intention.capacity, has_type) {
            return Err(CalculatorError::Other(format!(
                "CKB deposit Intention capacity {} is below Process MIN_DEPOSIT_CAPACITY {}",
                intention.capacity, MIN_DEPOSIT_CAPACITY
            )));
        }
    }
    let mut known = KnownCells::new();
    let mut inputs = Vec::with_capacity(1 + intentions.len());

    let process_out = process_output(process);
    known.insert(
        process.out_point.as_slice().to_vec(),
        (process_out, process.data.clone()),
    );
    inputs.push(
        CellInput::new_builder()
            .previous_output(process.out_point.clone())
            .build(),
    );

    for intention in intentions {
        let output = intention_output(intention)?;
        known.insert(
            intention.out_point.as_slice().to_vec(),
            (output.clone(), intention.data.clone()),
        );
        inputs.push(
            CellInput::new_builder()
                .previous_output(intention.out_point.clone())
                .build(),
        );
    }

    let mut cell_deps = Vec::with_capacity(extras.cell_deps.len());
    for dep in &extras.cell_deps {
        let key = dep.cell_dep.out_point().as_slice().to_vec();
        known.insert(key, (dep.output.clone(), dep.data.clone()));
        cell_deps.push(dep.cell_dep.clone());
    }

    let draft = Transaction::new_builder()
        .raw(
            RawTransaction::new_builder()
                .inputs(inputs.pack())
                .cell_deps(cell_deps.pack())
                .build(),
        )
        .build()
        .as_slice()
        .to_vec();
    Ok((draft, known))
}

fn seed_mock_from_known(known: &KnownCells) -> Result<MockHost> {
    let host = MockHost::new();
    host.insert_block_hash(0, [0xAB; 32]);
    for (op, (output, data)) in known {
        host.insert_cell_bytes(op, output.as_slice(), data.clone())
            .map_err(CalculatorError::Other)?;
    }
    Ok(host)
}

fn build_draft_and_host(
    intentions: &[IntentionCell],
    process: &ProcessStateCell,
    extras: &AssembleExtras,
) -> Result<(Vec<u8>, MockHost, KnownCells)> {
    let (draft, known) = build_draft_and_known(intentions, process, extras)?;
    let host = seed_mock_from_known(&known)?;
    Ok((draft, host, known))
}

fn assemble_with_host<T: RPC, H: HostQuery + Send + 'static>(
    intentions: &[IntentionCell],
    process: &ProcessStateCell,
    extras: &AssembleExtras,
    host: H,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, AssembledTx, Vec<Instruction<T>>)> {
    let binary = load_process_binary(bin_dir)?;
    let executor = ProcessExecutor::new(binary);

    let (min, max) = executor
        .get_batch_bounds()
        .map_err(|e| CalculatorError::Other(format!("failed to get batch bounds: {}", e)))?;

    let count = intentions.len() as u32;
    if count < min || count > max {
        return Err(CalculatorError::Other(format!(
            "intention count {} out of bounds [{}, {}]",
            count, min, max
        )));
    }

    require_submit_cell_deps(extras)?;
    require_process_matches_intentions(process, intentions)?;
    let (draft, known) = build_draft_and_known(intentions, process, extras)?;
    let assembled = executor
        .assemble(&draft, host)
        .map_err(|e| CalculatorError::Other(format!("executor assemble failed: {}", e)))?;

    let skeleton = skeleton_from_assembled(&assembled, &known)?;
    Ok((skeleton, assembled, vec![]))
}

/// Resolve a live Process state cell whose **type** script equals `type_script`
/// (exact indexer match). Errors explicitly when no live cell is found.
pub fn resolve_process_state_by_type<T: RPC>(
    rpc: &T,
    type_script: &Script,
) -> Result<ProcessStateCell> {
    use ckb_cinnabar_calculator::indexer::{CellQueryOptions, GetCellsIter, SearchMode};

    let code: [u8; 32] = {
        use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Unpack;
        type_script.code_hash().unpack()
    };
    if code != TYPE_ID_CODE_HASH {
        return Err(CalculatorError::Other(
            "process type script must be built-in TYPE_ID".into(),
        ));
    }

    let mut query = CellQueryOptions::new_type(type_script.clone());
    query.script_search_mode = Some(SearchMode::Exact);
    let mut iter = GetCellsIter::new(rpc, query.into());
    let live = iter
        .next()?
        .ok_or_else(|| CalculatorError::Other("no live Process state cell for Type ID".into()))?;

    let typ =
        live.output.type_().to_opt().ok_or_else(|| {
            CalculatorError::Other("Process state cell missing type script".into())
        })?;

    Ok(ProcessStateCell {
        out_point: live.out_point,
        lock: live.output.lock(),
        type_script: typ,
        capacity: {
            use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Unpack;
            live.output.capacity().unpack()
        },
        data: live.output_data,
    })
}

fn require_process_matches_intentions(
    process: &ProcessStateCell,
    intentions: &[IntentionCell],
) -> Result<()> {
    use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Unpack;
    let pth: [u8; 32] = process.type_script.calc_script_hash().unpack();
    for intention in intentions {
        let cid = CidArgs::decode(&intention.lock_args)
            .map_err(|e| CalculatorError::Other(format!("intention lock args: {:?}", e)))?;
        if cid.process_type_hash != pth {
            return Err(CalculatorError::Other(
                "Intention process_type_hash does not match Process state TYPE_ID".into(),
            ));
        }
    }
    Ok(())
}

/// Process digest without a live state cell — always errors after batch bounds.
///
/// Callers must use [`process_digest_ok_with_extras`] (submit) or
/// [`process_digest_ok_with_process`] with a live [`ProcessStateCell`] and
/// non-empty code deps, or [`resolve_process_state_by_type`] via RPC.
pub fn process_digest_ok<T: RPC>(
    _rpc: &T,
    intentions: Vec<IntentionCell>,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, Vec<Instruction<T>>)> {
    let binary = load_process_binary(bin_dir)?;
    let executor = ProcessExecutor::new(binary);
    let (min, max) = executor
        .get_batch_bounds()
        .map_err(|e| CalculatorError::Other(format!("failed to get batch bounds: {}", e)))?;
    let count = intentions.len() as u32;
    if count < min || count > max {
        return Err(CalculatorError::Other(format!(
            "intention count {} out of bounds [{}, {}]",
            count, min, max
        )));
    }
    Err(CalculatorError::Other(
        "Process state cell required: use process_digest_ok_with_process with a live cell, \
         or resolve_process_state_by_type via RPC"
            .into(),
    ))
}

/// Shape-check / offline helper with required code deps.
///
/// Prefer [`process_digest_ok_with_extras`] for submit (returns packed
/// [`AssembledTx`]). This entry cannot silently emit a dep-less tx: `cell_deps`
/// must be non-empty (CID, Process, and any xUDT code).
pub fn process_digest_ok_with_process<T: RPC>(
    rpc: &T,
    intentions: Vec<IntentionCell>,
    process: ProcessStateCell,
    cell_deps: Vec<DraftCellDep>,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, Vec<Instruction<T>>)> {
    let (skeleton, _assembled, instructions) = process_digest_ok_with_extras(
        rpc,
        intentions,
        process,
        AssembleExtras {
            funding_cells: vec![],
            cell_deps,
        },
        bin_dir,
    )?;
    Ok((skeleton, instructions))
}

/// Documented submit entry with live Process state and required `cell_deps`.
/// Set `funding_cells` to an empty vector for the example Process.
///
/// Seeds an offline [`MockHost`] from the draft cells. For a live RPC/indexer
/// host, use [`process_digest_ok_with_host`] instead.
///
/// Returns the skeleton (witnesses preserved as raw bytes), the packed
/// [`AssembledTx`] for byte-identical submit, and instructions (empty).
/// Rejects empty `cell_deps` so a dep-less tx cannot be produced silently.
pub fn process_digest_ok_with_extras<T: RPC>(
    _rpc: &T,
    intentions: Vec<IntentionCell>,
    process: ProcessStateCell,
    extras: AssembleExtras,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, AssembledTx, Vec<Instruction<T>>)> {
    let (_draft, host, _known) = build_draft_and_host(&intentions, &process, &extras)?;
    // Re-run through the generic host path (MockHost implements HostQuery).
    assemble_with_host(&intentions, &process, &extras, host, bin_dir)
}

/// Submit assemble with a caller-supplied [`HostQuery`] (RPC-backed or custom).
///
/// `host` must resolve draft out points / tip queries interactively during VM
/// execution. Draft shape and `cell_deps` rules match
/// [`process_digest_ok_with_extras`].
pub fn process_digest_ok_with_host<T: RPC, H: HostQuery + Send + 'static>(
    _rpc: &T,
    intentions: Vec<IntentionCell>,
    process: ProcessStateCell,
    extras: AssembleExtras,
    host: H,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, AssembledTx, Vec<Instruction<T>>)> {
    assemble_with_host(&intentions, &process, &extras, host, bin_dir)
}

/// Rebuild a skeleton from the assembled molecule Transaction **as-is**.
/// Witnesses are kept as raw packed bytes (`WitnessEx::new_plain`) so
/// `Some(empty)` WitnessArgs fields are not collapsed to `None` by Cinnabar.
fn skeleton_from_assembled(
    assembled: &AssembledTx,
    known: &KnownCells,
) -> Result<TransactionSkeleton> {
    let tx = Transaction::from_slice(assembled.as_molecule_bytes())
        .map_err(|e| CalculatorError::Other(format!("Transaction molecule bridge: {:?}", e)))?;
    let raw = tx.raw();
    if raw.outputs().len() != raw.outputs_data().len() {
        return Err(CalculatorError::Other(
            "assemble Transaction outputs / outputs_data length mismatch".into(),
        ));
    }

    let mut skeleton = TransactionSkeleton::from_packed_transaction(&tx);
    // Preserve guest witness bytes exactly (submit path must not rewrite).
    skeleton.witnesses.clear();
    for witness in tx.witnesses().into_iter() {
        let raw_bytes = witness.raw_data().to_vec();
        if raw_bytes.is_empty() {
            skeleton.witness(WitnessEx::default());
        } else {
            skeleton.witness(WitnessEx::new_plain(raw_bytes));
        }
    }
    for input in &mut skeleton.inputs {
        let key = input.input.previous_output().as_slice().to_vec();
        if let Some((output, data)) = known.get(&key) {
            *input = CellInputEx::new(input.input.clone(), output.clone(), Some(data.clone()));
        }
    }
    // Hydrate cell_dep payloads from known live cells (same out-points as the
    // packed tx). Does not change molecule bytes — only FakeRpc/VM metadata.
    for dep in &mut skeleton.celldeps {
        let key = dep.celldep.out_point().as_slice().to_vec();
        if let Some((output, data)) = known.get(&key) {
            *dep = CellDepEx::new(
                dep.name.clone(),
                dep.celldep.clone(),
                output.clone(),
                Some(data.clone()),
            );
        }
    }

    Ok(skeleton)
}

/// Byte-for-byte round-trip: packed `Transaction` → skeleton → packed, with
/// explicit empty `WitnessArgs` fields preserved via plain witnesses.
pub fn assembled_tx_round_trip_bytes(assembled: &AssembledTx) -> Result<Vec<u8>> {
    let skeleton = skeleton_from_assembled(assembled, &KnownCells::new())?;
    Ok(skeleton.into_packed_transaction().as_slice().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cid_protocol::{CidArgs, Payload, Script as CidScript};
    use ckb_cinnabar_calculator::operation::basic::AddCellDep;
    use ckb_cinnabar_calculator::re_exports::ckb_types::core::DepType;
    use ckb_cinnabar_calculator::re_exports::ckb_types::packed::{
        Bytes as PackedBytes, WitnessArgs,
    };
    use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Unpack;
    use ckb_cinnabar_calculator::re_exports::ckb_types::H256;
    use ckb_cinnabar_calculator::simulation::{
        always_success_script, fake_outpoint, FakeRpcClient,
    };
    use ckb_cinnabar_calculator::skeleton::CellOutputEx;
    use ckb_cinnabar_calculator::types::{h256_to_hash, DepType as CalcDepType};
    use ckb_cinnabar_calculator::TransactionCalculator;

    fn sample_type_id() -> Script {
        Script::new_builder()
            .code_hash(TYPE_ID_CODE_HASH.pack())
            .hash_type(ScriptHashType::Type)
            .args(
                {
                    let mut a = [0u8; 32];
                    a[..18].copy_from_slice(b"process-type-id-v0");
                    a.to_vec()
                }
                .pack(),
            )
            .build()
    }

    fn sample_code_dep() -> DraftCellDep {
        let dep_out = fake_outpoint();
        DraftCellDep {
            cell_dep: CellDep::new_builder()
                .out_point(dep_out)
                .dep_type(DepType::Code)
                .build(),
            output: CellOutput::new_builder()
                .capacity(10_000_000_000u64)
                .lock(
                    Script::new_builder()
                        .code_hash([0xDD; 32].pack())
                        .hash_type(ScriptHashType::Data1)
                        .build(),
                )
                .build(),
            data: vec![0xCA, 0xFE],
        }
    }

    fn sample_beneficiary_payload() -> Payload {
        Payload::from_script(&CidScript {
            code_hash: [0xBE; 32],
            hash_type: 1,
            args: vec![0x01],
        })
        .unwrap()
    }

    fn sample_process_and_intention() -> (ProcessStateCell, Vec<IntentionCell>) {
        let type_id = sample_type_id();
        let pth: [u8; 32] = type_id.calc_script_hash().unpack();
        let lock_args = CidArgs::new(pth, CidScript::default(), sample_beneficiary_payload())
            .encode()
            .unwrap();
        let process = ProcessStateCell {
            out_point: fake_outpoint(),
            lock: Script::new_builder()
                .code_hash([0xAA; 32].pack())
                .hash_type(ScriptHashType::Data1)
                .build(),
            type_script: type_id,
            capacity: 200_000_000_000,
            data: vec![],
        };
        let intentions = vec![IntentionCell {
            out_point: fake_outpoint(),
            lock_args,
            capacity: 100_000_000_000,
            data: vec![],
            type_script: None,
            lock: None,
        }];
        (process, intentions)
    }

    #[test]
    fn test_load_binary_missing() {
        assert!(load_process_binary("/nonexistent").is_err());
    }

    #[test]
    fn test_create_process_state_by_outpoint_type_id_and_lock() {
        use ckb_cinnabar_calculator::TYPE_ID_CODE_HASH as CINNABAR_TYPE_ID;

        let funding_tx = [0x11u8; 32];
        let process_code_tx = [0x22u8; 32];
        let process_code_args = [0xABu8; 32];
        let lock_args = b"instance-1".to_vec();

        let process_type_script = Script::new_builder()
            .code_hash(CINNABAR_TYPE_ID.pack())
            .hash_type(ScriptHashType::Type)
            .args(process_code_args.to_vec().pack())
            .build();
        let expected_lock_code: [u8; 32] = process_type_script.calc_script_hash().unpack();

        let mut rpc = FakeRpcClient::default();
        let funding_lock = always_success_script(vec![0x01]);
        let funding_cell =
            CellOutputEx::new_from_scripts(funding_lock, None, vec![], Some(500_0000_0000u64))
                .unwrap();
        rpc.insert_fake_cell(
            OutPoint::new(H256(funding_tx).pack(), 0),
            funding_cell,
            None,
        );
        let code_cell = CellOutputEx::new_from_scripts(
            Script::default(),
            Some(process_type_script),
            vec![0xCAu8, 0xFE],
            None,
        )
        .unwrap();
        rpc.insert_fake_cell(
            OutPoint::new(H256(process_code_tx).pack(), 0),
            code_cell,
            None,
        );

        let mut skeleton = TransactionSkeleton::default();
        let add_dep = Instruction::new(vec![Box::new(AddCellDep {
            name: PROCESS_CONTRACT_NAME.into(),
            tx_hash: h256_to_hash(&H256(process_code_tx)),
            index: 0,
            dep_type: CalcDepType::Code,
            with_data: true,
        })]);
        TransactionCalculator::default()
            .instruction(add_dep)
            .apply_skeleton(&rpc, &mut skeleton)
            .expect("add process code dep");

        let create = create_process_state_by_outpoint(
            (h256_to_hash(&H256(funding_tx)), 0),
            lock_args.clone(),
            DEFAULT_PROCESS_STATE_CAPACITY,
        );
        TransactionCalculator::default()
            .instruction(create)
            .apply_skeleton(&rpc, &mut skeleton)
            .expect("create process state");

        assert_eq!(skeleton.inputs.len(), 1);
        assert_eq!(skeleton.outputs.len(), 1);
        let out = &skeleton.outputs[0];
        assert_eq!(out.capacity_shannons(), DEFAULT_PROCESS_STATE_CAPACITY);
        assert!(out.data.is_empty());

        let lock = out.lock_script();
        let lock_code: [u8; 32] = lock.code_hash().unpack();
        let lock_ht: u8 = lock.hash_type().into();
        assert_eq!(lock_ht, 1, "Process lock must use hash_type=type");
        assert_eq!(lock_code, expected_lock_code);
        assert_eq!(lock.args().raw_data().as_ref(), lock_args.as_slice());

        let typ = out.type_script().expect("TYPE_ID type required");
        let typ_code: [u8; 32] = typ.code_hash().unpack();
        assert_eq!(typ_code, TYPE_ID_CODE_HASH);
        let typ_ht: u8 = typ.hash_type().into();
        assert_eq!(typ_ht, 1);
        assert_eq!(typ.args().raw_data().len(), 32);

        let expected_args = skeleton.calc_type_id(0).expect("type id");
        let got_args: [u8; 32] = {
            let mut a = [0u8; 32];
            a.copy_from_slice(&typ.args().raw_data());
            a
        };
        assert_eq!(got_args, expected_args);
    }

    #[test]
    fn calculator_rejects_untyped_process_code() {
        let funding_tx = [0x11u8; 32];
        let process_code_tx = [0x22u8; 32];
        let lock_args = b"instance-1".to_vec();

        let mut rpc = FakeRpcClient::default();
        let funding_lock = always_success_script(vec![0x01]);
        let funding_cell =
            CellOutputEx::new_from_scripts(funding_lock, None, vec![], Some(500_0000_0000u64))
                .unwrap();
        rpc.insert_fake_cell(
            OutPoint::new(H256(funding_tx).pack(), 0),
            funding_cell,
            None,
        );
        let code_cell =
            CellOutputEx::new_from_scripts(Script::default(), None, vec![0xCAu8, 0xFE], None)
                .unwrap();
        rpc.insert_fake_cell(
            OutPoint::new(H256(process_code_tx).pack(), 0),
            code_cell,
            None,
        );

        let mut skeleton = TransactionSkeleton::default();
        let add_dep = Instruction::new(vec![Box::new(AddCellDep {
            name: PROCESS_CONTRACT_NAME.into(),
            tx_hash: h256_to_hash(&H256(process_code_tx)),
            index: 0,
            dep_type: CalcDepType::Code,
            with_data: true,
        })]);
        TransactionCalculator::default()
            .instruction(add_dep)
            .apply_skeleton(&rpc, &mut skeleton)
            .expect("add process code dep");

        let create = create_process_state_by_outpoint(
            (h256_to_hash(&H256(funding_tx)), 0),
            lock_args.clone(),
            DEFAULT_PROCESS_STATE_CAPACITY,
        );
        let result = TransactionCalculator::default()
            .instruction(create)
            .apply_skeleton(&rpc, &mut skeleton);
        if result.is_ok() {
            let hash_type: u8 = skeleton.outputs[0].lock_script().hash_type().into();
            panic!("untyped code produced Process output with hash_type={hash_type}");
        }
    }

    #[test]
    fn test_create_process_state_type_id_matches_independent_blake2b() {
        use ckb_cinnabar_calculator::re_exports::ckb_hash::Blake2bBuilder;
        use ckb_cinnabar_calculator::TYPE_ID_CODE_HASH as CINNABAR_TYPE_ID;

        let funding_tx = [0x31u8; 32];
        let process_code_tx = [0x32u8; 32];
        let process_type_script = Script::new_builder()
            .code_hash(CINNABAR_TYPE_ID.pack())
            .hash_type(ScriptHashType::Type)
            .args([0x99u8; 32].to_vec().pack())
            .build();

        let mut rpc = FakeRpcClient::default();
        rpc.insert_fake_cell(
            OutPoint::new(H256(funding_tx).pack(), 0),
            CellOutputEx::new_from_scripts(
                always_success_script(vec![0x02]),
                None,
                vec![],
                Some(500_0000_0000u64),
            )
            .unwrap(),
            None,
        );
        rpc.insert_fake_cell(
            OutPoint::new(H256(process_code_tx).pack(), 0),
            CellOutputEx::new_from_scripts(
                Script::default(),
                Some(process_type_script),
                vec![0xBE, 0xEF],
                None,
            )
            .unwrap(),
            None,
        );

        let mut skeleton = TransactionSkeleton::default();
        TransactionCalculator::default()
            .instruction(Instruction::new(vec![Box::new(AddCellDep {
                name: PROCESS_CONTRACT_NAME.into(),
                tx_hash: h256_to_hash(&H256(process_code_tx)),
                index: 0,
                dep_type: CalcDepType::Code,
                with_data: true,
            })]))
            .apply_skeleton(&rpc, &mut skeleton)
            .unwrap();
        TransactionCalculator::default()
            .instruction(create_process_state_by_outpoint(
                (h256_to_hash(&H256(funding_tx)), 0),
                b"tid-check".to_vec(),
                DEFAULT_PROCESS_STATE_CAPACITY,
            ))
            .apply_skeleton(&rpc, &mut skeleton)
            .unwrap();

        let first_input = &skeleton.inputs[0].input;
        let out_index = 0u64;
        let mut hasher = Blake2bBuilder::new(32)
            .personal(b"ckb-default-hash")
            .build();
        hasher.update(first_input.as_slice());
        hasher.update(&out_index.to_le_bytes());
        let mut independent = [0u8; 32];
        hasher.finalize(&mut independent);

        let typ = skeleton.outputs[0].type_script().unwrap();
        let mut got = [0u8; 32];
        got.copy_from_slice(&typ.args().raw_data());
        assert_eq!(
            got, independent,
            "TYPE_ID args must equal blake2b(first CellInput molecule || le64(out_index))"
        );
        assert_eq!(got, skeleton.calc_type_id(0).unwrap());
    }

    #[test]
    fn test_create_process_state_address_scanned() {
        use ckb_cinnabar_calculator::address::{Address, AddressPayload};
        use ckb_cinnabar_calculator::rpc::Network;
        use ckb_cinnabar_calculator::TYPE_ID_CODE_HASH as CINNABAR_TYPE_ID;

        let funding_lock = always_success_script(vec![0x03]);
        let address = Address::new(Network::Fake, AddressPayload::from(funding_lock.clone()));
        let process_code_tx = [0x42u8; 32];
        let process_type_script = Script::new_builder()
            .code_hash(CINNABAR_TYPE_ID.pack())
            .hash_type(ScriptHashType::Type)
            .args([0x77u8; 32].to_vec().pack())
            .build();
        let expected_lock_code: [u8; 32] = process_type_script.calc_script_hash().unpack();

        let mut rpc = FakeRpcClient::default();
        // Address scan finds this live cell by lock.
        rpc.insert_fake_cell(
            OutPoint::new(H256([0x55u8; 32]).pack(), 0),
            CellOutputEx::new_from_scripts(funding_lock, None, vec![], Some(500_0000_0000u64))
                .unwrap(),
            None,
        );
        rpc.insert_fake_cell(
            OutPoint::new(H256(process_code_tx).pack(), 0),
            CellOutputEx::new_from_scripts(
                Script::default(),
                Some(process_type_script),
                vec![0x11, 0x22],
                None,
            )
            .unwrap(),
            None,
        );

        let mut skeleton = TransactionSkeleton::default();
        TransactionCalculator::default()
            .instruction(Instruction::new(vec![Box::new(AddCellDep {
                name: PROCESS_CONTRACT_NAME.into(),
                tx_hash: h256_to_hash(&H256(process_code_tx)),
                index: 0,
                dep_type: CalcDepType::Code,
                with_data: true,
            })]))
            .apply_skeleton(&rpc, &mut skeleton)
            .unwrap();
        TransactionCalculator::default()
            .instruction(create_process_state(
                address,
                b"addr-instance".to_vec(),
                DEFAULT_PROCESS_STATE_CAPACITY,
            ))
            .apply_skeleton(&rpc, &mut skeleton)
            .expect("address-scanned create_process_state");

        assert_eq!(skeleton.inputs.len(), 1);
        assert_eq!(skeleton.outputs.len(), 1);
        let lock = skeleton.outputs[0].lock_script();
        let lock_code: [u8; 32] = lock.code_hash().unpack();
        assert_eq!(u8::from(lock.hash_type()), 1);
        assert_eq!(lock_code, expected_lock_code);
        let typ = skeleton.outputs[0].type_script().expect("TYPE_ID");
        assert_eq!(typ.args().raw_data().len(), 32);
    }

    #[test]
    fn test_ckb_deposit_capacity_ok() {
        assert!(ckb_deposit_capacity_ok(MIN_DEPOSIT_CAPACITY, false));
        assert!(!ckb_deposit_capacity_ok(MIN_DEPOSIT_CAPACITY - 1, false));
        // xUDT Intentions are not CKB deposits; Process CKB min is not applied.
        assert!(ckb_deposit_capacity_ok(1, true));
    }

    #[test]
    fn test_assemble_rejects_ckb_deposit_below_process_minimum() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let rpc = FakeRpcClient::default();
        let (process, mut intentions) = sample_process_and_intention();
        // Createable Intention capacity, but below Process MIN_DEPOSIT.
        intentions[0].capacity = MIN_DEPOSIT_CAPACITY / 2;
        let err = process_digest_ok_with_extras(
            &rpc,
            intentions,
            process,
            AssembleExtras {
                funding_cells: vec![],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .err()
        .expect("deposit below Process MIN_DEPOSIT must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("MIN_DEPOSIT_CAPACITY") || msg.contains("DepositBelowMinimum"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn test_ssri_guest_rejects_ckb_deposit_below_process_minimum() {
        use cid_process_executor::ExecutorError;

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let executor = ProcessExecutor::new(load_process_binary(bin_dir).unwrap());
        let (process, mut intentions) = sample_process_and_intention();
        intentions[0].capacity = MIN_DEPOSIT_CAPACITY;
        let extras = AssembleExtras {
            funding_cells: vec![],
            cell_deps: vec![sample_code_dep()],
        };
        let (draft, mut known) = build_draft_and_known(&intentions, &process, &extras).unwrap();
        // Change the cell served to the guest so the host draft guard does not
        // intercept the below-minimum case.
        let (output, _) = known.get_mut(intentions[0].out_point.as_slice()).unwrap();
        *output = output
            .clone()
            .as_builder()
            .capacity(MIN_DEPOSIT_CAPACITY - 1)
            .build();
        let result = executor.assemble(&draft, seed_mock_from_known(&known).unwrap());
        assert!(
            matches!(result, Err(ExecutorError::Incompatible(39))),
            "{result:?}"
        );
    }

    #[test]
    fn test_assemble_rejects_intention_process_type_hash_mismatch() {
        // Calculator host guard (`require_process_matches_intentions`) — bd11d44.
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let rpc = FakeRpcClient::default();
        let (process, mut intentions) = sample_process_and_intention();
        let foreign_pth = [0xFEu8; 32];
        intentions[0].lock_args = CidArgs::new(
            foreign_pth,
            CidScript::default(),
            sample_beneficiary_payload(),
        )
        .encode()
        .unwrap();
        let err = process_digest_ok_with_extras(
            &rpc,
            intentions,
            process,
            AssembleExtras {
                funding_cells: vec![],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .err()
        .expect("foreign process_type_hash must fail before guest");
        let msg = format!("{err}");
        assert!(
            msg.contains("process_type_hash does not match"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn test_ssri_guest_rejects_intention_process_type_hash_mismatch() {
        // Guest assemble bd11d44 check: Intention process_type_hash must equal
        // Process TYPE_ID (no CID Type ID involved). Bypass the calculator guard
        // by mutating the cell the guest loads.
        use cid_process_executor::ExecutorError;

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let executor = ProcessExecutor::new(load_process_binary(bin_dir).unwrap());
        let (process, intentions) = sample_process_and_intention();
        let extras = AssembleExtras {
            funding_cells: vec![],
            cell_deps: vec![sample_code_dep()],
        };
        let (draft, mut known) = build_draft_and_known(&intentions, &process, &extras).unwrap();
        let foreign_args = CidArgs::new(
            [0xFEu8; 32],
            CidScript::default(),
            sample_beneficiary_payload(),
        )
        .encode()
        .unwrap();
        let (output, _) = known.get_mut(intentions[0].out_point.as_slice()).unwrap();
        let lock = output.lock();
        *output = output
            .clone()
            .as_builder()
            .lock(lock.as_builder().args(foreign_args.pack()).build())
            .build();
        let result = executor.assemble(&draft, seed_mock_from_known(&known).unwrap());
        assert!(
            matches!(result, Err(ExecutorError::Incompatible(21))),
            "guest must reject mismatched process_type_hash: {result:?}"
        );
    }

    #[test]
    fn test_assemble_deposit_by_consuming_intention_at_min() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let rpc = FakeRpcClient::default();
        let (process, mut intentions) = sample_process_and_intention();
        intentions[0].capacity = MIN_DEPOSIT_CAPACITY;
        let (skeleton, _, _) = process_digest_ok_with_extras(
            &rpc,
            intentions,
            process,
            AssembleExtras {
                funding_cells: vec![],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .expect("min deposit Intention must assemble");
        // Beneficiary output is funded by consuming the Intention capacity.
        assert_eq!(
            skeleton.outputs[1].capacity_shannons(),
            MIN_DEPOSIT_CAPACITY
        );
    }

    #[test]
    fn test_process_digest_ok_requires_live_state() {
        use ckb_cinnabar_calculator::simulation::FakeRpcClient;

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let rpc = FakeRpcClient::default();
        let type_id = sample_type_id();
        let pth: [u8; 32] = type_id.calc_script_hash().unpack();
        let lock_args = CidArgs::new(pth, CidScript::default(), Payload::default())
            .encode()
            .unwrap();
        let intentions = vec![IntentionCell {
            out_point: Default::default(),
            lock_args,
            capacity: 100_000_000_000,
            data: vec![],
            type_script: None,
            lock: None,
        }];
        match process_digest_ok(&rpc, intentions, bin_dir) {
            Err(CalculatorError::Other(msg)) => {
                assert!(
                    msg.contains("Process state cell required"),
                    "unexpected error: {msg}"
                );
            }
            Ok(_) => panic!("expected Process state required error, got Ok"),
            Err(e) => panic!("expected Process state required error, got {e}"),
        }
    }

    #[test]
    fn test_process_digest_rejects_empty_cell_deps() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let rpc = FakeRpcClient::default();
        let (process, intentions) = sample_process_and_intention();
        let err = process_digest_ok_with_extras(
            &rpc,
            intentions,
            process,
            AssembleExtras::default(),
            bin_dir,
        )
        .err()
        .expect("empty cell_deps must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("cell_deps required"),
            "expected cell_deps required error, got {msg}"
        );
    }

    #[test]
    fn test_process_digest_complete_tx_outputs() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }

        let rpc = FakeRpcClient::default();
        let (process, mut intentions) = sample_process_and_intention();
        intentions[0].data = vec![0xaa, 0xbb];
        let (skeleton, assembled, _) = process_digest_ok_with_extras(
            &rpc,
            intentions,
            process,
            AssembleExtras {
                funding_cells: vec![],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .expect("digest n=1");
        assert_eq!(skeleton.inputs.len(), 2);
        assert_eq!(skeleton.outputs.len(), 2);
        assert!(skeleton.outputs[0].capacity_shannons() < 200_000_000_000);
        assert!(!assembled.cell_deps.is_empty());
        // Submit path: packed bytes survive skeleton round-trip with plain witnesses.
        let round = assembled_tx_round_trip_bytes(&assembled).expect("round-trip");
        assert_eq!(round, assembled.as_molecule_bytes());
    }

    #[test]
    fn test_witness_args_some_empty_round_trip() {
        // Explicit WitnessArgs { lock: Some(empty), input_type: Some(empty), … }
        // must not collapse to None through the submit skeleton path.
        let empty = PackedBytes::default();
        let wa = WitnessArgs::new_builder()
            .lock(Some(empty.clone()))
            .input_type(Some(empty.clone()))
            .output_type(Some(empty))
            .build();
        let wa_bytes = wa.as_bytes().pack();
        let tx = Transaction::new_builder()
            .raw(RawTransaction::new_builder().build())
            .witnesses(vec![wa_bytes].pack())
            .build();
        let assembled = AssembledTx::from_bytes(tx.as_slice()).expect("parse");
        let round = assembled_tx_round_trip_bytes(&assembled).expect("round-trip");
        assert_eq!(
            round,
            assembled.as_molecule_bytes(),
            "Some(empty) WitnessArgs must be byte-identical after skeleton round-trip"
        );
    }

    #[test]
    fn test_assemble_rejects_always_success_funding() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }

        let rpc = FakeRpcClient::default();
        let (process, intentions) = sample_process_and_intention();
        for args in [vec![], intentions[0].lock_args.clone()] {
            let funding = ProcessStateCell {
                out_point: fake_outpoint(),
                lock: always_success_script(args),
                type_script: Script::default(),
                capacity: 50_000_000_000,
                data: vec![],
            };
            let err = process_digest_ok_with_extras(
                &rpc,
                intentions.clone(),
                process.clone(),
                AssembleExtras {
                    funding_cells: vec![funding],
                    cell_deps: vec![sample_code_dep()],
                },
                bin_dir,
            )
            .err()
            .expect("AlwaysSuccess funding must fail without a change policy");
            assert!(
                err.to_string()
                    .contains("does not support extra funding inputs"),
                "{err}"
            );
        }
    }

    #[test]
    fn test_assemble_rejects_unsupported_funding_lock() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }
        let rpc = FakeRpcClient::default();
        let (process, intentions) = sample_process_and_intention();

        // Secp-like / arbitrary lock: empty witness cannot unlock it.
        let bad_funding = ProcessStateCell {
            out_point: fake_outpoint(),
            lock: Script::new_builder()
                .code_hash([0xFE; 32].pack())
                .hash_type(ScriptHashType::Data1)
                .build(),
            type_script: Script::default(),
            capacity: 50_000_000_000,
            data: vec![],
        };
        let err = process_digest_ok_with_extras(
            &rpc,
            intentions.clone(),
            process.clone(),
            AssembleExtras {
                funding_cells: vec![bad_funding],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .err()
        .expect("non-AlwaysSuccess funding must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("does not support extra funding inputs"),
            "expected funding input rejection, got {msg}"
        );

        // Same Process lock as state would break on-chain 1→1.
        let same_lock_funding = ProcessStateCell {
            out_point: fake_outpoint(),
            lock: process.lock.clone(),
            type_script: Script::default(),
            capacity: 50_000_000_000,
            data: vec![],
        };
        let err = process_digest_ok_with_extras(
            &rpc,
            intentions,
            process,
            AssembleExtras {
                funding_cells: vec![same_lock_funding],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .err()
        .expect("same-Process-lock funding must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("does not support extra funding inputs"),
            "expected funding input rejection for same Process lock, got {msg}"
        );
    }

    fn filter_intention_cell(
        pth: [u8; 32],
        cid_tid: [u8; 32],
        capacity: u64,
        type_script: Option<Script>,
        data: Vec<u8>,
        foreign_pth: Option<[u8; 32]>,
    ) -> IntentionCell {
        let target = foreign_pth.unwrap_or(pth);
        let lock_args = CidArgs::new(target, CidScript::default(), sample_beneficiary_payload())
            .encode()
            .unwrap();
        let lock = Script::new_builder()
            .code_hash(cid_tid.pack())
            .hash_type(ScriptHashType::Type)
            .args(lock_args.clone().pack())
            .build();
        IntentionCell {
            out_point: fake_outpoint(),
            lock_args,
            capacity,
            data,
            type_script: type_script.map(|s| s.as_slice().to_vec()),
            lock: Some(lock),
        }
    }

    #[test]
    fn test_process_filter_empty_batch() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        if !std::path::Path::new(&format!("{bin_dir}/{PROCESS_CONTRACT_NAME}")).exists() {
            eprintln!("Skipping test: binary not built");
            return;
        }
        let reasons = process_filter(&[], bin_dir).expect("empty filter");
        assert!(reasons.is_empty());
    }

    #[test]
    fn test_process_filter_mixed_batch_business_rules() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        if !std::path::Path::new(&format!("{bin_dir}/{PROCESS_CONTRACT_NAME}")).exists() {
            eprintln!("Skipping test: binary not built");
            return;
        }
        let (process, _) = sample_process_and_intention();
        let pth: [u8; 32] = process.type_script.calc_script_hash().unpack();
        let cid_tid = [0xC1; 32];
        let valid = filter_intention_cell(pth, cid_tid, MIN_DEPOSIT_CAPACITY, None, vec![], None);
        // Foreign process_type_hash: filter no longer rejects ownership; args still
        // decode with a beneficiary → business ACCEPTED after Sequencer routing.
        let foreign_pth = filter_intention_cell(
            pth,
            cid_tid,
            MIN_DEPOSIT_CAPACITY,
            None,
            vec![],
            Some([0xB2; 32]),
        );
        let below =
            filter_intention_cell(pth, cid_tid, MIN_DEPOSIT_CAPACITY / 2, None, vec![], None);
        let malformed = {
            let mut c =
                filter_intention_cell(pth, cid_tid, MIN_DEPOSIT_CAPACITY, None, vec![], None);
            c.lock_args = vec![0u8; 40];
            if let Some(ref mut lock) = c.lock {
                *lock = lock
                    .clone()
                    .as_builder()
                    .args(c.lock_args.clone().pack())
                    .build();
            }
            c
        };
        let xudt = {
            let typ = Script::new_builder()
                .code_hash(cid_protocol::XUDT_CODE_HASH.pack())
                .hash_type(ScriptHashType::Data1)
                .args([0u8; 32])
                .build();
            filter_intention_cell(pth, cid_tid, 1, Some(typ), vec![0u8; 16], None)
        };
        let batch = vec![valid, foreign_pth, below, malformed, xudt];
        let reasons = process_filter(&batch, bin_dir).expect("filter");
        assert_eq!(reasons.len(), 5);
        assert_eq!(reasons[0], FILTER_ACCEPTED);
        assert_eq!(reasons[1], FILTER_ACCEPTED); // ownership not checked in filter
        assert_eq!(reasons[2], FILTER_DEPOSIT_BELOW_MINIMUM);
        assert_eq!(reasons[3], FILTER_MALFORMED_DIGEST);
        assert_eq!(reasons[4], FILTER_ACCEPTED);
    }

    #[test]
    fn test_process_filter_accepted_matches_assemble() {
        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        if !std::path::Path::new(&format!("{bin_dir}/{PROCESS_CONTRACT_NAME}")).exists() {
            eprintln!("Skipping test: binary not built");
            return;
        }
        let rpc = FakeRpcClient::default();
        let (process, intentions) = sample_process_and_intention();
        let cid_tid = [0xC1; 32];
        let with_lock: Vec<_> = intentions
            .iter()
            .map(|i| {
                let mut c = i.clone();
                c.lock = Some(
                    Script::new_builder()
                        .code_hash(cid_tid.pack())
                        .hash_type(ScriptHashType::Type)
                        .args(c.lock_args.clone().pack())
                        .build(),
                );
                c
            })
            .collect();
        let reasons = process_filter(&with_lock, bin_dir).expect("filter");
        assert!(reasons.iter().all(|r| *r == FILTER_ACCEPTED));
        process_digest_ok_with_extras(
            &rpc,
            with_lock,
            process,
            AssembleExtras {
                funding_cells: vec![],
                cell_deps: vec![sample_code_dep()],
            },
            bin_dir,
        )
        .expect("accepted Intentions must assemble");
    }

    #[test]
    fn test_filter_accepted_equals_business_classify_mixed_corpus() {
        use cid_process_example_policy::classify_intention;
        use cid_protocol::Script as CidScript;

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        if !std::path::Path::new(&format!("{bin_dir}/{PROCESS_CONTRACT_NAME}")).exists() {
            eprintln!("Skipping test: binary not built");
            return;
        }
        let (process, _) = sample_process_and_intention();
        let pth: [u8; 32] = process.type_script.calc_script_hash().unpack();
        let cid_tid = [0xC1; 32];
        let valid = filter_intention_cell(pth, cid_tid, MIN_DEPOSIT_CAPACITY, None, vec![], None);
        let below =
            filter_intention_cell(pth, cid_tid, MIN_DEPOSIT_CAPACITY / 2, None, vec![], None);
        let xudt = {
            let typ = Script::new_builder()
                .code_hash(cid_protocol::XUDT_CODE_HASH.pack())
                .hash_type(ScriptHashType::Data1)
                .args([0u8; 32])
                .build();
            filter_intention_cell(pth, cid_tid, 1, Some(typ), vec![0u8; 16], None)
        };
        let batch = vec![valid, below, xudt];
        let reasons = process_filter(&batch, bin_dir).expect("filter");
        assert_eq!(reasons.len(), batch.len());
        for (cell, &reason) in batch.iter().zip(reasons.iter()) {
            let lock = cell.lock.as_ref().unwrap();
            let ht: u8 = lock.hash_type().into();
            let cid_lock = CidScript {
                code_hash: lock.code_hash().unpack(),
                hash_type: ht,
                args: cell.lock_args.clone(),
            };
            let typ = cell.type_script.as_ref().map(|bytes| {
                let s = Script::from_slice(bytes).unwrap();
                let code: [u8; 32] = s.code_hash().unpack();
                let ht: u8 = s.hash_type().into();
                (code, ht)
            });
            let typ_ref = typ.as_ref().map(|(c, ht)| (c, *ht));
            let classified = classify_intention(&cid_lock, cell.capacity, typ_ref, &cell.data);
            assert_eq!(classified, reason, "filter vs business classify mismatch");
        }
        assert_eq!(reasons[0], FILTER_ACCEPTED);
        assert_eq!(reasons[1], FILTER_DEPOSIT_BELOW_MINIMUM);
        assert_eq!(reasons[2], FILTER_ACCEPTED);
    }

    #[test]
    fn test_filter_stdin_accepts_exact_max_rejects_over() {
        use cid_intention_filter::{encode_filter_request, FILTER_STDIN_MAX};
        use cid_process_executor::ExecutorError;

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        if !std::path::Path::new(&format!("{bin_dir}/{PROCESS_CONTRACT_NAME}")).exists() {
            eprintln!("Skipping test: binary not built");
            return;
        }
        let executor = ProcessExecutor::new(load_process_binary(bin_dir).unwrap());
        let mut req = FilterRequest {
            cells: vec![FilterCell {
                out_point: vec![1],
                output: vec![2],
                data: vec![],
            }],
        };
        let base = encode_filter_request(&req).unwrap();
        assert!(base.len() < FILTER_STDIN_MAX);
        let pad = FILTER_STDIN_MAX - base.len();
        req.cells[0].data = vec![0xAB; pad];
        let exact = encode_filter_request(&req).unwrap();
        assert_eq!(exact.len(), FILTER_STDIN_MAX);
        let reasons = executor
            .filter(&exact)
            .expect("exact FILTER_STDIN_MAX must be accepted");
        assert_eq!(reasons.len(), 1);

        req.cells[0].data.push(0xCD);
        let over = encode_filter_request(&req).unwrap();
        assert_eq!(over.len(), FILTER_STDIN_MAX + 1);
        let err = executor
            .filter(&over)
            .expect_err("> FILTER_STDIN_MAX must fail");
        assert!(
            matches!(
                err,
                ExecutorError::Incompatible(_) | ExecutorError::VmError(_)
            ),
            "{err:?}"
        );
    }

    #[test]
    fn test_route_helper_groups_before_filter() {
        let cid_tid = [0xC1; 32];
        let p_a = [0xA1; 32];
        let p_b = [0xB2; 32];
        let locks = [
            CidScript {
                code_hash: cid_tid,
                hash_type: 1,
                args: CidArgs::new(p_a, CidScript::default(), sample_beneficiary_payload())
                    .encode()
                    .unwrap(),
            },
            CidScript {
                code_hash: cid_tid,
                hash_type: 1,
                args: CidArgs::new(p_b, CidScript::default(), sample_beneficiary_payload())
                    .encode()
                    .unwrap(),
            },
        ];
        assert_eq!(intention_process_type_id(&locks[0], &cid_tid).unwrap(), p_a);
        assert_eq!(intention_process_type_id(&locks[1], &cid_tid).unwrap(), p_b);
        assert!(intention_belongs_to_process(&locks[0], &cid_tid, &p_a).is_ok());
        assert_eq!(
            intention_belongs_to_process(&locks[0], &cid_tid, &p_b),
            Err(IntentionRouteError::ProcessMismatch)
        );
    }
}
