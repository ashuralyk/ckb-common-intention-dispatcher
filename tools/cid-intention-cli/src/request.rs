use serde::{Deserialize, Serialize};

use crate::build::CliError;
use crate::hexutil::{decode_hash32, decode_hex};

/// Offline create request (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRequest {
    /// `testnet` or `fake` (unit tests). Mainnet rejected in v1.
    pub network: String,
    pub fee_shannons: u64,
    pub intention_capacity: u64,
    pub funding: FundingCell,
    /// Live CID lock **code** cell. Must include Type ID `type_script` so the
    /// Intention lock resolves as `hash_type=type` (no data-hash fallback).
    pub cid_lock_dep: NamedDep,
    /// Genesis secp code + data cell deps (with binary `data` for offline signing).
    pub sighash_deps: Vec<NamedDep>,
    /// Full Process **state** type script (not the code cell alone).
    pub process_type_script: ScriptJson,
    /// Beneficiary lock encoded into `CidEntry.payload` via `Payload::from_script`.
    pub beneficiary_lock: ScriptJson,
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
