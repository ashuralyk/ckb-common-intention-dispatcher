# SSRI backend interface (Process assemble + host query)

Authoritative contract between the off-chain SSRI host (backend) and
`cid_process_example` after the Process-as-lock architecture.

## Process state cell

| Field | Value |
|-------|--------|
| **lock** | Process contract, referenced by its **code Type ID** (`hash_type = type`) |
| **type** | Built-in **TYPE_ID** script (`code_hash = TYPE_ID`, `hash_type = type`, unique args) |
| **identity** | Script hash of the TYPE_ID type script (= Intention `process_type_hash`) |

CID only checks that this type hash appears in **both** inputs and outputs
(`has_matching_process`). All app-layer rules run in the Process **lock**,
including **TYPE_ID continuity** (Process lock input type hash == output type
hash) and forbidding that Type ID on non-Process-lock outputs.

**Process lock args** must be distinct per live instance. Empty / shared args
merge cells into one lock group (`ProcessPairInvalid` on multi-Process txs).

**Creation:** CKB builds lock groups from inputs, so an output-only Process lock
on first mint never runs `verify_create`. Creation tools (CLI / deploy) must
validate Process **lock** + **TYPE_ID** shape off-chain; on-chain Process lock
checks run on **spend** (Transfer).

## DigestOk transaction shape

```
inputs:
  - Intention cell(s)   (CID lock; optional xUDT type)
  - Process state cell  (Process lock + TYPE_ID type)
  - optional counterparty / fee cells added by Process

outputs:
  - Process state cell recreate (same TYPE_ID type; lock stays Process)
  - beneficiary / asset cells required by the Process
  - optional change

cell_deps / header_deps / witnesses:
  - filled by Process.assemble as needed
```

The backend **must not** rewrite or fill anything in the molecule transaction
returned by `Process.assemble` — including witnesses. Capacity balance and fee
are Process responsibilities; submit the returned bytes unchanged.

## Sequencer flow (scan → route → filter → assemble)

1. **Scan** live Intention cells (indexer / node).
2. **Route** with the Process-independent helper
   [`intention_process_type_id`](../crates/cid-intention-filter/src/route.rs)
   (crate `cid-intention-filter`, not linked into the CID lock): require CID
   lock `hash_type = type` and `code_hash =` the Sequencer's CID code Type ID
   script hash; extract `process_type_hash`. Group Intentions by that Process
   Type ID. Non-Intentions / malformed cells get a typed
   [`IntentionRouteError`]. **Ownership is solely the Sequencer's job.**
3. **Filter** per Process: call SSRI `Process.filter` with the already-routed
   cell batch. The filter judges **Process business compliance only**
   (malformed cell/digest, unsupported asset/action, deposit below Process
   minimum, …) — it does **not** re-check CID Type-ID ownership.
4. **Assemble** with the accepted Intentions + live Process state via
   `Process.assemble` (plain molecule `Transaction` draft).

Host wrapper: `cid_process_example_calculator::process_filter`.

## SSRI methods

### `Process.batch`

- **stdin:** empty  
- **stdout:** 8 bytes LE = `batch_min` ‖ `batch_max` (example: `1` / `4`)

### `Process.filter`

- **stdin:** [`FilterRequest`](../crates/cid-intention-filter/src/filter.rs) via
  `encode_filter_request`:
  - `count:u32 LE` then per cell: length-prefixed packed `OutPoint`, packed
    `CellOutput`, and `data` (same molecule cell encoding as elsewhere)
  - empty stdin or `count = 0` → empty verdict list
  - soft cap: `FILTER_MAX_CELLS` (64)
  - **Callers must route first** — no `process_type_hash` /
    `cid_code_type_id_hash` in the request
- **stdout:** `count:u32 LE || reason:i8 * count` (same order as input)
- **Semantics:** per-cell; one bad cell does **not** abort the batch.
  Process-owned business rules only (deposit min, supported assets, digest).

