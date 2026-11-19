//! Mockable SSRI host-query surface for offline `Process.assemble`.
//!
//! Serves the full ckb-ssri-std native-injection catalog used by the Process
//! SDK (`docs/ssri-backend-interface.md`):
//!
//! | Syscall | Number |
//! |---------|--------|
//! | `find_cell_by_out_point` | 2287 |
//! | `find_cell_data_by_out_point` | 2297 |
//! | `get_live_cell` | 2317 |
//! | `get_header` | 2327 |
//! | `get_header_by_number` | 2337 |
//! | `get_block_hash` | 2347 |
//! | `get_cells` | 2367 |
//!
//! `SearchKey` / `Pagination` / `LiveCell` encodings match ckb-ssri-std
//! `indexer.rs` (`serde_molecule` tables).

use ckb_types::packed::{CellOutput, Header, OutPoint};
use ckb_types::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One live cell the guest may resolve by out point.
#[derive(Clone, Debug)]
pub struct HostCell {
    pub output: CellOutput,
    pub data: Vec<u8>,
    /// Optional block hash that committed this cell (`get_live_cell`).
    pub block_hash: Option<[u8; 32]>,
    /// Optional block number (indexer `get_cells`). Missing → `0`.
    pub block_number: Option<u64>,
    /// Optional tx index within the block (indexer `get_cells`). Missing → `0`.
    pub tx_index: Option<u32>,
}

impl HostCell {
    pub fn new(output: CellOutput, data: Vec<u8>) -> Self {
        Self {
            output,
            data,
            block_hash: None,
            block_number: None,
            tx_index: None,
        }
    }
}

/// Errors from host-query argument decoding / encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostQueryError {
    /// Register / molecule args invalid → syscall errno 1 (`IndexOutOfBound`).
    InvalidArgs,
    /// Encoding failure → treated as invalid args for the offline VM.
    Encoding,
}

/// Full host-query catalog (ckb-ssri-std native injection).
///
/// Backends implement this for live RPC/indexer resolution; [`MockHost`] is the
/// offline test impl. `ProcessExecutor::assemble` is generic over this trait so
/// syscalls invoke the host interactively during VM execution.
pub trait HostQuery {
    /// 2287 — packed `CellOutput` molecule bytes, or `None` if missing.
    fn find_cell_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>>;

    /// 2297 — raw cell data, or `None` if missing.
    fn find_cell_data_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>>;

    /// 2317 — `serde_molecule` [`LiveCell`]. Unknown cells encode `output: None`
    /// (success, not item-missing), matching ckb-ssri-std on-chain behavior.
    fn get_live_cell(&self, out_point: &[u8], with_data: bool) -> Result<Vec<u8>, HostQueryError>;

    /// 2327 — packed `Header` molecule bytes for `block_hash` (32 bytes).
    fn get_header(&self, block_hash: &[u8]) -> Option<Vec<u8>>;

    /// 2337 — packed `Header` for block number.
    fn get_header_by_number(&self, number: u64) -> Option<Vec<u8>>;

    /// 2347 — 32-byte block hash for number.
    fn get_block_hash(&self, number: u64) -> Option<[u8; 32]>;

    /// 2367 — `serde_molecule` [`Pagination`] for indexer search.
    ///
    /// - `search_key`: molecule `SearchKey` bytes  
    /// - `order`: `0` = Asc, `1` = Desc (ckb-ssri-std [`Order`])  
    /// - `last_cursor`: **opaque** indexer cursor of **arbitrary length**
    ///   (empty = start). Do not assume a fixed 0/8-byte layout — live
    ///   ckb-indexer cursors are host-defined opaque bytes.  
    /// - `limit`: page size (`page_window`)
    ///
    /// Result cell order matches ckb-indexer:
    /// `(block_number, tx_index, output_index)`.
    fn get_cells(
        &self,
        search_key: &[u8],
        order: u64,
        limit: u64,
        last_cursor: &[u8],
    ) -> Result<Vec<u8>, HostQueryError>;
}

