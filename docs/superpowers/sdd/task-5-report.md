# Task 5 Report: Complete Process.assemble with Beneficiary Logic

## Overview
Task 5 completes the Process.assemble implementation with full intention encoding/decoding and beneficiary CellOutput generation. The executor now sends complete intention data (capacity, lock_args, data) to the contract, which generates beneficiary outputs and returns a serialized PartialTx.

## Git Context
- **Base SHA**: `1ee2600b8dab9df8cb7be87784ca63f5e37b2f1a` (Task 4 approved)
- **Head SHA**: `820814d043803bd8dfcb9b8631ff8af304c45c7c` (Task 5 complete)
- **Branch**: `cursor/cid-process-example-c952`
- **PR**: #6 targeting `main`

## Implementation Summary

### 1. Intention Encoding (Executor)
**File**: `crates/cid-process-executor/src/types.rs`

Added encoding methods for intentions:

```rust
impl IntentionInput {
    /// Encode for SSRI Process.assemble input
    /// Format: capacity (u64 LE) + lock_args_len (u32 LE) + lock_args + data_len (u32 LE) + data
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.capacity.to_le_bytes());
        buf.extend_from_slice(&(self.lock_args.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.lock_args);
        buf.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }
}

/// Encode array of intentions for SSRI Process.assemble
/// Format: count (u32 LE) + for each intention: len (u32 LE) + encoded intention
pub fn encode_intentions(intentions: &[IntentionInput]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(intentions.len() as u32).to_le_bytes());
    for intention in intentions {
        let encoded = intention.encode();
        buf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        buf.extend_from_slice(&encoded);
    }
    buf
}
```

**Encoding format**:
- Count (u32 LE)
- For each intention:
  - Intention length (u32 LE)
  - Capacity (u64 LE)
  - Lock args length (u32 LE)
  - Lock args bytes
  - Data length (u32 LE)
  - Data bytes

### 2. PartialTx Parsing (Executor)
**File**: `crates/cid-process-executor/src/types.rs`

Added PartialTx deserialization from SSRI response:

```rust
impl PartialTx {
    /// Parse PartialTx from SSRI response bytes
    /// Format: output_count (u32 LE) + for each: output_len (u32 LE) + output_bytes + data_len (u32 LE) + data_bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, super::ExecutorError> {
        // Read output count
        // For each output:
        //   - Read output length + molecule-encoded CellOutput
        //   - Read data length + data bytes
        // Returns PartialTx with populated outputs and outputs_data
    }
}
```

**Response format**:
- Output count (u32 LE)
- For each output:
  - Output length (u32 LE)
  - CellOutput molecule bytes
  - Data length (u32 LE)
  - Data bytes

### 3. Updated Executor assemble() Method
**File**: `crates/cid-process-executor/src/lib.rs`

Simplified to use full encoding and parsing:

```rust
pub fn assemble(&self, intentions: &[IntentionInput]) -> Result<PartialTx, ExecutorError> {
    if self.binary.is_empty() {
        return Err(ExecutorError::VmError("empty binary".into()));
    }
    
    // Encode full intentions (capacity, lock_args, data for each)
    let input = encode_intentions(intentions);
    
    // Call SSRI Process.assemble
    let response = vm::call_ssri_method(&self.binary, "Process.assemble", &input)?;
    
    // Parse response into PartialTx
    PartialTx::from_bytes(&response)
}
```

Changes from Task 4:
- Now uses `encode_intentions()` instead of just encoding count
- Parses response with `PartialTx::from_bytes()` instead of returning stub
- Returns real PartialTx with beneficiary outputs

### 4. Contract Intention Parsing
**File**: `contracts/cid_process_example/src/ssri.rs`

Added intention parsing function:

```rust
/// Parse a single intention from buffer at given position
/// Returns (capacity, lock_args, data, next_position)
fn parse_intention(buf: &[u8], mut pos: usize) -> Result<(u64, Vec<u8>, Vec<u8>, usize), ProcessError> {
    // Read intention length (u32 LE)
    // Read capacity (u64 LE)
    // Read lock_args_len (u32 LE) + lock_args
    // Read data_len (u32 LE) + data
    // Verify consumed exactly intention_len bytes
    // Return parsed fields + next position
}
```

