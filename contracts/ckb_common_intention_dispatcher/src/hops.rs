use ckb_cinnabar_verifier::{
    re_exports::ckb_std::debug,
    Result, Verification,
};
use cid_protocol::TIMEOUT_BLOCKS;

use crate::{
    context::Context,
    error::CidError,
    utils::{load_judge_block_number, load_pack_block_number, refund_assets_match},
};

#[derive(Default)]
pub struct DigestOk;

impl Verification<Context> for DigestOk {
    fn verify(&mut self, name: &str, ctx: &mut Context) -> Result<Option<&str>> {
        debug!("Entering [{name}]");
        if !ctx.process_matched {
            return Err(CidError::ProcessMismatch.into());
        }
        Ok(None)
    }
}

#[derive(Default)]
pub struct DigestIncompatible;

impl Verification<Context> for DigestIncompatible {
    fn verify(&mut self, name: &str, ctx: &mut Context) -> Result<Option<&str>> {
        debug!("Entering [{name}]");
        let refund = ctx.refund_output.ok_or(CidError::RefundMissing)?;
        let asset = ctx.asset.as_ref().ok_or(CidError::ZeroAsset)?;
        if !refund_assets_match(refund, asset)? {
            return Err(CidError::RefundAssetMismatch.into());
        }
        Ok(None)
    }
}

#[derive(Default)]
pub struct TimeoutBurn;

impl Verification<Context> for TimeoutBurn {
    fn verify(&mut self, name: &str, ctx: &mut Context) -> Result<Option<&str>> {
        debug!("Entering [{name}]");
        if !ctx.user_lock_in_inputs {
            return Err(CidError::UnknownBranch.into());
        }
        let refund = ctx.refund_output.ok_or(CidError::RefundMissing)?;
        let asset = ctx.asset.as_ref().ok_or(CidError::ZeroAsset)?;
        if !refund_assets_match(refund, asset)? {
            return Err(CidError::RefundAssetMismatch.into());
        }
        let pack = load_pack_block_number()?;
        let judge = load_judge_block_number()?;
        if judge < pack || judge - pack < TIMEOUT_BLOCKS {
            return Err(CidError::TimeoutNotReached.into());
        }
        Ok(None)
    }
}
