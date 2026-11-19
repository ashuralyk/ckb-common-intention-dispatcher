//! Mockable SSRI host-query backing store for offline `Process.assemble`.
//!
//! Implements the ckb-ssri-std native-injection syscalls used by the Process
//! SDK (`find_cell_by_out_point`, `find_cell_data_by_out_point`,
//! `get_block_hash`, …). See `docs/ssri-backend-interface.md`.

use ckb_types::packed::{CellOutput, OutPoint};
use ckb_types::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One live cell the guest may resolve by out point.
#[derive(Clone, Debug)]
pub struct HostCell {
    pub output: CellOutput,
    pub data: Vec<u8>,
}

/// In-memory host the offline VM serves to the guest.
#[derive(Clone, Default)]
pub struct MockHost {
    inner: Arc<Mutex<MockHostInner>>,
}

#[derive(Default)]
struct MockHostInner {
    cells: HashMap<Vec<u8>, HostCell>,
    /// block_number → block hash (32 bytes)
    block_hashes: HashMap<u64, [u8; 32]>,
}

impl MockHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_cell(&self, out_point: OutPoint, output: CellOutput, data: Vec<u8>) {
        let key = out_point.as_slice().to_vec();
        self.inner
            .lock()
            .unwrap()
            .cells
            .insert(key, HostCell { output, data });
    }

    /// Insert from molecule byte slices (cross-version bridge).
    pub fn insert_cell_bytes(
        &self,
        out_point: &[u8],
        output: &[u8],
        data: Vec<u8>,
    ) -> Result<(), String> {
        let output = CellOutput::from_slice(output).map_err(|e| format!("CellOutput: {:?}", e))?;
        self.inner
            .lock()
            .unwrap()
            .cells
            .insert(out_point.to_vec(), HostCell { output, data });
        Ok(())
    }

    pub fn insert_block_hash(&self, number: u64, hash: [u8; 32]) {
        self.inner.lock().unwrap().block_hashes.insert(number, hash);
    }

    pub fn get_cell(&self, out_point: &[u8]) -> Option<HostCell> {
        self.inner.lock().unwrap().cells.get(out_point).cloned()
    }

    /// Returns `None` when the height was never inserted (no zero-hash fallback).
    pub fn get_block_hash(&self, number: u64) -> Option<[u8; 32]> {
        self.inner
            .lock()
            .unwrap()
            .block_hashes
            .get(&number)
            .copied()
    }
}
