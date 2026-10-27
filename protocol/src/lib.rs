//! Shared CID v0.1 byte layout (Calculate ↔ Verify).
//!
//! Lock args: `process_type_hash[32] || molecule_table(CidEntry)`.
//! `CidEntry` is versioned and embeds the user lock [`Script`] plus an opaque
//! [`Payload`]. Constants that appear on-chain and off-chain are dual-written
//! here on purpose.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// CID args version encoded in [`CidEntry::version`].
pub const CID_VERSION: u8 = 0;

/// Process Type Hash prefix length (blake2b script hash).
pub const PROCESS_TYPE_HASH_LEN: usize = 32;

/// Timeout window in blocks (~36h at ~12s/block). Dual-written: on-chain Verify
/// and off-chain Calculate/tests **must** stay in sync.
pub const TIMEOUT_BLOCKS: u64 = 10800;

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
    let entry: CidEntry =
        serde_molecule::from_slice(&args[PROCESS_TYPE_HASH_LEN..], false).map_err(|_| CodecError::Decode)?;
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
        assert_eq!(bytes.len() > PROCESS_TYPE_HASH_LEN, true);
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
}
