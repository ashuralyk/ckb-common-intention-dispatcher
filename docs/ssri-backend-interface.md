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
(`has_matching_process`). All app-layer rules run in the Process **lock**.

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

The backend **must not** rewrite the molecule transaction returned by
`Process.assemble` (except filling unlock witnesses the Process left empty,
if the Process documents that). Capacity balance and fee are Process
responsibilities.

## SSRI methods

### `Process.batch`

- **stdin:** empty  
- **stdout:** 8 bytes LE = `batch_min` ‖ `batch_max` (example: `1` / `4`)

### `Process.assemble`

- **stdin:** packed molecule `Transaction` **draft** whose `raw.inputs` list the
  Intention + Process state out points (outputs/deps may be empty).
- **stdout:** packed molecule `Transaction` that is **complete and
  directly submittable**: `inputs`, `outputs`, `outputs_data`, `cell_deps`,
  `header_deps`, `witnesses`.

Guest resolution of draft inputs uses host-query syscalls (below). The example
Process recreates the Process cell, emits one beneficiary per Intention, copies
draft deps, and attaches one empty witness per input.

## Host-query syscalls (ckb-ssri-std)

Process SDK wrappers live in `contracts/cid_process_example/src/host.rs` and
call **ckb-ssri-std** native-injection syscalls. The offline executor mocks the
same numbers in `crates/cid-process-executor`.

| Syscall | Number | Purpose |
|---------|--------|---------|
| `find_cell_by_out_point` | 2287 | `CellOutput` molecule for an out point |
| `find_cell_data_by_out_point` | 2297 | cell data bytes |
| `get_live_cell` | 2317 | molecule live cell (optional richer path) |
| `get_header` | 2327 | header by hash |
| `get_header_by_number` | 2337 | header by number (tip / deps) |
| `get_block_hash` | 2347 | block hash by number (tip probe) |
| `get_cells` | 2367 | indexer search (counterparties) |

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
get_cells_by_search_key(search_key, limit) -> Pagination
header_by_number(n) / block_hash(n)       -> tip / header info
```

`SearchKey` / `Pagination` encodings are the ckb-indexer molecule tables
documented in `ckb-ssri-std` (`indexer.rs`).

## Executor API (Rust)

```rust
use cid_process_executor::{build_draft_tx, AssembledTx, DraftCell, MockHost, ProcessExecutor};

let host = MockHost::new();
// host.insert_cell(out_point, output, data);
// host.insert_block_hash(0, tip_hash);
let draft = build_draft_tx(&cells);
let assembled: AssembledTx = executor.assemble(&draft, host)?;
// assembled.as_molecule_bytes() → submit
```

## Intention CLI

`process_type_script` in the create request is the Process state cell’s
**TYPE_ID type script**, not the Process code cell. Its `calc_script_hash` is
written into Intention lock args as `process_type_hash`.
