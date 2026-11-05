use ckb_types::packed::{CellOutput, OutPoint};

/// Input intention cell for assemble
#[derive(Clone, Debug, Default)]
pub struct IntentionInput {
    pub out_point: OutPoint,
    pub lock_args: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
}

/// Partial transaction returned by assemble
#[derive(Clone, Debug)]
pub struct PartialTx {
    pub outputs: Vec<CellOutput>,
    pub outputs_data: Vec<Vec<u8>>,
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
            ExecutorError::Incompatible(code) => write!(f, "Process rejected batch (exit {})", code),
            ExecutorError::VmError(msg) => write!(f, "VM error: {}", msg),
            ExecutorError::InvalidResponse(msg) => write!(f, "Invalid SSRI response: {}", msg),
        }
    }
}

impl std::error::Error for ExecutorError {}

/// Custom error start for domain errors
pub const CUSTOM_ERROR_START: i8 = 20;
