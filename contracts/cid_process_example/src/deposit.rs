//! CKB deposit policy for this Process example.
//!
//! DigestOk CKB Intentions are deposits fulfilled by consuming the Intention
//! cell's capacity into the beneficiary output. The minimum must sit strictly
//! above [`cid_protocol::DEFAULT_MIN_INTENTION_CAPACITY`]. Creation may further
//! increase the deposit to cover the Intention cell's occupied capacity.

use cid_protocol::{
    assert_ckb_deposit_policy, CkbDepositPolicy, DEFAULT_MIN_INTENTION_CAPACITY,
    EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY,
};

/// Minimum CKB capacity (shannons) for a deposit Intention.
///
/// Single source: [`EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY`] in `cid-protocol`.
pub const MIN_DEPOSIT_CAPACITY: u64 = EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY;

/// Example Process deposit policy (CKB Intentions only; xUDT amounts are separate).
pub struct ExampleDepositPolicy;

impl CkbDepositPolicy for ExampleDepositPolicy {
    const MIN_DEPOSIT_CAPACITY: u64 = MIN_DEPOSIT_CAPACITY;
}

const _: () = assert_ckb_deposit_policy::<ExampleDepositPolicy>();
const _: () = assert!(ExampleDepositPolicy::MIN_DEPOSIT_CAPACITY > DEFAULT_MIN_INTENTION_CAPACITY);

/// Reject CKB-only deposit Intentions below [`MIN_DEPOSIT_CAPACITY`].
#[inline]
pub fn ckb_deposit_capacity_ok(capacity: u64, has_type_script: bool) -> bool {
    has_type_script || capacity >= MIN_DEPOSIT_CAPACITY
}
