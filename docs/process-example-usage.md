# CID Process Example Usage

Sample Process implementation for CID v0.1: an offline SSRI host, a RISC-V
Process with `cinnabar_main!` SSRI handlers, and FakeRpc tests that call
`process_digest_ok` against the real binary.

This is **not** a production Process. Process recreate is **not** yet emitted
on the PartialTx / skeleton. On-chain, Create initializes Type ID args via
`calc_type_id`; Transfer checks the 1→1 pair, batch bounds, and beneficiary
Script / capacity / data / supported assets from each CID payload.

## Components

| Path | Role |
|------|------|
| `contracts/cid_process_example/` | RISC-V Process: `Process.batch` / `Process.assemble` + on-chain verify |
| `crates/cid-process-executor/` | Greenfield offline SSRI host (ckb-vm, argv method id, `vm_version = u64::MAX`) |
| `calculator/process/` | `process_digest_ok` — batch bounds, assemble, skeleton inputs + beneficiary outputs |
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
One test stays `#[ignore]`: `reject_missing_type_id_pair` — the Process type
script does not execute when the Process cell is absent (CID reports
`ERROR_PROCESS_MISMATCH` instead). `reject_wrong_beneficiary` and
amount/data/asset mismatch tests run on-chain.

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
  (`capacity`, `lock_args`, `data`, optional type-script molecule). Returns
  a packed molecule `Transaction` with beneficiary `outputs` + `outputs_data`
  (other fields empty). When `lock_args` decode as CID args, the lock is the
  `CidEntry.payload` Script (including Data2); otherwise a placeholder code
  hash plus the raw `lock_args` (host unit tests). An official xUDT type on
  the intention is copied onto the output.

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
- `ExecutorError::CyclesExceeded` — guest hit the instruction cycle budget
  (`estimate_cycles`; default `MAX_CYCLES = 70_000_000`). Host stdout/pipe
  buffers are capped at 1 MiB.
- `ExecutorError::InvalidResponse` — SSRI payload parse failure (including
  molecule `Transaction` verification and output-count / byte-budget guards
  in `PartialTx::from_bytes`)

## Calculator and molecule Transaction

`process_digest_ok(rpc, intentions, bin_dir)`:

1. Loads `cid_process_example` from `bin_dir`
2. Checks count against SSRI batch bounds
3. Calls `assemble`, which returns a packed molecule `Transaction`
4. Adds intention inputs to a `TransactionSkeleton`
5. Parses that same `Transaction` with calculator ckb-types (1.x) and
   appends beneficiary outputs

`instructions` is currently an empty `Vec`. Process recreate is **not**
added to the skeleton (Task 5 carry-forward). The calculator `IntentionCell`
OutPoint is used on the skeleton; the executor `OutPoint` in
`to_intention_input` is a placeholder (unused by assemble encode).

## On-chain verification (Process type)

`verify.rs` (custom errors 30–35):

1. **Create** (0 type-group inputs, 1 output): Type ID args must be 32 bytes
   and equal `calc_type_id(output_index)`. No intention batch on Create.
2. **Transfer** (1→1 type-group pair): this group's intention count in
   `[1, 4]`. Unique-assign outputs transaction-wide across (a) DigestOk
   payload payments whose Process type hash is in both inputs and outputs
   (any hash_type alias of this example, including Data1/Data2) and
   (b) DigestIncompatible and TimeoutBurn refunds (user_lock, never the
   payload). Refunds are not treated as beneficiary payments, but their
   refund cells cannot be reused by a payment. Payload Script hash types
   0/1/2/4 (Data/Type/Data1/Data2) are accepted. Matching unused output
   must have the same lock, capacity, data, and type (CKB or official xUDT).
3. **Burn** is rejected (`ProcessPairInvalid`).

| Code | Error |
|------|--------|
| 30 | `ProcessPairInvalid` |
| 31 | `BatchCountInvalid` |
| 32 | `BeneficiaryMissing` |
| 33 | `BeneficiaryAmountMismatch` |
| 34 | `UnsupportedAsset` |
| 35 | `TypeIdInvalid` |

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
  CID lock basic rules; Process type: Create Type ID init, or Transfer
  1→1 pair + batch [1,4] + tx-wide unique beneficiary Script/capacity/data/asset
```

## What is deferred

- Process consume + recreate cell on PartialTx / skeleton
- Official SSRI daemon (this host is greenfield, not vendored ssri-server)
- Ignored FakeRpc test `reject_missing_type_id_pair` (Process type does not
  run when the Process cell is absent)
