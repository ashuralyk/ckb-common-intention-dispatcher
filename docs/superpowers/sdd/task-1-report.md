# Task 1 Report: Workspace Scaffolding and Executor Crate Foundation

**Date**: 2026-11-05  
**Task**: Task 1 of CID Process Example (Approach A)  
**Status**: ✅ DONE

## Implementation Summary

Successfully scaffolded the workspace structure and created foundation crates for the CID Process example following Approach A (vertical slice: sample Process + SSRI offline executor + Process Calculator).

### Changes Implemented

#### 1. Workspace Configuration (`Cargo.toml`)
- **Added workspace dependency**: `ckb-ssri-std` (git rev `8c2c245`)
- **Added workspace members**:
  - `crates/cid-process-executor` - Greenfield offline SSRI host
  - `process_calculator` - Process Calculator for CID Process example

#### 2. Executor Crate (`crates/cid-process-executor/`)

**Files Created**:
- `Cargo.toml` - Dependencies: ckb-vm, ckb-types, ckb-ssri-std, anyhow
- `src/types.rs` - Core type definitions
- `src/vm.rs` - VM stub (Task 2)
- `src/lib.rs` - Public API and tests

**Type Definitions** (`src/types.rs`):
- `IntentionInput` - Input intention cell with out_point, lock_args, capacity, data
- `PartialTx` - Partial transaction with outputs and outputs_data
- `ExecutorError` - Error enum:
  - `Incompatible(i8)` - Custom exit code >= 20 (NOT SSRIError per constraint)
  - `VmError(String)` - VM execution failures
  - `InvalidResponse(String)` - SSRI response validation errors
- `CUSTOM_ERROR_START` - Constant = 20 (domain error threshold)

**Public API** (`src/lib.rs`):
- `ProcessExecutor::new(binary: Vec<u8>) -> Self` - Load Process binary
- `ProcessExecutor::get_batch_bounds(&self) -> Result<(u32, u32), ExecutorError>` - Query batch min/max via SSRI Process.batch
- `ProcessExecutor::assemble(&self, _intentions: &[IntentionInput]) -> Result<PartialTx, ExecutorError>` - Stub (returns VmError, will implement in Task 2)
- `parse_batch_bounds(response: &[u8]) -> Result<(u32, u32), ExecutorError>` - Helper to parse 8-byte little-endian response

**VM Stub** (`src/vm.rs`):
- `call_ssri_method(_binary: &[u8], _method: &str, _args: &[u8]) -> Result<Vec<u8>, ExecutorError>` - Stub returns VmError (Task 2 will implement greenfield SSRI host with CKB-VM)

**Tests** (`src/lib.rs`):
- `test_parse_batch_bounds()` - Validates parsing of (1, 4) from 8-byte LE buffer
- `test_empty_binary_returns_error()` - Validates VmError on empty binary

#### 3. Process Calculator Crate (`process_calculator/`)

**Files Created**:
- `Cargo.toml` - Dependencies: ckb-cinnabar-calculator, cid-protocol, cid-process-executor
- `src/lib.rs` - Binary loader utility and test

**API**:
- `PROCESS_CONTRACT_NAME` - Constant = "cid_process_example"
- `load_process_binary(bin_dir: &str) -> Result<Vec<u8>>` - Loads Process binary from build/release/<name>

**Tests**:
- `test_load_binary_missing()` - Validates error on missing binary path

## Test Results

### Workspace Build
```bash
cd /agent/repos/ckb-common-intention-dispatcher
cargo check --workspace
```
**Result**: ✅ PASS (12.43s)  
All workspace members compile successfully.

### Executor Tests
```bash
cd crates/cid-process-executor
cargo test
```
**Result**: ✅ 2/2 passed (9.60s)
- `test_parse_batch_bounds` - ✅ ok
- `test_empty_binary_returns_error` - ✅ ok

### Process Calculator Tests
```bash
cd process_calculator
cargo test
```
**Result**: ✅ 1/1 passed (25.29s)
- `test_load_binary_missing` - ✅ ok

## TDD Evidence

Yes, TDD approach was followed for core parsing logic:

1. **Test-first**: Created `test_parse_batch_bounds()` before implementing parser
2. **Implementation**: Wrote `parse_batch_bounds()` to satisfy test (8-byte LE u32 pair)
3. **Validation**: Test passes, validates (1, 4) batch bounds per global constraint

Error handling tests were written alongside implementation but follow fail-fast pattern (empty binary → VmError).

## Files Changed

**Total**: 7 files created, 1 file modified

### Modified
- `Cargo.toml` - Added workspace dependencies and members

### Created
- `crates/cid-process-executor/Cargo.toml`
- `crates/cid-process-executor/src/lib.rs`
- `crates/cid-process-executor/src/types.rs`
- `crates/cid-process-executor/src/vm.rs`
- `process_calculator/Cargo.toml`
- `process_calculator/src/lib.rs`

## Commit Details

**Branch**: `cursor/cid-process-example-c952`  
**Base SHA**: `7f603250168273b8d646641de4fcf0e02feb9e3f`  
**Head SHA**: `4faae1a3c1986ebb7bdaa546f788d61c65c6d839`

**Commit**:
```
4faae1a feat: add executor crate and Process calculator scaffolding
```

## Global Constraints Verified

✅ **BATCH_MIN = 1, BATCH_MAX = 4** - Documented in test (1, 4)  
✅ **Executor greenfield** - No vendored ssri-server, fresh VM stub created  
✅ **Incompatible = custom exit >= 20** - `ExecutorError::Incompatible(i8)` with `CUSTOM_ERROR_START = 20`, NOT SSRIError  
✅ **Paths correct**:
  - `crates/cid-process-executor` ✓
  - `process_calculator` with package `cid_process_example-calculator` ✓
✅ **CID calculator unchanged** - No modifications to existing calculator

## Concerns

None. All steps completed successfully with passing tests.

## Next Steps

- **Task 2**: Implement real SSRI VM executor in `vm.rs`
  - CKB-VM instantiation with argv-based SSRI method invocation
  - vm_version = u64::MAX convention for SSRI mode
  - Complete `call_ssri_method()` implementation
  - Complete `ProcessExecutor::assemble()` implementation with intention serialization

- **Task 3**: Implement Process contract
  - SSRI interface implementation
  - Process.batch handler (return min=1, max=4)
  - Process.assemble handler (digest transaction assembly)
  - Build RISC-V binary

- **Integration**: Wire Process Calculator into CID dispatcher
  - Keep create/refund/timeout paths unchanged per constraint

## PR Information

**PR URL**: https://github.com/ashuralyk/ckb-common-intention-dispatcher/pull/6  
**Status**: Draft  
**Title**: feat: CID Process example (Approach A) - Task 1

---

**Report generated**: 2026-11-05  
**Task duration**: ~10 minutes  
**Methodology**: Subagent-Driven Development (Task 1 only)
