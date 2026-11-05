use ckb_types::packed::{CellOutput, OutPoint, Transaction};
use ckb_types::prelude::*;

/// Input intention cell for assemble
#[derive(Clone, Debug, Default)]
pub struct IntentionInput {
    pub out_point: OutPoint,
    pub lock_args: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
    /// Packed molecule `Script` of the intention type, if any (official xUDT).
    pub type_script: Option<Vec<u8>>,
}

impl IntentionInput {
    /// Encode for SSRI Process.assemble input
    /// Format: capacity (u64 LE) + lock_args_len (u32 LE) + lock_args + data_len (u32 LE) + data
    /// + optional type_len (u32 LE) + type_script molecule
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.capacity.to_le_bytes());
        buf.extend_from_slice(&(self.lock_args.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.lock_args);
        buf.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.data);
        if let Some(type_script) = &self.type_script {
            buf.extend_from_slice(&(type_script.len() as u32).to_le_bytes());
            buf.extend_from_slice(type_script);
        }
        buf
    }
}

/// Encode array of intentions for SSRI Process.assemble
/// Format: count (u32 LE) + for each intention: len (u32 LE) + encoded intention
pub fn encode_intentions(intentions: &[IntentionInput]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(intentions.len() as u32).to_le_bytes());
    for intention in intentions {
        let encoded = intention.encode();
        buf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        buf.extend_from_slice(&encoded);
    }
    buf
}

/// Partial transaction returned by assemble.
///
/// Wire format is a packed molecule `Transaction` (typically outputs +
/// `outputs_data` only; other fields empty).
#[derive(Clone, Debug)]
pub struct PartialTx {
    packed: Vec<u8>,
    pub outputs: Vec<CellOutput>,
    pub outputs_data: Vec<Vec<u8>>,
}

/// Explicit output-count budget. Assemble only emits BATCH_MAX=4; this is a
/// parser ceiling so a malicious vector cannot allocate unbounded outputs.
const MAX_PARTIAL_TX_OUTPUTS: usize = 256;
const MAX_PARTIAL_TX_BYTES: usize = 1024 * 1024;

impl PartialTx {
    /// Parse a packed molecule `Transaction` from SSRI response bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, super::ExecutorError> {
        if bytes.len() > MAX_PARTIAL_TX_BYTES {
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
        let output_count = packed_outputs.len();

        if output_count > MAX_PARTIAL_TX_OUTPUTS {
            return Err(super::ExecutorError::InvalidResponse(format!(
                "output count {} exceeds budget {}",
                output_count, MAX_PARTIAL_TX_OUTPUTS
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
        outputs.try_reserve(output_count).map_err(|_| {
            super::ExecutorError::InvalidResponse("output vec reserve failed".into())
        })?;
        outputs_data.try_reserve(output_count).map_err(|_| {
            super::ExecutorError::InvalidResponse("output data vec reserve failed".into())
        })?;

        for output in packed_outputs.into_iter() {
            outputs.push(output);
        }
        for data in packed_data.into_iter() {
            let unpacked: Vec<u8> = data.unpack();
            outputs_data.push(unpacked);
        }

        Ok(PartialTx {
            packed: bytes.to_vec(),
            outputs,
            outputs_data,
        })
    }

    /// Original packed molecule `Transaction` bytes.
    pub fn as_molecule_bytes(&self) -> &[u8] {
        &self.packed
    }
}

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
