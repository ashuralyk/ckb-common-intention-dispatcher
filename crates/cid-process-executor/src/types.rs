use ckb_types::packed::{
    Byte32, Bytes, CellDep, CellInput, CellOutput, OutPoint, RawTransaction, Transaction,
};
use ckb_types::prelude::*;

/// Live cell attached to a draft input for offline assemble.
#[derive(Clone, Debug)]
pub struct DraftCell {
    pub out_point: OutPoint,
    pub output: CellOutput,
    pub data: Vec<u8>,
}

/// Build a molecule `Transaction` draft from input cells (outputs/deps empty).
pub fn build_draft_tx(cells: &[DraftCell]) -> Vec<u8> {
    let inputs: Vec<CellInput> = cells
        .iter()
        .map(|c| {
            CellInput::new_builder()
                .previous_output(c.out_point.clone())
                .build()
        })
        .collect();
    Transaction::new_builder()
        .raw(RawTransaction::new_builder().inputs(inputs.pack()).build())
        .build()
        .as_slice()
        .to_vec()
}

/// Complete transaction returned by `Process.assemble`.
///
/// Wire format is a packed molecule `Transaction` with inputs, outputs,
/// outputs_data, cell_deps, header_deps, and witnesses filled as needed.
/// The backend submits this blob unchanged.
#[derive(Clone, Debug)]
pub struct AssembledTx {
    packed: Vec<u8>,
    pub inputs: Vec<CellInput>,
    pub outputs: Vec<CellOutput>,
    pub outputs_data: Vec<Vec<u8>>,
    pub cell_deps: Vec<CellDep>,
    pub header_deps: Vec<Byte32>,
    pub witnesses: Vec<Bytes>,
}

/// Explicit budgets so a malicious vector cannot allocate unbounded structures.
const MAX_TX_OUTPUTS: usize = 256;
const MAX_TX_INPUTS: usize = 256;
const MAX_TX_BYTES: usize = 1024 * 1024;

impl AssembledTx {
    /// Parse a packed molecule `Transaction` from SSRI response bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, super::ExecutorError> {
        if bytes.len() > MAX_TX_BYTES {
            return Err(super::ExecutorError::InvalidResponse(
                "response exceeds byte budget".into(),
            ));
        }

        let tx = Transaction::from_slice(bytes).map_err(|e| {
            super::ExecutorError::InvalidResponse(format!("invalid Transaction molecule: {:?}", e))
        })?;
        let raw = tx.raw();
        let packed_outputs = raw.outputs();
        let packed_data = raw.outputs_data();
        let packed_inputs = raw.inputs();
        let output_count = packed_outputs.len();
        let input_count = packed_inputs.len();

        if output_count > MAX_TX_OUTPUTS {
            return Err(super::ExecutorError::InvalidResponse(format!(
                "output count {} exceeds budget {}",
                output_count, MAX_TX_OUTPUTS
            )));
        }
        if input_count > MAX_TX_INPUTS {
            return Err(super::ExecutorError::InvalidResponse(format!(
                "input count {} exceeds budget {}",
                input_count, MAX_TX_INPUTS
            )));
        }
        if packed_data.len() != output_count {
            return Err(super::ExecutorError::InvalidResponse(format!(
                "outputs ({}) / outputs_data ({}) length mismatch",
                output_count,
                packed_data.len()
            )));
        }

        let mut outputs = Vec::new();
        let mut outputs_data = Vec::new();
        let mut inputs = Vec::new();
        outputs.try_reserve(output_count).map_err(|_| {
            super::ExecutorError::InvalidResponse("output vec reserve failed".into())
        })?;
        outputs_data.try_reserve(output_count).map_err(|_| {
            super::ExecutorError::InvalidResponse("output data vec reserve failed".into())
        })?;
        inputs.try_reserve(input_count).map_err(|_| {
            super::ExecutorError::InvalidResponse("input vec reserve failed".into())
        })?;

        for output in packed_outputs.into_iter() {
            outputs.push(output);
        }
        for data in packed_data.into_iter() {
            let unpacked: Vec<u8> = data.unpack();
            outputs_data.push(unpacked);
        }
        for input in packed_inputs.into_iter() {
            inputs.push(input);
        }

        let cell_deps: Vec<CellDep> = raw.cell_deps().into_iter().collect();
        let header_deps: Vec<Byte32> = raw.header_deps().into_iter().collect();
        let witnesses: Vec<Bytes> = tx.witnesses().into_iter().collect();

        Ok(AssembledTx {
            packed: bytes.to_vec(),
            inputs,
            outputs,
            outputs_data,
            cell_deps,
            header_deps,
            witnesses,
        })
    }

    /// Original packed molecule `Transaction` bytes.
    pub fn as_molecule_bytes(&self) -> &[u8] {
        &self.packed
    }
}

/// Legacy alias kept for call-site clarity during the PartialTx → AssembledTx rename.
pub type PartialTx = AssembledTx;

/// Executor errors
#[derive(Debug)]
pub enum ExecutorError {
    /// Process rejected the batch (incompatible) with custom exit code >= 20
    Incompatible(i8),
    /// VM execution failed
    VmError(String),
    /// Guest exceeded the instruction cycle budget
    CyclesExceeded,
    /// SSRI response invalid
    InvalidResponse(String),
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorError::Incompatible(code) => {
                write!(f, "Process rejected batch (exit {})", code)
            }
            ExecutorError::VmError(msg) => write!(f, "VM error: {}", msg),
            ExecutorError::CyclesExceeded => write!(f, "VM cycle budget exceeded"),
            ExecutorError::InvalidResponse(msg) => write!(f, "Invalid SSRI response: {}", msg),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Custom error start for domain errors
pub const CUSTOM_ERROR_START: i8 = 20;
