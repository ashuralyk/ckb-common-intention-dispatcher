//! Shared CID v0.1 byte layout (Calculate ↔ Verify).
//!
//! Lock args: `process_type_hash[32] || molecule_table(CidEntry)`.
//! `CidEntry` is versioned and embeds the user lock [`Script`] plus an opaque
//! [`Payload`]. Constants that appear on-chain and off-chain are dual-written
//! here on purpose.

#![no_std]

extern crate alloc;

mod host_query;

pub use host_query::{is_type_id_script, SCRIPT_HASH_TYPE_TYPE, TYPE_ID_CODE_HASH};

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// CID args version encoded in [`CidEntry::version`].
pub const CID_VERSION: u8 = 0;

/// Process Type Hash prefix length (blake2b script hash).
pub const PROCESS_TYPE_HASH_LEN: usize = 32;

/// Timeout window in blocks (~36h at ~12s/block). Dual-written: on-chain Verify
/// and off-chain Calculate/tests **must** stay in sync.
pub const TIMEOUT_BLOCKS: u64 = 10800;

/// Ceiling on how much CKB capacity (shannons) one DigestIncompatible refund
/// may contribute toward the transaction miner fee. Dual-written: on-chain
/// Verify and off-chain assemblers **must** stay in sync.
///
/// `0.01 CKB` (= `1_000_000` shannons). At testnet `min_fee_rate = 1000`
/// shannons/KW this covers roughly a megabyte of tx weight; even at 100× that
/// rate it still covers a ~10 KB refund tx.
///
/// This is **only a ceiling**, never a license to skim. DigestIncompatible also
/// requires `tx_fee >= sum(shortfalls)` over every DigestIncompatible Intention
/// in the transaction (`fee_covers_shortfalls`), so a shortfall cannot fund
/// other outputs and cannot be double-counted across lock groups. TimeoutBurn
/// keeps exact capacity. No Intention-args field: a fixed protocol constant
/// avoids a v0.1 encoding/version break.
pub const MAX_REFUND_FEE: u64 = 1_000_000;

/// Default minimum Intention cell capacity (shannons): **500 CKB**.
///
/// Policy floor for Intention creates and for Process CKB-deposit minima
/// (`min_deposit > DEFAULT_MIN_INTENTION_CAPACITY`). Off-chain create paths
/// size each Intention to `max(requested, this floor, occupied)` so the cell
/// is always createable even when lock args / xUDT push occupied above the
/// floor (e.g. multisig×multisig ≈ 338 CKB, blake160+xUDT ≈ 329 CKB,
/// blake160 with 256-byte beneficiary args ≈ 484 CKB). Shapes whose occupied
/// capacity exceeds this floor still work via the occupied bump; the floor
/// remains the value deposit policies compare against.
///
/// Dual-written: CLI / calculator create sizing, and Process deposit-policy
/// validation. Intention Create does **not** run on-chain CID Verify.
pub const DEFAULT_MIN_INTENTION_CAPACITY: u64 = 500_0000_0000;

/// Example Process (`cid_process_example`) CKB deposit minimum: **600 CKB**.
///
/// Single source of truth for on-chain Verify/assemble and the host calculator.
/// Strictly greater than [`DEFAULT_MIN_INTENTION_CAPACITY`]. Creation may raise
/// the effective capacity further to cover the cell's occupied capacity.
pub const EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY: u64 = 600_0000_0000;

const _: () = assert!(EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY > DEFAULT_MIN_INTENTION_CAPACITY);

/// True when a Process CKB-deposit minimum is **strictly greater** than
/// [`DEFAULT_MIN_INTENTION_CAPACITY`].
///
/// Process authors: for every CKB deposit action fulfilled by consuming an
/// Intention's capacity, declare a `min_deposit` above the create floor.
/// The concrete cell's occupied capacity can still require a larger deposit.
pub const fn deposit_min_satisfies_intention_floor(min_deposit: u64) -> bool {
    min_deposit > DEFAULT_MIN_INTENTION_CAPACITY
}

