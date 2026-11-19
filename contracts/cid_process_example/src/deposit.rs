//! CKB deposit policy for this Process example.
//!
//! Re-exports the Process-owned policy crate so Verify / assemble keep a stable
//! `crate::deposit` path. Protocol / CID impose no Intention or deposit floor.
//!
//! Deposit / ownership checks go through `classify_intention` (filter, assemble,
//! on-chain verify); this module only re-exports the Process minimum constant.

pub use cid_process_example_policy::MIN_DEPOSIT_CAPACITY;
