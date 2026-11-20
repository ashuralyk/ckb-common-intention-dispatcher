//! Process.filter request / response framing and stable reason codes.
//!
//! Wire format is Process-protocol (shared by every Process implementation).
//! Application rules that produce these codes are Process-owned.
//!
//! **Callers must route first** with [`crate::intention_process_type_id`] /
//! [`crate::intention_belongs_to_process`]. `Process.filter` does **not** verify
//! CID Type-ID ownership — that is solely the Sequencer's responsibility.

use alloc::vec::Vec;
use cid_protocol::CodecError;

/// Accepted by `Process.filter`.
pub const FILTER_ACCEPTED: i8 = 0;
/// Packed `CellOutput` / framing for this cell could not be parsed.
pub const FILTER_MALFORMED_CELL: i8 = 1;
/// Reserved (was ownership / not-Intention). Ownership is Sequencer-only.
pub const FILTER_RESERVED_2: i8 = 2;
/// Reserved (was process mismatch). Ownership is Sequencer-only.
pub const FILTER_RESERVED_3: i8 = 3;
/// Typed asset is not supported by this Process (e.g. non-official xUDT).
pub const FILTER_UNSUPPORTED_ASSET: i8 = 4;
/// CKB deposit capacity below this Process's minimum.
pub const FILTER_DEPOSIT_BELOW_MINIMUM: i8 = 5;
/// Intention digest / payload cannot be decoded for this Process's actions.
pub const FILTER_MALFORMED_DIGEST: i8 = 6;
/// Intention encodes an action this Process does not support.
pub const FILTER_UNSUPPORTED_ACTION: i8 = 7;

/// Soft cap on cells per `Process.filter` call (request framing / guest buffer).
pub const FILTER_MAX_CELLS: u32 = 64;

/// One Intention candidate: packed molecule `OutPoint`, packed `CellOutput`, data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterCell {
    pub out_point: Vec<u8>,
    pub output: Vec<u8>,
    pub data: Vec<u8>,
}

/// `Process.filter` stdin payload.
///
/// Cells only — Sequencer must already have routed Intentions to this Process.
/// No `process_type_hash` / `cid_code_type_id_hash` (ownership is not re-checked).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterRequest {
    pub cells: Vec<FilterCell>,
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn read_u32(bytes: &[u8], off: &mut usize) -> Result<u32, CodecError> {
    let end = off.checked_add(4).ok_or(CodecError::TooShort)?;
    if bytes.len() < end {
        return Err(CodecError::TooShort);
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[*off..end]);
    *off = end;
    Ok(u32::from_le_bytes(buf))
}

fn read_bytes(bytes: &[u8], off: &mut usize) -> Result<Vec<u8>, CodecError> {
    let len = read_u32(bytes, off)? as usize;
    let end = off.checked_add(len).ok_or(CodecError::TooShort)?;
    if bytes.len() < end {
        return Err(CodecError::TooShort);
    }
    let slice = bytes[*off..end].to_vec();
    *off = end;
    Ok(slice)
}

/// Encode a [`FilterRequest`] for SSRI stdin.
///
/// Layout (little-endian lengths):
/// `count:u32 || repeated (out_point_len:u32 || out_point || output_len:u32 || output
///               || data_len:u32 || data)`
pub fn encode_filter_request(req: &FilterRequest) -> Result<Vec<u8>, CodecError> {
    if req.cells.len() > FILTER_MAX_CELLS as usize {
        return Err(CodecError::Encode);
    }
    let mut out = Vec::new();
    push_u32(&mut out, req.cells.len() as u32);
    for cell in &req.cells {
        push_u32(&mut out, cell.out_point.len() as u32);
        out.extend_from_slice(&cell.out_point);
        push_u32(&mut out, cell.output.len() as u32);
        out.extend_from_slice(&cell.output);
        push_u32(&mut out, cell.data.len() as u32);
        out.extend_from_slice(&cell.data);
    }
    Ok(out)
}

/// Decode a [`FilterRequest`] from SSRI stdin.
pub fn decode_filter_request(bytes: &[u8]) -> Result<FilterRequest, CodecError> {
    let mut off = 0;
    let count = read_u32(bytes, &mut off)?;
    if count > FILTER_MAX_CELLS {
        return Err(CodecError::Decode);
    }
    let mut cells = Vec::with_capacity(count as usize);
    for _ in 0..count {
        cells.push(FilterCell {
            out_point: read_bytes(bytes, &mut off)?,
            output: read_bytes(bytes, &mut off)?,
            data: read_bytes(bytes, &mut off)?,
        });
    }
    if off != bytes.len() {
        return Err(CodecError::Decode);
    }
    Ok(FilterRequest { cells })
}

