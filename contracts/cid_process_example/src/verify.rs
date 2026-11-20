extern crate alloc;

use crate::classify::{classify_live_intention_excluding, filter_reason_to_process_error};
use crate::{Context, ProcessError};
use alloc::vec;
use alloc::vec::Vec;
use cid_intention_filter::FILTER_ACCEPTED;
use cid_process_example_policy::{fee_from_tx_molecule_len, is_supported_script_hash_type};
use cid_protocol::{
    is_standard_xudt, is_type_id_script, CidArgs, Script as CidScript, PROCESS_TYPE_HASH_LEN,
    XUDT_AMOUNT_LEN,
};
use ckb_cinnabar_verifier::{
    this_script_indices, this_script_pattern, Result, ScriptPattern, ScriptPlace, Verification,
};
use ckb_std::{
    ckb_constants::Source,
    ckb_types::{
        core::ScriptHashType,
        packed::Script,
        prelude::{Builder, Entity, Pack, Unpack},
    },
    high_level::{
        load_cell_capacity, load_cell_data, load_cell_lock, load_cell_type, load_script,
        load_transaction, QueryIter,
    },
};

const BATCH_MIN: usize = 1;
const BATCH_MAX: usize = 4;

#[derive(Default)]
pub struct Root;

impl Verification<Context> for Root {
    fn verify(&mut self, _name: &str, _ctx: &mut Context) -> Result<Option<&str>> {
        verify_process()?;
        Ok(None)
    }
}

/// Process verification runs as the **lock** of the state cell.
/// State cell **type** is the built-in TYPE_ID script (identity).
fn verify_process() -> Result<Option<&'static str>> {
    match this_script_pattern(ScriptPlace::Lock)? {
        ScriptPattern::Create => verify_create(),
        ScriptPattern::Transfer => {
            verify_lock_pair()?;
            verify_state_fee_budget()?;
            verify_intention_batch_and_beneficiaries()?;
            Ok(None)
        }
        // Process must persist; burning the state cell is not a digest path.
        ScriptPattern::Burn => Err(ProcessError::ProcessPairInvalid.into()),
    }
}

/// First Process state cell: lock-group Create. Type must be built-in TYPE_ID;
/// uniqueness is enforced by that system script, not by this lock.
fn verify_create() -> Result<Option<&'static str>> {
    let output_indices = this_script_indices(Source::Output, ScriptPlace::Lock)?;
    if output_indices.len() != 1 {
        return Err(ProcessError::ProcessPairInvalid.into());
    }
    require_type_id_type(output_indices[0], Source::Output)?;
    Ok(None)
}

/// Transfer: exactly one Process lock input and one Process lock output.
///
/// Identity continuity: input and output must carry the **same** TYPE_ID type
/// script hash. The TYPE_ID must not appear on any non-Process-lock output
/// (prevents CID DigestOk matching via an attacker cell while the Process
/// cell migrates to a new Type ID).
///
/// Note: Process lock **args** should be distinct per live instance. Empty or
/// shared args merge cells into one lock group (1→1 becomes N→N and fails
/// `ProcessPairInvalid`).
fn verify_lock_pair() -> Result<Option<&'static str>> {
    let input_indices = this_script_indices(Source::Input, ScriptPlace::Lock)?;
    let output_indices = this_script_indices(Source::Output, ScriptPlace::Lock)?;

    if input_indices.len() != 1 || output_indices.len() != 1 {
        return Err(ProcessError::ProcessPairInvalid.into());
    }
    let in_hash = require_type_id_type(input_indices[0], Source::Input)?;
    let out_hash = require_type_id_type(output_indices[0], Source::Output)?;
    if in_hash != out_hash {
        return Err(ProcessError::TypeIdInvalid.into());
    }
    forbid_type_id_on_non_process_outputs(&in_hash)?;
    Ok(None)
}

