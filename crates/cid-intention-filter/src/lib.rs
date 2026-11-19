//! Intention routing (Sequencer) and `Process.filter` wire format.
//!
//! Process-independent helpers shared by the Sequencer host and Process
//! filter/assemble paths. **Not** linked into the CID lock binary — lives
//! outside `cid-protocol` so CID stays byte-identical across filter work.

#![no_std]

extern crate alloc;

mod filter;
mod route;

pub use filter::{
    decode_filter_request, decode_filter_response, encode_filter_request, encode_filter_response,
    FilterCell, FilterRequest, FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM,
    FILTER_MALFORMED_CELL, FILTER_MALFORMED_DIGEST, FILTER_MAX_CELLS, FILTER_NOT_INTENTION,
    FILTER_PROCESS_MISMATCH, FILTER_UNSUPPORTED_ACTION, FILTER_UNSUPPORTED_ASSET,
};
pub use route::{
    intention_belongs_to_process, intention_process_type_id, intention_process_type_id_parts,
    IntentionRouteError,
};
