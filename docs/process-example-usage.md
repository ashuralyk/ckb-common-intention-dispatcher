# CID Process Example Usage

Sample Process implementation for CID v0.1: an offline SSRI host, a RISC-V
Process with `cinnabar_main!` SSRI handlers, and FakeRpc tests that call
`process_digest_ok` against the real binary.

This is **not** a production Process. Beneficiary locks use a placeholder
code hash; Process recreate is **not** yet emitted on the PartialTx /
skeleton (Type ID pair is checked on-chain when that cell is present).

## Components

| Path | Role |
|------|------|
| `contracts/cid_process_example/` | RISC-V Process: `Process.batch` / `Process.assemble` + on-chain verify |
| `crates/cid-process-executor/` | Greenfield offline SSRI host (ckb-vm, argv method id, `vm_version = u64::MAX`) |
| `process_calculator/` | `process_digest_ok` — batch bounds, assemble, skeleton inputs + beneficiary outputs |
| `tests/src/process_example.rs` | FakeRpcClient digest tests (n=1, 2, 4) and batch-boundary negatives |

## Build

First-time RISC-V target:

```bash
make prepare
```

Both contracts into `build/release/`:

```bash
./scripts/build-all-contracts.sh
```

Or individually:

```bash
cd contracts/cid_process_example && make build
# CID copy to build/release/ needs BUILD_DIR from the repo root Makefile
# (or: make -C contracts/ckb_common_intention_dispatcher with TOP/BUILD_DIR exported)
```

Expected artifacts:

- `build/release/cid_process_example`
- `build/release/ckb_common_intention_dispatcher` (and `cid_process_fixture` when the CID makefile copies it)

## Test

Executor (unit + real-binary smoke, fixtures included):

```bash
cargo test -p cid-process-executor
```

Calculator (includes FakeRpcClient output-count smoke):

```bash
cargo test -p cid_process_example-calculator
```

Process example FakeRpc tests (requires binaries in `build/release/`):

```bash
cd tests && cargo test process_example -- --nocapture
```

`require_binaries()` fails loudly if the CID or Process binary is missing.
Two tests stay `#[ignore]`: `reject_wrong_beneficiary` and
`reject_missing_type_id_pair` (need full on-chain FakeRpc verify).

## Batch configuration

Enforced in SSRI (`assemble_handler`) and on-chain (`verify.rs`):

- **BATCH_MIN** = 1
- **BATCH_MAX** = 4

Empty batch and n=5 return `CalculatorError::Other` with `"out of bounds"`
from `process_digest_ok` (after `get_batch_bounds`). The VM path maps
custom exit ≥ 20 to `ExecutorError::Incompatible` (e.g. 21 for
`IncompatibleBatch`).

## SSRI methods (`cinnabar_main!`)

- **Process.batch** — 8 LE bytes: `min` then `max` → `(1, 4)`
- **Process.assemble** — stdin: `count` + length-prefixed intentions
  (`capacity`, `lock_args`, `data`). Returns serialized PartialTx:
  beneficiary `CellOutput`s (placeholder lock code hash + intention
  `lock_args`) plus output data.

Host argv[0] is the hex-encoded blake2b method id (not the UTF-8 path).

## Executor API

```rust
use cid_process_executor::{IntentionInput, ProcessExecutor};

let binary = std::fs::read("build/release/cid_process_example")?;
let executor = ProcessExecutor::new(binary);

let (min, max) = executor.get_batch_bounds()?; // (1, 4)
let partial_tx = executor.assemble(&intentions)?;
```

- `ExecutorError::Incompatible(code)` — Process rejected the batch (exit ≥ 20)
- `ExecutorError::VmError` — VM / load failure
- `ExecutorError::InvalidResponse` — SSRI payload parse failure

## Calculator and molecule bridge

`process_digest_ok(rpc, intentions, bin_dir)`:

1. Loads `cid_process_example` from `bin_dir`
2. Checks count against SSRI batch bounds
3. Calls `assemble`
4. Adds intention inputs to a `TransactionSkeleton`
5. Bridges each executor `CellOutput` (ckb-gen-types 0.119) to calculator
   types (1.x) via molecule bytes (`PartialTx::output_molecule_pairs` +
   `CellOutput::from_slice`) and appends beneficiary outputs

`instructions` is currently an empty `Vec`. Process recreate is **not**
added to the skeleton (Task 5 carry-forward). The calculator `IntentionCell`
OutPoint is used on the skeleton; the executor `OutPoint` in
`to_intention_input` is a placeholder (unused by assemble encode).

## On-chain verification (Process type)

`verify.rs` (custom errors 30–32):

1. Type ID pair: exactly 1 GroupInput + 1 GroupOutput with the Process type
2. Intention count in `[1, 4]` (CID lock args prefix = Process type hash,
   `args.len() > 32`)
3. All outputs counted via `load_cell_lock`; require
   `output_count >= intention_count + 1` (beneficiaries + Process)

Full FakeRpc on-chain reject paths are the ignored Task 10 placeholders.

## Architecture

```
User / CID calculator (create, refund, timeout)
        │
        ▼
process_digest_ok  →  cid-process-executor  →  CKB-VM + cid_process_example
        │                     SSRI Process.assemble
        ▼
TransactionSkeleton: intention inputs + beneficiary outputs
        │  (Process recreate not composed yet)
        ▼
On-chain (when a full tx is submitted):
  CID lock basic rules; Process type: pair, batch [1,4], beneficiary count
```

## What is deferred

- Process consume + recreate cell on PartialTx / skeleton
- Real beneficiary Script from CID payload (placeholder `DEFAULT_LOCK_CODE_HASH`)
- Official SSRI daemon (this host is greenfield, not vendored ssri-server)
- Ignored FakeRpc tests for wrong beneficiary / missing Type ID pair
