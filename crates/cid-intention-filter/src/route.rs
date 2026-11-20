//! Intention ownership / Process Type ID extraction (Process-independent).
//!
//! Used by the **Sequencer** to group scanned Intentions by Process Type ID
//! before calling each Process's `Process.filter`. Ownership is **solely** the
//! Sequencer's responsibility — `Process.filter` / assemble do not re-check it.
//! On-chain Process Verify keeps its own `ours_prefix` scoping.
//!
//! Single source of truth for CID Intention lock shape and Process identity
//! extraction. Does not run Process application logic.

use cid_protocol::{CidArgs, CodecError, Script, PROCESS_TYPE_HASH_LEN, SCRIPT_HASH_TYPE_TYPE};

/// Why an Intention cell cannot be treated as owned CID Intention for a caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentionRouteError {
    /// Lock `hash_type` is not `type` (CID Intentions must reference code by Type ID).
    WrongHashType,
    /// Lock `code_hash` is not the configured CID code Type ID script hash.
    WrongCodeHash,
    /// Lock args are too short to hold a Process Type Hash prefix.
    ArgsTooShort,
    /// Lock args are not a valid CID v0.1 Intention encoding.
    MalformedArgs,
    /// CID entry version is not supported.
    UnsupportedVersion,
    /// Valid CID Intention whose `process_type_hash` is not the expected Process.
    ProcessMismatch,
}

impl From<CodecError> for IntentionRouteError {
    fn from(e: CodecError) -> Self {
        match e {
            CodecError::TooShort => IntentionRouteError::ArgsTooShort,
            CodecError::Version => IntentionRouteError::UnsupportedVersion,
            CodecError::Decode | CodecError::Encode => IntentionRouteError::MalformedArgs,
        }
    }
}

/// Validate that `lock` is a CID Type-ID Intention lock and return the target
/// Process Type ID (`process_type_hash`).
///
/// `cid_code_type_id_hash` is the script hash of the live CID lock **code**
/// cell's Type ID type script (`lock.code_hash` for Intentions that use
/// `hash_type = type`). Callers supply the CID deployment they recognize.
///
/// Cell `data` is unused for ownership (Process identity lives in lock args).
pub fn intention_process_type_id(
    lock: &Script,
    cid_code_type_id_hash: &[u8; 32],
) -> Result<[u8; 32], IntentionRouteError> {
    intention_process_type_id_parts(
        &lock.code_hash,
        lock.hash_type,
        &lock.args,
        cid_code_type_id_hash,
    )
}

/// Same as [`intention_process_type_id`] with raw lock fields.
pub fn intention_process_type_id_parts(
    lock_code_hash: &[u8; 32],
    lock_hash_type: u8,
    lock_args: &[u8],
    cid_code_type_id_hash: &[u8; 32],
) -> Result<[u8; 32], IntentionRouteError> {
    if lock_hash_type != SCRIPT_HASH_TYPE_TYPE {
        return Err(IntentionRouteError::WrongHashType);
    }
    if lock_code_hash != cid_code_type_id_hash {
        return Err(IntentionRouteError::WrongCodeHash);
    }
    if lock_args.len() < PROCESS_TYPE_HASH_LEN {
        return Err(IntentionRouteError::ArgsTooShort);
    }
    let cid = CidArgs::decode(lock_args)?;
    Ok(cid.process_type_hash)
}