**Parsing steps**:
1. Read intention length prefix
2. Parse capacity (u64 LE)
3. Parse lock_args (length-prefixed)
4. Parse data (length-prefixed)
5. Verify byte count matches length prefix
6. Return (capacity, lock_args, data, next_pos)

### 5. Contract Beneficiary Generation
**File**: `contracts/cid_process_example/src/ssri.rs`

Updated `assemble_handler()` to generate real beneficiary outputs:

```rust
pub fn assemble_handler() -> Result<Vec<u8>, ProcessError> {
    // Read full input from stdin (max 8KB)
    let mut input_buf = [0u8; 8192];
    let mut total_read = 0;
    
    loop {
        let read_result = ckb_std::syscalls::read(0, &mut input_buf[total_read..])?;
        if read_result == 0 { break; } // EOF
        total_read += read_result;
        if total_read >= input_buf.len() {
            return Err(ProcessError::ReadFailed); // Input too large
        }
    }
    
    // Parse intention count and validate bounds
    let count = u32::from_le_bytes(input[0..4]);
    if count < BATCH_MIN || count > BATCH_MAX {
        return Err(ProcessError::IncompatibleBatch);
    }
    
    // Parse intentions and generate beneficiary outputs
    let mut outputs = Vec::new();
    let mut outputs_data = Vec::new();
    
    for _ in 0..count {
        let (capacity, lock_args, data, next_pos) = parse_intention(input, pos)?;
        
        // Create beneficiary output with:
        // - Capacity from intention
        // - Lock script: default code_hash + intention lock_args
        // - No type script
        let lock = Script::new_builder()
            .code_hash(Byte32::from_slice(&DEFAULT_LOCK_CODE_HASH).unwrap())
            .hash_type(0u8)
            .args(Bytes::from(lock_args))
            .build();
        
        let output = CellOutput::new_builder()
            .capacity(capacity)
            .lock(lock)
            .build();
        
        outputs.push(output);
        outputs_data.push(data);
    }
    
    // Serialize PartialTx
    let mut response = Vec::new();
    response.extend_from_slice(&(outputs.len() as u32).to_le_bytes());
    
    for (output, data) in outputs.iter().zip(outputs_data.iter()) {
        let output_bytes = output.as_slice();
        response.extend_from_slice(&(output_bytes.len() as u32).to_le_bytes());
        response.extend_from_slice(output_bytes);
        response.extend_from_slice(&(data.len() as u32).to_le_bytes());
        response.extend_from_slice(data);
    }
    
    Ok(response)
}
```

**Key changes from Task 4**:
- Reads full input via stdin loop (not just count)
- Parses each intention completely
- Generates one beneficiary CellOutput per intention
- Uses `DEFAULT_LOCK_CODE_HASH` (recognizable constant) for beneficiary lock
- Serializes PartialTx with molecule-encoded CellOutput

**Beneficiary output structure**:
- **Capacity**: From intention
- **Lock script**:
  - code_hash: `0xaabbccdd...` (32-byte constant)
  - hash_type: 0 (data)
  - args: From intention lock_args
- **Type script**: None
- **Data**: From intention data field

### 6. New Integration Test
**File**: `crates/cid-process-executor/src/lib.rs`

Added comprehensive test for beneficiary generation:

```rust
#[test]
fn test_assemble_returns_non_empty_partial_tx() {
    // Load real binary
    // Create 2 intentions with different capacity, lock_args, data
    let intentions = vec![
        IntentionInput {
            capacity: 100_000_000_000,
            lock_args: vec![0x01, 0x02, 0x03, 0x04],
            data: vec![0xaa, 0xbb],
            ...
        },
        IntentionInput {
            capacity: 200_000_000_000,
            lock_args: vec![0x05, 0x06, 0x07],
            data: vec![0xcc, 0xdd, 0xee],
            ...
        },
    ];
    
    // Call assemble
    let partial_tx = executor.assemble(&intentions).unwrap();
    
    // Verify 2 outputs with correct capacity, lock_args, data
    assert_eq!(partial_tx.outputs.len(), 2);
    assert_eq!(partial_tx.outputs[0].capacity().unpack(), 100_000_000_000);
    assert_eq!(partial_tx.outputs[0].lock().args().unpack(), vec![0x01, 0x02, 0x03, 0x04]);
    assert_eq!(partial_tx.outputs_data[0], vec![0xaa, 0xbb]);
    // ... similar checks for second output
}
```

