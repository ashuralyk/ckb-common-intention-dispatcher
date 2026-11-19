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
  inputs and required code `cell_deps`); stdout =
  complete molecule `Transaction` (inputs, outputs, outputs_data, cell_deps,
  header_deps, witnesses). Cells are loaded via ckb-ssri-std host queries
  (`find_cell_by_out_point`, …). Extra funding inputs return
  `UnsupportedFundingLock` (38). The Process state pays the fee.

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
| 38 | `UnsupportedFundingLock` (extra funding input) |
| 39 | `DepositBelowMinimum` (CKB Intention capacity &lt; Process `MIN_DEPOSIT_CAPACITY`) |

## Intention filter (`Process.filter`)

SSRI method for the Sequencer to pre-select Intentions before assemble:

1. Sequencer routes with `cid_intention_filter::intention_process_type_id`.
2. Calls `Process.filter` (host: `process_filter`) for that Process Type ID.
3. Filter **requires** ownership: CID Type-ID lock + `process_type_hash` equals
   this Process (same helper / `ours_prefix` semantics), then applies deposit /
   asset / digest rules from `cid-process-example-policy`.
4. Assemble only the accepted cells.

Reason codes and wire format: [ssri-backend-interface.md](ssri-backend-interface.md).

## CKB deposit minimum (Intention consume)

CKB Intention creation uses `max(requested, occupied + 1 shannon)` because the
CID lock requires spendable capacity. xUDT creation uses
`max(requested, occupied)`. Each Process owns its deposit minima.

This example Process treats DigestOk **CKB** Intentions **for its own**
`process_type_hash` as deposits fulfilled by consuming the Intention into the
beneficiary, and rejects capacities below its Process-local minimum:

| Constant | Value | Where |
|----------|-------|--------|
| `MIN_DEPOSIT_CAPACITY` | 600 CKB | `crates/cid-process-example-policy` (shared by on-chain contract + calculator/CLI) |

Enforced in Process lock Verify (own DigestOk only), `Process.assemble`, and the
calculator draft path. xUDT Intentions are not subject to this CKB deposit
minimum (token amount rules are separate).

### Upgrading an existing Process deployment

Replacing the example Process binary at the same code Type ID applies the new
minimum to existing Intention cells that target its live Process state. Pause
new deposits and check for live CKB Intentions below 600 CKB for every Process
instance using that code Type ID. Settle them using the previous assembler
and deployed binary, or refund them before replacing the code cell. After the
upgrade, any remaining sub-minimum Intention fails DigestOk with error 39. Use
the CID refund path without a matching Process pair to return capacity to its
User Lock.

## Capacity / fee (example assemble)

`Process.assemble` computes fee as `FeeRate(FEE_RATE).fee(serialized_size_in_block)`
— i.e. `floor((molecule_len + 4) × FEE_RATE / 1000)` with Process-owned
`FEE_RATE = 1000` (testnet `min_fee_rate`) — and deducts it from the recreated
state cell. The `+ 4` matches CKB `serialized_size_in_block` (molecule
`NUMBER_SIZE` in the block), so the fee is never below the node min fee. If the
result would be below occupied capacity, assemble errors. Draft must include
CID, Process, and applicable xUDT cell_deps; the example copies them through.
Submit returned molecule bytes unchanged.
