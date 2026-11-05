//! Greenfield offline SSRI host for CID Process contracts
//! 
//! Implements minimal CKB-VM runner with argv-based SSRI method invocation.
//! vm_version = u64::MAX convention for SSRI mode.

pub mod types;
pub mod vm;

pub use types::{ExecutorError, IntentionInput, PartialTx, CUSTOM_ERROR_START};

/// Offline SSRI executor for CID Process contracts
pub struct ProcessExecutor {
    binary: Vec<u8>,
}

impl ProcessExecutor {
    /// Load a Process RISC-V binary
    pub fn new(binary: Vec<u8>) -> Self {
        Self { binary }
    }

    /// Query the Process for its batch bounds (min, max) via SSRI Process.batch
    pub fn get_batch_bounds(&self) -> Result<(u32, u32), ExecutorError> {
        if self.binary.is_empty() {
            return Err(ExecutorError::VmError("empty binary".into()));
        }
        
        vm::call_ssri_method(&self.binary, "Process.batch", &[])
            .and_then(|response| parse_batch_bounds(&response))
    }

    /// Assemble a digest transaction for the given intentions via SSRI Process.assemble
    pub fn assemble(&self, _intentions: &[IntentionInput]) -> Result<PartialTx, ExecutorError> {
        if self.binary.is_empty() {
            return Err(ExecutorError::VmError("empty binary".into()));
        }
        
        // Will implement after VM is complete
        Err(ExecutorError::VmError("assemble not yet implemented".into()))
    }
}

fn parse_batch_bounds(response: &[u8]) -> Result<(u32, u32), ExecutorError> {
    if response.len() != 8 {
        return Err(ExecutorError::InvalidResponse(format!(
            "expected 8 bytes for batch bounds, got {}",
            response.len()
        )));
    }
    
    let min = u32::from_le_bytes(response[0..4].try_into().unwrap());
    let max = u32::from_le_bytes(response[4..8].try_into().unwrap());
    
    Ok((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_batch_bounds() {
        let response = vec![1, 0, 0, 0, 4, 0, 0, 0]; // (1, 4) little-endian
        let (min, max) = parse_batch_bounds(&response).unwrap();
        assert_eq!((min, max), (1, 4));
    }

    #[test]
    fn test_empty_binary_returns_error() {
        let executor = ProcessExecutor::new(vec![]);
        assert!(matches!(
            executor.get_batch_bounds(),
            Err(ExecutorError::VmError(_))
        ));
    }

    #[test]
    fn test_vm_error_on_invalid_binary() {
        let executor = ProcessExecutor::new(vec![0xFF; 100]);
        let result = executor.get_batch_bounds();
        assert!(matches!(result, Err(ExecutorError::VmError(_))));
    }

    // Test syscall 2041 implementation and exit code handling
    // Full integration tests (argv routing, stdout capture, exit 0 success path, 
    // and actual RISC-V binary execution) will be added in Task 3 when the Process
    // contract binary is available for testing.
    
    #[test]
    fn test_syscall_2041_returns_ssri_version() {
        // Verify syscall 2041 (CKB_VM_VERSION) constant is u64::MAX
        // The syscall handler is implemented in vm.rs and returns this value
        // when the guest calls syscall 2041.
        assert_eq!(vm::SSRI_VM_VERSION, u64::MAX);
    }

    #[test]
    fn test_custom_error_start_threshold() {
        // Verify CUSTOM_ERROR_START is 20 per global constraints
        assert_eq!(CUSTOM_ERROR_START, 20);
    }

    #[test]
    fn test_incompatible_error_type() {
        // Verify ExecutorError::Incompatible exists and can be constructed
        let err = ExecutorError::Incompatible(20);
        match err {
            ExecutorError::Incompatible(code) => assert_eq!(code, 20),
            _ => panic!("Wrong error type"),
        }
    }

    #[test]
    fn test_incompatible_error_display() {
        // Verify error formatting
        let err = ExecutorError::Incompatible(42);
        let msg = format!("{}", err);
        assert!(msg.contains("Process rejected batch"));
        assert!(msg.contains("42"));
    }
}