impl<T: HostQuery + ?Sized> HostQuery for &T {
    fn find_cell_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        (**self).find_cell_by_out_point(out_point)
    }
    fn find_cell_data_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        (**self).find_cell_data_by_out_point(out_point)
    }
    fn get_live_cell(&self, out_point: &[u8], with_data: bool) -> Result<Vec<u8>, HostQueryError> {
        (**self).get_live_cell(out_point, with_data)
    }
    fn get_header(&self, block_hash: &[u8]) -> Option<Vec<u8>> {
        (**self).get_header(block_hash)
    }
    fn get_header_by_number(&self, number: u64) -> Option<Vec<u8>> {
        (**self).get_header_by_number(number)
    }
    fn get_block_hash(&self, number: u64) -> Option<[u8; 32]> {
        (**self).get_block_hash(number)
    }
    fn get_cells(
        &self,
        search_key: &[u8],
        order: u64,
        limit: u64,
        last_cursor: &[u8],
    ) -> Result<Vec<u8>, HostQueryError> {
        (**self).get_cells(search_key, order, limit, last_cursor)
    }
}

impl<T: HostQuery + ?Sized> HostQuery for Box<T> {
    fn find_cell_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        (**self).find_cell_by_out_point(out_point)
    }
    fn find_cell_data_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        (**self).find_cell_data_by_out_point(out_point)
    }
    fn get_live_cell(&self, out_point: &[u8], with_data: bool) -> Result<Vec<u8>, HostQueryError> {
        (**self).get_live_cell(out_point, with_data)
    }
    fn get_header(&self, block_hash: &[u8]) -> Option<Vec<u8>> {
        (**self).get_header(block_hash)
    }
    fn get_header_by_number(&self, number: u64) -> Option<Vec<u8>> {
        (**self).get_header_by_number(number)
    }
    fn get_block_hash(&self, number: u64) -> Option<[u8; 32]> {
        (**self).get_block_hash(number)
    }
    fn get_cells(
        &self,
        search_key: &[u8],
        order: u64,
        limit: u64,
        last_cursor: &[u8],
    ) -> Result<Vec<u8>, HostQueryError> {
        (**self).get_cells(search_key, order, limit, last_cursor)
    }
}

impl<T: HostQuery + ?Sized> HostQuery for std::sync::Arc<T> {
    fn find_cell_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        (**self).find_cell_by_out_point(out_point)
    }
    fn find_cell_data_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        (**self).find_cell_data_by_out_point(out_point)
    }
    fn get_live_cell(&self, out_point: &[u8], with_data: bool) -> Result<Vec<u8>, HostQueryError> {
        (**self).get_live_cell(out_point, with_data)
    }
    fn get_header(&self, block_hash: &[u8]) -> Option<Vec<u8>> {
        (**self).get_header(block_hash)
    }
    fn get_header_by_number(&self, number: u64) -> Option<Vec<u8>> {
        (**self).get_header_by_number(number)
    }
    fn get_block_hash(&self, number: u64) -> Option<[u8; 32]> {
        (**self).get_block_hash(number)
    }
    fn get_cells(
        &self,
        search_key: &[u8],
        order: u64,
        limit: u64,
        last_cursor: &[u8],
    ) -> Result<Vec<u8>, HostQueryError> {
        (**self).get_cells(search_key, order, limit, last_cursor)
    }
}

// --- serde_molecule shapes (ckb-ssri-std indexer.rs) -------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    pub code_hash: [u8; 32],
    pub hash_type: u8,
    pub args: Vec<u8>,
}

