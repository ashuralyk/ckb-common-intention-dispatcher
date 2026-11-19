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
| `calculator/process/` | submit: `process_digest_ok_with_extras` (required deps); `_with_process` also requires deps |
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
  inputs + optional AlwaysSuccess funding; required code `cell_deps`); stdout =
  complete molecule `Transaction` (inputs, outputs, outputs_data, cell_deps,
  header_deps, witnesses). Cells are loaded via ckb-ssri-std host queries
  (`find_cell_by_out_point`, …). Keyless Process: funding must be AlwaysSuccess
  (`Data1`); secp / same-Process-lock funding → `UnsupportedFundingLock` (38).

## On-chain verification (Process **lock**)

CKB builds lock groups from **inputs**, so an output-only Process lock on first
mint never runs. Off-chain creation tools must validate Process lock + TYPE_ID;
on-chain Process lock checks run on **spend**:

1. **Transfer** (1→1 lock pair): TYPE_ID on both sides **with equal type-script
   hash** (identity continuity); that TYPE_ID must not appear on any
   non-Process-lock output; batch `[1,4]`; unique beneficiary / refund
   assignment.  
2. **Burn** rejected.  
3. **Create** path in `verify.rs` exists for completeness but is not executed
   for output-only mints.

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
| 37 | `CapacityInsufficient` (fee would drop state below occupied capacity) |
| 38 | `UnsupportedFundingLock` (funding not AlwaysSuccess / same Process lock) |

## Capacity / fee (example assemble)

`Process.assemble` computes fee as `FeeRate(FEE_RATE).fee(serialized_size_in_block)`
— i.e. `floor((molecule_len + 4) × FEE_RATE / 1000)` with Process-owned
`FEE_RATE = 1000` (testnet `min_fee_rate`) — and deducts it from the recreated
state cell. The `+ 4` matches CKB `serialized_size_in_block` (molecule
`NUMBER_SIZE` in the block), so the fee is never below the node min fee. If the
result would be below occupied capacity, assemble errors. Draft must include
CID, Process, and applicable xUDT cell_deps; the example copies them through.
Submit returned molecule bytes unchanged.
