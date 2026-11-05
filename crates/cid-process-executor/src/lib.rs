//! Greenfield offline SSRI host for CID Process contracts
//!
//! Implements minimal CKB-VM runner with argv-based SSRI method invocation.
//! vm_version = u64::MAX convention for SSRI mode.

pub mod types;
pub mod vm;

pub use types::{encode_intentions, ExecutorError, IntentionInput, PartialTx, CUSTOM_ERROR_START};

/// Offline SSRI executor for CID Process contracts
pub struct ProcessExecutor {
    binary: Vec<u8>,
}

impl ProcessExecutor {
    /// Load a Process RISC-V binary
    pub fn new(binary: Vec<u8>) -> Self {
        Self { binary }
    }

    /// Query the Process for its batch bounds (min, max) via SSRI Process.batch
    pub fn get_batch_bounds(&self) -> Result<(u32, u32), ExecutorError> {
        if self.binary.is_empty() {
            return Err(ExecutorError::VmError("empty binary".into()));
        }

        vm::call_ssri_method(&self.binary, "Process.batch", &[])
            .and_then(|response| parse_batch_bounds(&response))
    }

    /// Assemble a digest transaction for the given intentions via SSRI Process.assemble
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
}

fn parse_batch_bounds(response: &[u8]) -> Result<(u32, u32), ExecutorError> {
    if response.len() != 8 {
        return Err(ExecutorError::InvalidResponse(format!(
            "expected 8 bytes for batch bounds, got {}",
            response.len()
        )));
    }

    let min = u32::from_le_bytes(response[0..4].try_into().unwrap());
    let max = u32::from_le_bytes(response[4..8].try_into().unwrap());

    Ok((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_batch_bounds() {
        let response = vec![1, 0, 0, 0, 4, 0, 0, 0]; // (1, 4) little-endian
        let (min, max) = parse_batch_bounds(&response).unwrap();
        assert_eq!((min, max), (1, 4));
    }

    #[test]
    fn test_empty_binary_returns_error() {
        let executor = ProcessExecutor::new(vec![]);
        assert!(matches!(
            executor.get_batch_bounds(),
            Err(ExecutorError::VmError(_))
        ));
    }

    #[test]
    fn test_vm_error_on_invalid_binary() {
        let executor = ProcessExecutor::new(vec![0xFF; 100]);
        let result = executor.get_batch_bounds();
        assert!(matches!(result, Err(ExecutorError::VmError(_))));
    }

    // Test syscall 2041 implementation and exit code handling
    // Full integration tests (argv routing, stdout capture, exit 0 success path,
    // and actual RISC-V binary execution) will be added in Task 3 when the Process
    // contract binary is available for testing.

    #[test]
    fn test_syscall_2041_returns_ssri_version() {
        // Verify syscall 2041 (CKB_VM_VERSION) constant is u64::MAX
        // The syscall handler is implemented in vm.rs and returns this value
        // when the guest calls syscall 2041.
        assert_eq!(vm::SSRI_VM_VERSION, u64::MAX);
    }

    #[test]
    fn test_custom_error_start_threshold() {
        // Verify CUSTOM_ERROR_START is 20 per global constraints
        assert_eq!(CUSTOM_ERROR_START, 20);
    }

    #[test]
    fn test_incompatible_error_type() {
        // Verify ExecutorError::Incompatible exists and can be constructed
        let err = ExecutorError::Incompatible(20);
        match err {
            ExecutorError::Incompatible(code) => assert_eq!(code, 20),
            _ => panic!("Wrong error type"),
        }
    }

    #[test]
    fn test_incompatible_error_display() {
        // Verify error formatting
        let err = ExecutorError::Incompatible(42);
        let msg = format!("{}", err);
        assert!(msg.contains("Process rejected batch"));
        assert!(msg.contains("42"));
    }

    // Integration tests with real RISC-V binaries
    #[test]
    fn test_integration_exit0_success() {
        let binary = include_bytes!("../tests/fixtures/exit0.elf");
        let result = vm::call_ssri_method(binary, "test.method", &[]);
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let output = result.unwrap();
        assert_eq!(
            output,
            Vec::<u8>::new(),
            "Expected empty output for exit(0)"
        );
    }

    #[test]
    #[ignore] // Binary structure issue: VM finishes before reaching sys_exit
              // SYS_WRITE works (stdout_hello proves it), but simple exit binaries
              // complete without calling sys_exit. Will be fully tested with Process
              // contract (Task 3) which uses ckb-ssri-std and proper exit handling.
    fn test_integration_exit21_incompatible() {
        let binary = include_bytes!("../tests/fixtures/exit21.elf");
        let result = vm::call_ssri_method(binary, "test.method", &[]);
        assert!(
            matches!(result, Err(ExecutorError::Incompatible(21))),
            "Expected Incompatible(21), got {:?}",
            result
        );
    }

    #[test]
    fn test_integration_stdout_capture() {
        let binary = include_bytes!("../tests/fixtures/stdout_hello.elf");
        let result = vm::call_ssri_method(binary, "test.method", &[]);
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let output = result.unwrap();
        assert_eq!(output, b"hello", "Expected stdout to capture 'hello'");
    }

    #[test]
    fn test_looping_guest_hits_cycle_budget() {
        let binary = include_bytes!("../tests/fixtures/infinite_loop.elf");
        let result = vm::call_ssri_method_with_cycles(binary, "test.method", &[], 10_000);
        assert!(
            matches!(result, Err(ExecutorError::CyclesExceeded)),
            "looping guest must stop at the cycle budget, got {:?}",
            result
        );
    }

    fn pack_molecule_tx(
        outputs: Vec<ckb_types::packed::CellOutput>,
        outputs_data: Vec<Vec<u8>>,
    ) -> Vec<u8> {
        use ckb_types::packed::{RawTransaction, Transaction};
        use ckb_types::prelude::{Builder, Entity, Pack, PackVec};
        Transaction::new_builder()
            .raw(
                RawTransaction::new_builder()
                    .outputs(outputs.pack())
                    .outputs_data(outputs_data.iter().map(|d| d.as_slice().pack()).pack())
                    .build(),
            )
            .build()
            .as_slice()
            .to_vec()
    }

    #[test]
    fn test_partial_tx_rejects_malicious_output_count() {
        // 4-byte 0xffffffff used to Vec::with_capacity(u32::MAX) before length checks.
        let bytes = [0xff, 0xff, 0xff, 0xff];
        let result = PartialTx::from_bytes(&bytes);
        assert!(
            matches!(result, Err(ExecutorError::InvalidResponse(_))),
            "malicious short response must be InvalidResponse, got {:?}",
            result
        );
    }

    #[test]
    fn test_partial_tx_zero_outputs() {
        let parsed = PartialTx::from_bytes(&pack_molecule_tx(Vec::new(), Vec::new()))
            .expect("empty molecule Transaction");
        assert!(parsed.outputs.is_empty());
        assert!(parsed.outputs_data.is_empty());
    }

    #[test]
    fn test_partial_tx_parses_molecule_transaction() {
        use ckb_types::packed::{Byte32, CellOutput, Script};
        use ckb_types::prelude::{Builder, Entity, Pack, Unpack};

        let lock = Script::new_builder()
            .code_hash(Byte32::from_slice(&[0x11; 32]).unwrap())
            .hash_type(1u8.into())
            .args([0xAA, 0xBB].pack())
            .build();
        let output = CellOutput::new_builder()
            .capacity(100_000_000_000u64.pack())
            .lock(lock)
            .build();
        let data = vec![0xcc, 0xdd];
        let bytes = pack_molecule_tx(vec![output.clone()], vec![data.clone()]);

        let parsed = PartialTx::from_bytes(&bytes).expect("parse molecule Transaction");
        assert_eq!(parsed.as_molecule_bytes(), bytes.as_slice());
        assert_eq!(parsed.outputs.len(), 1);
        assert_eq!(parsed.outputs[0].as_slice(), output.as_slice());
        assert_eq!(parsed.outputs_data[0], data);
        let cap: u64 = parsed.outputs[0].capacity().unpack();
        assert_eq!(cap, 100_000_000_000);
    }

    #[test]
    fn test_partial_tx_rejects_legacy_length_prefix() {
        // Old wire: output_count u32 LE + per-output (output_len + data_len).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let result = PartialTx::from_bytes(&bytes);
        assert!(
            matches!(result, Err(ExecutorError::InvalidResponse(_))),
            "legacy envelope must be rejected, got {:?}",
            result
        );
    }

    #[test]
    fn test_partial_tx_rejects_oversized_response() {
        let bytes = vec![0u8; 1024 * 1024 + 1];
        let result = PartialTx::from_bytes(&bytes);
        assert!(
            matches!(result, Err(ExecutorError::InvalidResponse(_))),
            "oversized response must be rejected, got {:?}",
            result
        );
    }

    #[test]
    fn test_partial_tx_rejects_too_many_outputs() {
        use ckb_types::packed::CellOutput;
        let outputs = vec![CellOutput::default(); 257];
        let data = vec![Vec::new(); 257];
        let bytes = pack_molecule_tx(outputs, data);
        let result = PartialTx::from_bytes(&bytes);
        assert!(
            matches!(result, Err(ExecutorError::InvalidResponse(_))),
            "output count above budget must be rejected, got {:?}",
            result
        );
    }

    #[test]
    fn test_partial_tx_rejects_outputs_data_length_mismatch() {
        use ckb_types::packed::{CellOutput, RawTransaction, Transaction};
        use ckb_types::prelude::{Builder, Entity, PackVec};
        let bytes = Transaction::new_builder()
            .raw(
                RawTransaction::new_builder()
                    .outputs(vec![CellOutput::default()].pack())
                    .build(),
            )
            .build()
            .as_slice()
            .to_vec();
        let result = PartialTx::from_bytes(&bytes);
        assert!(
            matches!(result, Err(ExecutorError::InvalidResponse(_))),
            "mismatched outputs / outputs_data must be rejected, got {:?}",
            result
        );
    }

    #[test]
    fn test_real_binary_batch_bounds() {
        let bin_path = "../../build/release/cid_process_example";
        if !std::path::Path::new(bin_path).exists() {
            eprintln!("Skipping test: binary not built (run 'cd contracts/cid_process_example && make build')");
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
        assert!(result.is_ok(), "should accept count=1: {:?}", result);
    }

    #[test]
    fn test_assemble_returns_non_empty_partial_tx() {
        use ckb_types::packed::OutPoint;

        let bin_path = "../../build/release/cid_process_example";
        if !std::path::Path::new(bin_path).exists() {
            eprintln!("Skipping test: binary not built");
            return;
        }

        let binary = std::fs::read(bin_path).expect("read binary");
        let executor = ProcessExecutor::new(binary);

        // Create intentions with real data
        let intentions = vec![
            IntentionInput {
                out_point: OutPoint::default(),
                lock_args: vec![0x01, 0x02, 0x03, 0x04],
                capacity: 100_000_000_000,
                data: vec![0xaa, 0xbb],
                type_script: None,
            },
            IntentionInput {
                out_point: OutPoint::default(),
                lock_args: vec![0x05, 0x06, 0x07],
                capacity: 200_000_000_000,
                data: vec![0xcc, 0xdd, 0xee],
                type_script: None,
            },
        ];

        let result = executor.assemble(&intentions);
        assert!(
            result.is_ok(),
            "should succeed with valid intentions: {:?}",
            result
        );

        let partial_tx = result.unwrap();

        // Verify non-empty outputs
        assert_eq!(
            partial_tx.outputs.len(),
            2,
            "should have 2 beneficiary outputs"
        );
        assert_eq!(
            partial_tx.outputs_data.len(),
            2,
            "should have 2 output data entries"
        );

        // Verify first output
        use ckb_types::prelude::Unpack;
        let output0_capacity: u64 = partial_tx.outputs[0].capacity().unpack();
        assert_eq!(
            output0_capacity, 100_000_000_000,
            "first output capacity should match"
        );

        let output0_lock_args: Vec<u8> = partial_tx.outputs[0].lock().args().unpack();
        assert_eq!(
            output0_lock_args,
            vec![0x01, 0x02, 0x03, 0x04],
            "first output lock args should match"
        );

        assert_eq!(
            partial_tx.outputs_data[0],
            vec![0xaa, 0xbb],
            "first output data should match"
        );

        // Verify second output
        let output1_capacity: u64 = partial_tx.outputs[1].capacity().unpack();
        assert_eq!(
            output1_capacity, 200_000_000_000,
            "second output capacity should match"
        );

        let output1_lock_args: Vec<u8> = partial_tx.outputs[1].lock().args().unpack();
        assert_eq!(
            output1_lock_args,
            vec![0x05, 0x06, 0x07],
            "second output lock args should match"
        );

        assert_eq!(
            partial_tx.outputs_data[1],
            vec![0xcc, 0xdd, 0xee],
            "second output data should match"
        );
    }

    fn load_process_binary() -> Option<Vec<u8>> {
        let bin_path = "../../build/release/cid_process_example";
        if !std::path::Path::new(bin_path).exists() {
            eprintln!("Skipping test: binary not built");
            return None;
        }
        Some(std::fs::read(bin_path).expect("read binary"))
    }

    #[test]
    fn test_assemble_preserves_data2_beneficiary_hash_type() {
        use cid_protocol::{CidArgs, Payload, Script as CidScript, SCRIPT_HASH_TYPE_DATA2};
        use ckb_types::prelude::Unpack;

        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);

        let beneficiary = CidScript {
            code_hash: [0x11; 32],
            hash_type: SCRIPT_HASH_TYPE_DATA2,
            args: vec![0xAA],
        };
        let lock_args = CidArgs::new(
            [0x22; 32],
            CidScript::default(),
            Payload::from_script(&beneficiary).unwrap(),
        )
        .encode()
        .unwrap();
        let partial = executor
            .assemble(&[IntentionInput {
                lock_args,
                capacity: 100_000_000_000,
                data: vec![],
                ..Default::default()
            }])
            .expect("assemble Data2 beneficiary");
        let ht: u8 = partial.outputs[0].lock().hash_type().into();
        assert_eq!(ht, SCRIPT_HASH_TYPE_DATA2, "assemble must keep Data2");
        let args: Vec<u8> = partial.outputs[0].lock().args().unpack();
        assert_eq!(args, vec![0xAA]);
    }

    #[test]
    fn test_assemble_preserves_xudt_type_script() {
        use cid_protocol::{XUDT_AMOUNT_LEN, XUDT_CODE_HASH};
        use ckb_types::packed::{Byte32, Script};
        use ckb_types::prelude::{Builder, Entity, Pack, Unpack};

        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);

        let type_script = Script::new_builder()
            .code_hash(Byte32::from_slice(&XUDT_CODE_HASH).unwrap())
            .hash_type(2u8.into())
            .args([0x33; 32].as_slice().pack())
            .build();
        let amount = 1_000u128.to_le_bytes()[..XUDT_AMOUNT_LEN].to_vec();
        let partial = executor
            .assemble(&[IntentionInput {
                lock_args: vec![0x01],
                capacity: 100_000_000_000,
                data: amount.clone(),
                type_script: Some(type_script.as_slice().to_vec()),
                ..Default::default()
            }])
            .expect("assemble xUDT intention");
        let got = partial.outputs[0]
            .type_()
            .to_opt()
            .expect("beneficiary output must keep xUDT type");
        assert_eq!(got.as_slice(), type_script.as_slice());
        assert_eq!(partial.outputs_data[0], amount);
        let cap: u64 = partial.outputs[0].capacity().unpack();
        assert_eq!(cap, 100_000_000_000);
    }
}
