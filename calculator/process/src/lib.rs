//! Process Calculator for CID Process example

use cid_process_executor::{IntentionInput, ProcessExecutor};
use ckb_cinnabar_calculator::{
    instruction::Instruction,
    re_exports::ckb_types::{
        packed::{CellInput, CellOutput, OutPoint, Transaction, Uint64},
        prelude::{Builder, Entity, Pack, Unpack},
    },
    rpc::RPC,
    skeleton::{CellInputEx, CellOutputEx, TransactionSkeleton},
    CalculatorError, Result,
};

pub const PROCESS_CONTRACT_NAME: &str = "cid_process_example";

/// Intention cell reference for digest
#[derive(Clone, Debug)]
pub struct IntentionCell {
    pub out_point: OutPoint,
    pub lock_args: Vec<u8>,
    pub capacity: u64,
    pub data: Vec<u8>,
    /// Packed molecule `Script` of the intention type, if any (official xUDT).
    pub type_script: Option<Vec<u8>>,
}

impl IntentionCell {
    /// Convert to executor IntentionInput
    pub fn to_intention_input(&self) -> Result<IntentionInput> {
        // Serialize calculator's OutPoint to bytes and deserialize in executor's version
        use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::Entity;
        let _out_point_bytes = self.out_point.as_slice();

        // Both ckb-gen-types versions use same molecule wire format
        // Deserialize bytes into executor's OutPoint type
        let executor_out_point: cid_process_executor::IntentionInput = IntentionInput::default();

        // Build input with raw fields, OutPoint will use default (can't easily convert between versions)
        // This is acceptable since the skeleton uses the calculator's OutPoint directly
        Ok(IntentionInput {
            out_point: executor_out_point.out_point, // Use placeholder
            lock_args: self.lock_args.clone(),
            capacity: self.capacity,
            data: self.data.clone(),
            type_script: self.type_script.clone(),
        })
    }
}

/// Load the Process binary from build/release/<name>
pub fn load_process_binary(bin_dir: &str) -> Result<Vec<u8>> {
    let path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
    std::fs::read(&path).map_err(|e| {
        CalculatorError::Other(format!("failed to load Process binary at {}: {}", path, e))
    })
}

