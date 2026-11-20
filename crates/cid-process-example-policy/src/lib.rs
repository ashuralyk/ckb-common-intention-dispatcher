//! Example Process deposit + Intention classification policy (Process-owned).
//!
//! Single source of truth for **business** rules shared by `Process.filter`,
//! `Process.assemble`, and on-chain Verify (deposit floor, supported assets,
//! digest/beneficiary shape, fee budget). Protocol / CID impose no Intention
//! or deposit floor.
//!
//! **Ownership is not checked here.** The Sequencer routes with
//! [`cid_intention_filter::intention_belongs_to_process`] before calling filter;
//! on-chain Verify keeps its own `ours_prefix` scoping.

#![no_std]

extern crate alloc;

use cid_intention_filter::{
    FILTER_ACCEPTED, FILTER_DEPOSIT_BELOW_MINIMUM, FILTER_MALFORMED_DIGEST,
    FILTER_UNSUPPORTED_ASSET,
};
use cid_protocol::{is_standard_xudt, CidArgs, Script, XUDT_AMOUNT_LEN};

/// Minimum CKB capacity (shannons) for a deposit Intention owned by this
/// Process: **600 CKB**.
pub const MIN_DEPOSIT_CAPACITY: u64 = 600_0000_0000;

/// Process-owned fee rate (shannons per kB), matching testnet `min_fee_rate`.
pub const FEE_RATE: u64 = 1000;

/// CKB `FeeRate(FEE_RATE).fee(serialized_size_in_block)` with
/// `serialized_size_in_block = molecule_len + 4`.
#[inline]
pub fn fee_from_tx_molecule_len(molecule_len: u64) -> u64 {
    let weight = molecule_len.saturating_add(4);
    weight.saturating_mul(FEE_RATE) / 1000
}

/// Reject CKB-only deposit Intentions below [`MIN_DEPOSIT_CAPACITY`].
#[inline]
pub fn ckb_deposit_capacity_ok(capacity: u64, has_type_script: bool) -> bool {
    has_type_script || capacity >= MIN_DEPOSIT_CAPACITY
}

/// Beneficiary / lock `hash_type` values CKB recognizes (`data`/`type`/`data1`/`data2`).
#[inline]
pub fn is_supported_script_hash_type(hash_type: u8) -> bool {
    matches!(hash_type, 0 | 1 | 2 | 4)
}

/// Classify one Intention for Process **business** compliance only.
///
/// Returns a `FILTER_*` reason (`FILTER_ACCEPTED` on success). Does **not**
/// verify CID Type-ID ownership or `process_type_hash` — callers must route
/// first (Sequencer) or use on-chain `ours_prefix` scoping (Verify).
///
/// When `process_lock_code_hash` is `Some`, rejects beneficiaries whose
/// `code_hash` equals that value (parity with on-chain Verify, which skips
/// every output locked by this Process binary). `Process.filter` SSRI has no
/// Process state / `load_script()`, so it passes `None`; Sequencers that know
/// the live Process lock `code_hash` should call this helper (or
/// [`classify_intention_excluding_process_lock`]) before assemble.
pub fn classify_intention(
    lock: &Script,
    capacity: u64,
    type_script: Option<(&[u8; 32], u8)>,
    data: &[u8],
) -> i8 {
    classify_intention_excluding_process_lock(lock, capacity, type_script, data, None)
}

/// Like [`classify_intention`], optionally forbidding beneficiaries that use
/// the Process lock `code_hash` (same exclusion as on-chain `claim_output`).
pub fn classify_intention_excluding_process_lock(
    lock: &Script,
    capacity: u64,
    type_script: Option<(&[u8; 32], u8)>,
    data: &[u8],
    process_lock_code_hash: Option<&[u8; 32]>,
) -> i8 {
    let Ok(cid) = CidArgs::decode(&lock.args) else {
        return FILTER_MALFORMED_DIGEST;
    };
    // Example Process DigestOk requires a beneficiary Script with a CKB-valid
    // hash_type (shared with assemble / on-chain `packed_script`).
    let Ok(beneficiary) = cid.entry.payload.to_script() else {
        return FILTER_MALFORMED_DIGEST;
    };
    if !is_supported_script_hash_type(beneficiary.hash_type) {
        return FILTER_MALFORMED_DIGEST;
    }
    if let Some(forbidden) = process_lock_code_hash {
        if &beneficiary.code_hash == forbidden {
            return FILTER_MALFORMED_DIGEST;
        }
    }

    match type_script {
        None => {
            if !data.is_empty() {
                return FILTER_UNSUPPORTED_ASSET;
            }
            if !ckb_deposit_capacity_ok(capacity, false) {
                return FILTER_DEPOSIT_BELOW_MINIMUM;
            }
            // Capacity field + lock code hash, hash type and args. The CKB
            // Intention has no type or data and must exceed its occupied bytes.
            let occupied = (41 + lock.args.len() as u64) * 100_000_000;
            if capacity <= occupied {
                return FILTER_UNSUPPORTED_ASSET;
            }
        }
        Some((code_hash, hash_type)) => {
            if !is_standard_xudt(code_hash, hash_type) {
                return FILTER_UNSUPPORTED_ASSET;
            }
            let Some(amount) = data.get(..XUDT_AMOUNT_LEN) else {
                return FILTER_UNSUPPORTED_ASSET;
            };
            if amount == [0; XUDT_AMOUNT_LEN] {
                return FILTER_UNSUPPORTED_ASSET;
            }
        }
    }

    FILTER_ACCEPTED
}

