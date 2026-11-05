extern crate alloc;

use alloc::{borrow::Cow, vec, vec::Vec};
use crate::Error;

const BATCH_MIN: u32 = 1;
const BATCH_MAX: u32 = 4;

/// SSRI handler for Process.batch
/// Returns (batch_min, batch_max) as 8 bytes little-endian
pub fn batch_handler() -> Result<Cow<'static, [u8]>, Error> {
    let mut output = vec![0u8; 8];
    output[0..4].copy_from_slice(&BATCH_MIN.to_le_bytes());
    output[4..8].copy_from_slice(&BATCH_MAX.to_le_bytes());
    
    Ok(Cow::from(output))
}

/// SSRI handler for Process.assemble
/// Reads intentions from stdin, generates partial tx, writes to stdout
pub fn assemble_handler() -> Result<Cow<'static, [u8]>, Error> {
    // Read intention count from stdin
    let mut count_buf = [0u8; 4];
    let bytes_read = ckb_std::syscalls::read(0, &mut count_buf)
        .map_err(|_| Error::VerifyError(20))?;
    
    if bytes_read != count_buf.len() {
        return Err(Error::VerifyError(20)); // CUSTOM_ERROR_START: read failed
    }
    
    let count = u32::from_le_bytes(count_buf);
    
    // Validate batch bounds
    if count < BATCH_MIN || count > BATCH_MAX {
        return Err(Error::VerifyError(21)); // INCOMPATIBLE_BATCH_COUNT
    }
    
    // For this sample, stub partial tx output
    // Real implementation would:
    // 1. Read full intention data from stdin
    // 2. Parse payloads to extract beneficiary Scripts
    // 3. Generate outputs: beneficiary cells + Process recreate
    // 4. Serialize partial tx to stdout
    
    // Stub: write minimal valid response
    let stub_output = vec![0u8; 4];
    Ok(Cow::from(stub_output))
}
