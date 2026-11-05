//! Process Calculator for CID Process example

use ckb_cinnabar_calculator::{CalculatorError, Result};

pub const PROCESS_CONTRACT_NAME: &str = "cid_process_example";

/// Load the Process binary from build/release/<name>
pub fn load_process_binary(bin_dir: &str) -> Result<Vec<u8>> {
    let path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
    std::fs::read(&path).map_err(|e| {
        CalculatorError::Other(format!("failed to load Process binary at {}: {}", path, e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_binary_missing() {
        let result = load_process_binary("/nonexistent");
        assert!(result.is_err());
    }
}