/// Host/guest helper when type script is absent or presented as code_hash+hash_type.
pub fn classify_intention_ckb_or_xudt(
    lock: &Script,
    capacity: u64,
    type_code_hash: Option<[u8; 32]>,
    type_hash_type: u8,
    data: &[u8],
) -> i8 {
    let typ = type_code_hash.as_ref().map(|h| (h, type_hash_type));
    classify_intention(lock, capacity, typ, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cid_protocol::{Payload, SCRIPT_HASH_TYPE_TYPE, XUDT_CODE_HASH};

    fn lock_for(pth: [u8; 32]) -> Script {
        lock_for_beneficiary(
            pth,
            Script {
                code_hash: [0xBE; 32],
                hash_type: 1,
                args: alloc::vec![1],
            },
        )
    }

    fn lock_for_beneficiary(pth: [u8; 32], beneficiary: Script) -> Script {
        let args = CidArgs::new(
            pth,
            Script::default(),
            Payload::from_script(&beneficiary).unwrap(),
        )
        .encode()
        .unwrap();
        Script {
            code_hash: [0xC1; 32],
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args,
        }
    }

    #[test]
    fn accepts_ckb_deposit_at_minimum() {
        let lock = lock_for([0xA1; 32]);
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_ACCEPTED
        );
    }

    #[test]
    fn rejects_below_min_deposit() {
        let lock = lock_for([0xA1; 32]);
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY - 1, None, &[]),
            FILTER_DEPOSIT_BELOW_MINIMUM
        );
    }

    #[test]
    fn rejects_malformed_digest() {
        let mut lock = lock_for([0xA1; 32]);
        lock.args = alloc::vec![0u8; 40];
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_MALFORMED_DIGEST
        );
    }

    #[test]
    fn rejects_unsupported_beneficiary_hash_type() {
        // hash_type=3 is not a CKB ScriptHashType; verify's packed_script rejects it.
        let lock = lock_for_beneficiary(
            [0xA1; 32],
            Script {
                code_hash: [0xBE; 32],
                hash_type: 3,
                args: alloc::vec![1],
            },
        );
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_MALFORMED_DIGEST
        );
    }

    #[test]
    fn accepts_data2_beneficiary_hash_type() {
        let lock = lock_for_beneficiary(
            [0xA1; 32],
            Script {
                code_hash: [0xBE; 32],
                hash_type: 4,
                args: alloc::vec![1],
            },
        );
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_ACCEPTED
        );
    }

    #[test]
    fn accepts_xudt_below_ckb_min() {
        let lock = lock_for([0xA1; 32]);
        let data = 1u128.to_le_bytes();
        assert_eq!(
            classify_intention(&lock, 1, Some((&XUDT_CODE_HASH, 2)), &data),
            FILTER_ACCEPTED
        );
    }

    #[test]
    fn rejects_invalid_ckb_data_and_zero_xudt_amount() {
        let lock = lock_for([0xA1; 32]);
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY, None, &[1]),
            FILTER_UNSUPPORTED_ASSET
        );
        let mut data = alloc::vec![0; XUDT_AMOUNT_LEN];
        data.push(1); // An extension byte does not make the amount nonzero.
        assert_eq!(
            classify_intention(&lock, 1, Some((&XUDT_CODE_HASH, 2)), &data),
            FILTER_UNSUPPORTED_ASSET
        );
    }

    #[test]
    fn rejects_ckb_with_no_capacity_above_occupied() {
        let lock = lock_for_beneficiary(
            [0xA1; 32],
            Script {
                code_hash: [0xBE; 32],
                hash_type: 1,
                args: alloc::vec![1; 600],
            },
        );
        let occupied = (41 + lock.args.len() as u64) * 100_000_000;
        assert!(occupied >= MIN_DEPOSIT_CAPACITY);
        assert_eq!(
            classify_intention(&lock, occupied, None, &[]),
            FILTER_UNSUPPORTED_ASSET
        );
        assert_eq!(
            classify_intention(&lock, occupied + 1, None, &[]),
            FILTER_ACCEPTED
        );
    }

    #[test]
    fn rejects_unsupported_asset() {
        let lock = lock_for([0xA1; 32]);
        let fake = [0xEE; 32];
        assert_eq!(
            classify_intention(&lock, 1, Some((&fake, 2)), &[0u8; 16]),
            FILTER_UNSUPPORTED_ASSET
        );
    }

    #[test]
    fn rejects_beneficiary_matching_process_lock_code_hash() {
        let process_code = [0xB1; 32];
        let lock = lock_for_beneficiary(
            [0xA1; 32],
            Script {
                code_hash: process_code,
                hash_type: 1,
                args: alloc::vec![0x01],
            },
        );
        assert_eq!(
            classify_intention(&lock, MIN_DEPOSIT_CAPACITY, None, &[]),
            FILTER_ACCEPTED
        );
        assert_eq!(
            classify_intention_excluding_process_lock(
                &lock,
                MIN_DEPOSIT_CAPACITY,
                None,
                &[],
                Some(&process_code),
            ),
            FILTER_MALFORMED_DIGEST
        );
    }

    #[test]
    fn fee_from_tx_molecule_len_matches_feerate_formula() {
        let len = 1000u64;
        assert_eq!(fee_from_tx_molecule_len(len), (len + 4) * FEE_RATE / 1000);
    }
}
