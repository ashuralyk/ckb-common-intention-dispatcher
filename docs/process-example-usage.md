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
- **Process.assemble** — stdin = molecule `Transaction` draft (Intention +
  Process inputs and required code `cell_deps`); stdout = complete molecule
  `Transaction` (inputs, outputs, outputs_data, cell_deps, header_deps,
  witnesses). Cells are loaded via ckb-ssri-std host queries
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

**Lock args** (`ProcessLockArgs`):

```text
cid_code_hash[32] || cid_hash_type[1] || instance_salt[0..]
```

- `cid_code_hash` / `cid_hash_type`: exact Intention lock identity this Process
  authenticates on-chain (live CID code Type ID script hash + `hash_type=type`
  in normal deployments).
- `instance_salt`: optional trailing bytes so multiple state cells that share
  Process code + CID identity still have distinct locks (identical locks merge
  into one group → N→N / `ProcessPairInvalid`).

Recreate must keep lock args byte-identical (CID identity must not change).
Malformed args → `ProcessLockArgsInvalid` (41). An Intention targeting this
Process TYPE_ID whose lock `code_hash`/`hash_type` differ → `CidLockMismatch`
(40).

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
| 40 | `CidLockMismatch` (Intention lock ≠ CID identity in Process lock args) |
| 41 | `ProcessLockArgsInvalid` (malformed Process state lock args) |

## Intention filter (`Process.filter`)

SSRI method for the Sequencer to pre-select Intentions before assemble:

1. Sequencer **routes** with `cid_intention_filter::intention_process_type_id`
   / `intention_belongs_to_process` (ownership is Sequencer-only).
2. Calls `Process.filter` (host: `process_filter`) on the routed batch.
3. Filter judges **business compliance only** (deposit / asset / digest) from
   `cid-process-example-policy` — it does **not** re-check ownership.
4. Assemble only the accepted cells.

Reason codes and wire format: [ssri-backend-interface.md](ssri-backend-interface.md).

## CKB deposit minimum (Intention consume)

Intention creates size capacity to `max(requested, occupied)`. Protocol / CID
impose **no** Intention or deposit capacity floor — each Process owns its own
minima (if any).

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

**This release changes Process lock args** (CID identity embedded) and the
Process binary.

**Do not replace the Process code cell in place at the same code Type ID.** New
Verify rejects legacy state cells whose lock args are not `ProcessLockArgs`
(e.g. opaque `0x01`), while recreate requires byte-identical lock scripts —
so an in-place swap **permanently strands** those legacy state cells.

Recommended cutover (parallel deployment):

1. Deploy a **new** Process **code** cell under a **new** code Type ID (new
   binary with this Verify).
2. Create **new** Process **state** cells locked to that code Type ID with
   `ProcessLockArgs` = `cid_code_hash[32]||cid_hash_type[1]||salt` matching the
   live CID Intention lock (typically Type ID script hash + `hash_type=type`).
3. Point new Intentions / registry entries at the new state cell TYPE_IDs.
4. Keep the **old** Process code cell live until legacy state cells are drained
   (settle / refund with the previous assembler + binary) or **explicitly
   abandoned** (Lucas’s direct cutover: leave them unspendable and stop
   routing to them). Consensus does not include a legacy-args migration path.

Deposit-minimum note: once Intentions target a Process instance on the new
code Type ID, the 600 CKB example floor applies. Pause new deposits on the old
instance and settle or refund sub-minimum Intentions with the previous
assembler before abandoning that instance. After cutover, remaining
sub-minimum Intentions on the new code fail DigestOk with error 39.

## Capacity / fee (example assemble)

`Process.assemble` computes fee as `FeeRate(FEE_RATE).fee(serialized_size_in_block)`
— i.e. `floor((molecule_len + 4) × FEE_RATE / 1000)` with Process-owned
`FEE_RATE = 1000` (testnet `min_fee_rate`, shared via
`cid-process-example-policy`) — and deducts it from the recreated state cell.
The `+ 4` matches CKB `serialized_size_in_block` (molecule `NUMBER_SIZE` in the
block), so the fee is never below the node min fee. If the result would be
below occupied capacity, assemble errors. On-chain Transfer verify (1) caps
each Process state-cell debit by that fee formula and (2) requires
`tx_fee = Σ(input caps) − Σ(output caps)` to cover the **sum of per-lock-group
positive debits** across Process locks sharing this binary’s
`code_hash`/`hash_type` (group = full lock script; a credit on Process B does
**not** offset a debit on Process A — rejects diverted drains with
`CapacityInsufficient` / 37). Draft must include CID, Process, and applicable
xUDT cell_deps; the example copies them through. Submit returned molecule bytes
unchanged.
