use ckb_cinnabar_verifier::{Result, Verification};
use crate::Context;

#[derive(Default)]
pub struct Root;

impl Verification<Context> for Root {
    fn verify(&mut self, _name: &str, _ctx: &mut Context) -> Result<Option<&str>> {
        // On-chain verification (runs in transaction context)
        // Will implement in Task 6
        Ok(None)
    }
}
