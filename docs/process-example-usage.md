# CID Process Example Usage

Sample Process for CID after the Process-as-lock architecture: offline SSRI host,
RISC-V Process with `cinnabar_main!` SSRI handlers, and FakeRpc tests.

## Architecture (confirmed)

| Piece | Role |
|-------|------|
| Process **lock** | Process contract (code Type ID, `hash_type=type`) — all app-layer verify |
| Process **type** | Built-in TYPE_ID (identity / Process Type ID) |
| Intention `process_type_hash` | Script hash of that TYPE_ID type script |
| CID | Only checks type-hash presence in inputs **and** outputs |
| `Process.assemble` | Returns a **complete** molecule `Transaction` |

See also [`ssri-backend-interface.md`](./ssri-backend-interface.md).

## Components

| Path | Role |
|------|------|
| `contracts/cid_process_example/` | RISC-V Process lock + SSRI + host SDK wrappers |
| `crates/cid-process-executor/` | Offline SSRI VM + `MockHost` for cell/tip queries |
| `calculator/process/` | `process_digest_ok` / `_with_process` → full-tx skeleton |
| `tests/src/process_example.rs` | FakeRpc + SSRI round-trips |

## Build / test

```bash
make prepare
./scripts/build-all-contracts.sh
cargo test -p cid-process-executor
cargo test -p cid_process_example-calculator
cd tests && cargo test process_example -- --nocapture
```

## SSRI

- **Process.batch** — `(min, max)` = `(1, 4)` as 8 LE bytes  
- **Process.assemble** — stdin = molecule `Transaction` draft (Intention + Process
  inputs); stdout = complete molecule `Transaction` (inputs, outputs,
  outputs_data, cell_deps, header_deps, witnesses). Cells are loaded via
  ckb-ssri-std host queries (`find_cell_by_out_point`, …).

## On-chain verification (Process **lock**)

1. **Create** (0 lock inputs, 1 output): state cell type must be built-in TYPE_ID.  
2. **Transfer** (1→1 lock pair): TYPE_ID on both sides **with equal type-script
   hash** (identity continuity); that TYPE_ID must not appear on any
   non-Process-lock output; batch `[1,4]`; unique beneficiary / refund
   assignment.  
3. **Burn** rejected.

**Lock args:** use distinct Process lock args per live state cell. Identical
args (including empty) merge cells into one lock group, so a two-Process tx
becomes N→N and fails `ProcessPairInvalid` instead of running two 1→1 pairs.

| Code | Error |
|------|--------|
| 30 | `ProcessPairInvalid` |
| 31 | `BatchCountInvalid` |
| 32 | `BeneficiaryMissing` |
| 33 | `BeneficiaryAmountMismatch` |
| 34 | `UnsupportedAsset` |
| 35 | `TypeIdInvalid` (missing / non-TYPE_ID / continuity break / smuggled Type ID) |
| 36 | `HostQueryFailed` |

## Capacity / fee (example assemble)

`Process.assemble` deducts a Process-owned miner fee from the recreated state
cell capacity (`ASSEMBLE_FEE`). Optional Process-owned funding inputs in the
draft stay in the returned transaction; their surplus capacity also pays fee.
The backend must submit the molecule `Transaction` without rewriting structure
to “fix” balance.
