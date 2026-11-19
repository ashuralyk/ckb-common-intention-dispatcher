//! CKB deposit policy for this Process example.
//!
//! DigestOk CKB Intentions are deposits fulfilled by consuming the Intention
//! cell's capacity into the beneficiary output. The minimum must sit strictly
//! above [`cid_protocol::DEFAULT_MIN_INTENTION_CAPACITY`] so a minimum deposit
//! Intention is always createable (covers occupied capacity) and can fund the
//! deposit alone.

use cid_protocol::{
    deposit_min_satisfies_intention_floor, CkbDepositPolicy, DEFAULT_MIN_INTENTION_CAPACITY,
};

/// Minimum CKB capacity (shannons) for a deposit Intention: **400 CKB**.
///
/// Strictly greater than [`DEFAULT_MIN_INTENTION_CAPACITY`] (300 CKB).
pub const MIN_DEPOSIT_CAPACITY: u64 = 400_0000_0000;

/// Example Process deposit policy (CKB Intentions only; xUDT amounts are separate).
pub struct ExampleDepositPolicy;

impl CkbDepositPolicy for ExampleDepositPolicy {
    const MIN_DEPOSIT_CAPACITY: u64 = MIN_DEPOSIT_CAPACITY;
}

const _: () = assert!(deposit_min_satisfies_intention_floor(
    ExampleDepositPolicy::MIN_DEPOSIT_CAPACITY
));
const _: () = assert!(ExampleDepositPolicy::MIN_DEPOSIT_CAPACITY > DEFAULT_MIN_INTENTION_CAPACITY);

/// Reject CKB-only deposit Intentions below [`MIN_DEPOSIT_CAPACITY`].
#[inline]
pub fn ckb_deposit_capacity_ok(capacity: u64, has_type_script: bool) -> bool {
    has_type_script || capacity >= MIN_DEPOSIT_CAPACITY
}