/// Process-author contract for CKB deposits funded by consuming an Intention.
///
/// Set [`CkbDepositPolicy::MIN_DEPOSIT_CAPACITY`] above the Intention floor.
/// Reference [`CkbDepositPolicy::ASSERT_ABOVE_INTENTION_FLOOR`] (or call
/// [`assert_ckb_deposit_policy`]) so the check is evaluated at compile time.
pub trait CkbDepositPolicy {
    /// Minimum CKB capacity (shannons) accepted for a deposit Intention.
    const MIN_DEPOSIT_CAPACITY: u64;

    /// Compile-time guard: `MIN_DEPOSIT_CAPACITY > DEFAULT_MIN_INTENTION_CAPACITY`.
    /// Force evaluation with `const _: () = Self::ASSERT_ABOVE_INTENTION_FLOOR;`.
    const ASSERT_ABOVE_INTENTION_FLOOR: () = assert!(deposit_min_satisfies_intention_floor(
        Self::MIN_DEPOSIT_CAPACITY
    ));
}

/// Evaluate [`CkbDepositPolicy::ASSERT_ABOVE_INTENTION_FLOOR`] for `P`.
pub const fn assert_ckb_deposit_policy<P: CkbDepositPolicy>() {
    let () = P::ASSERT_ABOVE_INTENTION_FLOOR;
}

/// True when a DigestIncompatible refund output's CKB capacity is within the
/// per-Intention fee ceiling (token amount/type checked separately):
/// - `got <= want` (no capacity inflation on the refund cell)
/// - `got >= want.saturating_sub(MAX_REFUND_FEE)` (ceiling)
/// - `got >= occupied` (refund cell must remain liveable)
///
/// Callers must still enforce [`fee_covers_shortfalls`] over the whole tx.
pub fn refund_capacity_allowed(got: u64, want: u64, occupied: u64) -> bool {
    if got > want || got < occupied {
        return false;
    }
    got >= want.saturating_sub(MAX_REFUND_FEE)
}

/// True when miner fee covers the aggregate DigestIncompatible shortfall.
///
/// Let `S` be Σ(intention_cap − refund_cap) over DigestIncompatible inputs and
/// `F = Σinputs − Σoutputs`. Then `F >= S` iff non-refund outputs are fully
/// funded by inputs **not** counted in `S` (non-DigestIncompatible inputs) — so
/// DigestIncompatible shortfalls cannot be redirected into third-party outputs.
/// Each verifying lock group must use the **same** tx-wide `S` (not its own
/// shortfall alone) so fee cannot be reused across Intention groups.
pub fn fee_covers_shortfalls(fee: u64, total_shortfall: u64) -> bool {
    fee >= total_shortfall
}

/// Standard xUDT amount width (little-endian `u128`).
pub const XUDT_AMOUNT_LEN: usize = 16;

/// Data1 code hash of the official xUDT script (mainnet = testnet).
/// Dual-written with `ckb_cinnabar_calculator::operation::udt::XUDT_CODE_HASH`.
pub const XUDT_CODE_HASH: [u8; 32] = [
    0x50, 0xbd, 0x8d, 0x66, 0x80, 0xb8, 0xb9, 0xcf, 0x98, 0xb7, 0x3f, 0x3c, 0x08, 0xfa, 0xf8, 0xb2,
    0xa2, 0x19, 0x14, 0x31, 0x19, 0x54, 0x11, 0x8a, 0xd6, 0x60, 0x9b, 0xe6, 0xe7, 0x8a, 0x1b, 0x95,
];

/// `ScriptHashType::Data1` as packed byte.
pub const SCRIPT_HASH_TYPE_DATA1: u8 = 2;

/// `ScriptHashType::Data2` as packed byte.
pub const SCRIPT_HASH_TYPE_DATA2: u8 = 4;

/// Custom script exit codes. Must match `define_errors!` in the lock crate.
pub const ERROR_ARGS_INVALID: i8 = 20;
pub const ERROR_UNKNOWN_BRANCH: i8 = 21;
pub const ERROR_PROCESS_MISMATCH: i8 = 22;
pub const ERROR_PARTIAL_DIGEST: i8 = 23;
pub const ERROR_REFUND_MISSING: i8 = 24;
pub const ERROR_REFUND_ASSET_MISMATCH: i8 = 25;
pub const ERROR_TIMEOUT_NOT_REACHED: i8 = 26;
pub const ERROR_HEADER_MISSING: i8 = 27;
pub const ERROR_ZERO_ASSET: i8 = 28;
pub const ERROR_UNSUPPORTED_TYPE: i8 = 29;
/// DigestIncompatible tx contains another CID-shaped lock with a different
/// `code_hash` / `hash_type` (other deployment, hash_type alias, or foreign spoof).
pub const ERROR_MIXED_CID_LOCK: i8 = 30;

