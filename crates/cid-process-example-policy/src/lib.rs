//! Example Process deposit policy (Process-owned, not protocol).
//!
//! Single source of truth for the example Process CKB deposit minimum, shared
//! by the RISC-V contract (`cid_process_example`) and the host calculator/CLI.
//! Protocol / CID impose no Intention or deposit capacity floor.

#![no_std]

/// Minimum CKB capacity (shannons) for a deposit Intention owned by this
/// Process: **600 CKB**.
pub const MIN_DEPOSIT_CAPACITY: u64 = 600_0000_0000;

/// Reject CKB-only deposit Intentions below [`MIN_DEPOSIT_CAPACITY`].
#[inline]
pub fn ckb_deposit_capacity_ok(capacity: u64, has_type_script: bool) -> bool {
    has_type_script || capacity >= MIN_DEPOSIT_CAPACITY
}
