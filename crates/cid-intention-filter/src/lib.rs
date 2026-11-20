//! Intention routing (Sequencer) and `Process.filter` wire format.
//!
//! - **Routing / ownership:** [`intention_process_type_id`] /
//!   [`intention_belongs_to_process`] — Sequencer-only.
//! - **Filter wire:** request/response framing + reason codes for Process
//!   business compliance (no ownership re-check).
//!
//! **Not** linked into the CID lock binary — lives outside `cid-protocol` so
//! CID stays byte-identical across filter work.

#![no_std]

extern crate alloc;

mod filter;
mod route;

use alloc::vec::Vec;

pub use filter::{
    decode_filter_request, decode_filter_response, encode_filter_request, encode_filter_response,
    FilterCell, FilterRequest, FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM,
    FILTER_MALFORMED_CELL, FILTER_MALFORMED_DIGEST, FILTER_MAX_CELLS, FILTER_RESERVED_2,
    FILTER_RESERVED_3, FILTER_UNSUPPORTED_ACTION, FILTER_UNSUPPORTED_ASSET,
};
pub use route::{
    intention_belongs_to_process, intention_process_type_id, intention_process_type_id_parts,
    IntentionRouteError,
};

/// Soft cap for `Process.filter` stdin (bytes). Exactly this size is accepted;
/// anything larger is rejected by the guest reader.
pub const FILTER_STDIN_MAX: usize = 64 * 1024;

/// Why a capped read failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CappedReadError {
    /// More than `max` bytes were offered, or length arithmetic overflowed.
    Overflow,
    /// The underlying reader returned an error.
    Read,
}

/// Accumulate bytes from successive `read` calls up to `max` **inclusive**.
///
/// Returns [`CappedReadError::Overflow`] if more than `max` bytes are offered.
/// Used by the Process guest stdin reader so a payload of exactly `max`
/// succeeds and only `> max` fails — unit-tested with slice-backed readers.
pub fn read_capped_from<F>(max: usize, mut read: F) -> Result<Vec<u8>, CappedReadError>
where
    F: FnMut(&mut [u8]) -> Result<usize, CappedReadError>,
{
    let cap = max.checked_add(1).ok_or(CappedReadError::Overflow)?;
    let mut buf = alloc::vec![0u8; cap];
    let mut total = 0usize;
    loop {
        let n = read(&mut buf[total..])?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n).ok_or(CappedReadError::Overflow)?;
        if total > max {
            return Err(CappedReadError::Overflow);
        }
    }
    buf.truncate(total);
    Ok(buf)
}

#[cfg(test)]
mod capped_read_tests {
    use super::*;
    use alloc::vec;

    fn slice_reader<'a>(
        data: &'a [u8],
        chunk: usize,
    ) -> impl FnMut(&mut [u8]) -> Result<usize, CappedReadError> + 'a {
        let mut offset = 0usize;
        move |dst: &mut [u8]| {
            if offset >= data.len() || dst.is_empty() {
                return Ok(0);
            }
            let n = core::cmp::min(chunk, core::cmp::min(dst.len(), data.len() - offset));
            dst[..n].copy_from_slice(&data[offset..offset + n]);
            offset += n;
            Ok(n)
        }
    }

    #[test]
    fn accepts_exact_max() {
        let data = vec![0xABu8; 100];
        let out = read_capped_from(100, slice_reader(&data, 32)).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn accepts_exact_max_single_read() {
        let data = vec![0xCDu8; 64];
        let out = read_capped_from(64, slice_reader(&data, 64)).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn rejects_one_past_max() {
        let data = vec![0xEFu8; 101];
        assert!(read_capped_from(100, slice_reader(&data, 40)).is_err());
    }

    #[test]
    fn accepts_empty() {
        let data: &[u8] = &[];
        let out = read_capped_from(10, slice_reader(data, 1)).unwrap();
        assert!(out.is_empty());
    }
}