/// Domain hop names registered in `cinnabar_main!`.
pub const HOP_DIGEST_OK: &str = "digest_ok";
pub const HOP_DIGEST_INCOMPATIBLE: &str = "digest_incompatible";
pub const HOP_TIMEOUT_BURN: &str = "timeout_burn";

/// CKB lock/type script (molecule table via serde_molecule).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Script {
    pub code_hash: [u8; 32],
    pub hash_type: u8,
    pub args: Vec<u8>,
}

/// Opaque intention payload. CID v0.1 does not interpret these bytes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Payload {
    pub data: Vec<u8>,
}

/// Versioned args body after the Process Type Hash prefix.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct CidEntry {
    pub version: u8,
    pub user_lock: Script,
    pub payload: Payload,
}

/// Codec failure while reading or writing lock args.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    TooShort,
    Decode,
    Encode,
    Version,
}

/// Parsed lock args.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CidArgs {
    pub process_type_hash: [u8; 32],
    pub entry: CidEntry,
}

impl CidArgs {
    pub fn new(process_type_hash: [u8; 32], user_lock: Script, payload: Payload) -> Self {
        Self {
            process_type_hash,
            entry: CidEntry {
                version: CID_VERSION,
                user_lock,
                payload,
            },
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        encode_args(&self.process_type_hash, &self.entry)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        let (process_type_hash, entry) = decode_args(bytes)?;
        Ok(Self {
            process_type_hash,
            entry,
        })
    }
}

impl Payload {
    /// Encode a beneficiary [`Script`] into opaque payload bytes.
    ///
    /// Process example: `CidEntry.payload` is this molecule table, not a
    /// free-form string. CID itself still treats `data` as opaque.
    pub fn from_script(script: &Script) -> Result<Self, CodecError> {
        Ok(Self {
            data: serde_molecule::to_vec(script, false).map_err(|_| CodecError::Encode)?,
        })
    }