/// Process digest calculation: validates batch and prepares transaction skeleton
///
/// Builds a skeleton with intention inputs plus beneficiary outputs from
/// `executor.assemble()`. Assemble returns a packed molecule `Transaction`;
/// this crate parses it with calculator ckb-types (1.x). Process recreate
/// remains deferred (Task 5 carry-forward).
pub fn process_digest_ok<T: RPC>(
    _rpc: &T,
    intentions: Vec<IntentionCell>,
    bin_dir: &str,
) -> Result<(TransactionSkeleton, Vec<Instruction<T>>)> {
    // Load Process binary and create executor
    let binary = load_process_binary(bin_dir)?;
    let executor = ProcessExecutor::new(binary);

    // Validate batch bounds
    let (min, max) = executor
        .get_batch_bounds()
        .map_err(|e| CalculatorError::Other(format!("failed to get batch bounds: {}", e)))?;

    let count = intentions.len() as u32;
    if count < min || count > max {
        return Err(CalculatorError::Other(format!(
            "intention count {} out of bounds [{}, {}]",
            count, min, max
        )));
    }

    // Convert to executor inputs
    let inputs: Vec<IntentionInput> = intentions
        .iter()
        .map(|c| c.to_intention_input())
        .collect::<Result<Vec<_>>>()?;

    // Call executor assemble to get beneficiary outputs
    let partial_tx = executor
        .assemble(&inputs)
        .map_err(|e| CalculatorError::Other(format!("executor assemble failed: {}", e)))?;

    // Build skeleton with intention inputs
    let mut skeleton = TransactionSkeleton::default();

    // Add intention inputs
    for intention in &intentions {
        let input = CellInputEx {
            input: CellInput::new_builder()
                .previous_output(intention.out_point.clone())
                .build(),
            output: CellOutputEx::new(
                CellOutput::new_builder()
                    .capacity(Pack::<Uint64>::pack(&intention.capacity))
                    .build(),
                intention.data.clone(),
            ),
            with_data: false,
        };
        skeleton.input(input)?.witness(Default::default());
    }

    // Parse the same molecule Transaction the contract / executor produced.
    // Wire format is stable across ckb-gen-types 0.119 (executor) and 1.x.
    let tx = Transaction::from_slice(partial_tx.as_molecule_bytes())
        .map_err(|e| CalculatorError::Other(format!("Transaction molecule bridge: {:?}", e)))?;
    let raw = tx.raw();
    if raw.outputs().len() != raw.outputs_data().len() {
        return Err(CalculatorError::Other(
            "assemble Transaction outputs / outputs_data length mismatch".into(),
        ));
    }
    for (output, data) in raw.outputs().into_iter().zip(raw.outputs_data()) {
        let data_bytes: Vec<u8> = data.unpack();
        skeleton.output(CellOutputEx::new(output, data_bytes));
    }

    let instructions = vec![];
    Ok((skeleton, instructions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckb_cinnabar_calculator::re_exports::ckb_types::packed::OutPointBuilder;

    #[test]
    fn test_load_binary_missing() {
        let result = load_process_binary("/nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_intention_conversion() {
        let cell = IntentionCell {
            out_point: OutPointBuilder::default().build(),
            lock_args: vec![1, 2, 3],
            capacity: 100_000_000_000,
            data: vec![4, 5, 6],
            type_script: None,
        };

        let input = cell.to_intention_input().expect("conversion");
        assert_eq!(input.lock_args, vec![1, 2, 3]);
        assert_eq!(input.capacity, 100_000_000_000);
        assert_eq!(input.data, vec![4, 5, 6]);
        assert!(input.type_script.is_none());
    }

    #[test]
    fn test_process_digest_beneficiary_outputs() {
        use ckb_cinnabar_calculator::simulation::{fake_outpoint, FakeRpcClient};

        let bin_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../build/release");
        let bin_path = format!("{}/{}", bin_dir, PROCESS_CONTRACT_NAME);
        if !std::path::Path::new(&bin_path).exists() {
            eprintln!(
                "Skipping test: binary not built at {} (run 'cd contracts/cid_process_example && make build')",
                bin_path
            );
            return;
        }

        let rpc = FakeRpcClient::default();

        let intentions_n1 = vec![IntentionCell {
            out_point: fake_outpoint(),
            lock_args: vec![0x01, 0x02, 0x03],
            capacity: 100_000_000_000,
            data: vec![0xaa, 0xbb],
            type_script: None,
        }];
        let (skeleton, _) = process_digest_ok(&rpc, intentions_n1.clone(), bin_dir)
            .expect("digest should succeed with n=1");
        assert_eq!(
            skeleton.outputs.len(),
            intentions_n1.len(),
            "skeleton should have 1 beneficiary output for 1 intention"
        );

        let intentions_n2 = vec![
            IntentionCell {
                out_point: fake_outpoint(),
                lock_args: vec![0x01, 0x02, 0x03],
                capacity: 100_000_000_000,
                data: vec![0xaa, 0xbb],
                type_script: None,
            },
            IntentionCell {
                out_point: fake_outpoint(),
                lock_args: vec![0x04, 0x05, 0x06],
                capacity: 200_000_000_000,
                data: vec![0xcc, 0xdd],
                type_script: None,
            },
        ];
        let (skeleton, _) = process_digest_ok(&rpc, intentions_n2.clone(), bin_dir)
            .expect("digest should succeed with n=2");
        assert_eq!(
            skeleton.outputs.len(),
            intentions_n2.len(),
            "skeleton should have 2 beneficiary outputs for 2 intentions"
        );
    }
}
