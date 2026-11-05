# Task 8 Report: Test Infrastructure Setup

## Overview
Task 8 adds Process example test infrastructure to the existing `tests/` crate. Helpers and constants only — no digest / FakeRpc e2e cases (Task 9+).

## Git Context
- **Base SHA**: `f5bf15aaffb1dcd4ec52646e2dcacc72b1af60ed` (Task 7 approved)
- **Head SHA**: `b4f64b48189bf8f89f875e3104c78126096fc37e` (code; docs SHA follows)
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

## Implementation

### 1. `tests/Cargo.toml`
Added the Process calculator dependency (package name from `process_calculator/Cargo.toml`):

```toml
cid_process_example-calculator = { path = "../process_calculator" }
```

Existing CID calculator, FakeRpc, tokio, and script deps are unchanged.

### 2. `tests/src/process_example.rs`
Helpers only, adapted to this crate’s FakeRpc layout (`CONTRACT_BIN = "../build/release"`, same `Script` / `blake2b_256` / `Data1` pattern as `faker.rs`):

- `PROCESS_CONTRACT_NAME` — re-exported from `cid_process_example_calculator`
- `CID_CONTRACT_NAME` — `ckb_common_intention_dispatcher`
- `CONTRACT_BIN` — `../build/release`
- `MAX_CYCLES` — `50_000_000` (for Task 9+ simulator)
- `require_binaries()` — asserts CID + Process binaries exist
- `process_type_script()` — Data1 lock on the Process binary hash, args `process-type-id-v0`

No Task 9 digest tests. `#[allow(dead_code)]` on helpers that Task 9 will call.

### 3. `tests/src/lib.rs`
Exported as `mod process_example;` next to `faker` / `tests` / `signed` (this crate is `#![cfg(test)]` and uses private mods, not `pub mod`).

## Verification

```
$ cd tests && cargo check
    Checking cid_process_example-calculator v0.1.0
    Checking ckb_common_intention_dispatcher-tests v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.31s
```

## Scope
In scope: calculator dep, helper module, export, `cargo check`.  
Out of scope: Task 9 single/multi digest tests, Task 10 negatives, contract / assemble changes.

## Carry-forward Minors (untouched)
No Process recreate on PartialTx; placeholder executor OutPoint; empty `instructions`; soft-skip / require-binaries patterns left as-is.

## Conclusion
Test infrastructure is in place for Task 9. `cd tests && cargo check` is green.

**Status**: READY FOR REVIEW
