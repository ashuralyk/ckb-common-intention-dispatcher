# Task 4 Report: ProcessExecutor Integration Tests

## Overview
Task 4 wires the `ProcessExecutor` public API to the real Process contract binary and adds integration tests verifying batch bounds and count validation with the real SSRI implementation.

## Git Context
- **Base SHA**: `52db65d9c9bf45b5da508131d2e063054f236b9c` (Task 3 approved)
- **Head SHA**: `a6a3274a0b1754fd3c9e6f5e298d49e9362b0d39` (Task 4 complete)
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

## Implementation Summary

### 1. ProcessExecutor::assemble() Implementation
**File**: `crates/cid-process-executor/src/lib.rs`

Implemented the `assemble()` method per Task 4 scope:

```rust
pub fn assemble(&self, intentions: &[IntentionInput]) -> Result<PartialTx, ExecutorError> {
    if self.binary.is_empty() {
        return Err(ExecutorError::VmError("empty binary".into()));
    }
    
    // Encode input: count (u32 LE) + intention data
    // For Task 4: only send count; full encoding in Task 5
    let mut input = Vec::new();
    input.extend_from_slice(&(intentions.len() as u32).to_le_bytes());
    
    // Call SSRI Process.assemble
    let _response = vm::call_ssri_method(&self.binary, "Process.assemble", &input)?;
    
    // Parse response into PartialTx
    // Stub for Task 4: full parsing in Task 5+
    Ok(PartialTx {
        outputs: vec![],
        outputs_data: vec![],
    })
}
```

**Key points**:
- Encodes `intentions.len()` as u32 little-endian bytes (only count for Task 4)
- Calls `vm::call_ssri_method()` with method path `"Process.assemble"`
- Returns stub `PartialTx` with empty outputs (full parsing deferred to Task 5)
- Propagates `Incompatible`/`VmError` via `?` operator

### 2. Integration Tests
**File**: `crates/cid-process-executor/src/lib.rs`

#### test_real_binary_batch_bounds
Tests `get_batch_bounds()` with the real Process binary:
```rust
#[test]
fn test_real_binary_batch_bounds() {
    let bin_path = "../../build/release/cid_process_example";
    if !std::path::Path::new(bin_path).exists() {
        eprintln!("Skipping test: binary not built");
        return;
    }
    
    let binary = std::fs::read(bin_path).expect("read binary");
    let executor = ProcessExecutor::new(binary);
    
    let result = executor.get_batch_bounds();
    assert!(result.is_ok(), "should query batch bounds: {:?}", result);
    
    let (min, max) = result.unwrap();
    assert_eq!(min, 1, "BATCH_MIN should be 1");
    assert_eq!(max, 4, "BATCH_MAX should be 4");
}
```

**Verifies**:
- Binary can be loaded from `build/release/cid_process_example`
- `get_batch_bounds()` successfully calls SSRI `Process.batch`
- Returns expected bounds: `(1, 4)` per global constraints

#### test_assemble_validates_count
Tests count validation in `assemble()` method:
```rust
#[test]
fn test_assemble_validates_count() {
    let bin_path = "../../build/release/cid_process_example";
    if !std::path::Path::new(bin_path).exists() {
        eprintln!("Skipping test: binary not built");
        return;
    }
    
    let binary = std::fs::read(bin_path).expect("read binary");
    let executor = ProcessExecutor::new(binary);
    
    // Empty intentions (< BATCH_MIN)
    let result = executor.assemble(&[]);
    assert!(
        matches!(result, Err(ExecutorError::Incompatible(21))),
        "should reject empty batch: {:?}",
        result
    );
    
    // Count=5 (> BATCH_MAX)
    let intentions = vec![
        IntentionInput::default(),
        IntentionInput::default(),
        IntentionInput::default(),
        IntentionInput::default(),
        IntentionInput::default(),
    ];
    let result = executor.assemble(&intentions);
    assert!(
        matches!(result, Err(ExecutorError::Incompatible(21))),
        "should reject count=5: {:?}",
        result
    );
    
    // Count=1 (valid, at BATCH_MIN)
    let intentions = vec![IntentionInput::default()];
    let result = executor.assemble(&intentions);
    assert!(
        result.is_ok(),
        "should accept count=1: {:?}",
        result
    );
}
```

**Verifies**:
- Empty intentions (count=0, below BATCH_MIN=1) → `Incompatible(21)`
- Count=5 (above BATCH_MAX=4) → `Incompatible(21)` 
- Count=1 (valid, at BATCH_MIN) → `Ok` with stub PartialTx

### 3. Supporting Changes
**File**: `crates/cid-process-executor/src/types.rs`

Added `Default` derive to `IntentionInput`:
```rust
#[derive(Clone, Debug, Default)]
pub struct IntentionInput {
    pub out_point: OutPoint,
    pub lock_args: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
}
```

This enables test code to use `IntentionInput::default()` for creating test fixtures.

## Test Results