/// Bound Process state-cell capacity debits and require they fund the tx fee.
///
/// 1. This lock-group pair's debit is capped by `fee_from_tx_molecule_len(tx)`.
/// 2. `tx_fee = sum(all input caps) - sum(all output caps)` (fail closed on
///    underflow) must cover the **aggregate** positive debit across every
///    Process state cell that shares this lock `code_hash` + `hash_type`
///    (distinct args → distinct lock groups; all still count). Diverting a
///    debit into another output without paying fee is rejected.
fn verify_state_fee_budget() -> Result<Option<&'static str>> {
    let input_indices = this_script_indices(Source::Input, ScriptPlace::Lock)?;
    let output_indices = this_script_indices(Source::Output, ScriptPlace::Lock)?;
    if input_indices.len() != 1 || output_indices.len() != 1 {
        return Err(ProcessError::ProcessPairInvalid.into());
    }
    let in_cap = load_cell_capacity(input_indices[0], Source::Input)
        .map_err(|_| ProcessError::ProcessPairInvalid)?;
    let out_cap = load_cell_capacity(output_indices[0], Source::Output)
        .map_err(|_| ProcessError::ProcessPairInvalid)?;
    let this_debit = in_cap.saturating_sub(out_cap);
    let tx = load_transaction().map_err(|_| ProcessError::ProcessPairInvalid)?;
    let max_fee = fee_from_tx_molecule_len(tx.as_slice().len() as u64);
    if this_debit > max_fee {
        return Err(ProcessError::CapacityInsufficient.into());
    }

    let ours = load_script().map_err(|_| ProcessError::ProcessPairInvalid)?;
    let our_code = ours.code_hash();
    let our_ht = ours.hash_type();

    let process_in = sum_capacity_matching_lock(Source::Input, &our_code, our_ht)?;
    let process_out = sum_capacity_matching_lock(Source::Output, &our_code, our_ht)?;
    let aggregate_debit = process_in.saturating_sub(process_out);

    let all_in = sum_capacity_all(Source::Input)?;
    let all_out = sum_capacity_all(Source::Output)?;
    let tx_fee = all_in
        .checked_sub(all_out)
        .ok_or(ProcessError::CapacityInsufficient)?;
    if tx_fee < aggregate_debit {
        return Err(ProcessError::CapacityInsufficient.into());
    }
    Ok(None)
}

fn sum_capacity_all(source: Source) -> core::result::Result<u64, ProcessError> {
    let mut sum = 0u64;
    for i in 0.. {
        match load_cell_capacity(i, source) {
            Ok(c) => {
                sum = sum
                    .checked_add(c)
                    .ok_or(ProcessError::CapacityInsufficient)?;
            }
            Err(ckb_std::error::SysError::IndexOutOfBound) => break,
            Err(_) => return Err(ProcessError::ProcessPairInvalid),
        }
    }
    Ok(sum)
}

fn sum_capacity_matching_lock(
    source: Source,
    code_hash: &ckb_std::ckb_types::packed::Byte32,
    hash_type: ckb_std::ckb_types::packed::Byte,
) -> core::result::Result<u64, ProcessError> {
    let mut sum = 0u64;
    for i in 0.. {
        match load_cell_lock(i, source) {
            Ok(lock) => {
                if &lock.code_hash() == code_hash && lock.hash_type() == hash_type {
                    let c = load_cell_capacity(i, source)
                        .map_err(|_| ProcessError::ProcessPairInvalid)?;
                    sum = sum
                        .checked_add(c)
                        .ok_or(ProcessError::CapacityInsufficient)?;
                }
            }
            Err(ckb_std::error::SysError::IndexOutOfBound) => break,
            Err(_) => return Err(ProcessError::ProcessPairInvalid),
        }
    }
    Ok(sum)
}