| Reason | Code | Meaning |
|--------|------|---------|
| `FILTER_ACCEPTED` | 0 | Meets this Process's business requirements |
| `FILTER_MALFORMED_CELL` | 1 | Packed `CellOutput` / framing invalid |
| `FILTER_RESERVED_2` | 2 | Reserved (was ownership / not-Intention) |
| `FILTER_RESERVED_3` | 3 | Reserved (was process mismatch) |
| `FILTER_UNSUPPORTED_ASSET` | 4 | Typed asset not supported (e.g. non-xUDT) |
| `FILTER_DEPOSIT_BELOW_MINIMUM` | 5 | CKB deposit below Process minimum |
| `FILTER_MALFORMED_DIGEST` | 6 | Payload / digest undecodable for this Process |
| `FILTER_UNSUPPORTED_ACTION` | 7 | Action not supported (reserved) |

Example Process **business** classification is shared with assemble / on-chain
verify via `crates/cid-process-example-policy` (`classify_intention`). On-chain
Verify keeps `ours_prefix` scoping separately (bd11d44 semantics); it does not
re-check CID Type-ID ownership.

`Process.filter` has no Process state cell and cannot learn its own lock
`code_hash`, so it does **not** reject beneficiaries that reuse the Process
binary as lock. Assemble and Verify do (via
`classify_intention_excluding_process_lock`). Sequencers that know the live
Process lock `code_hash` should pre-check with that helper before assemble.

On-chain Transfer also requires `tx_fee = Σ(input caps) − Σ(output caps)` to
cover the aggregate positive Process state-cell debit for every lock sharing
this Process `code_hash`/`hash_type` (in addition to the per-cell fee-budget
cap), so diverted state capacity cannot masquerade as fee.

### `Process.assemble`

- **stdin:** packed molecule `Transaction` **draft**. Each Process defines its
  own draft preconditions. For `cid_process_example`, the draft must include:
  - `raw.inputs`: Intention and Process state out points only
  - `raw.cell_deps`: at least CID lock code, Process code, and any official
    xUDT code deps needed to execute the returned tx
  - `header_deps` / `witnesses` as the Process documents (may be empty)
- **stdout:** packed molecule `Transaction` that is **complete and
  directly submittable**: `inputs`, `outputs`, `outputs_data`, `cell_deps`,
  `header_deps`, `witnesses`. Submit these bytes without rewriting structure
  (including witness encodings).

Guest resolution of draft inputs uses host-query syscalls (below). The example
Process recreates the Process cell (capacity minus fee =
`FeeRate(FEE_RATE).fee(serialized_size_in_block)`, with
`serialized_size_in_block = molecule_len + 4`), emits one beneficiary per
Intention, keeps every draft input **including `since`**, copies draft
deps/header_deps, and attaches one empty witness per input (complete witness
vector — backend must not fill any). If fee would drop the state cell below
occupied capacity, assemble returns `CapacityInsufficient` (37).

The example pays the fee from the Process state and rejects extra funding inputs
with `UnsupportedFundingLock` (38) when invoked directly. CKB Intention
capacity below Process `MIN_DEPOSIT_CAPACITY` returns `DepositBelowMinimum`
(39). The calculator rejects a non-empty `funding_cells` list before building
the draft, and rejects CKB deposits below that minimum.

## Host-query syscalls (ckb-ssri-std)

Process SDK wrappers live in `contracts/cid_process_example/src/host.rs` and
call **ckb-ssri-std** native-injection syscalls. The offline executor exposes the
same catalog through the `HostQuery` trait (`MockHost` implements all seven) in
`crates/cid-process-executor`.