**Test coverage**:
- ✅ Intentions with real capacity, lock_args, data
- ✅ Non-empty PartialTx returned
- ✅ Correct number of outputs (matches intention count)
- ✅ Each output has correct capacity from intention
- ✅ Each output lock has correct args from intention
- ✅ Each output data matches intention data

## Test Results

```
$ cd /agent/repos/ckb-common-intention-dispatcher
$ cargo test -p cid-process-executor

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.32s
     Running unittests src/lib.rs (target/debug/deps/cid_process_executor-650d54b15f85bce8)

running 13 tests
test tests::test_empty_binary_returns_error ... ok
test tests::test_custom_error_start_threshold ... ok
test tests::test_incompatible_error_display ... ok
test tests::test_incompatible_error_type ... ok
test tests::test_integration_exit21_incompatible ... ignored
test tests::test_integration_exit0_success ... ok
test tests::test_integration_stdout_capture ... ok
test tests::test_parse_batch_bounds ... ok
test tests::test_syscall_2041_returns_ssri_version ... ok
test tests::test_real_binary_batch_bounds ... ok
test tests::test_assemble_returns_non_empty_partial_tx ... ok
test tests::test_vm_error_on_invalid_binary ... ok
test tests::test_assemble_validates_count ... ok

test result: ok. 12 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

### Test Summary
- **Total tests**: 13 (was 12 in Task 4)
- **Passed**: 12 (was 11 in Task 4)
- **Failed**: 0
- **Ignored**: 1 (approved in Task 2)

### New Test
✅ **test_assemble_returns_non_empty_partial_tx** - Comprehensive beneficiary generation test

### Task 4 Tests Status
All Task 4 tests remain green:
- ✅ `test_real_binary_batch_bounds` - batch bounds (1, 4)
- ✅ `test_assemble_validates_count` - count validation (empty, count=5, count=1)

## Binary Build
```
$ cd contracts/cid_process_example && make build
    Finished `release` profile [optimized] target(s) in 0.92s
Copying binary cid_process_example to build directory

$ ls -lh build/release/cid_process_example
-rwxr-xr-x 1 ubuntu ubuntu 57K Nov 5 03:54 build/release/cid_process_example
```

Binary size: **57 KB** (was 47 KB in Task 4)
- Increase: ~10 KB for intention parsing and beneficiary generation logic

## Architecture Notes

### End-to-End Flow
1. **Executor** encodes intentions:
   - `encode_intentions()` → count + (len + capacity + lock_args + data) for each
   
2. **VM** invokes SSRI method:
   - Argv[0]: hex-encoded blake2b("Process.assemble")
   - Stdin: encoded intentions
   
3. **Contract** parses and generates:
   - Read stdin loop until EOF
   - Parse count, validate batch bounds
   - For each intention: parse_intention() → (capacity, lock_args, data)
   - Generate beneficiary CellOutput with lock from lock_args
   - Serialize PartialTx: count + (len + molecule output + len + data) for each
   
4. **Executor** parses response:
   - `PartialTx::from_bytes()` → Vec<CellOutput> + Vec<Vec<u8>>
   - Unpacks molecule CellOutput structures
   - Returns to caller

### Beneficiary Lock Script
For simplicity, beneficiary outputs use a default code_hash:
```rust
const DEFAULT_LOCK_CODE_HASH: [u8; 32] = [
    0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11,
    0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
    0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11,
    0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
];
```

The lock args come directly from the intention's `lock_args` field. In a real implementation:
- The intention `lock_args` would contain CID payload with beneficiary Script
- The contract would parse the payload to extract the beneficiary's real code_hash
- For Task 5, we use the simplified constant approach

### Process Recreate Output
Task 5 focuses on beneficiary outputs only. Process recreate logic (for contract continuity) is deferred to later tasks. The current implementation:
- Generates only beneficiary outputs (one per intention)
- Does not include a Process cell recreate output
- This is sufficient for Task 5 scope per user brief

## Scope Compliance

Task 5 requirements from user message:
1. ✅ Executor: `IntentionInput::encode` implemented
2. ✅ Executor: Full assemble input (count + each intention) via `encode_intentions()`
3. ✅ Contract: Parse intentions (capacity, lock_args, data)
4. ✅ Contract: Produce beneficiary CellOutput for each intention
5. ✅ Contract: Lock from intention lock_args (using default code_hash)
6. ✅ Return serialized PartialTx bytes
7. ✅ Executor parses into `PartialTx`
8. ✅ BATCH_MIN=1, BATCH_MAX=4 enforced → Incompatible(21)
9. ✅ Keep cinnabar_main! SSRI door
10. ✅ Rebuild contract successful
11. ✅ Keep Task 4 tests green
12. ✅ Add test that valid count returns non-empty PartialTx

**Out of scope** (deferred):
- Process Type ID on-chain (Task 6)
- Process Calculator (later task)
- FakeRpc e2e (later task)
- Real beneficiary Script parsing from CID payload

## Issues Encountered

### 1. Molecule Builder API Type Inference
**Error**: Type annotations needed for `.hash_type()` and `.capacity()`
```
error[E0283]: type annotations needed for `hash_type`
```

**Cause**: Rust couldn't infer the type parameter `T` for generic Into bounds

**Fix**: Removed `.into()` and `.pack()` calls - pass values directly:
```rust
// Before
.hash_type(0u8.into())
.capacity(capacity.pack())

