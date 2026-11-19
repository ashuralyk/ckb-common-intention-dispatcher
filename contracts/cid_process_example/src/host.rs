//! Process SDK wrappers over ckb-ssri-std host queries (mockable in the offline VM).
//!
//! Backend authors: implement the same syscall numbers / encodings documented in
//! `docs/ssri-backend-interface.md` (ckb-ssri-std catalog).

#![allow(dead_code)] // SDK surface for Process authors / future assemble paths

extern crate alloc;

use crate::ProcessError;
use alloc::vec::Vec;
use ckb_ssri_std::high_level::{
    find_cell_by_out_point, find_cell_data_by_out_point, get_block_hash, get_cells,
    get_header_by_number,
};
use ckb_ssri_std::indexer::{Order, SearchKey};
use ckb_std::ckb_types::packed::{Byte32, CellOutput, Header, OutPoint};

/// Cell loaded from the SSRI host by out point.
#[derive(Clone, Debug)]
pub struct HostCell {
    pub output: CellOutput,
    pub data: Vec<u8>,
}

pub fn get_cell_by_out_point(out_point: &OutPoint) -> Result<HostCell, ProcessError> {
    let output =
        find_cell_by_out_point(out_point.clone()).map_err(|_| ProcessError::HostQueryFailed)?;
    let data = find_cell_data_by_out_point(out_point.clone())
        .map_err(|_| ProcessError::HostQueryFailed)?;
    Ok(HostCell { output, data })
}

pub fn get_cells_by_search_key(
    search_key: &SearchKey,
    limit: u64,
) -> Result<ckb_ssri_std::indexer::Pagination, ProcessError> {
    get_cells(search_key, Order::Asc, limit, &[]).map_err(|_| ProcessError::HostQueryFailed)
}

/// Tip / header helpers. Pass the tip height the backend advertises (or a
/// header-dep height from the draft tx).
pub fn header_by_number(block_number: u64) -> Result<Header, ProcessError> {
    get_header_by_number(block_number).map_err(|_| ProcessError::HostQueryFailed)
}

pub fn block_hash(block_number: u64) -> Result<Byte32, ProcessError> {
    get_block_hash(block_number).map_err(|_| ProcessError::HostQueryFailed)
}