fn require_type_id_type(index: usize, source: Source) -> Result<[u8; 32]> {
    let typ = load_cell_type(index, source).map_err(|_| ProcessError::ProcessPairInvalid)?;
    let Some(typ) = typ else {
        return Err(ProcessError::TypeIdInvalid.into());
    };
    let code_hash: [u8; 32] = typ.code_hash().unpack();
    let hash_type: u8 = typ.hash_type().into();
    if !is_type_id_script(&code_hash, hash_type) {
        return Err(ProcessError::TypeIdInvalid.into());
    }
    Ok(typ.calc_script_hash().unpack())
}

/// Our Process Type ID must only sit on Process-lock cells (the recreate).
fn forbid_type_id_on_non_process_outputs(our_type_hash: &[u8; 32]) -> Result<()> {
    let process_script = load_script().map_err(|_| ProcessError::ProcessPairInvalid)?;
    for i in 0.. {
        match load_cell_type(i, Source::Output) {
            Ok(Some(typ)) => {
                let hash: [u8; 32] = typ.calc_script_hash().unpack();
                if &hash == our_type_hash {
                    let lock = load_cell_lock(i, Source::Output)
                        .map_err(|_| ProcessError::ProcessPairInvalid)?;
                    if !is_example_process_lock(&lock, &process_script) {
                        return Err(ProcessError::TypeIdInvalid.into());
                    }
                }
            }
            Ok(None) => {}
            Err(ckb_std::error::SysError::IndexOutOfBound) => break,
            Err(_) => return Err(ProcessError::ProcessPairInvalid.into()),
        }
    }
    Ok(())
}

/// One uniquely owned output: a DigestOk payload payment or a DigestIncompatible
/// refund. Refunds use `user_lock`, never the payload beneficiary.
#[derive(Clone)]
struct OutputObligation {
    lock: Script,
    capacity: u64,
    data: Vec<u8>,
    type_script: Option<Script>,
}

fn verify_intention_batch_and_beneficiaries() -> Result<Option<&'static str>> {
    let (ours, obligations) = collect_output_obligations()?;
    if !(BATCH_MIN..=BATCH_MAX).contains(&ours.len()) {
        return Err(ProcessError::BatchCountInvalid.into());
    }
    // Transaction-wide unique ownership: DigestOk payments (any Process pair
    // in this tx) and DigestIncompatible / TimeoutBurn refunds cannot share
    // an output.
    verify_unique_outputs(&obligations)?;
    Ok(None)
}

