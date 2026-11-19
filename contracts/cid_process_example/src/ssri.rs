extern crate alloc;

use crate::host::{self, HostCell};
use crate::ProcessError;
use alloc::vec::Vec;
use cid_protocol::CidArgs;
use ckb_cinnabar_verifier::ssri::{SsriArgs, SsriSource};
use ckb_std::ckb_types::packed::{
    Byte32, Bytes, CellInput, CellOutput, OutPoint, RawTransaction, Script, Transaction,
};
use ckb_std::ckb_types::prelude::*;

const BATCH_MIN: u32 = 1;
const BATCH_MAX: u32 = 4;

/// Example miner fee deducted from the Process state cell capacity by assemble.
/// Capacity balance (and any Process-owned funding inputs in the draft) is the
/// Process's responsibility — the backend must not rewrite outputs to "fix" fee.
const ASSEMBLE_FEE: u64 = 1_000_000;

// Default lock code_hash for beneficiary outputs (placeholder for raw lock_args tests)
const DEFAULT_LOCK_CODE_HASH: [u8; 32] = [
    0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
    0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
];

/// SSRI handler for Process.batch
/// Returns (batch_min, batch_max) as 8 bytes little-endian
pub fn batch_handler(
    _src: &SsriSource,
    _args: SsriArgs,
) -> core::result::Result<Vec<u8>, ProcessError> {
    let mut output = Vec::with_capacity(8);
    output.extend_from_slice(&BATCH_MIN.to_le_bytes());
    output.extend_from_slice(&BATCH_MAX.to_le_bytes());
    Ok(output)
}

struct LoadedInput {
    out_point: OutPoint,
    cell: HostCell,
}

struct IntentionView {
    capacity: u64,
    lock_args: Vec<u8>,
    data: Vec<u8>,
    type_script: Option<Script>,
}