```
$ cd /agent/repos/ckb-common-intention-dispatcher
$ cargo test -p cid-process-executor

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.06s
     Running unittests src/lib.rs (target/debug/deps/cid_process_executor-650d54b15f85bce8)

running 12 tests
test tests::test_custom_error_start_threshold ... ok
test tests::test_empty_binary_returns_error ... ok
test tests::test_incompatible_error_display ... ok
test tests::test_incompatible_error_type ... ok
test tests::test_integration_exit21_incompatible ... ignored
test tests::test_integration_exit0_success ... ok
test tests::test_integration_stdout_capture ... ok
test tests::test_syscall_2041_returns_ssri_version ... ok
test tests::test_parse_batch_bounds ... ok
test tests::test_vm_error_on_invalid_binary ... ok
test tests::test_real_binary_batch_bounds ... ok
test tests::test_assemble_validates_count ... ok

test result: ok. 11 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests cid_process_executor

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### Test Summary
- **Total tests**: 12
- **Passed**: 11
- **Failed**: 0
- **Ignored**: 1 (`test_integration_exit21_incompatible` - approved to remain ignored per Task 2 review)

### Key Task 4 Tests
Both required integration tests **passed**:
1. ✅ `test_real_binary_batch_bounds` - verified `get_batch_bounds()` returns `(1, 4)`
2. ✅ `test_assemble_validates_count` - verified all count validation cases

## Binary Build
The Process contract binary was built successfully:
```
$ cd contracts/cid_process_example && make build
Finished `release` profile [optimized] target(s) in 0.06s
Copying binary cid_process_example to build directory

$ ls -lh build/release/cid_process_example
-rwxr-xr-x 1 ubuntu ubuntu 47K Nov 5 03:48 build/release/cid_process_example
```

Binary size: **47 KB** (consistent with Task 3)

## Architecture Notes

### SSRI Flow in assemble()
1. **Input encoding**: Convert `intentions.len()` to u32 LE bytes
2. **VM invocation**: Call `vm::call_ssri_method()` with:
   - Binary: Process contract RISC-V binary
   - Method: `"Process.assemble"`
   - Input: Count bytes
   - Argv[0]: Hex-encoded blake2b method ID
3. **Contract execution**:
   - SSRI router decodes argv[0] to match method
   - Handler reads count from stdin via `syscalls::read()`
   - Validates `BATCH_MIN <= count <= BATCH_MAX`
   - Returns `Ok(stub)` or `Err(ProcessError::IncompatibleBatch)` (exit 21)
4. **Error propagation**:
   - Exit 0 → `Ok(output)`
   - Exit 1-19 → `Err(VmError(...))`
   - Exit >=20 → `Err(Incompatible(code))`
5. **Response parsing**: Stub for Task 4, full PartialTx parsing in Task 5

### Test Coverage Expansion
Task 4 adds end-to-end integration coverage:
- Task 1-2 fixture tests: VM mechanics with minimal hand-crafted binaries
- Task 3 smoke tests: Basic SSRI method invocation (removed in Task 4)
- **Task 4 integration tests**: Full stack with real Cinnabar contract binary

The real binary provides comprehensive behavioral coverage including:
- Cinnabar entry/exit handling
- ckb-ssri-std SSRI routing
- ckb-std syscall usage
- Custom error codes (ProcessError)

## Scope Compliance
Task 4 scope per user brief:
1. ✅ `get_batch_bounds()` works with real binary
2. ✅ `assemble()` implemented with count encoding and SSRI call
3. ✅ Returns stub PartialTx (empty outputs)
4. ✅ Propagates Incompatible/VmError
5. ✅ Test: `test_real_binary_batch_bounds` → `(1, 4)`
6. ✅ Test: `test_assemble_validates_count` with empty/count=5/count=1
7. ✅ Fixture tests preserved (exit0, stdout_hello, exit21 ignored)

**Out of scope** (deferred to Task 5):
- Full intention payload encoding (OutPoint, lock_args, capacity, data)
- PartialTx parsing from SSRI response
- Real output generation in contract

## Issues Encountered
1. **Missing Default on IntentionInput**: Tests used `IntentionInput::default()` but the type didn't implement Default
   - **Fix**: Added `#[derive(Default)]` to IntentionInput in types.rs
   - Compilation succeeded (OutPoint from ckb-types supports Default)

## Next Steps (Task 5)
Per the plan:
1. Implement full intention payload encoding in `assemble()`:
   - Serialize OutPoint, lock_args, capacity, data for each intention
   - Send full intention data to Process.assemble
2. Implement PartialTx parsing from SSRI response:
   - Decode output cells (capacity, lock, type, data)
   - Populate PartialTx outputs and outputs_data
3. Update Process contract SSRI handler:
   - Read and parse full intention data
   - Generate real output cells
   - Return serialized PartialTx

## Conclusion
Task 4 successfully wires ProcessExecutor to the real Process binary and adds integration tests. All tests pass, demonstrating:
- Batch bounds query via SSRI
- Count validation with proper Incompatible(21) error propagation
- Valid count handling (stub PartialTx return)

The implementation is ready for Task 5 full payload encoding.
