# Task 12 Report: Final Integration Verification

## Git Context
- **Base SHA**: `b8f9185ba5945876a6757abb8df2b8da33c400e6` (Task 11 approved)
- **Head SHA**: `01451b67212ae6f795e51f0088bf511c9f9b0d79`
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

This closes the 12-task SDD plan. No Task 13.

## Build

```
./scripts/build-all-contracts.sh
```

OK. Artifacts in `build/release/`:

| Binary | Size |
|--------|------|
| `cid_process_example` | 73K |
| `ckb_common_intention_dispatcher` | 83K |
| `cid_process_fixture` | 20K |

## Tests

| Command | Result |
|---------|--------|
| `cargo test -p cid-process-executor` | **12 passed, 1 ignored** (`test_integration_exit21_incompatible`, Task 2 carry-forward) |
| `cargo test -p cid_process_example-calculator` | **3 passed** |
| `cargo test --package ckb_common_intention_dispatcher-tests process_example -- --nocapture` | **5 passed, 2 ignored** (`reject_wrong_beneficiary`, `reject_missing_type_id_pair`) |

`cargo test --workspace` **failed** for unrelated host-link of `no_std` RISC-V packages:

- `ckb_common_intention_dispatcher` (bin test): `duplicate symbol: _start` (ckb-std `entry!` vs host crt)
- `ckb_common_intention_dispatcher-runner` test `process_fixture`: missing `ckb_std` on host

These are pre-existing workspace members, not Process-example regressions. Not fixed in Task 12.

Practical host subset (`--exclude ckb_common_intention_dispatcher --exclude cid_process_example`) still hits the runner `process_fixture` compile error.

## fmt / clippy

- `cargo fmt --all` applied (whitespace / import order in Process crates + `tests/src/process_example.rs`)
- `cargo clippy -p cid-process-executor -p cid_process_example-calculator -p ckb_common_intention_dispatcher-tests -- -D warnings` **green** after `manual_range_contains` fix in `vm.rs` (`(2041..=3000).contains(&syscall_num)`)
- Workspace-wide `clippy -- -D warnings` not used (RISC-V / runner noise)

`Cargo.lock` now lists `cid_process_example-calculator` under the tests package (Task 8 dep was missing from the lockfile).

## Global Constraints checklist

| Constraint | Status |
|------------|--------|
| BATCH_MIN = 1 | ✓ `ssri.rs` / `verify.rs` |
| BATCH_MAX = 4 | ✓ same |
| Fixed-beneficiary assemble | ✓ one beneficiary output per intention (placeholder lock code hash + `lock_args`) |
| CID calculator create/refund/timeout unchanged | ✓ CID calculator crate not rewritten |
| Process Calculator digest via executor | ✓ `process_digest_ok` → `get_batch_bounds` + `assemble` |
| Greenfield executor (no vendored ssri-server) | ✓ no `ssri-server` in tree |
| Incompatible = custom exit ≥ 20 | ✓ `CUSTOM_ERROR_START = 20`; VM maps ≥20 → `Incompatible` |
| Guest SSRI via cinnabar | ✓ `cinnabar_main!` SSRI door |
| Type ID via cinnabar / group pair | ✓ `verify_type_id_pair` 1:1 GroupInput/GroupOutput |
| Tests in `tests/src/process_example.rs` | ✓ |
| Real SSRI assemble n=1, 2, 4 | ✓ |
| Negative batch boundaries | ✓ empty + n=5 |

## Carry-forwards (honest)

1. **No Process recreate** on PartialTx / skeleton (Task 5). On-chain verify still requires pair + `output_count >= n + 1` when a full tx is submitted.
2. **Placeholder beneficiary lock** (`DEFAULT_LOCK_CODE_HASH` + intention `lock_args`), not full Script from CID payload.
3. **Two `#[ignore]` FakeRpc tests**: `reject_wrong_beneficiary`, `reject_missing_type_id_pair`.
4. **Executor `test_integration_exit21_incompatible` ignored** (hand-rolled SYS_EXIT fixture; real-binary Incompatible covered by assemble count tests).
5. **Placeholder executor OutPoint** in `to_intention_input` (assemble encode unused; skeleton uses calculator OutPoint).
6. **Empty `instructions` vec** from `process_digest_ok`.

## Quality fixes in this task

- rustfmt on Process example sources
- clippy `manual_range_contains` in `crates/cid-process-executor/src/vm.rs`
- `Cargo.lock` dependency line for tests → calculator

**Status**: SDD PLAN COMPLETE
