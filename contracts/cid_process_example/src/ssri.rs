extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use ckb_cinnabar_verifier::ssri::{SsriArgs, SsriSource};
use crate::ProcessError;

const BATCH_MIN: u32 = 1;
const BATCH_MAX: u32 = 4;

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

/// SSRI handler for Process.assemble
/// Reads intentions from stdin, generates partial tx
pub fn assemble_handler(
    _src: &SsriSource,
    _args: SsriArgs,
) -> core::result::Result<Vec<u8>, ProcessError> {
    // Read intention count from stdin (fd=0)
    let mut count_buf = [0u8; 4];
    let bytes_read = ckb_std::syscalls::read(0, &mut count_buf)
        .map_err(|_| ProcessError::ReadFailed)?;
    
    if bytes_read != count_buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    
    let count = u32::from_le_bytes(count_buf);
    
    // Validate batch bounds
    if count < BATCH_MIN || count > BATCH_MAX {
        return Err(ProcessError::IncompatibleBatch);
    }
    
    // For this sample, stub partial tx output
    // Real implementation would:
    // 1. Read full intention data from stdin
    // 2. Parse payloads to extract beneficiary Scripts
    // 3. Generate outputs: beneficiary cells + Process recreate
    // 4. Serialize partial tx to stdout
    
    // Stub: return minimal valid response
    Ok(vec![0u8; 4])
}
