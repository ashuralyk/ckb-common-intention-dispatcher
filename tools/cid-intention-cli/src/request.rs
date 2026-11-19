use serde::{Deserialize, Serialize};

use crate::build::CliError;
use crate::hexutil::{decode_hash32, decode_hex};

/// Offline Intention create request (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRequest {
    /// `testnet` or `fake` (unit tests). Mainnet rejected in v1.
    pub network: String,
    pub fee_shannons: u64,
    /// Requested Intention capacity (shannons). Defaults to the example Process
    /// deposit minimum. Effective capacity is `max(requested, occupied)` and
    /// must meet the example Process minimum (protocol has no Intention floor).
    #[serde(default = "default_intention_capacity")]
    pub intention_capacity: u64,
    pub funding: FundingCell,
    /// Live CID lock **code** cell. Must include Type ID `type_script` so the
    /// Intention lock resolves as `hash_type=type` (no data-hash fallback).
    pub cid_lock_dep: NamedDep,
    /// Genesis secp code + data cell deps (with binary `data` for offline signing).
    pub sighash_deps: Vec<NamedDep>,
    /// Process Type ID: the built-in TYPE_ID **type** script of the live Process
    /// state cell (`code_hash = TYPE_ID`, `hash_type = type`, unique args).
    /// Intention args commit `calc_script_hash` of this script.
    pub process_type_script: ScriptJson,
    /// Beneficiary lock encoded into `CidEntry.payload` via `Payload::from_script`.
    pub beneficiary_lock: ScriptJson,
}

/// Offline Process state cell create request (no secrets).
///
/// Produces a state cell whose TYPE_ID type-script hash is the Intention
/// `process_type_hash` (`identity_type_id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProcessRequest {
    /// `testnet` or `fake` (unit tests). Mainnet rejected in v1.
    pub network: String,
    /// Process state capacity in shannons. Default: 300 CKB when omitted.
    #[serde(default = "default_process_capacity")]
    pub process_capacity: u64,
    /// Distinct Process lock args for this instance (must be non-empty).
    pub process_lock_args: String,
    pub funding: FundingCell,
    /// Live CID lock **code** cell (Type ID required; live-verified on `send`).
    pub cid_lock_dep: NamedDep,
    /// Live Process **code** cell. Must include Type ID `type_script` so the
    /// state lock resolves as `hash_type=type` (no data-hash fallback).
    pub process_code_dep: NamedDep,
    /// Genesis secp code + data cell deps (with binary `data` for offline signing).
    pub sighash_deps: Vec<NamedDep>,
}

fn default_process_capacity() -> u64 {
    cid_process_example_calculator::DEFAULT_PROCESS_STATE_CAPACITY
}

fn default_intention_capacity() -> u64 {
    // Example Process deposit min (600 CKB). Protocol has no Intention floor.
    cid_process_example_calculator::MIN_DEPOSIT_CAPACITY
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingCell {
    pub tx_hash: String,
    pub index: u32,
    pub capacity: u64,
    pub lock: ScriptJson,
    #[serde(default = "default_empty_hex")]
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedDep {
    pub name: String,
    pub tx_hash: String,
    pub index: u32,
    /// Cell data hex; required so the dep cell (and offline signing) can be seeded.
    pub data: String,
    /// Type script of the dep cell. **Required** on `cid_lock_dep` (Type ID);
    /// optional on sighash deps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_script: Option<ScriptJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptJson {
    pub code_hash: String,
    pub hash_type: String,
    pub args: String,
}

fn default_empty_hex() -> String {
    "0x".into()
}

impl ScriptJson {
    pub fn parse_hash_type(&self) -> Result<u8, CliError> {
        match self.hash_type.to_ascii_lowercase().as_str() {
            "data" => Ok(0),
            "type" => Ok(1),
            "data1" => Ok(2),
            "data2" => Ok(4),
            other => Err(CliError::InvalidRequest(format!(
                "unknown hash_type `{other}`"
            ))),
        }
    }

    pub fn to_cid_script(&self) -> Result<cid_protocol::Script, CliError> {
        Ok(cid_protocol::Script {
            code_hash: decode_hash32(&self.code_hash)?,
            hash_type: self.parse_hash_type()?,
            args: decode_hex(&self.args)?,
        })
    }
}