fn collect_output_obligations(
) -> core::result::Result<(Vec<OutputObligation>, Vec<OutputObligation>), ProcessError> {
    // Intention `process_type_hash` is the TYPE_ID type-script hash of this
    // Process state cell — not the Process lock script hash.
    // Scoping matches bd11d44: `ours_prefix` only (no CID Type-ID ownership
    // re-check — that is Sequencer-side via cid-intention-filter).
    let our_type_hash = our_process_type_id()?;
    let digest_ok_hashes = type_hashes_in_both_io()?;

    let mut ours = Vec::new();
    let mut obligations = Vec::new();
    for i in 0.. {
        let lock = match load_cell_lock(i, Source::Input) {
            Ok(lock) => lock,
            Err(ckb_std::error::SysError::IndexOutOfBound) => break,
            Err(_) => return Err(ProcessError::ProcessPairInvalid),
        };

        let args = lock.args().raw_data();
        if args.len() <= PROCESS_TYPE_HASH_LEN {
            continue;
        }
        let Ok(cid) = CidArgs::decode(&args) else {
            continue;
        };
        let ours_prefix = cid.process_type_hash == our_type_hash;
        let digest_ok = digest_ok_hashes.iter().any(|h| h == &cid.process_type_hash);

        if digest_ok {
            if ours_prefix {
                // Shared business classify (deposit / asset / digest) — same SoT
                // as assemble. Ownership is not re-checked here. Pass Process
                // lock code_hash so a Process-binary beneficiary is rejected
                // (claim_output would skip it as a recreate cell).
                let capacity = load_cell_capacity(i, Source::Input)
                    .map_err(|_| ProcessError::ProcessPairInvalid)?;
                let data = load_cell_data(i, Source::Input)
                    .map_err(|_| ProcessError::ProcessPairInvalid)?;
                let type_script = load_cell_type(i, Source::Input)
                    .map_err(|_| ProcessError::ProcessPairInvalid)?;
                let process_script = load_script().map_err(|_| ProcessError::ProcessPairInvalid)?;
                let process_code: [u8; 32] = process_script.code_hash().unpack();
                let reason = classify_live_intention_excluding(
                    &lock,
                    capacity,
                    type_script.as_ref(),
                    &data,
                    Some(&process_code),
                );
                if reason != FILTER_ACCEPTED {
                    return Err(filter_reason_to_process_error(reason)
                        .unwrap_or(ProcessError::IncompatibleBatch));
                }
                let transfer = match parse_payment(i, &cid)? {
                    Some(t) => t,
                    None => return Err(ProcessError::BeneficiaryMissing),
                };
                ours.push(transfer.clone());
                obligations.push(transfer);
            } else {
                match parse_payment(i, &cid)? {
                    Some(t) => obligations.push(t),
                    None => continue,
                }
            }
        } else {
            // Refund branch (DigestIncompatible and TimeoutBurn): unique-assign
            // the user_lock output, never the payload. TimeoutBurn still requires
            // a full (exact capacity) refund. Shaved DigestIncompatible refunds
            // must not be co-bundled with DigestOk; CID enforces that separately.
            let Ok(user) = packed_script(&cid.entry.user_lock) else {
                continue;
            };
            obligations.push(parse_transfer(i, user)?);
        }
    }

    Ok((ours, obligations))
}

/// TYPE_ID type-script hash of this Process state cell (lock group).
fn our_process_type_id() -> core::result::Result<[u8; 32], ProcessError> {
    let typ = load_cell_type(0, Source::GroupInput)
        .or_else(|_| load_cell_type(0, Source::GroupOutput))
        .map_err(|_| ProcessError::ProcessPairInvalid)?;
    let Some(typ) = typ else {
        return Err(ProcessError::TypeIdInvalid);
    };
    Ok(typ.calc_script_hash().unpack())
}

fn type_hashes_in_both_io() -> core::result::Result<Vec<[u8; 32]>, ProcessError> {
    let input_hashes = type_hashes(Source::Input)?;
    let output_hashes = type_hashes(Source::Output)?;
    Ok(input_hashes
        .into_iter()
        .filter(|h| output_hashes.iter().any(|o| o == h))
        .collect())
}

fn type_hashes(source: Source) -> core::result::Result<Vec<[u8; 32]>, ProcessError> {
    let mut hashes = Vec::new();
    for i in 0.. {
        match load_cell_type(i, source) {
            Ok(Some(type_script)) => {
                let hash: [u8; 32] = type_script.calc_script_hash().unpack();
                if !hashes.contains(&hash) {
                    hashes.push(hash);
                }
            }
            Ok(None) => {}
            Err(ckb_std::error::SysError::IndexOutOfBound) => break,
            Err(_) => return Err(ProcessError::ProcessPairInvalid),
        }
    }
    Ok(hashes)
}

fn parse_payment(
    index: usize,
    cid: &CidArgs,
) -> core::result::Result<Option<OutputObligation>, ProcessError> {
    let Ok(beneficiary_cid) = cid.entry.payload.to_script() else {
        return Ok(None);
    };
    let lock = packed_script(&beneficiary_cid)?;
    Ok(Some(parse_transfer(index, lock)?))
}

