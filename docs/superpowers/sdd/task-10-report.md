# Task 10 Report: Negative Batch Boundary Tests

## Overview
Task 10 adds empty-batch and n=5>BATCH_MAX rejection tests, plus two `#[ignore]` placeholders for on-chain validation that is not wired yet.

## Git Context
- **Base SHA**: `e84fea530a2e384e7e2c00688ef2b34ba14dae3d` (Task 9 approved)
- **Head SHA**: `b33766bf5ae170b920f3ec0dc2ae70cd37568aaf`
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

## Implementation

**File**: `tests/src/process_example.rs`

### Active negatives
- `reject_empty_batch_real_ssri` — `process_digest_ok(..., vec![])` → `CalculatorError::Other` containing `"out of bounds"`
- `reject_count_exceeds_max_real_ssri` — n=5 via `mock_intention` / `fake_outpoint()` → same error shape

Calculator path: `get_batch_bounds()` then `count < min || count > max` → `Other("intention count {} out of bounds [{}, {}]")`.

### Ignored placeholders
- `reject_wrong_beneficiary` — TODO beneficiary mismatch / ERROR_BENEFICIARY_MISSING
- `reject_missing_type_id_pair` — TODO missing Process recreate / ERROR_PROCESS_PAIR_INVALID

## Test output

```
$ cd tests && cargo test process_example -- --nocapture

running 7 tests
Empty batch rejected: SUCCESS
test process_example::reject_empty_batch_real_ssri ... ok
test process_example::reject_missing_type_id_pair ... ignored
test process_example::reject_wrong_beneficiary ... ignored
Count > BATCH_MAX rejected: SUCCESS
test process_example::reject_count_exceeds_max_real_ssri ... ok
digest n=2 ... ok
digest n=1 ... ok
digest n=4 ... ok

test result: ok. 5 passed; 0 failed; 2 ignored
```

## Scope
In: Task 10 negatives + ignore placeholders. Out: Task 11 docs/build, on-chain FakeRpc verify.

**Status**: READY FOR REVIEW
