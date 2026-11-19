//! Greenfield offline SSRI host for CID Process contracts
//!
//! Implements minimal CKB-VM runner with argv-based SSRI method invocation.
//! vm_version = u64::MAX convention for SSRI mode.

pub mod host_query;
pub mod types;
pub mod vm;

pub use host_query::{HostCell, HostQuery, HostQueryError, MockHost, Pagination, SearchKey};
pub use types::{
    build_draft_tx, AssembledTx, DraftCell, ExecutorError, PartialTx, CUSTOM_ERROR_START,
};

use ckb_types::packed::{Byte32, CellOutput, OutPoint, Script};
use ckb_types::prelude::*;

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

    /// Assemble a complete digest transaction via SSRI `Process.assemble`.
    ///
    /// `draft_tx` is a packed molecule `Transaction` whose inputs are the
    /// Intention + Process out points. `host` must resolve those out points
    /// (and any tip queries) so the guest can build a directly-submittable tx.
    pub fn assemble(&self, draft_tx: &[u8], host: MockHost) -> Result<AssembledTx, ExecutorError> {
        if self.binary.is_empty() {
            return Err(ExecutorError::VmError("empty binary".into()));
        }
        let response =
            vm::call_ssri_method_with_host(&self.binary, "Process.assemble", draft_tx, host)?;
        AssembledTx::from_bytes(&response)
    }

    /// Convenience: seed `host` from `cells`, build a draft, and assemble.
    pub fn assemble_from_cells(&self, cells: &[DraftCell]) -> Result<AssembledTx, ExecutorError> {
        let host = MockHost::new();
        host.insert_block_hash(0, [0xAB; 32]);
        for cell in cells {
            host.insert_cell(
                cell.out_point.clone(),
                cell.output.clone(),
                cell.data.clone(),
            );
        }
        let draft = build_draft_tx(cells);
        self.assemble(&draft, host)
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

/// Build a CKB-only Intention draft cell (lock args + capacity + data).
pub fn intention_draft_cell(
    out_point: OutPoint,
    lock_args: Vec<u8>,
    capacity: u64,
    type_script: Option<Script>,
    data: Vec<u8>,
) -> DraftCell {
    let lock = Script::new_builder()
        .code_hash(Byte32::from_slice(&[0xC1; 32]).unwrap())
        .hash_type(1u8.into())
        .args(lock_args.pack())
        .build();
    let mut output = CellOutput::new_builder()
        .capacity(capacity.pack())
        .lock(lock);
    if let Some(typ) = type_script {
        output = output.type_(Some(typ).pack());
    }
    DraftCell {
        out_point,
        output: output.build(),
        data,
    }
}

/// Build a Process state draft cell (Process lock + TYPE_ID type).
pub fn process_draft_cell(
    out_point: OutPoint,
    process_lock: Script,
    type_id_type: Script,
    capacity: u64,
    data: Vec<u8>,
) -> DraftCell {
    let output = CellOutput::new_builder()
        .capacity(capacity.pack())
        .lock(process_lock)
        .type_(Some(type_id_type).pack())
        .build();
    DraftCell {
        out_point,
        output,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cid_protocol::{SCRIPT_HASH_TYPE_TYPE, TYPE_ID_CODE_HASH};
    use ckb_types::packed::{Byte32, CellOutput, OutPoint, RawTransaction, Script, Transaction};
    use ckb_types::prelude::{Builder, Entity, Pack, PackVec, Unpack};

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

    #[test]
    fn test_syscall_2041_returns_ssri_version() {
        assert_eq!(vm::SSRI_VM_VERSION, u64::MAX);
    }

    #[test]
    fn test_custom_error_start_threshold() {
        assert_eq!(CUSTOM_ERROR_START, 20);
    }

    #[test]
    fn test_incompatible_error_type() {
        let err = ExecutorError::Incompatible(20);
        match err {
            ExecutorError::Incompatible(code) => assert_eq!(code, 20),
            _ => panic!("Wrong error type"),
        }
    }

    #[test]
    fn test_incompatible_error_display() {
        let err = ExecutorError::Incompatible(42);
        let msg = format!("{}", err);
        assert!(msg.contains("Process rejected batch"));
        assert!(msg.contains("42"));
    }

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
        let result =
            vm::call_ssri_method_with_cycles(binary, "test.method", &[], MockHost::new(), 10_000);
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
    fn test_assembled_tx_rejects_malicious_output_count() {
        let bytes = [0xff, 0xff, 0xff, 0xff];
        let result = AssembledTx::from_bytes(&bytes);
        assert!(
            matches!(result, Err(ExecutorError::InvalidResponse(_))),
            "malicious short response must be InvalidResponse, got {:?}",
            result
        );
    }

    #[test]
    fn test_assembled_tx_zero_outputs() {
        let parsed = AssembledTx::from_bytes(&pack_molecule_tx(Vec::new(), Vec::new()))
            .expect("empty molecule Transaction");
        assert!(parsed.outputs.is_empty());
        assert!(parsed.outputs_data.is_empty());
        assert!(parsed.inputs.is_empty());
    }

    #[test]
    fn test_assembled_tx_parses_molecule_transaction() {
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

        let parsed = AssembledTx::from_bytes(&bytes).expect("parse molecule Transaction");
        assert_eq!(parsed.as_molecule_bytes(), bytes.as_slice());
        assert_eq!(parsed.outputs.len(), 1);
        assert_eq!(parsed.outputs[0].as_slice(), output.as_slice());
        assert_eq!(parsed.outputs_data[0], data);
        let cap: u64 = parsed.outputs[0].capacity().unpack();
        assert_eq!(cap, 100_000_000_000);
    }

    #[test]
    fn test_assembled_tx_rejects_oversized_response() {
        let bytes = vec![0u8; 1024 * 1024 + 1];
        let result = AssembledTx::from_bytes(&bytes);
        assert!(matches!(result, Err(ExecutorError::InvalidResponse(_))));
    }

    #[test]
    fn test_assembled_tx_rejects_too_many_outputs() {
        let outputs = vec![CellOutput::default(); 257];
        let data = vec![Vec::new(); 257];
        let bytes = pack_molecule_tx(outputs, data);
        let result = AssembledTx::from_bytes(&bytes);
        assert!(matches!(result, Err(ExecutorError::InvalidResponse(_))));
    }

    #[test]
    fn test_assembled_tx_rejects_outputs_data_length_mismatch() {
        let bytes = Transaction::new_builder()
            .raw(
                RawTransaction::new_builder()
                    .outputs(vec![CellOutput::default()].pack())
                    .build(),
            )
            .build()
            .as_slice()
            .to_vec();
        let result = AssembledTx::from_bytes(&bytes);
        assert!(matches!(result, Err(ExecutorError::InvalidResponse(_))));
    }

    fn load_process_binary() -> Option<Vec<u8>> {
        let bin_path = "../../build/release/cid_process_example";
        if !std::path::Path::new(bin_path).exists() {
            eprintln!("Skipping test: binary not built");
            return None;
        }
        Some(std::fs::read(bin_path).expect("read binary"))
    }

    fn type_id_script(tag: &[u8]) -> Script {
        let mut args = [0u8; 32];
        let n = tag.len().min(32);
        args[..n].copy_from_slice(&tag[..n]);
        Script::new_builder()
            .code_hash(Byte32::from_slice(&TYPE_ID_CODE_HASH).unwrap())
            .hash_type(SCRIPT_HASH_TYPE_TYPE.into())
            .args(args.as_slice().pack())
            .build()
    }

    fn process_lock_script() -> Script {
        Script::new_builder()
            .code_hash(Byte32::from_slice(&[0xAA; 32]).unwrap())
            .hash_type(2u8.into())
            .build()
    }

    fn make_assemble_cells(n: usize) -> Vec<DraftCell> {
        let type_id = type_id_script(b"process-type-id-v0");
        let pth: [u8; 32] = type_id.calc_script_hash().unpack();
        let mut cells = Vec::new();
        // Process state cell — assemble finds TYPE_ID typed non-CID cell.
        cells.push(process_draft_cell(
            OutPoint::new_builder()
                .tx_hash(Byte32::from_slice(&[0x01; 32]).unwrap())
                .index(0u32.pack())
                .build(),
            process_lock_script(),
            type_id,
            200_000_000_000,
            vec![],
        ));
        for i in 0..n {
            let lock_args = cid_protocol::CidArgs::new(
                pth,
                cid_protocol::Script::default(),
                cid_protocol::Payload::default(),
            )
            .encode()
            .unwrap();
            cells.push(intention_draft_cell(
                OutPoint::new_builder()
                    .tx_hash(Byte32::from_slice(&[0x10 + i as u8; 32]).unwrap())
                    .index(0u32.pack())
                    .build(),
                lock_args,
                100_000_000_000,
                None,
                vec![0xaa, 0xbb],
            ));
        }
        cells
    }

    #[test]
    fn test_real_binary_batch_bounds() {
        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);
        let (min, max) = executor.get_batch_bounds().expect("batch bounds");
        assert_eq!((min, max), (1, 4));
    }

    #[test]
    fn test_assemble_validates_count() {
        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);

        // Empty draft (no intentions)
        let result = executor.assemble_from_cells(&[]);
        assert!(
            matches!(result, Err(ExecutorError::Incompatible(21))),
            "should reject empty batch: {:?}",
            result
        );

        // Count=5 (> BATCH_MAX)
        let cells = make_assemble_cells(5);
        let result = executor.assemble_from_cells(&cells);
        assert!(
            matches!(result, Err(ExecutorError::Incompatible(21))),
            "should reject count=5: {:?}",
            result
        );

        // Count=1 (valid)
        let cells = make_assemble_cells(1);
        let result = executor.assemble_from_cells(&cells);
        assert!(result.is_ok(), "should accept count=1: {:?}", result);
    }

    #[test]
    fn test_assemble_returns_complete_tx() {
        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);
        let cells = make_assemble_cells(2);
        let assembled = executor
            .assemble_from_cells(&cells)
            .expect("assemble complete tx");

        // inputs: process + 2 intentions
        assert_eq!(assembled.inputs.len(), 3);
        // outputs: process recreate + 2 beneficiaries
        assert_eq!(assembled.outputs.len(), 3);
        assert_eq!(assembled.outputs_data.len(), 3);
        // Complete witnesses: one per input; backend must not fill any.
        assert_eq!(assembled.witnesses.len(), assembled.inputs.len());

        let cap0: u64 = assembled.outputs[1].capacity().unpack();
        assert_eq!(cap0, 100_000_000_000);
        assert_eq!(assembled.outputs_data[1], vec![0xaa, 0xbb]);
    }

    #[test]
    fn test_assemble_fee_meets_feerate_serialized_size_in_block() {
        use ckb_types::core::FeeRate;

        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);
        let cells = make_assemble_cells(1);
        let process_cap_in: u64 = cells[0].output.capacity().unpack();
        let assembled = executor
            .assemble_from_cells(&cells)
            .expect("assemble for fee check");
        let process_cap_out: u64 = assembled.outputs[0].capacity().unpack();
        let fee = process_cap_in
            .checked_sub(process_cap_out)
            .expect("fee deducted from process recreate");
        let weight = assembled.as_molecule_bytes().len() as u64 + 4; // serialized_size_in_block
        let min = FeeRate::from_u64(1000).fee(weight).as_u64();
        assert!(
            fee >= min,
            "assemble fee {fee} must be >= FeeRate(1000).fee(serialized_size_in_block={weight})={min}"
        );
    }

    #[test]
    fn test_assemble_preserves_data2_beneficiary_hash_type() {
        use cid_protocol::{CidArgs, Payload, Script as CidScript, SCRIPT_HASH_TYPE_DATA2};

        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);
        let type_id = type_id_script(b"process-type-id-v0");
        let pth: [u8; 32] = type_id.calc_script_hash().unpack();
        let beneficiary = CidScript {
            code_hash: [0x11; 32],
            hash_type: SCRIPT_HASH_TYPE_DATA2,
            args: vec![0xAA],
        };
        let lock_args = CidArgs::new(
            pth,
            CidScript::default(),
            Payload::from_script(&beneficiary).unwrap(),
        )
        .encode()
        .unwrap();

        let cells = vec![
            process_draft_cell(
                OutPoint::new_builder()
                    .tx_hash(Byte32::from_slice(&[0x01; 32]).unwrap())
                    .index(0u32.pack())
                    .build(),
                process_lock_script(),
                type_id,
                200_000_000_000,
                vec![],
            ),
            intention_draft_cell(
                OutPoint::new_builder()
                    .tx_hash(Byte32::from_slice(&[0x02; 32]).unwrap())
                    .index(0u32.pack())
                    .build(),
                lock_args,
                100_000_000_000,
                None,
                vec![],
            ),
        ];
        let assembled = executor
            .assemble_from_cells(&cells)
            .expect("assemble Data2 beneficiary");
        let ht: u8 = assembled.outputs[1].lock().hash_type().into();
        assert_eq!(ht, SCRIPT_HASH_TYPE_DATA2);
        let args: Vec<u8> = assembled.outputs[1].lock().args().unpack();
        assert_eq!(args, vec![0xAA]);
    }

    #[test]
    fn test_host_query_mock_serves_cells() {
        let host = MockHost::new();
        let op = OutPoint::new_builder()
            .tx_hash(Byte32::from_slice(&[0x11; 32]).unwrap())
            .index(0u32.pack())
            .build();
        let output = CellOutput::new_builder()
            .capacity(42u64.pack())
            .lock(
                Script::new_builder()
                    .code_hash(Byte32::from_slice(&[0x22; 32]).unwrap())
                    .build(),
            )
            .build();
        host.insert_cell(op.clone(), output.clone(), vec![1, 2, 3]);
        let got = host.get_cell(op.as_slice()).expect("cell");
        assert_eq!(got.output.as_slice(), output.as_slice());
        assert_eq!(got.data, vec![1, 2, 3]);
    }

    #[test]
    fn test_host_query_get_block_hash_missing_is_none() {
        let host = MockHost::new();
        assert!(host.get_block_hash(0).is_none());
        host.insert_block_hash(0, [0xAB; 32]);
        assert_eq!(host.get_block_hash(0), Some([0xAB; 32]));
        assert!(host.get_block_hash(1).is_none());
    }

    #[test]
    fn test_assemble_preserves_input_since() {
        use ckb_types::packed::{CellInput, RawTransaction, Transaction};

        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);
        let mut cells = make_assemble_cells(1);
        let since = 0x0100_0000_0000_00FFu64;
        let host = MockHost::new();
        host.insert_block_hash(0, [0xAB; 32]);
        for cell in &cells {
            host.insert_cell(
                cell.out_point.clone(),
                cell.output.clone(),
                cell.data.clone(),
            );
        }
        let inputs: Vec<CellInput> = cells
            .drain(..)
            .enumerate()
            .map(|(i, c)| {
                let mut b = CellInput::new_builder().previous_output(c.out_point);
                if i == 0 {
                    b = b.since(since.pack());
                }
                b.build()
            })
            .collect();
        let draft = Transaction::new_builder()
            .raw(RawTransaction::new_builder().inputs(inputs.pack()).build())
            .build()
            .as_slice()
            .to_vec();
        let assembled = executor.assemble(&draft, host).expect("assemble");
        let got: u64 = assembled.inputs[0].since().unpack();
        assert_eq!(got, since, "assemble must preserve CellInput.since");
    }

    #[test]
    fn test_assemble_rejects_fee_below_occupied_capacity() {
        let Some(binary) = load_process_binary() else {
            return;
        };
        let executor = ProcessExecutor::new(binary);
        let type_id = type_id_script(b"process-type-id-v0");
        let pth: [u8; 32] = type_id.calc_script_hash().unpack();
        let lock_args = cid_protocol::CidArgs::new(
            pth,
            cid_protocol::Script::default(),
            cid_protocol::Payload::default(),
        )
        .encode()
        .unwrap();
        // Mirror Process guest occupied formula: pin capacity exactly at floor
        // so any nonzero fee leaves recreate below occupied.
        let process_lock = process_lock_script();
        let occupied_bytes = 8
            + 32
            + 1
            + process_lock.args().raw_data().len()
            + 32
            + 1
            + type_id.args().raw_data().len();
        let occupiedish = occupied_bytes as u64 * 100_000_000;
        let cells = vec![
            process_draft_cell(
                OutPoint::new_builder()
                    .tx_hash(Byte32::from_slice(&[0x01; 32]).unwrap())
                    .index(0u32.pack())
                    .build(),
                process_lock,
                type_id,
                occupiedish,
                vec![],
            ),
            intention_draft_cell(
                OutPoint::new_builder()
                    .tx_hash(Byte32::from_slice(&[0x02; 32]).unwrap())
                    .index(0u32.pack())
                    .build(),
                lock_args,
                100_000_000_000,
                None,
                vec![],
            ),
        ];
        let result = executor.assemble_from_cells(&cells);
        assert!(
            matches!(result, Err(ExecutorError::Incompatible(37))),
            "fee below occupied should be CapacityInsufficient(37): {:?}",
            result
        );
    }
}
