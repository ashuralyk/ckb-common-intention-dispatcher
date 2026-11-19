#![no_main]
#![no_std]

//! CID v0.1 lock: destroy-only digestion of a single Intention cell.
//!
//! Lock Create does not run this script. Root classifies the three Burn hops
//! from transaction structure (not a witness enum):
//! - `digest_ok` — matching Process type hash in inputs AND outputs; no user input required
//! - `digest_incompatible` — no matching Process pair; fee-bounded CKB refund (≤ MAX_REFUND_FEE
//!   and `tx_fee >= Σ shortfalls` for this program identity), exact tokens; rejects other
//!   CID lock identities in the same tx (`MixedCidLock`); user not in inputs
//! - `timeout_burn` — no matching Process pair; user input + timeout headers + **exact** refund

use cid_protocol::{HOP_DIGEST_INCOMPATIBLE, HOP_DIGEST_OK, HOP_TIMEOUT_BURN};
use ckb_cinnabar_verifier::{cinnabar_main, TREE_ROOT};

mod context;
mod error;
mod hops;
mod root;
mod utils;

use context::Context;
use hops::{DigestIncompatible, DigestOk, TimeoutBurn};
use root::Root;

cinnabar_main!(
    Context,
    (TREE_ROOT, Root),
    (HOP_DIGEST_OK, DigestOk),
    (HOP_DIGEST_INCOMPATIBLE, DigestIncompatible),
    (HOP_TIMEOUT_BURN, TimeoutBurn),
);
