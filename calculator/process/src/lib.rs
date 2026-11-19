//! Process Calculator for CID Process example

use std::collections::HashMap;

use cid_process_executor::{AssembledTx, MockHost, ProcessExecutor};
use cid_protocol::TYPE_ID_CODE_HASH;
use ckb_cinnabar_calculator::{
    instruction::Instruction,
    re_exports::ckb_types::{
        core::ScriptHashType,
        packed::{CellDep, CellInput, CellOutput, OutPoint, RawTransaction, Script, Transaction},
        prelude::{Builder, Entity, Pack, PackVec},
    },
    rpc::RPC,
    skeleton::{CellInputEx, TransactionSkeleton},
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

/// Optional Process-owned funding cell / cell_dep included in the draft so
/// assemble can demonstrate capacity/fee balance and dep preservation.
#[derive(Clone, Debug, Default)]
pub struct AssembleExtras {
    pub funding_cells: Vec<ProcessStateCell>,
    pub cell_deps: Vec<CellDep>,
}

/// Load the Process binary from build/release/<name>
pub fn load_process_binary(bin_dir: &str) -> Result<Vec<u8>> {
    let path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
    std::fs::read(&path).map_err(|e| {
        CalculatorError::Other(format!("failed to load Process binary at {}: {}", path, e))
    })
}

fn default_process_state(intentions: &[IntentionCell]) -> ProcessStateCell {
    let type_script = Script::new_builder()
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
        .build();
    let lock = Script::new_builder()
        .code_hash([0xAA; 32].pack())
        .hash_type(ScriptHashType::Data1)
        .build();
    let out_point = intentions
        .first()
        .map(|i| i.out_point.clone().as_builder().index(99u32).build())
        .unwrap_or_default();
    ProcessStateCell {
        out_point,
        lock,
        type_script,
        capacity: 200_000_000_000,
        data: vec![],
    }
}

fn intention_output(intention: &IntentionCell) -> Result<CellOutput> {
    let lock = Script::new_builder()
        .code_hash([0xC1; 32].pack())
        .hash_type(ScriptHashType::Type)
        .args(intention.lock_args.clone().pack())
        .build();
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

fn build_draft_and_host(
    intentions: &[IntentionCell],
    process: &ProcessStateCell,
    extras: &AssembleExtras,
) -> Result<(Vec<u8>, MockHost, KnownCells)> {
    let host = MockHost::new();
    host.insert_block_hash(0, [0xAB; 32]);

    let mut known = KnownCells::new();
    let mut inputs = Vec::with_capacity(1 + intentions.len() + extras.funding_cells.len());

    let process_out = process_output(process);
    host.insert_cell_bytes(
        process.out_point.as_slice(),
        process_out.as_slice(),
        process.data.clone(),
    )
    .map_err(CalculatorError::Other)?;
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
        host.insert_cell_bytes(
            intention.out_point.as_slice(),
            output.as_slice(),
            intention.data.clone(),
        )
        .map_err(CalculatorError::Other)?;
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

    for funding in &extras.funding_cells {
        let output = process_output(funding);
        // Funding cells are Process-owned capacity (no TYPE_ID required for the
        // example assemble path — lock-only so they are not mistaken for state).
        let output = output.as_builder().type_(None).build();
        host.insert_cell_bytes(
            funding.out_point.as_slice(),
            output.as_slice(),
            funding.data.clone(),
        )
        .map_err(CalculatorError::Other)?;
        known.insert(
            funding.out_point.as_slice().to_vec(),
            (output, funding.data.clone()),
        );
        inputs.push(
            CellInput::new_builder()
                .previous_output(funding.out_point.clone())
                .build(),
        );
    }

    let draft = Transaction::new_builder()
        .raw(
            RawTransaction::new_builder()
                .inputs(inputs.pack())
                .cell_deps(extras.cell_deps.clone().pack())
                .build(),
        )
        .build()
        .as_slice()
        .to_vec();
    Ok((draft, host, known))
}

/// Process digest: batch-check + SSRI assemble → skeleton from the complete tx.
pub fn process_digest_ok<T: RPC>(
    rpc: &T,
    intentions: Vec<IntentionCell>,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, Vec<Instruction<T>>)> {
    process_digest_ok_with_process(rpc, intentions, None, bin_dir)
}

/// Same as [`process_digest_ok`] with an explicit Process state cell.
pub fn process_digest_ok_with_process<T: RPC>(
    rpc: &T,
    intentions: Vec<IntentionCell>,
    process: Option<ProcessStateCell>,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, Vec<Instruction<T>>)> {
    process_digest_ok_with_extras(rpc, intentions, process, AssembleExtras::default(), bin_dir)
}

/// Assemble with optional Process-owned funding inputs and draft cell_deps.
pub fn process_digest_ok_with_extras<T: RPC>(
    _rpc: &T,
    intentions: Vec<IntentionCell>,
    process: Option<ProcessStateCell>,
    extras: AssembleExtras,
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

    let process = process.unwrap_or_else(|| default_process_state(&intentions));
    let (draft, host, known) = build_draft_and_host(&intentions, &process, &extras)?;
    let assembled = executor
        .assemble(&draft, host)
        .map_err(|e| CalculatorError::Other(format!("executor assemble failed: {}", e)))?;

    skeleton_from_assembled(&assembled, &known)
}

/// Rebuild a skeleton from the assembled molecule Transaction **as-is**
/// (inputs, cell_deps, header_deps, witnesses, outputs, outputs_data).
/// Live cell data for inputs is filled from `known` when present.
fn skeleton_from_assembled<T: RPC>(
    assembled: &AssembledTx,
    known: &KnownCells,
) -> Result<(TransactionSkeleton, Vec<Instruction<T>>)> {
    let tx = Transaction::from_slice(assembled.as_molecule_bytes())
        .map_err(|e| CalculatorError::Other(format!("Transaction molecule bridge: {:?}", e)))?;
    let raw = tx.raw();
    if raw.outputs().len() != raw.outputs_data().len() {
        return Err(CalculatorError::Other(
            "assemble Transaction outputs / outputs_data length mismatch".into(),
        ));
    }

    let mut skeleton = TransactionSkeleton::from_packed_transaction(&tx);
    for input in &mut skeleton.inputs {
        let key = input.input.previous_output().as_slice().to_vec();
        if let Some((output, data)) = known.get(&key) {
            *input = CellInputEx::new(input.input.clone(), output.clone(), Some(data.clone()));
        }
    }

    Ok((skeleton, vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cid_protocol::{CidArgs, Payload, Script as CidScript};
    use ckb_cinnabar_calculator::re_exports::ckb_types::core::DepType;
    use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Unpack;

    #[test]
    fn test_load_binary_missing() {
        assert!(load_process_binary("/nonexistent").is_err());
    }

    #[test]
    fn test_process_digest_complete_tx_outputs() {
        use ckb_cinnabar_calculator::simulation::{fake_outpoint, FakeRpcClient};

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }

        let rpc = FakeRpcClient::default();
        let type_id = Script::new_builder()
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
            .build();
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
            data: vec![0xaa, 0xbb],
            type_script: None,
        }];
        let (skeleton, _) =
            process_digest_ok_with_process(&rpc, intentions, Some(process), bin_dir)
                .expect("digest n=1");
        assert_eq!(skeleton.inputs.len(), 2);
        assert_eq!(skeleton.outputs.len(), 2);
        // Process-paid assemble fee: recreate capacity < input process capacity.
        assert!(skeleton.outputs[0].capacity_shannons() < 200_000_000_000);
        assert_eq!(
            skeleton.outputs[0].capacity_shannons(),
            200_000_000_000 - 1_000_000
        );
    }

    #[test]
    fn test_skeleton_preserves_extra_input_and_cell_dep() {
        use ckb_cinnabar_calculator::simulation::{fake_outpoint, FakeRpcClient};

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!("Skipping test: binary not built at {bin_path}");
            return;
        }

        let rpc = FakeRpcClient::default();
        let type_id = Script::new_builder()
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
            .build();
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
        let funding = ProcessStateCell {
            out_point: fake_outpoint(),
            lock: Script::new_builder()
                .code_hash([0xFE; 32].pack())
                .hash_type(ScriptHashType::Data1)
                .build(),
            type_script: Script::default(),
            capacity: 50_000_000_000,
            data: vec![],
        };
        let dep_out = fake_outpoint();
        let cell_dep = CellDep::new_builder()
            .out_point(dep_out)
            .dep_type(DepType::Code)
            .build();

        let intentions = vec![IntentionCell {
            out_point: fake_outpoint(),
            lock_args,
            capacity: 100_000_000_000,
            data: vec![],
            type_script: None,
        }];
        let extras = AssembleExtras {
            funding_cells: vec![funding.clone()],
            cell_deps: vec![cell_dep.clone()],
        };
        let (skeleton, _) =
            process_digest_ok_with_extras(&rpc, intentions, Some(process), extras, bin_dir)
                .expect("digest with extras");
        assert_eq!(
            skeleton.inputs.len(),
            3,
            "process + intention + funding input"
        );
        assert_eq!(skeleton.celldeps.len(), 1, "draft cell_dep preserved");
        assert_eq!(skeleton.celldeps[0].celldep.as_slice(), cell_dep.as_slice());
        assert!(
            skeleton
                .inputs
                .iter()
                .any(|i| i.input.previous_output() == funding.out_point),
            "funding out point must appear in skeleton inputs"
        );
        assert_eq!(skeleton.witnesses.len(), 3);
    }
}
