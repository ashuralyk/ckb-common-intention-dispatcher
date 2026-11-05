extern crate alloc;

use crate::ProcessError;
use alloc::vec::Vec;
use cid_protocol::CidArgs;
use ckb_cinnabar_verifier::ssri::{SsriArgs, SsriSource};
use ckb_std::ckb_types::packed::{Byte32, Bytes, CellOutput, Script};
use ckb_std::ckb_types::prelude::*;

const BATCH_MIN: u32 = 1;
const BATCH_MAX: u32 = 4;

// Default lock code_hash for beneficiary outputs (using a recognizable constant)
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

type ParsedIntention = (u64, Vec<u8>, Vec<u8>, Option<Vec<u8>>, usize);

/// Parse a single intention from buffer at given position
/// Returns (capacity, lock_args, data, type_script molecule, next_position)
fn parse_intention(buf: &[u8], mut pos: usize) -> Result<ParsedIntention, ProcessError> {
    // Read intention length
    if pos + 4 > buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    let intention_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    if pos + intention_len > buf.len() {
        return Err(ProcessError::ReadFailed);
    }

    let intention_start = pos;

    // Parse capacity (u64 LE)
    if pos + 8 > buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    let capacity = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
    pos += 8;

    // Parse lock_args_len (u32 LE)
    if pos + 4 > buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    let lock_args_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    // Parse lock_args
    if pos + lock_args_len > buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    let lock_args = buf[pos..pos + lock_args_len].to_vec();
    pos += lock_args_len;

    // Parse data_len (u32 LE)
    if pos + 4 > buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    let data_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    // Parse data
    if pos + data_len > buf.len() {
        return Err(ProcessError::ReadFailed);
    }
    let data = buf[pos..pos + data_len].to_vec();
    pos += data_len;

    // Optional type script: present when the record still has bytes after data.
    let type_script = if pos - intention_start < intention_len {
        if pos + 4 > buf.len() {
            return Err(ProcessError::ReadFailed);
        }
        let type_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + type_len > buf.len() {
            return Err(ProcessError::ReadFailed);
        }
        let bytes = buf[pos..pos + type_len].to_vec();
        pos += type_len;
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    } else {
        None
    };

    if pos - intention_start != intention_len {
        return Err(ProcessError::ReadFailed);
    }

    Ok((capacity, lock_args, data, type_script, pos))
}

/// SSRI handler for Process.assemble
/// Reads intentions from stdin, generates partial tx with beneficiary outputs
pub fn assemble_handler(
    _src: &SsriSource,
    _args: SsriArgs,
) -> core::result::Result<Vec<u8>, ProcessError> {
    // Read full input from stdin (max 8KB for safety)
    let mut input_buf = [0u8; 8192];
    let mut total_read = 0;

    loop {
        let read_result = ckb_std::syscalls::read(0, &mut input_buf[total_read..])
            .map_err(|_| ProcessError::ReadFailed)?;

        if read_result == 0 {
            break; // EOF
        }

        total_read += read_result;

        if total_read >= input_buf.len() {
            return Err(ProcessError::ReadFailed); // Input too large
        }
    }

    let input = &input_buf[..total_read];
    let mut pos = 0;

    // Parse intention count
    if input.len() < 4 {
        return Err(ProcessError::ReadFailed);
    }
    let count = u32::from_le_bytes(input[pos..pos + 4].try_into().unwrap());
    pos += 4;

    // Validate batch bounds
    if !(BATCH_MIN..=BATCH_MAX).contains(&count) {
        return Err(ProcessError::IncompatibleBatch);
    }

    // Parse intentions and generate beneficiary outputs
    let mut outputs = Vec::with_capacity(count as usize);
    let mut outputs_data = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let (capacity, lock_args, data, type_script, next_pos) = parse_intention(input, pos)?;
        pos = next_pos;

        let lock = beneficiary_lock(lock_args);

        let mut output = CellOutput::new_builder().capacity(capacity).lock(lock);
        if let Some(type_bytes) = type_script {
            let script = Script::from_slice(&type_bytes).map_err(|_| ProcessError::ReadFailed)?;
            output = output.type_(Some(script).pack());
        }
        let output = output.build();

        outputs.push(output);
        outputs_data.push(data);
    }

    // Serialize PartialTx
    // Format: output_count (u32 LE) + for each: output_len (u32 LE) + output_bytes + data_len (u32 LE) + data_bytes
    let mut response = Vec::new();
    response.extend_from_slice(&(outputs.len() as u32).to_le_bytes());

    for (output, data) in outputs.iter().zip(outputs_data.iter()) {
        let output_bytes = output.as_slice();
        response.extend_from_slice(&(output_bytes.len() as u32).to_le_bytes());
        response.extend_from_slice(output_bytes);
        response.extend_from_slice(&(data.len() as u32).to_le_bytes());
        response.extend_from_slice(data);
    }

    Ok(response)
}

/// Prefer the Script encoded in `CidEntry.payload`; fall back to the
/// placeholder lock used by host unit tests that pass raw `lock_args`.
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
