//! CKB deposit policy for this Process example.
//!
//! DigestOk CKB Intentions owned by this Process are deposits fulfilled by
//! consuming the Intention cell's capacity into the beneficiary output. The
//! minimum is Process-local — protocol / CID impose no Intention or deposit
//! floor.

/// Minimum CKB capacity (shannons) for a deposit Intention owned by this
/// Process: **600 CKB**.
///
/// Dual-written with `calculator/process` (`MIN_DEPOSIT_CAPACITY`). Keep both
/// literals identical when changing the example Process deposit policy.
pub const MIN_DEPOSIT_CAPACITY: u64 = 600_0000_0000;

/// Reject CKB-only deposit Intentions below [`MIN_DEPOSIT_CAPACITY`].
#[inline]
pub fn ckb_deposit_capacity_ok(capacity: u64, has_type_script: bool) -> bool {
    has_type_script || capacity >= MIN_DEPOSIT_CAPACITY
}
