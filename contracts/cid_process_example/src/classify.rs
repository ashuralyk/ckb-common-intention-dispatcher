//! Shared Intention **business** classification for filter / assemble / verify.
//!
//! Wraps `cid_process_example_policy::classify_intention` with packed cell
//! parsing. Does **not** check CID Type-ID ownership — Sequencer routes first;
//! on-chain Verify keeps `ours_prefix` scoping separately.

use cid_intention_filter::{
    FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM, FILTER_MALFORMED_CELL, FILTER_MALFORMED_DIGEST,
    FILTER_UNSUPPORTED_ASSET,
};
use cid_process_example_policy::classify_intention;
use cid_protocol::Script as CidScript;
use ckb_std::ckb_types::packed::{CellOutput, Script};
use ckb_std::ckb_types::prelude::*;

use crate::ProcessError;

fn packed_to_cid_script(script: &Script) -> CidScript {
    let code_hash: [u8; 32] = script.code_hash().unpack();
    let hash_type: u8 = script.hash_type().into();
    CidScript {
        code_hash,
        hash_type,
        args: script.args().raw_data().to_vec(),
    }
}

/// Classify one packed Intention cell for Process business rules only.
pub fn classify_packed_intention(output_bytes: &[u8], data: &[u8]) -> i8 {
    let Ok(output) = CellOutput::from_slice(output_bytes) else {
        return FILTER_MALFORMED_CELL;
    };
    let lock = packed_to_cid_script(&output.lock());
    let capacity: u64 = output.capacity().unpack();
    let type_script = output.type_().to_opt().map(|t| {
        let code_hash: [u8; 32] = t.code_hash().unpack();
        let hash_type: u8 = t.hash_type().into();
        (code_hash, hash_type)
    });
    let typ_ref = type_script.as_ref().map(|(h, ht)| (h, *ht));
    classify_intention(&lock, capacity, typ_ref, data)
}

/// Map filter reason → assemble / on-chain ProcessError (batch-aborting path).
pub fn filter_reason_to_process_error(reason: i8) -> Option<ProcessError> {
    match reason {
        x if x == FILTER_ACCEPTED => None,
        x if x == FILTER_MALFORMED_CELL => Some(ProcessError::IncompatibleBatch),
        x if x == FILTER_UNSUPPORTED_ASSET => Some(ProcessError::UnsupportedAsset),
        x if x == FILTER_MALFORMED_DIGEST => Some(ProcessError::BeneficiaryMissing),
        x if x == FILTER_DEPOSIT_BELOW_MINIMUM => Some(ProcessError::DepositBelowMinimum),
        _ => Some(ProcessError::IncompatibleBatch),
    }
}

/// Classify a live packed lock + capacity + optional type for assemble/verify.
pub fn classify_live_intention(
    lock: &Script,
    capacity: u64,
    type_script: Option<&Script>,
    data: &[u8],
) -> i8 {
    let cid_lock = packed_to_cid_script(lock);
    let typ = type_script.map(|t| {
        let code_hash: [u8; 32] = t.code_hash().unpack();
        let hash_type: u8 = t.hash_type().into();
        (code_hash, hash_type)
    });
    let typ_ref = typ.as_ref().map(|(h, ht)| (h, *ht));
    classify_intention(&cid_lock, capacity, typ_ref, data)
}