fn parse_transfer(
    index: usize,
    lock: Script,
) -> core::result::Result<OutputObligation, ProcessError> {
    let capacity =
        load_cell_capacity(index, Source::Input).map_err(|_| ProcessError::ProcessPairInvalid)?;
    let data =
        load_cell_data(index, Source::Input).map_err(|_| ProcessError::ProcessPairInvalid)?;
    let type_script =
        load_cell_type(index, Source::Input).map_err(|_| ProcessError::ProcessPairInvalid)?;
    validate_supported_asset(&type_script, &data)?;
    Ok(OutputObligation {
        lock,
        capacity,
        data,
        type_script,
    })
}

fn validate_supported_asset(
    type_script: &Option<Script>,
    data: &[u8],
) -> core::result::Result<(), ProcessError> {
    match type_script {
        None => Ok(()),
        Some(type_script) => {
            let code_hash: [u8; 32] = type_script.code_hash().unpack();
            let hash_type: u8 = type_script.hash_type().into();
            if !is_standard_xudt(&code_hash, hash_type) {
                return Err(ProcessError::UnsupportedAsset);
            }
            if data.len() < XUDT_AMOUNT_LEN {
                return Err(ProcessError::UnsupportedAsset);
            }
            Ok(())
        }
    }
}

fn verify_unique_outputs(obligations: &[OutputObligation]) -> Result<Option<&'static str>> {
    let process_script = load_script().map_err(|_| ProcessError::ProcessPairInvalid)?;
    let output_count = QueryIter::new(load_cell_lock, Source::Output).count();
    let mut claimed = vec![false; output_count];

    for obligation in obligations {
        claim_output(obligation, &process_script, &mut claimed)?;
    }

    Ok(None)
}

fn claim_output(
    obligation: &OutputObligation,
    process_script: &Script,
    claimed: &mut [bool],
) -> core::result::Result<(), ProcessError> {
    let mut lock_seen = false;

    for (i, claimed_flag) in claimed.iter_mut().enumerate() {
        if *claimed_flag {
            continue;
        }

        let lock =
            load_cell_lock(i, Source::Output).map_err(|_| ProcessError::ProcessPairInvalid)?;
        let type_script =
            load_cell_type(i, Source::Output).map_err(|_| ProcessError::ProcessPairInvalid)?;
        // Skip Process recreate cells (this binary as lock, any hash_type alias).
        if is_example_process_lock(&lock, process_script) {
            continue;
        }
        if lock != obligation.lock {
            continue;
        }

        lock_seen = true;
        let capacity =
            load_cell_capacity(i, Source::Output).map_err(|_| ProcessError::ProcessPairInvalid)?;
        let data =
            load_cell_data(i, Source::Output).map_err(|_| ProcessError::ProcessPairInvalid)?;
        if capacity == obligation.capacity
            && data.as_slice() == obligation.data.as_slice()
            && type_script == obligation.type_script
        {
            *claimed_flag = true;
            return Ok(());
        }
    }

    if lock_seen {
        Err(ProcessError::BeneficiaryAmountMismatch)
    } else {
        Err(ProcessError::BeneficiaryMissing)
    }
}

/// Skip Process recreate cells of this binary (any hash_type alias).
fn is_example_process_lock(lock: &Script, ours: &Script) -> bool {
    lock.code_hash() == ours.code_hash()
}

fn packed_script(script: &CidScript) -> core::result::Result<Script, ProcessError> {
    // Same hash_type allow-list as `classify_intention` / assemble.
    if !is_supported_script_hash_type(script.hash_type) {
        return Err(ProcessError::BeneficiaryMissing);
    }
    let hash_type = match script.hash_type {
        0 => ScriptHashType::Data,
        1 => ScriptHashType::Type,
        2 => ScriptHashType::Data1,
        4 => ScriptHashType::Data2,
        _ => return Err(ProcessError::BeneficiaryMissing),
    };
    Ok(Script::new_builder()
        .code_hash(script.code_hash.pack())
        .hash_type(hash_type)
        .args(script.args.pack())
        .build())
}
