# Task 9 Report: Single / Multi Intention FakeRpc SSRI Tests

## Overview
Task 9 adds n=1, n=2, and n=4 digest tests that call `process_digest_ok` against the real Process binary via `FakeRpcClient`.

## Git Context
- **Base SHA**: `33077fb824b63578f4c5d546b898ba1a9fa48d36` (Task 8 approved)
- **Head SHA**: `a8834b95210c1158e3a599a5309a7058e381e7c8` (code; docs SHA follows)
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

## Implementation

**File**: `tests/src/process_example.rs`

Shared helper `assert_digest_counts(n)`:
1. `require_binaries()` — CID + Process binaries must exist
2. Build `n` `IntentionCell`s with **distinct** `fake_outpoint()` values
3. `process_digest_ok(&FakeRpcClient::default(), intentions, CONTRACT_BIN)`
4. Assert `skeleton.inputs.len() == n` and `skeleton.outputs.len() == n`

Does **not** use `skeleton.operations` (that field does not exist).

Tests (tokio, matching `tests/` style):
- `single_intention_digest_real_ssri` — n=1
- `multi_intention_digest_n2_real_ssri` — n=2
- `multi_intention_digest_n4_real_ssri` — n=4 (BATCH_MAX)

## Binaries
```
make build
build/release/cid_process_example
build/release/ckb_common_intention_dispatcher
```

## Test output

```
$ cd tests && cargo test process_example -- --nocapture

running 3 tests
digest n=1 with real SSRI: SUCCESS (inputs=1, outputs=1)
test process_example::single_intention_digest_real_ssri ... ok
digest n=2 with real SSRI: SUCCESS (inputs=2, outputs=2)
test process_example::multi_intention_digest_n2_real_ssri ... ok
digest n=4 with real SSRI: SUCCESS (inputs=4, outputs=4)
test process_example::multi_intention_digest_n4_real_ssri ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 28 filtered out
```

## Adaptations vs plan stubs
- Distinct OutPoints via `fake_outpoint()` (duplicate defaults → `input already exists`)
- Assert `inputs` / `outputs` lengths, not `skeleton.operations`
- Panic on `CalculatorError` via `Display` (Instruction is not Debug)

## Carry-forward Minors (untouched)
No Process recreate on skeleton; placeholder executor OutPoint; empty instructions vec.

## Scope
In: Task 9 digest tests. Out: Task 10 negatives, Task 11 docs/build, contract changes.

**Status**: READY FOR REVIEW
