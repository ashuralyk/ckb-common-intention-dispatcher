//! CKB deposit policy for this Process example.
//!
//! Re-exports the Process-owned policy crate so Verify / assemble keep a stable
//! `crate::deposit` path. Protocol / CID impose no Intention or deposit floor.

pub use cid_process_example_policy::{ckb_deposit_capacity_ok, MIN_DEPOSIT_CAPACITY};