    /// Decode a beneficiary [`Script`] from payload bytes.
    pub fn to_script(&self) -> Result<Script, CodecError> {
        serde_molecule::from_slice(&self.data, false).map_err(|_| CodecError::Decode)
    }
}

/// `process_type_hash[32] || molecule_table(entry)`.
pub fn encode_args(process_type_hash: &[u8; 32], entry: &CidEntry) -> Result<Vec<u8>, CodecError> {
    let body = serde_molecule::to_vec(entry, false).map_err(|_| CodecError::Encode)?;
    let mut out = Vec::with_capacity(PROCESS_TYPE_HASH_LEN + body.len());
    out.extend_from_slice(process_type_hash);
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn decode_args(args: &[u8]) -> Result<([u8; 32], CidEntry), CodecError> {
    if args.len() < PROCESS_TYPE_HASH_LEN {
        return Err(CodecError::TooShort);
    }
    let mut process_type_hash = [0u8; 32];
    process_type_hash.copy_from_slice(&args[..PROCESS_TYPE_HASH_LEN]);
    let entry: CidEntry = serde_molecule::from_slice(&args[PROCESS_TYPE_HASH_LEN..], false)
        .map_err(|_| CodecError::Decode)?;
    if entry.version != CID_VERSION {
        return Err(CodecError::Version);
    }
    Ok((process_type_hash, entry))
}

/// True when `code_hash` + `hash_type` identify official xUDT (Data1).
pub fn is_standard_xudt(code_hash: &[u8], hash_type: u8) -> bool {
    hash_type == SCRIPT_HASH_TYPE_DATA1 && code_hash == XUDT_CODE_HASH.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_roundtrip() {
        let args = CidArgs::new(
            [0x11; 32],
            Script {
                code_hash: [0x22; 32],
                hash_type: SCRIPT_HASH_TYPE_DATA1,
                args: alloc::vec![1, 2, 3],
            },
            Payload {
                data: alloc::vec![9, 8],
            },
        );
        let bytes = args.encode().unwrap();
        assert!(bytes.len() > PROCESS_TYPE_HASH_LEN);
        let decoded = CidArgs::decode(&bytes).unwrap();
        assert_eq!(args, decoded);
    }

    #[test]
    fn rejects_short_and_bad_version() {
        assert_eq!(CidArgs::decode(&[0u8; 16]), Err(CodecError::TooShort));
        let mut args = CidArgs::new([0u8; 32], Script::default(), Payload::default());
        args.entry.version = 7;
        let bytes = encode_args(&args.process_type_hash, &args.entry).unwrap();
        assert_eq!(CidArgs::decode(&bytes), Err(CodecError::Version));
    }

    #[test]
    fn script_payload_roundtrip() {
        let script = Script {
            code_hash: [0xAB; 32],
            hash_type: SCRIPT_HASH_TYPE_DATA1,
            args: alloc::vec![1, 2, 3, 4],
        };
        let payload = Payload::from_script(&script).unwrap();
        assert_eq!(payload.to_script().unwrap(), script);
    }

    #[test]
    fn deposit_min_must_exceed_intention_floor() {
        assert!(!deposit_min_satisfies_intention_floor(
            DEFAULT_MIN_INTENTION_CAPACITY
        ));
        assert!(!deposit_min_satisfies_intention_floor(
            DEFAULT_MIN_INTENTION_CAPACITY.saturating_sub(1)
        ));
        assert!(deposit_min_satisfies_intention_floor(
            DEFAULT_MIN_INTENTION_CAPACITY + 1
        ));
        assert!(deposit_min_satisfies_intention_floor(
            EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY
        ));
        const _: () = assert!(DEFAULT_MIN_INTENTION_CAPACITY == 500_0000_0000);
        const _: () = assert!(EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY == 600_0000_0000);
        const _: () = assert_ckb_deposit_policy::<ExamplePolicyProbe>();
    }

    struct ExamplePolicyProbe;
    impl CkbDepositPolicy for ExamplePolicyProbe {
        const MIN_DEPOSIT_CAPACITY: u64 = EXAMPLE_PROCESS_MIN_DEPOSIT_CAPACITY;
    }

    #[test]
    fn refund_capacity_window() {
        let want = 200_000_000_000u64;
        let occupied = 6_100_000_000u64; // 61 CKB typical secp cell floor
        assert!(refund_capacity_allowed(want, want, occupied));
        assert!(refund_capacity_allowed(
            want - MAX_REFUND_FEE,
            want,
            occupied
        ));
        assert!(refund_capacity_allowed(
            want - MAX_REFUND_FEE / 2,
            want,
            occupied
        ));
        assert!(!refund_capacity_allowed(
            want - MAX_REFUND_FEE - 1,
            want,
            occupied
        ));
        assert!(!refund_capacity_allowed(want + 1, want, occupied));
        assert!(!refund_capacity_allowed(occupied - 1, want, occupied));
        // Fee room collapses when want is only slightly above occupied.
        let tight = occupied + MAX_REFUND_FEE / 2;
        assert!(refund_capacity_allowed(occupied, tight, occupied));
        assert!(!refund_capacity_allowed(occupied - 1, tight, occupied));
    }

    #[test]
    fn fee_must_cover_aggregate_shortfall() {
        assert!(fee_covers_shortfalls(MAX_REFUND_FEE, MAX_REFUND_FEE));
        assert!(fee_covers_shortfalls(MAX_REFUND_FEE + 1, MAX_REFUND_FEE));
        assert!(!fee_covers_shortfalls(MAX_REFUND_FEE - 1, MAX_REFUND_FEE));
        // Two intentions each shaving MAX require 2× fee — no reuse of one fee.
        assert!(!fee_covers_shortfalls(MAX_REFUND_FEE, MAX_REFUND_FEE * 2));
        assert!(fee_covers_shortfalls(
            MAX_REFUND_FEE * 2,
            MAX_REFUND_FEE * 2
        ));
    }
}
