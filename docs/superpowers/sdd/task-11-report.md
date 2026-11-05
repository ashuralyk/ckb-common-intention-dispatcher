# Task 11 Report: Build Script and Usage Docs

## Overview
Task 11 adds `scripts/build-all-contracts.sh`, `docs/process-example-usage.md`, and a README Process Example section. Docs describe what is implemented (molecule bridge, FakeRpc tests, BATCH 1–4, deferred Process recreate).

## Git Context
- **Base SHA**: `fb21a31587fb4be21682c340a6872e817104602a` (Task 10 approved)
- **Head SHA**: `0f5a770c7b4a6892d77030ad806202baca314a8a`
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

## Files

### `scripts/build-all-contracts.sh` (executable)
- Resolves repo root from the script path
- Exports `TOP` + `BUILD_DIR=build/release` so CID copies artifacts (its Makefile leaves `BUILD_DIR` empty otherwise)
- `make -e -C` CID then Process
- `ls -lh build/release/`

Verified:

```
./scripts/build-all-contracts.sh
All contracts built successfully:
  cid_process_example  73K
  cid_process_fixture  20K
  ckb_common_intention_dispatcher  83K
```

### `docs/process-example-usage.md`
Accurate to current code: SSRI methods, executor API, calculator molecule bridge, batch 1–4, on-chain errors 30–32, deferred recreate / placeholder lock / ignored FakeRpc verify tests.

### `README.md`
- Qualifies the old “Process out of scope” line
- Layout rows for Process contract, executor, calculator
- **Process Example** section + link + build/test commands

## Scope
In: script + docs + README. Out: Task 12 full workspace verification marathon.

**Status**: READY FOR REVIEW