/// Require that `lock` is a CID Type-ID Intention targeting
/// `expected_process_type_id` (Process filter / `ours_prefix` ownership check).
pub fn intention_belongs_to_process(
    lock: &Script,
    cid_code_type_id_hash: &[u8; 32],
    expected_process_type_id: &[u8; 32],
) -> Result<(), IntentionRouteError> {
    let pth = intention_process_type_id(lock, cid_code_type_id_hash)?;
    if &pth != expected_process_type_id {
        return Err(IntentionRouteError::ProcessMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use cid_protocol::{Payload, CID_VERSION};

    fn cid_args(pth: [u8; 32]) -> Vec<u8> {
        CidArgs::new(
            pth,
            Script {
                code_hash: [0x11; 32],
                hash_type: 1,
                args: vec![1, 2, 3],
            },
            Payload::default(),
        )
        .encode()
        .unwrap()
    }

    #[test]
    fn routes_valid_cid_type_id_intention() {
        let cid_tid = [0xC1; 32];
        let pth = [0xA1; 32];
        let lock = Script {
            code_hash: cid_tid,
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args: cid_args(pth),
        };
        assert_eq!(intention_process_type_id(&lock, &cid_tid).unwrap(), pth);
        assert!(intention_belongs_to_process(&lock, &cid_tid, &pth).is_ok());
    }

    #[test]
    fn rejects_wrong_hash_type() {
        let cid_tid = [0xC1; 32];
        let lock = Script {
            code_hash: cid_tid,
            hash_type: 2, // data1
            args: cid_args([0xAB; 32]),
        };
        assert_eq!(
            intention_process_type_id(&lock, &cid_tid),
            Err(IntentionRouteError::WrongHashType)
        );
    }

    #[test]
    fn rejects_foreign_lock_code_hash() {
        let cid_tid = [0xC1; 32];
        let lock = Script {
            code_hash: [0xFF; 32],
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args: cid_args([0xAB; 32]),
        };
        assert_eq!(
            intention_process_type_id(&lock, &cid_tid),
            Err(IntentionRouteError::WrongCodeHash)
        );
    }

    #[test]
    fn rejects_process_mismatch_on_belongs_check() {
        let cid_tid = [0xC1; 32];
        let lock = Script {
            code_hash: cid_tid,
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args: cid_args([0xA1; 32]),
        };
        assert_eq!(
            intention_belongs_to_process(&lock, &cid_tid, &[0xB2; 32]),
            Err(IntentionRouteError::ProcessMismatch)
        );
    }

    #[test]
    fn rejects_malformed_args() {
        let cid_tid = [0xC1; 32];
        let lock = Script {
            code_hash: cid_tid,
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args: vec![0u8; 40],
        };
        assert_eq!(
            intention_process_type_id(&lock, &cid_tid),
            Err(IntentionRouteError::MalformedArgs)
        );
    }

    #[test]
    fn rejects_short_args() {
        let cid_tid = [0xC1; 32];
        let lock = Script {
            code_hash: cid_tid,
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args: vec![0u8; 16],
        };
        assert_eq!(
            intention_process_type_id(&lock, &cid_tid),
            Err(IntentionRouteError::ArgsTooShort)
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        let cid_tid = [0xC1; 32];
        let mut args = CidArgs::new([0xAB; 32], Script::default(), Payload::default());
        args.entry.version = CID_VERSION + 1;
        let bytes = cid_protocol::encode_args(&args.process_type_hash, &args.entry).unwrap();
        let lock = Script {
            code_hash: cid_tid,
            hash_type: SCRIPT_HASH_TYPE_TYPE,
            args: bytes,
        };
        assert_eq!(
            intention_process_type_id(&lock, &cid_tid),
            Err(IntentionRouteError::UnsupportedVersion)
        );
    }

    #[test]
    fn groups_mixed_processes_by_extracted_type_id() {
        let cid_tid = [0xC1; 32];
        let p_a = [0xA1; 32];
        let p_b = [0xB2; 32];
        let cells = [
            Script {
                code_hash: cid_tid,
                hash_type: SCRIPT_HASH_TYPE_TYPE,
                args: cid_args(p_a),
            },
            Script {
                code_hash: cid_tid,
                hash_type: SCRIPT_HASH_TYPE_TYPE,
                args: cid_args(p_b),
            },
            Script {
                code_hash: cid_tid,
                hash_type: SCRIPT_HASH_TYPE_TYPE,
                args: cid_args(p_a),
            },
        ];
        let mut group_a = Vec::new();
        let mut group_b = Vec::new();
        for (i, lock) in cells.iter().enumerate() {
            let pth = intention_process_type_id(lock, &cid_tid).unwrap();
            if pth == p_a {
                group_a.push(i);
            } else if pth == p_b {
                group_b.push(i);
            }
        }
        assert_eq!(group_a, vec![0, 2]);
        assert_eq!(group_b, vec![1]);
    }
}
