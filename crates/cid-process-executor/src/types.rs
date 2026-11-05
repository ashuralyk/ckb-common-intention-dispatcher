use ckb_types::packed::{CellOutput, OutPoint};
use ckb_types::prelude::*;

/// Input intention cell for assemble
#[derive(Clone, Debug, Default)]
pub struct IntentionInput {
    pub out_point: OutPoint,
    pub lock_args: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
}

impl IntentionInput {
    /// Encode for SSRI Process.assemble input
    /// Format: capacity (u64 LE) + lock_args_len (u32 LE) + lock_args + data_len (u32 LE) + data
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.capacity.to_le_bytes());
        buf.extend_from_slice(&(self.lock_args.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.lock_args);
        buf.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.data);
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

/// Partial transaction returned by assemble
#[derive(Clone, Debug)]
pub struct PartialTx {
    pub outputs: Vec<CellOutput>,
    pub outputs_data: Vec<Vec<u8>>,
}

impl PartialTx {
    /// Parse PartialTx from SSRI response bytes
    /// Format: output_count (u32 LE) + for each: output_len (u32 LE) + output_bytes + data_len (u32 LE) + data_bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, super::ExecutorError> {
        let mut pos = 0;

        if bytes.len() < 4 {
            return Err(super::ExecutorError::InvalidResponse(
                "response too short for output count".into(),
            ));
        }

        let output_count = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        let mut outputs = Vec::with_capacity(output_count);
        let mut outputs_data = Vec::with_capacity(output_count);

        for _ in 0..output_count {
            // Read output length
            if pos + 4 > bytes.len() {
                return Err(super::ExecutorError::InvalidResponse(
                    "truncated output length".into(),
                ));
            }
            let output_len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;

            // Read output bytes
            if pos + output_len > bytes.len() {
                return Err(super::ExecutorError::InvalidResponse(
                    "truncated output bytes".into(),
                ));
            }
            let output = CellOutput::from_slice(&bytes[pos..pos + output_len]).map_err(|e| {
                super::ExecutorError::InvalidResponse(format!(
                    "invalid CellOutput molecule: {:?}",
                    e
                ))
            })?;
            pos += output_len;
            outputs.push(output);

            // Read data length
            if pos + 4 > bytes.len() {
                return Err(super::ExecutorError::InvalidResponse(
                    "truncated data length".into(),
                ));
            }
            let data_len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;

            // Read data bytes
            if pos + data_len > bytes.len() {
                return Err(super::ExecutorError::InvalidResponse(
                    "truncated data bytes".into(),
                ));
            }
            outputs_data.push(bytes[pos..pos + data_len].to_vec());
            pos += data_len;
        }

        Ok(PartialTx {
            outputs,
            outputs_data,
        })
    }

    /// Molecule wire bytes for each output plus its data.
    ///
    /// Lets host crates (e.g. the Process calculator) reconstruct CellOutput
    /// with a different ckb-gen-types version without importing Entity.
    pub fn output_molecule_pairs(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.outputs
            .iter()
            .zip(self.outputs_data.iter())
            .map(|(output, data)| (output.as_slice().to_vec(), data.clone()))
            .collect()
    }
}

/// Executor errors
#[derive(Debug)]
pub enum ExecutorError {
    /// Process rejected the batch (incompatible) with custom exit code >= 20
    Incompatible(i8),
    /// VM execution failed
    VmError(String),
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
            ExecutorError::InvalidResponse(msg) => write!(f, "Invalid SSRI response: {}", msg),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Custom error start for domain errors
pub const CUSTOM_ERROR_START: i8 = 20;