// After
.hash_type(0u8)
.capacity(capacity)
```

The builder methods already handle the conversion to molecule types.

### 2. Unused Import Warning
**Warning**: `use alloc::vec;` unused in ssri.rs

**Fix**: Removed `alloc::vec` import, kept only `alloc::vec::Vec`

### 3. OutPoint Import in Test
**Error**: `OutPoint` not in scope in test module

**Fix**: Added explicit import in test function:
```rust
use ckb_types::packed::OutPoint;
```

## Performance Notes

### Binary Size Growth
- Task 4: 47 KB
- Task 5: 57 KB
- Growth: +10 KB (~21%)

Growth attributed to:
- Intention parsing logic (~3-4 KB)
- CellOutput construction and molecule operations (~3-4 KB)
- PartialTx serialization (~2-3 KB)

Still well within typical CKB contract size limits (~300 KB).

### Input Buffer Size
Contract uses 8 KB input buffer (8192 bytes):
```rust
let mut input_buf = [0u8; 8192];
```

**Capacity analysis**:
- Count: 4 bytes
- Per intention (assuming 32-byte lock_args, 128-byte data):
  - Intention len: 4 bytes
  - Capacity: 8 bytes
  - Lock args len + args: 4 + 32 = 36 bytes
  - Data len + data: 4 + 128 = 132 bytes
  - Total: ~180 bytes per intention
- With BATCH_MAX=4: 4 + (4+180)×4 = ~740 bytes
- 8 KB buffer provides ~10× headroom

Buffer size is safe for current constraints and allows for larger payloads in future tasks.

## Next Steps (Task 6+)

Per the plan:
1. **Task 6**: On-chain verification
   - Implement `verify()` in verify.rs
   - Validate intention inputs exist
   - Verify beneficiary outputs match assemble() results
   - Check Process Type ID continuity

2. **Later tasks**: Process Calculator, FakeRpc, Type ID management

3. **Potential enhancements**:
   - Parse real CID beneficiary Script from payload
   - Add Process recreate output
   - Optimize buffer sizes based on real usage

## Conclusion

Task 5 successfully implements full Process.assemble with:
- Complete intention encoding (capacity, lock_args, data)
- Beneficiary CellOutput generation in contract
- PartialTx serialization/deserialization
- Comprehensive test coverage

All tests pass (12 passed, 1 ignored), including new test verifying non-empty PartialTx with correct beneficiary outputs. The implementation is ready for Task 6 on-chain verification.

**Status**: READY FOR REVIEW
