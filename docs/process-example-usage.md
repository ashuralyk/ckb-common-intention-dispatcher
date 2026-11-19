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
2. **Transfer** (1→1 lock pair): TYPE_ID on both sides; batch `[1,4]`; unique
   beneficiary / refund assignment (same rules as before).  
3. **Burn** rejected.

| Code | Error |
|------|--------|
| 30 | `ProcessPairInvalid` |
| 31 | `BatchCountInvalid` |
| 32 | `BeneficiaryMissing` |
| 33 | `BeneficiaryAmountMismatch` |
| 34 | `UnsupportedAsset` |
| 35 | `TypeIdInvalid` (missing / non-TYPE_ID type) |
| 36 | `HostQueryFailed` |
