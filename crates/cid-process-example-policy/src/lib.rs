//! Example Process deposit + Intention classification policy (Process-owned).
//!
//! Single source of truth for rules shared by `Process.filter`, `Process.assemble`,
//! and on-chain Verify. Protocol / CID impose no Intention or deposit floor.
//!
//! Ownership (CID Type-ID lock + Process Type ID) uses
//! [`cid_protocol::intention_belongs_to_process`] — same helper the Sequencer
//! uses for routing.

#![no_std]

extern crate alloc;

use cid_protocol::{
    intention_belongs_to_process, is_standard_xudt, CidArgs, IntentionRouteError, Script,
    FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM, FILTER_MALFORMED_DIGEST, FILTER_NOT_INTENTION,
    FILTER_PROCESS_MISMATCH, FILTER_UNSUPPORTED_ASSET, XUDT_AMOUNT_LEN,
};

/// Minimum CKB capacity (shannons) for a deposit Intention owned by this
/// Process: **600 CKB**.
pub const MIN_DEPOSIT_CAPACITY: u64 = 600_0000_0000;

/// Reject CKB-only deposit Intentions below [`MIN_DEPOSIT_CAPACITY`].
#[inline]
pub fn ckb_deposit_capacity_ok(capacity: u64, has_type_script: bool) -> bool {
    has_type_script || capacity >= MIN_DEPOSIT_CAPACITY
}

/// Map protocol ownership errors to `Process.filter` reason codes.
pub fn filter_reason_from_route_error(err: IntentionRouteError) -> i8 {
    match err {
        IntentionRouteError::ProcessMismatch => FILTER_PROCESS_MISMATCH,
        IntentionRouteError::WrongHashType
        | IntentionRouteError::WrongCodeHash
        | IntentionRouteError::ArgsTooShort
        | IntentionRouteError::MalformedArgs
        | IntentionRouteError::UnsupportedVersion => FILTER_NOT_INTENTION,
    }
}

/// Classify one Intention for this Process (ownership + asset + deposit + digest).
///
/// Returns a `FILTER_*` reason (`FILTER_ACCEPTED` on success). Used by filter
/// and mirrored by assemble / verify so rules stay consistent.
pub fn classify_intention(
    expected_process_type_hash: &[u8; 32],
    cid_code_type_id_hash: &[u8; 32],
    lock: &Script,
    capacity: u64,
    type_script: Option<(&[u8; 32], u8)>,
    data: &[u8],
) -> i8 {
    if let Err(e) =
        intention_belongs_to_process(lock, cid_code_type_id_hash, expected_process_type_hash)
    {
        return filter_reason_from_route_error(e);
    }

    let Ok(cid) = CidArgs::decode(&lock.args) else {
        return FILTER_NOT_INTENTION;
    };
    // Example Process DigestOk requires a beneficiary Script in the payload.
    if cid.entry.payload.to_script().is_err() {
        return FILTER_MALFORMED_DIGEST;
    }

    match type_script {
        None => {
            if !ckb_deposit_capacity_ok(capacity, false) {
                return FILTER_DEPOSIT_BELOW_MINIMUM;
            }
        }
        Some((code_hash, hash_type)) => {
            if !is_standard_xudt(code_hash, hash_type) {
                return FILTER_UNSUPPORTED_ASSET;
            }
            if data.len() < XUDT_AMOUNT_LEN {
                return FILTER_UNSUPPORTED_ASSET;
            }
        }
    }

    FILTER_ACCEPTED
}

/// Host/guest helper when type script is absent or presented as code_hash+hash_type.
pub fn classify_intention_ckb_or_xudt(
    expected_process_type_hash: &[u8; 32],
    cid_code_type_id_hash: &[u8; 32],
    lock: &Script,
    capacity: u64,
    type_code_hash: Option<[u8; 32]>,
    type_hash_type: u8,
    data: &[u8],
) -> i8 {
    let typ = type_code_hash.as_ref().map(|h| (h, type_hash_type));
    classify_intention(
        expected_process_type_hash,
        cid_code_type_id_hash,
        lock,
        capacity,
        typ,
        data,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cid_protocol::{Payload, SCRIPT_HASH_TYPE_TYPE, XUDT_CODE_HASH};

    fn lock_for(pth: [u8; 32], cid_tid: [u8; 32]) -> Script {
        let args = CidArgs::new(
            pth,
            Script::default(),
            Payload::from_script(&Script {
                code_hash: [0xBE; 32],
                hash_type: 1,
                args: alloc::vec![1],
            })
            .unwrap(),
        )
        .encode()
        .unwrap();
        Script {
            code_hash: cid_tid,
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args,
        }
    }

    #[test]
    fn accepts_own_ckb_deposit_at_minimum() {
        let cid = [0xC1; 32];
        let pth = [0xA1; 32];
        let lock = lock_for(pth, cid);
        assert_eq!(
            classify_intention(&pth, &cid, &lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_ACCEPTED
        );
    }

    #[test]
    fn rejects_foreign_process() {
        let cid = [0xC1; 32];
        let lock = lock_for([0xA1; 32], cid);
        assert_eq!(
            classify_intention(&[0xB2; 32], &cid, &lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_PROCESS_MISMATCH
        );
    }

    #[test]
    fn rejects_non_cid_lock() {
        let cid = [0xC1; 32];
        let pth = [0xA1; 32];
        let mut lock = lock_for(pth, cid);
        lock.hash_type = 2;
        assert_eq!(
            classify_intention(&pth, &cid, &lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_NOT_INTENTION
        );
    }

    #[test]
    fn rejects_below_min_deposit() {
        let cid = [0xC1; 32];
        let pth = [0xA1; 32];
        let lock = lock_for(pth, cid);
        assert_eq!(
            classify_intention(&pth, &cid, &lock, MIN_DEPOSIT_CAPACITY - 1, None, &[]),
            FILTER_DEPOSIT_BELOW_MINIMUM
        );
    }

    #[test]
    fn accepts_xudt_below_ckb_min() {
        let cid = [0xC1; 32];
        let pth = [0xA1; 32];
        let lock = lock_for(pth, cid);
        let data = [0u8; 16];
        assert_eq!(
            classify_intention(&pth, &cid, &lock, 1, Some((&XUDT_CODE_HASH, 2)), &data),
            FILTER_ACCEPTED
        );
    }
}
