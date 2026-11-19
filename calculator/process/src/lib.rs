//! Process Calculator for CID Process example

use std::collections::HashMap;

use cid_process_executor::{AssembledTx, HostQuery, MockHost, ProcessExecutor};
use cid_protocol::{CidArgs, TYPE_ID_CODE_HASH};
use ckb_cinnabar_calculator::{
    instruction::Instruction,
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::{CellDep, CellInput, CellOutput, OutPoint, RawTransaction, Script, Transaction},
        prelude::{Builder, Entity, Pack, PackVec},
    },
    rpc::RPC,
    skeleton::{CellDepEx, CellInputEx, TransactionSkeleton, WitnessEx},
    CalculatorError, Result,
};

pub const PROCESS_CONTRACT_NAME: &str = "cid_process_example";

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
    use ckb_cinnabar_calculator::re_exports::ckb_types::core::DepType;
    use ckb_cinnabar_calculator::re_exports::ckb_types::packed::{
        Bytes as PackedBytes, WitnessArgs,
    };
    use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Unpack;
    use ckb_cinnabar_calculator::simulation::{
        always_success_script, fake_outpoint, FakeRpcClient,
    };

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

    fn sample_process_and_intention() -> (ProcessStateCell, Vec<IntentionCell>) {
        let type_id = sample_type_id();
        let pth: [u8; 32] = type_id.calc_script_hash().unpack();
        let lock_args = CidArgs::new(pth, CidScript::default(), Payload::default())
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
}