| Syscall | Number | Purpose |
|---------|--------|---------|
| `find_cell_by_out_point` | 2287 | `CellOutput` molecule for an out point |
| `find_cell_data_by_out_point` | 2297 | cell data bytes |
| `get_live_cell` | 2317 | molecule live cell (optional richer path) |
| `get_header` | 2327 | header by hash |
| `get_header_by_number` | 2337 | header by number (tip / deps) |
| `get_block_hash` | 2347 | block hash by number (tip probe) |
| `get_cells` | 2367 | indexer search with the requested order and cursor |

### Calling convention (`syscall_load`)

Registers match ckb-ssri-std `native.rs`:

- `a0` = response buffer pointer  
- `a1` = pointer to `usize` length (capacity in, actual length out)  
- `a2`…`a6` = method-specific (out point pointer, search key, block number, …)  
- `a7` = syscall number  

Return in `a0`: `0` success, `1` index/args error, `2` item missing. If actual
length exceeds capacity, guest retries with a larger buffer
(`SysError::LengthNotEnough`).

### Minimal Process SDK surface

```text
get_cell_by_out_point(out_point) -> (CellOutput, data)
get_cells_by_search_key(search_key, order, limit, last_cursor) -> Pagination
header_by_number(n) / block_hash(n)       -> tip / header info
```

`order` is Asc (`0`) or Desc (`1`). `last_cursor` is an **opaque** indexer
cursor of **arbitrary length** (empty = first page) — pass
`Pagination.last_cursor` from the previous page through unchanged. Do not
assume a fixed 0/8-byte layout; live ckb-indexer cursors are host-defined.

`SearchKey` / `Pagination` encodings are the ckb-indexer molecule tables
documented in `ckb-ssri-std` (`indexer.rs`).

For Exact and Prefix queries, `MockHost::get_cells` orders cells by the
matching script's full raw bytes followed by the big-endian block number,
transaction index, and output index, as the built-in CKB indexer does. It
reverses that order for Desc. Missing block and transaction metadata defaults
to zero. MockHost returns its own page-offset cursor. RPC-backed hosts use
the indexer's cursor.

## Executor API (Rust)

The calculator's submit helper builds a draft with the supplied live code
dependencies and returns the packed transaction from the guest.

```rust
use cid_process_example_calculator::{
    process_digest_ok_with_extras, AssembleExtras,
};

let (_, assembled, _) = process_digest_ok_with_extras(
    &rpc,
    intentions,
    process_state,
    AssembleExtras {
        funding_cells: vec![],
        cell_deps, // Live CID, Process, and applicable xUDT code dependencies.
    },
    bin_dir,
)?;
let transaction = assembled.as_molecule_bytes();
```

For direct executor calls, pass a packed draft that already includes those
dependencies. `build_draft_tx` only populates inputs for offline testing.

`ProcessExecutor::assemble` is generic over `H: HostQuery + Send + 'static`
(also accepts `Box<dyn HostQuery + Send>` / `Arc<T: HostQuery>` via blanket
impls — `Send` is required because ckb-vm `Syscalls` is `Send`). Calculator
`process_digest_ok_with_extras` seeds a `MockHost`; use
`process_digest_ok_with_host` to pass a live host.

Calculator helpers require a **live** [`ProcessStateCell`] (no fabricated
defaults). **Documented submit entry:** `process_digest_ok_with_extras` /
`process_digest_ok_with_host` with non-empty `AssembleExtras.cell_deps` (CID +
Process + any xUDT) and empty `funding_cells`. Returns packed
[`AssembledTx`] for byte-identical submit. `process_digest_ok_with_process`
still requires non-empty `cell_deps`. Resolve state via
`resolve_process_state_by_type` when you have the TYPE_ID type script.

## Intention CLI

`process_type_script` in the create request is the Process state cell’s
**TYPE_ID type script**, not the Process code cell. Its `calc_script_hash` is
written into Intention lock args as `process_type_hash`. The CLI rejects
non-TYPE_ID `process_type_script` values (code hash, `hash_type=type`, 32-byte
args). Process lock Create still does not run on-chain at mint — validate the
Process lock script off-chain when deploying the state cell.
