use cid_protocol::CidArgs;

use crate::utils::IntentionAsset;

/// Shared parse results filled by Root; hop nodes only read this.
#[derive(Default)]
pub struct Context {
    pub args: Option<CidArgs>,
    pub user_lock_hash: [u8; 32],
    pub user_lock_in_inputs: bool,
    pub refund_output: Option<usize>,
    pub process_matched: bool,
    pub asset: Option<IntentionAsset>,
}
