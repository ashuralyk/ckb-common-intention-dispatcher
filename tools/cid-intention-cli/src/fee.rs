//! Fee helpers: CKB `serialized_size_in_block` + rate-1000 min fee.

use ckb_cinnabar_calculator::re_exports::ckb_types::prelude::{Builder, Entity};
use ckb_cinnabar_calculator::skeleton::{TransactionSkeleton, WitnessEx};

use crate::build::CliError;

/// Testnet / Process-assemble `min_fee_rate` (shannons per 1000 bytes).
pub const MIN_FEE_RATE: u64 = 1000;

/// Secp256k1 sighash recoverable signature length (placeholder + final).
pub const SIGHASH_SIGNATURE_LEN: usize = 65;

/// CKB `serialized_size_in_block` = molecule tx bytes + 4 (`NUMBER_SIZE`).
pub fn serialized_size_in_block(skeleton: &TransactionSkeleton) -> u64 {
    let mol = skeleton.clone().into_packed_transaction().as_slice().len() as u64;
    mol.saturating_add(4)
}

/// `floor(serialized_size_in_block * rate / 1000)` — matches `FeeRate::fee`.
pub fn fee_at_rate(serialized_size_in_block: u64, rate: u64) -> u64 {
    serialized_size_in_block.saturating_mul(rate) / 1000
}

pub fn min_fee_for_skeleton(skeleton: &TransactionSkeleton, rate: u64) -> u64 {
    fee_at_rate(serialized_size_in_block(skeleton), rate)
}

/// Install WitnessArgs.lock placeholders (65 zero bytes) for every input so fee
/// sizing matches the post-sighash molecule size.
pub fn install_sighash_witness_placeholders(skeleton: &mut TransactionSkeleton) {
    let n = skeleton.inputs.len();
    while skeleton.witnesses.len() < n {
        skeleton.witness(WitnessEx::default());
    }
    for w in skeleton.witnesses.iter_mut().take(n) {
        *w = WitnessEx::new(vec![0u8; SIGHASH_SIGNATURE_LEN], vec![], vec![]);
    }
}

/// Shrink the change output so `Σinputs − Σoutputs == fee`. Does not add inputs
/// (preserves outpoint pinning).
pub fn apply_fee_by_shrinking_change(
    skeleton: &mut TransactionSkeleton,
    change_index: usize,
    fee: u64,
) -> Result<(), CliError> {
    if change_index >= skeleton.outputs.len() {
        return Err(CliError::Calculator(
            "change output index out of range".into(),
        ));
    }
    let total_in: u64 = skeleton
        .inputs
        .iter()
        .map(|i| i.output.capacity_shannons())
        .sum();
    let other_out: u64 = skeleton
        .outputs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != change_index)
        .map(|(_, o)| o.capacity_shannons())
        .sum();
    let new_change = total_in
        .checked_sub(other_out)
        .and_then(|v| v.checked_sub(fee))
        .ok_or(CliError::InsufficientFunds)?;
    let occupied = skeleton.outputs[change_index].occupied_capacity().as_u64();
    if new_change < occupied {
        return Err(CliError::InsufficientFunds);
    }
    skeleton.outputs[change_index].output = skeleton.outputs[change_index]
        .output
        .clone()
        .as_builder()
        .capacity(new_change)
        .build();
    let paid = paid_fee(skeleton);
    if paid != fee {
        return Err(CliError::Calculator(format!(
            "fee apply mismatch: paid={paid} want={fee}"
        )));
    }
    Ok(())
}

pub fn paid_fee(skeleton: &TransactionSkeleton) -> u64 {
    let total_in: u64 = skeleton
        .inputs
        .iter()
        .map(|i| i.output.capacity_shannons())
        .sum();
    let total_out: u64 = skeleton.outputs.iter().map(|o| o.capacity_shannons()).sum();
    total_in.saturating_sub(total_out)
}

/// Fail-closed: paid fee must cover `rate` on final `serialized_size_in_block`.
pub fn assert_fee_covers_rate(
    skeleton: &TransactionSkeleton,
    rate: u64,
) -> Result<(u64, u64), CliError> {
    let size = serialized_size_in_block(skeleton);
    let need = fee_at_rate(size, rate);
    let paid = paid_fee(skeleton);
    if paid < need {
        return Err(CliError::Calculator(format!(
            "fee {paid} < min fee {need} for serialized_size_in_block={size} at rate {rate}"
        )));
    }
    Ok((paid, need))
}
