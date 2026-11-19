//! Offline CKB-only Intention / Process create + env-key signing, plus explicit testnet send.

mod build;
mod hexutil;
mod process_build;
mod request;
mod send;

pub use build::{
    create_signed_intention, create_signed_intention_bundle,
    create_signed_intention_with_same_lock_decoy, load_privkey_from_env, parse_privkey,
    process_type_hash_of, CliError, SignedIntentionBundle, PRIVKEY_ENV,
};
pub use process_build::{
    create_signed_process_state, create_signed_process_state_bundle, SignedProcessBundle,
};
pub use request::{CreateProcessRequest, CreateRequest, ScriptJson};
pub use send::{send_signed_transaction, SendOptions};