impl From<&ckb_types::packed::Script> for Script {
    fn from(script: &ckb_types::packed::Script) -> Self {
        let mut code_hash = [0u8; 32];
        code_hash.copy_from_slice(script.code_hash().as_slice());
        Self {
            code_hash,
            hash_type: script.hash_type().into(),
            args: script.args().raw_data().to_vec(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptType {
    #[default]
    Lock,
    Type,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchMode {
    #[default]
    Prefix,
    Exact,
    Partial,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u64)]
pub enum Order {
    #[default]
    Asc = 0,
    Desc = 1,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchKeyFilter {
    pub script: Option<Script>,
    pub script_len_range: Option<[u64; 2]>,
    pub output_data: Option<Vec<u8>>,
    pub output_data_filter_mode: Option<SearchMode>,
    pub output_data_len_range: Option<[u64; 2]>,
    pub output_capacity_range: Option<[u64; 2]>,
    pub block_range: Option<[u64; 2]>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchKey {
    pub script: Script,
    pub script_type: ScriptType,
    pub script_search_mode: Option<SearchMode>,
    pub filter: Option<SearchKeyFilter>,
    pub with_data: Option<bool>,
    pub group_by_transaction: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IndexerCellOutput {
    pub capacity: u64,
    pub lock: Script,
    pub type_: Option<Script>,
}

impl From<&CellOutput> for IndexerCellOutput {
    fn from(output: &CellOutput) -> Self {
        Self {
            capacity: output.capacity().unpack(),
            lock: Script::from(&output.lock()),
            type_: output.type_().to_opt().as_ref().map(Script::from),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IndexerOutPoint {
    pub tx_hash: [u8; 32],
    pub index: u32,
}

impl From<&OutPoint> for IndexerOutPoint {
    fn from(op: &OutPoint) -> Self {
        let mut tx_hash = [0u8; 32];
        tx_hash.copy_from_slice(op.tx_hash().as_slice());
        Self {
            tx_hash,
            index: op.index().unpack(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LiveCell {
    pub output: Option<IndexerCellOutput>,
    pub data: Option<Vec<u8>>,
    pub block_hash: Option<[u8; 32]>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IndexerCell {
    pub output: IndexerCellOutput,
    pub output_data: Option<Vec<u8>>,
    #[serde(with = "serde_molecule::struct_serde")]
    pub out_point: IndexerOutPoint,
    pub block_number: u64,
    pub tx_index: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Pagination {
    #[serde(with = "serde_molecule::dynvec_serde")]
    pub objects: Vec<IndexerCell>,
    pub last_cursor: Vec<u8>,
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
    /// block_number → packed Header molecule bytes
    headers_by_number: HashMap<u64, Vec<u8>>,
    /// block_hash → packed Header molecule bytes
    headers_by_hash: HashMap<[u8; 32], Vec<u8>>,
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
            .insert(key, HostCell::new(output, data));
    }

    pub fn insert_cell_ex(&self, out_point: OutPoint, cell: HostCell) {
        let key = out_point.as_slice().to_vec();
        self.inner.lock().unwrap().cells.insert(key, cell);
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
            .insert(out_point.to_vec(), HostCell::new(output, data));
        Ok(())
    }

    pub fn insert_block_hash(&self, number: u64, hash: [u8; 32]) {
        self.inner.lock().unwrap().block_hashes.insert(number, hash);
    }

    /// Store a packed molecule `Header` under both its number and hash.
    pub fn insert_header(&self, number: u64, header: Header) {
        let bytes = header.as_slice().to_vec();
        let hash_byte32 = header.calc_header_hash();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(hash_byte32.as_slice());
        let mut inner = self.inner.lock().unwrap();
        inner.block_hashes.insert(number, hash);
        inner.headers_by_number.insert(number, bytes.clone());
        inner.headers_by_hash.insert(hash, bytes);
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

impl HostQuery for MockHost {
    fn find_cell_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        self.get_cell(out_point)
            .map(|c| c.output.as_slice().to_vec())
    }

    fn find_cell_data_by_out_point(&self, out_point: &[u8]) -> Option<Vec<u8>> {
        self.get_cell(out_point).map(|c| c.data)
    }

    fn get_live_cell(&self, out_point: &[u8], with_data: bool) -> Result<Vec<u8>, HostQueryError> {
        let live = match self.get_cell(out_point) {
            Some(cell) => LiveCell {
                output: Some(IndexerCellOutput::from(&cell.output)),
                data: with_data.then(|| cell.data.clone()),
                block_hash: cell.block_hash,
            },
            None => LiveCell {
                output: None,
                data: None,
                block_hash: None,
            },
        };
        serde_molecule::to_vec(&live, false).map_err(|_| HostQueryError::Encoding)
    }

    fn get_header(&self, block_hash: &[u8]) -> Option<Vec<u8>> {
        if block_hash.len() != 32 {
            return None;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(block_hash);
        self.inner
            .lock()
            .unwrap()
            .headers_by_hash
            .get(&key)
            .cloned()
    }

    fn get_header_by_number(&self, number: u64) -> Option<Vec<u8>> {
        self.inner
            .lock()
            .unwrap()
            .headers_by_number
            .get(&number)
            .cloned()
    }

    fn get_block_hash(&self, number: u64) -> Option<[u8; 32]> {
        MockHost::get_block_hash(self, number)
    }

    fn get_cells(
        &self,
        search_key: &[u8],
        order: u64,
        limit: u64,
        last_cursor: &[u8],
    ) -> Result<Vec<u8>, HostQueryError> {
        let key: SearchKey = serde_molecule::from_slice(search_key, false)
            .map_err(|_| HostQueryError::InvalidArgs)?;
        let order = match order {
            0 => Order::Asc,
            1 => Order::Desc,
            _ => return Err(HostQueryError::InvalidArgs),
        };
        // MockHost's opaque cursor encoding is little-endian page offset
        // (typically 8 bytes when non-empty). Accept 1..=8 as LE with
        // zero-extension so the API does not hard-require fixed 0/8 only;
        // longer opaque cursors are for live indexer hosts, not MockHost.
        let start = decode_mock_page_cursor(last_cursor)?;

        let inner = self.inner.lock().unwrap();
        let mut matched: Vec<(&Vec<u8>, &HostCell)> = inner
            .cells
            .iter()
            .filter(|(_, cell)| cell_matches(&key, cell))
            .collect();
        // ckb-indexer orders by (block_number, tx_index, output_index).
        matched.sort_by_key(|a| indexer_order_key(a.0, a.1));
        if matches!(order, Order::Desc) {
            matched.reverse();
        }

        let total = matched.len() as u64;
        let (page, next) = page_window(&matched, start, limit, total);
        let with_data = key.with_data.unwrap_or(true);
        let mut objects = Vec::with_capacity(page.len());
        for (op_bytes, cell) in page {
            let out_point = OutPoint::from_slice(op_bytes).map_err(|_| HostQueryError::Encoding)?;
            objects.push(IndexerCell {
                output: IndexerCellOutput::from(&cell.output),
                output_data: with_data.then(|| cell.data.clone()),
                out_point: IndexerOutPoint::from(&out_point),
                block_number: cell.block_number.unwrap_or(0),
                tx_index: cell.tx_index.unwrap_or(0),
            });
        }
        let pagination = Pagination {
            objects,
            last_cursor: if next >= total {
                Vec::new()
            } else {
                next.to_le_bytes().to_vec()
            },
        };
        serde_molecule::to_vec(&pagination, false).map_err(|_| HostQueryError::Encoding)
    }
}

fn page_window<'a, K, V>(
    matched: &[(&'a K, &'a V)],
    start: u64,
    limit: u64,
    total: u64,
) -> (Vec<(&'a K, &'a V)>, u64) {
    if start >= total {
        return (Vec::new(), total);
    }
    let start_index = start as usize;
    let end = if limit == 0 {
        start_index
    } else {
        start_index
            .saturating_add(limit as usize)
            .min(matched.len())
    };
    (matched[start_index..end].to_vec(), end as u64)
}

fn cell_matches(key: &SearchKey, cell: &HostCell) -> bool {
    let mode = key.script_search_mode.unwrap_or(SearchMode::Prefix);
    let primary = match key.script_type {
        ScriptType::Lock => script_matches(mode, &key.script, &cell.output.lock()),
        ScriptType::Type => cell
            .output
            .type_()
            .to_opt()
            .is_some_and(|script| script_matches(mode, &key.script, &script)),
    };
    if !primary {
        return false;
    }
    let Some(filter) = &key.filter else {
        return true;
    };
    if let Some(script) = &filter.script {
        let secondary = match key.script_type {
            ScriptType::Lock => cell.output.type_().to_opt().is_some_and(|type_script| {
                script_matches(SearchMode::Prefix, script, &type_script)
            }),
            ScriptType::Type => script_matches(SearchMode::Prefix, script, &cell.output.lock()),
        };
        if !secondary {
            return false;
        }
    }
    // ckb-indexer: filter by the *other* script's raw length
    // (code_hash ‖ hash_type ‖ args); absent secondary script → length 0.
    // Range is half-open [start, end).
    if let Some([start, end]) = filter.script_len_range {
        let script_len = match key.script_type {
            ScriptType::Lock => cell
                .output
                .type_()
                .to_opt()
                .map(|script| script_raw_len(&script))
                .unwrap_or(0),
            ScriptType::Type => script_raw_len(&cell.output.lock()),
        };
        if script_len < start || script_len >= end {
            return false;
        }
    }
    if let Some([start, end]) = filter.output_capacity_range {
        let capacity: u64 = cell.output.capacity().unpack();
        if capacity < start || capacity >= end {
            return false;
        }
    }
    if let Some([start, end]) = filter.output_data_len_range {
        let len = cell.data.len() as u64;
        if len < start || len >= end {
            return false;
        }
    }
    if let Some([start, end]) = filter.block_range {
        let number = cell.block_number.unwrap_or(0);
        if number < start || number >= end {
            return false;
        }
    }
    if let Some(output_data) = &filter.output_data {
        let mode = filter.output_data_filter_mode.unwrap_or(SearchMode::Prefix);
        let matched = match mode {
            SearchMode::Exact => cell.data == *output_data,
            SearchMode::Prefix => cell.data.starts_with(output_data),
            SearchMode::Partial => {
                output_data.is_empty()
                    || cell
                        .data
                        .windows(output_data.len())
                        .any(|window| window == output_data)
            }
        };
        if !matched {
            return false;
        }
    }
    true
}

fn script_matches(mode: SearchMode, query: &Script, cell: &ckb_types::packed::Script) -> bool {
    if cell.code_hash().as_slice() != query.code_hash {
        return false;
    }
    let ht: u8 = cell.hash_type().into();
    if ht != query.hash_type {
        return false;
    }
    let args = cell.args().raw_data();
    let cell_args = args.as_ref();
    match mode {
        SearchMode::Exact => cell_args == query.args,
        SearchMode::Prefix => cell_args.starts_with(&query.args),
        SearchMode::Partial => {
            query.args.is_empty()
                || cell_args
                    .windows(query.args.len())
                    .any(|window| window == query.args)
        }
    }
}

/// ckb-indexer script length: `code_hash (32) + hash_type (1) + args`.
fn script_raw_len(script: &ckb_types::packed::Script) -> u64 {
    (32 + 1 + script.args().raw_data().len()) as u64
}

/// ckb-indexer live-cell key suffix: `block_number ‖ tx_index ‖ output_index`
/// (each big-endian in the DB key; comparison order is numeric ascending).
fn indexer_order_key(op_bytes: &[u8], cell: &HostCell) -> (u64, u32, u32) {
    let output_index = OutPoint::from_slice(op_bytes)
        .map(|op| op.index().unpack())
        .unwrap_or(0);
    (
        cell.block_number.unwrap_or(0),
        cell.tx_index.unwrap_or(0),
        output_index,
    )
}

/// Decode MockHost's opaque page cursor (LE offset, empty or 1..=8 bytes).
fn decode_mock_page_cursor(last_cursor: &[u8]) -> Result<u64, HostQueryError> {
    if last_cursor.is_empty() {
        return Ok(0);
    }
    if last_cursor.len() > 8 {
        return Err(HostQueryError::InvalidArgs);
    }
    let mut buf = [0u8; 8];
    buf[..last_cursor.len()].copy_from_slice(last_cursor);
    Ok(u64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckb_types::packed::{Byte32, CellOutput, Header, OutPoint, Script as PackedScript};
    use ckb_types::prelude::{Builder, Entity, Pack};

    fn sample_out_point(tag: u8) -> OutPoint {
        OutPoint::new_builder()
            .tx_hash(Byte32::from_slice(&[tag; 32]).unwrap())
            .index(0u32.pack())
            .build()
    }

    fn sample_output(code: u8, args: &[u8]) -> CellOutput {
        CellOutput::new_builder()
            .capacity(100_000_000_000u64.pack())
            .lock(
                PackedScript::new_builder()
                    .code_hash(Byte32::from_slice(&[code; 32]).unwrap())
                    .hash_type(0u8.into())
                    .args(args.pack())
                    .build(),
            )
            .build()
    }

    #[test]
    fn find_cell_by_out_point_ok_and_missing() {
        let host = MockHost::new();
        let op = sample_out_point(1);
        let output = sample_output(0x11, &[0xaa]);
        host.insert_cell(op.clone(), output.clone(), vec![1, 2, 3]);
        assert_eq!(
            HostQuery::find_cell_by_out_point(&host, op.as_slice()).as_deref(),
            Some(output.as_slice())
        );
        assert!(HostQuery::find_cell_by_out_point(&host, sample_out_point(9).as_slice()).is_none());
    }

    #[test]
    fn find_cell_data_by_out_point_ok_and_missing() {
        let host = MockHost::new();
        let op = sample_out_point(2);
        host.insert_cell(op.clone(), sample_output(0x22, &[]), vec![0xde, 0xad]);
        assert_eq!(
            HostQuery::find_cell_data_by_out_point(&host, op.as_slice()),
            Some(vec![0xde, 0xad])
        );
        assert!(
            HostQuery::find_cell_data_by_out_point(&host, sample_out_point(9).as_slice()).is_none()
        );
    }

    #[test]
    fn get_live_cell_present_absent_and_with_data() {
        let host = MockHost::new();
        let op = sample_out_point(3);
        let mut cell = HostCell::new(sample_output(0x33, &[1]), vec![9, 8]);
        cell.block_hash = Some([0xAB; 32]);
        host.insert_cell_ex(op.clone(), cell);

        let with = HostQuery::get_live_cell(&host, op.as_slice(), true).unwrap();
        let live: LiveCell = serde_molecule::from_slice(&with, false).unwrap();
        assert!(live.output.is_some());
        assert_eq!(live.data.as_deref(), Some(&[9, 8][..]));
        assert_eq!(live.block_hash, Some([0xAB; 32]));

        let without = HostQuery::get_live_cell(&host, op.as_slice(), false).unwrap();
        let live: LiveCell = serde_molecule::from_slice(&without, false).unwrap();
        assert!(live.output.is_some());
        assert!(live.data.is_none());

        // Missing cell is encoded success with empty LiveCell (not item-missing).
        let missing =
            HostQuery::get_live_cell(&host, sample_out_point(9).as_slice(), true).unwrap();
        let live: LiveCell = serde_molecule::from_slice(&missing, false).unwrap();
        assert!(live.output.is_none());
        assert!(live.data.is_none());
        assert!(live.block_hash.is_none());
    }

    #[test]
    fn get_header_and_by_number_ok_and_missing() {
        let host = MockHost::new();
        let header = Header::default();
        let hash = header.calc_header_hash();
        host.insert_header(7, header.clone());

        let by_number = HostQuery::get_header_by_number(&host, 7).unwrap();
        assert_eq!(by_number, header.as_slice());
        assert!(HostQuery::get_header_by_number(&host, 8).is_none());

        let by_hash = HostQuery::get_header(&host, hash.as_slice()).unwrap();
        assert_eq!(by_hash, header.as_slice());
        assert!(HostQuery::get_header(&host, &[0u8; 32]).is_none());
        assert!(HostQuery::get_header(&host, &[0u8; 16]).is_none());
    }

    #[test]
    fn get_block_hash_ok_and_missing() {
        let host = MockHost::new();
        host.insert_block_hash(0, [0xCD; 32]);
        assert_eq!(HostQuery::get_block_hash(&host, 0), Some([0xCD; 32]));
        assert!(HostQuery::get_block_hash(&host, 1).is_none());
    }

    #[test]
    fn get_cells_bounded_pagination_and_errors() {
        let host = MockHost::new();
        let lock = PackedScript::new_builder()
            .code_hash(Byte32::from_slice(&[0x11; 32]).unwrap())
            .hash_type(0u8.into())
            .args([0xaa, 0x01].pack())
            .build();
        let lock2 = PackedScript::new_builder()
            .code_hash(Byte32::from_slice(&[0x11; 32]).unwrap())
            .hash_type(0u8.into())
            .args([0xaa, 0x02].pack())
            .build();
        let out1 = sample_out_point(0x10);
        let out2 = sample_out_point(0x20);
        host.insert_cell_ex(
            out1.clone(),
            HostCell {
                output: CellOutput::new_builder()
                    .capacity(100u64.pack())
                    .lock(lock.clone())
                    .build(),
                data: vec![1],
                block_hash: None,
                block_number: Some(8),
                tx_index: Some(0),
            },
        );
        host.insert_cell_ex(
            out2.clone(),
            HostCell {
                output: CellOutput::new_builder()
                    .capacity(200u64.pack())
                    .lock(lock2)
                    .build(),
                data: vec![2],
                block_hash: None,
                block_number: Some(9),
                tx_index: Some(0),
            },
        );

        let _ = lock; // lock args prefix matched via SearchKey below
        let key = SearchKey {
            script: Script {
                code_hash: [0x11; 32],
                hash_type: 0,
                args: vec![0xaa],
            },
            script_type: ScriptType::Lock,
            script_search_mode: Some(SearchMode::Prefix),
            filter: None,
            with_data: Some(true),
            group_by_transaction: None,
        };
        let key_bytes = serde_molecule::to_vec(&key, false).unwrap();

        let page0 = HostQuery::get_cells(&host, &key_bytes, 0, 1, &[]).unwrap();
        let pag: Pagination = serde_molecule::from_slice(&page0, false).unwrap();
        assert_eq!(pag.objects.len(), 1);
        assert_eq!(pag.last_cursor, 1u64.to_le_bytes().to_vec());

        let page1 = HostQuery::get_cells(&host, &key_bytes, 0, 1, &pag.last_cursor).unwrap();
        let pag1: Pagination = serde_molecule::from_slice(&page1, false).unwrap();
        assert_eq!(pag1.objects.len(), 1);
        assert!(pag1.last_cursor.is_empty());

        // limit=0 → empty page, cursor stays at start (0 encoded).
        let empty = HostQuery::get_cells(&host, &key_bytes, 0, 0, &[]).unwrap();
        let empty_pag: Pagination = serde_molecule::from_slice(&empty, false).unwrap();
        assert!(empty_pag.objects.is_empty());
        assert_eq!(empty_pag.last_cursor, 0u64.to_le_bytes().to_vec());

        // Invalid order
        assert_eq!(
            HostQuery::get_cells(&host, &key_bytes, 99, 1, &[]),
            Err(HostQueryError::InvalidArgs)
        );
        // Invalid search key
        assert_eq!(
            HostQuery::get_cells(&host, &[0xff, 0x00], 0, 1, &[]),
            Err(HostQueryError::InvalidArgs)
        );
        // Opaque cursor longer than MockHost's LE-u64 encoding is invalid here;
        // live indexer hosts accept arbitrary-length opaque cursors.
        assert_eq!(
            HostQuery::get_cells(&host, &key_bytes, 0, 1, &[0u8; 9]),
            Err(HostQueryError::InvalidArgs)
        );
        // Shorter opaque LE cursor (1..=8) is accepted (zero-extended).
        let short = HostQuery::get_cells(&host, &key_bytes, 0, 1, &[1u8]).unwrap();
        let short_pag: Pagination = serde_molecule::from_slice(&short, false).unwrap();
        assert_eq!(short_pag.objects.len(), 1);
        assert!(short_pag.last_cursor.is_empty());
    }

    #[test]
    fn get_cells_applies_script_len_range() {
        let host = MockHost::new();
        let lock = PackedScript::new_builder()
            .code_hash(Byte32::from_slice(&[0x11; 32]).unwrap())
            .hash_type(0u8.into())
            .args([0xaa].pack())
            .build();
        // Type script raw len = 32 + 1 + 2 = 35
        let typ = PackedScript::new_builder()
            .code_hash(Byte32::from_slice(&[0x22; 32]).unwrap())
            .hash_type(1u8.into())
            .args([0x10, 0x11].pack())
            .build();
        let with_type = sample_out_point(0x30);
        let without_type = sample_out_point(0x31);
        host.insert_cell(
            with_type.clone(),
            CellOutput::new_builder()
                .capacity(100u64.pack())
                .lock(lock.clone())
                .type_(Some(typ).pack())
                .build(),
            vec![],
        );
        host.insert_cell(
            without_type.clone(),
            CellOutput::new_builder()
                .capacity(100u64.pack())
                .lock(lock)
                .build(),
            vec![],
        );

        let base = SearchKey {
            script: Script {
                code_hash: [0x11; 32],
                hash_type: 0,
                args: vec![0xaa],
            },
            script_type: ScriptType::Lock,
            script_search_mode: Some(SearchMode::Exact),
            filter: None,
            with_data: Some(false),
            group_by_transaction: None,
        };

        // [0, 1) → only cells with no type script (len 0)
        let mut no_type = base.clone();
        no_type.filter = Some(SearchKeyFilter {
            script_len_range: Some([0, 1]),
            ..SearchKeyFilter::default()
        });
        let bytes = serde_molecule::to_vec(&no_type, false).unwrap();
        let page: Pagination = serde_molecule::from_slice(
            &HostQuery::get_cells(&host, &bytes, 0, 10, &[]).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(page.objects.len(), 1);
        assert_eq!(
            page.objects[0].out_point,
            IndexerOutPoint::from(&without_type)
        );

        // [35, 36) → only the typed cell (raw len 35)
        let mut typed = base;
        typed.filter = Some(SearchKeyFilter {
            script_len_range: Some([35, 36]),
            ..SearchKeyFilter::default()
        });
        let bytes = serde_molecule::to_vec(&typed, false).unwrap();
        let page: Pagination = serde_molecule::from_slice(
            &HostQuery::get_cells(&host, &bytes, 0, 10, &[]).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(page.objects.len(), 1);
        assert_eq!(page.objects[0].out_point, IndexerOutPoint::from(&with_type));
    }

    #[test]
    fn get_cells_orders_by_block_tx_output_index() {
        let host = MockHost::new();
        let lock = PackedScript::new_builder()
            .code_hash(Byte32::from_slice(&[0x11; 32]).unwrap())
            .hash_type(0u8.into())
            .args([0xaa].pack())
            .build();
        // Outpoint bytes would sort A (0x01) before B (0x02) before C (0x03),
        // but indexer order is (block, tx_index, output_index).
        let out_a = OutPoint::new_builder()
            .tx_hash(Byte32::from_slice(&[0x01; 32]).unwrap())
            .index(5u32.pack())
            .build();
        let out_b = OutPoint::new_builder()
            .tx_hash(Byte32::from_slice(&[0x02; 32]).unwrap())
            .index(0u32.pack())
            .build();
        let out_c = OutPoint::new_builder()
            .tx_hash(Byte32::from_slice(&[0x03; 32]).unwrap())
            .index(1u32.pack())
            .build();
        let mk = |op: OutPoint, block: u64, tx_index: u32| {
            host.insert_cell_ex(
                op,
                HostCell {
                    output: CellOutput::new_builder()
                        .capacity(100u64.pack())
                        .lock(lock.clone())
                        .build(),
                    data: vec![],
                    block_hash: None,
                    block_number: Some(block),
                    tx_index: Some(tx_index),
                },
            );
        };
        // Asc indexer order should be: C (block 1, tx 0, idx 1), B (block 2, tx 0),
        // A (block 2, tx 1) — not outpoint-byte order A,B,C.
        mk(out_a.clone(), 2, 1);
        mk(out_b.clone(), 2, 0);
        mk(out_c.clone(), 1, 0);

        let key = SearchKey {
            script: Script {
                code_hash: [0x11; 32],
                hash_type: 0,
                args: vec![0xaa],
            },
            script_type: ScriptType::Lock,
            script_search_mode: Some(SearchMode::Exact),
            filter: None,
            with_data: Some(false),
            group_by_transaction: None,
        };
        let key_bytes = serde_molecule::to_vec(&key, false).unwrap();
        let page: Pagination = serde_molecule::from_slice(
            &HostQuery::get_cells(&host, &key_bytes, 0, 10, &[]).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(page.objects.len(), 3);
        assert_eq!(page.objects[0].out_point, IndexerOutPoint::from(&out_c));
        assert_eq!(page.objects[0].block_number, 1);
        assert_eq!(page.objects[0].tx_index, 0);
        assert_eq!(page.objects[1].out_point, IndexerOutPoint::from(&out_b));
        assert_eq!(page.objects[1].block_number, 2);
        assert_eq!(page.objects[1].tx_index, 0);
        assert_eq!(page.objects[2].out_point, IndexerOutPoint::from(&out_a));
        assert_eq!(page.objects[2].block_number, 2);
        assert_eq!(page.objects[2].tx_index, 1);

        let desc: Pagination = serde_molecule::from_slice(
            &HostQuery::get_cells(&host, &key_bytes, 1, 10, &[]).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(desc.objects[0].out_point, IndexerOutPoint::from(&out_a));
        assert_eq!(desc.objects[2].out_point, IndexerOutPoint::from(&out_c));
    }
}
