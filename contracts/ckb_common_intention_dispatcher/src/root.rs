use ckb_cinnabar_verifier::{
    re_exports::ckb_std::{
        ckb_constants::Source,
        debug,
        high_level::load_cell_data,
    },
    this_script_args, this_script_count, this_script_pattern, Result, ScriptPattern, ScriptPlace,
    Verification,
};
use cid_protocol::{HOP_DIGEST_INCOMPATIBLE, HOP_DIGEST_OK, HOP_TIMEOUT_BURN};

use crate::{
    context::Context,
    error::CidError,
    utils::{
        find_output_by_lock_hash, has_input_lock_hash, has_matching_process_celldep,
        has_typed_celldep, load_intention_asset, packed_script, parse_args, script_hash_bytes,
    },
};

/// Classify a CID lock spend. Create never reaches this script.
#[derive(Default)]
pub struct Root;

impl Verification<Context> for Root {
    fn verify(&mut self, name: &str, ctx: &mut Context) -> Result<Option<&str>> {
        debug!("Entering [{name}]");

        let raw_args = this_script_args()?;
        let args = parse_args(&raw_args)?;
        let user_lock = packed_script(&args.entry.user_lock)?;
        ctx.user_lock_hash = script_hash_bytes(&user_lock);
        ctx.user_lock_in_inputs = has_input_lock_hash(&ctx.user_lock_hash);
        ctx.refund_output = find_output_by_lock_hash(&ctx.user_lock_hash);
        ctx.process_matched = has_matching_process_celldep(&args.process_type_hash);
        ctx.args = Some(args);

        match this_script_pattern(ScriptPlace::Lock)? {
            ScriptPattern::Create => return Err(CidError::UnknownBranch.into()),
            ScriptPattern::Transfer | ScriptPattern::Burn => {}
        }

        let inputs = this_script_count(Source::Input, ScriptPlace::Lock)?;
        let outputs = this_script_count(Source::Output, ScriptPlace::Lock)?;
        if inputs != 1 || outputs != 0 {
            return Err(CidError::PartialDigest.into());
        }

        // Intention data is empty on the CKB path; xUDT uses the standard 16-byte amount.
        let _ = load_cell_data(0, Source::GroupInput)?;
        ctx.asset = Some(load_intention_asset(0, Source::GroupInput)?);

        if ctx.refund_output.is_some() && !ctx.user_lock_in_inputs {
            return Ok(Some(HOP_DIGEST_INCOMPATIBLE));
        }
        if ctx.process_matched {
            return Ok(Some(HOP_DIGEST_OK));
        }
        if ctx.user_lock_in_inputs {
            return Ok(Some(HOP_TIMEOUT_BURN));
        }
        if has_typed_celldep() {
            return Err(CidError::ProcessMismatch.into());
        }
        Err(CidError::UnknownBranch.into())
    }
}