/// SSRI handler for Process.assemble
///
/// Stdin: packed molecule `Transaction` draft (Intentions + Process state cell as
/// inputs; outputs/deps may be empty). The guest resolves each input via the
/// SSRI host (`find_cell_by_out_point` / `find_cell_data_by_out_point`), may
/// query tip info, then returns a **complete** molecule `Transaction` the
/// backend submits unchanged.
pub fn assemble_handler(
    _src: &SsriSource,
    _args: SsriArgs,
) -> core::result::Result<Vec<u8>, ProcessError> {
    let draft_bytes = read_stdin()?;
    let draft = Transaction::from_slice(&draft_bytes).map_err(|_| ProcessError::ReadFailed)?;
    let raw = draft.raw();

    let mut loaded = Vec::new();
    for input in raw.inputs().into_iter() {
        let out_point = input.previous_output();
        let cell = host::get_cell_by_out_point(&out_point)?;
        loaded.push(LoadedInput { out_point, cell });
    }

    // Tip probe so backends exercise header/tip host queries during assemble.
    let _ = host::block_hash(0);

    let mut intentions = Vec::new();
    let mut process_cell: Option<&LoadedInput> = None;
    let mut process_type_hash: Option<[u8; 32]> = None;

    for item in &loaded {
        let lock = item.cell.output.lock();
        let args = lock.args().raw_data();
        if let Ok(cid) = CidArgs::decode(&args) {
            if let Some(expected) = process_type_hash {
                if cid.process_type_hash != expected {
                    return Err(ProcessError::IncompatibleBatch);
                }
            } else {
                process_type_hash = Some(cid.process_type_hash);
            }
            intentions.push(IntentionView {
                capacity: item.cell.output.capacity().unpack(),
                lock_args: args.to_vec(),
                data: item.cell.data.clone(),
                type_script: item.cell.output.type_().to_opt(),
            });
        } else if item.cell.output.type_().is_some() {
            // Candidate Process state cell (TYPE_ID type); keep the first.
            if process_cell.is_none() {
                process_cell = Some(item);
            }
        }
    }

    let count = intentions.len() as u32;
    if !(BATCH_MIN..=BATCH_MAX).contains(&count) {
        return Err(ProcessError::IncompatibleBatch);
    }

    let process = process_cell.ok_or(ProcessError::ReadFailed)?;
    if let Some(pth) = process_type_hash {
        let typ = process
            .cell
            .output
            .type_()
            .to_opt()
            .ok_or(ProcessError::ReadFailed)?;
        let hash: [u8; 32] = typ.calc_script_hash().unpack();
        if hash != pth {
            return Err(ProcessError::IncompatibleBatch);
        }
    }

    // Outputs: Process recreate (same lock/type/data; capacity minus Process-paid
    // fee) + beneficiaries. Extra draft inputs (Process-owned funding) stay in
    // `inputs` and their surplus capacity also contributes to tx fee.
    let mut outputs = Vec::with_capacity(1 + intentions.len());
    let mut outputs_data = Vec::with_capacity(1 + intentions.len());

    let process_cap: u64 = process.cell.output.capacity().unpack();
    let recreate_cap = process_cap.saturating_sub(ASSEMBLE_FEE);
    let recreate = process
        .cell
        .output
        .clone()
        .as_builder()
        .capacity(recreate_cap)
        .build();
    outputs.push(recreate);
    outputs_data.push(process.cell.data.clone());

    for intention in &intentions {
        let lock = beneficiary_lock(intention.lock_args.clone());
        let mut output = CellOutput::new_builder()
            .capacity(intention.capacity)
            .lock(lock);
        if let Some(ref typ) = intention.type_script {
            output = output.type_(Some(typ.clone()).pack());
        }
        outputs.push(output.build());
        outputs_data.push(intention.data.clone());
    }

    // Inputs: full draft order (includes any Process-owned funding cells).
    let inputs: Vec<CellInput> = loaded
        .iter()
        .map(|item| {
            CellInput::new_builder()
                .previous_output(item.out_point.clone())
                .build()
        })
        .collect();

    // Preserve draft deps as-is; witnesses: one empty per input (backend fills unlocks).
    let cell_deps = raw.cell_deps();
    let header_deps = raw.header_deps();
    let witnesses: Vec<Bytes> = (0..inputs.len()).map(|_| Bytes::default()).collect();

    let tx = Transaction::new_builder()
        .raw(
            RawTransaction::new_builder()
                .version(raw.version())
                .cell_deps(cell_deps)
                .header_deps(header_deps)
                .inputs(inputs.pack())
                .outputs(outputs.pack())
                .outputs_data(outputs_data.into_iter().map(Bytes::from).pack())
                .build(),
        )
        .witnesses(witnesses.pack())
        .build();
    Ok(tx.as_slice().to_vec())
}

fn read_stdin() -> Result<Vec<u8>, ProcessError> {
    let mut input_buf = [0u8; 16384];
    let mut total_read = 0;
    loop {
        let read_result = ckb_std::syscalls::read(0, &mut input_buf[total_read..])
            .map_err(|_| ProcessError::ReadFailed)?;
        if read_result == 0 {
            break;
        }
        total_read += read_result;
        if total_read >= input_buf.len() {
            return Err(ProcessError::ReadFailed);
        }
    }
    Ok(input_buf[..total_read].to_vec())
}

fn beneficiary_lock(lock_args: Vec<u8>) -> Script {
    if let Ok(cid) = CidArgs::decode(&lock_args) {
        if let Ok(script) = cid.entry.payload.to_script() {
            if let Ok(code_hash) = Byte32::from_slice(&script.code_hash) {
                return Script::new_builder()
                    .code_hash(code_hash)
                    .hash_type(script.hash_type)
                    .args(Bytes::from(script.args))
                    .build();
            }
        }
    }

    Script::new_builder()
        .code_hash(Byte32::from_slice(&DEFAULT_LOCK_CODE_HASH).unwrap())
        .hash_type(0u8)
        .args(Bytes::from(lock_args))
        .build()
}
