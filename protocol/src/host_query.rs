//! Process Type ID helpers and pointers to the SSRI host-query surface.
//!
//! Offline `Process.assemble` loads live cells and tip metadata through
//! **ckb-ssri-std** native-injection syscalls (not a custom CID envelope).
//! See `docs/ssri-backend-interface.md` for the syscall numbers, register
//! layout, and request/response encodings the backend must implement.
//!
//! Minimal Process-SDK surface used by `cid_process_example`:
//! - cells by out point (`find_cell_by_out_point` / `find_cell_data_by_out_point`
//!   or `get_live_cell`)
//! - cells by search key (`get_cells`)
//! - tip / header info (`get_header_by_number` / `get_block_hash`)

/// Built-in TYPE_ID code hash (`TYPE_ID` ASCII in the last 7 bytes).
pub const TYPE_ID_CODE_HASH: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x54, 0x59, 0x50, 0x45, 0x5f, 0x49, 0x44,
];

/// `ScriptHashType::Type` packed byte.
pub const SCRIPT_HASH_TYPE_TYPE: u8 = 1;

/// True when `code_hash` + `hash_type` identify the built-in TYPE_ID script.
pub fn is_type_id_script(code_hash: &[u8], hash_type: u8) -> bool {
    hash_type == SCRIPT_HASH_TYPE_TYPE && code_hash == TYPE_ID_CODE_HASH.as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_id_script_predicate() {
        assert!(is_type_id_script(&TYPE_ID_CODE_HASH, SCRIPT_HASH_TYPE_TYPE));
        assert!(!is_type_id_script(&TYPE_ID_CODE_HASH, 2));
        assert!(!is_type_id_script(&[0u8; 32], SCRIPT_HASH_TYPE_TYPE));
    }
}