/// Encode filter stdout: `count:u32 LE || reason:i8 * count` (order = request order).
pub fn encode_filter_response(reasons: &[i8]) -> Result<Vec<u8>, CodecError> {
    if reasons.len() > FILTER_MAX_CELLS as usize {
        return Err(CodecError::Encode);
    }
    let mut out = Vec::with_capacity(4 + reasons.len());
    push_u32(&mut out, reasons.len() as u32);
    for r in reasons {
        out.push(*r as u8);
    }
    Ok(out)
}

/// Decode filter stdout into per-cell reason codes.
pub fn decode_filter_response(bytes: &[u8]) -> Result<Vec<i8>, CodecError> {
    let mut off = 0;
    let count = read_u32(bytes, &mut off)? as usize;
    if count > FILTER_MAX_CELLS as usize {
        return Err(CodecError::Decode);
    }
    let end = off.checked_add(count).ok_or(CodecError::TooShort)?;
    if bytes.len() < end {
        return Err(CodecError::TooShort);
    }
    if bytes.len() > end {
        return Err(CodecError::Decode);
    }
    Ok(bytes[off..end].iter().map(|b| *b as i8).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use cid_protocol::CodecError;

    #[test]
    fn request_roundtrip_empty() {
        let req = FilterRequest { cells: vec![] };
        let bytes = encode_filter_request(&req).unwrap();
        assert_eq!(decode_filter_request(&bytes).unwrap(), req);
        let resp = encode_filter_response(&[]).unwrap();
        assert_eq!(decode_filter_response(&resp).unwrap(), vec![]);
    }

    #[test]
    fn request_response_roundtrip() {
        let req = FilterRequest {
            cells: vec![
                FilterCell {
                    out_point: vec![1, 2, 3],
                    output: vec![4, 5],
                    data: vec![],
                },
                FilterCell {
                    out_point: vec![9],
                    output: vec![8, 7, 6],
                    data: vec![0xAB],
                },
            ],
        };
        let bytes = encode_filter_request(&req).unwrap();
        assert_eq!(decode_filter_request(&bytes).unwrap(), req);
        let reasons = vec![FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM];
        let resp = encode_filter_response(&reasons).unwrap();
        assert_eq!(decode_filter_response(&resp).unwrap(), reasons);
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let req = FilterRequest { cells: vec![] };
        let mut bytes = encode_filter_request(&req).unwrap();
        bytes.push(0xFF);
        assert_eq!(decode_filter_request(&bytes), Err(CodecError::Decode));
    }

    #[test]
    fn decode_rejects_oversize_count() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(FILTER_MAX_CELLS + 1).to_le_bytes());
        assert_eq!(decode_filter_request(&bytes), Err(CodecError::Decode));
    }

    #[test]
    fn decode_rejects_truncated_after_count() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(decode_filter_request(&bytes), Err(CodecError::TooShort));
    }

    #[test]
    fn decode_rejects_truncated_length_prefix() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2]);
        assert_eq!(decode_filter_request(&bytes), Err(CodecError::TooShort));
    }

    #[test]
    fn decode_rejects_short_header() {
        assert_eq!(decode_filter_request(&[]), Err(CodecError::TooShort));
    }

    #[test]
    fn decode_rejects_length_overflow_on_offset() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_filter_request(&bytes), Err(CodecError::TooShort));
    }

    #[test]
    fn decode_response_rejects_trailing_bytes() {
        let mut resp = encode_filter_response(&[FILTER_ACCEPTED]).unwrap();
        resp.push(0);
        assert_eq!(decode_filter_response(&resp), Err(CodecError::Decode));
    }

    #[test]
    fn decode_response_rejects_truncated() {
        assert_eq!(
            decode_filter_response(&[1, 0, 0, 0]),
            Err(CodecError::TooShort)
        );
    }

    #[test]
    fn decode_response_rejects_oversize_count() {
        let mut bytes = (FILTER_MAX_CELLS + 1).to_le_bytes().to_vec();
        bytes.extend(core::iter::repeat_n(0u8, (FILTER_MAX_CELLS + 1) as usize));
        assert_eq!(decode_filter_response(&bytes), Err(CodecError::Decode));
    }
}
