use crate::build::CliError;

pub fn decode_hex(s: &str) -> Result<Vec<u8>, CliError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        return Err(CliError::InvalidHex("odd length".into()));
    }
    hex::decode(s).map_err(|e| CliError::InvalidHex(e.to_string()))
}

pub fn decode_hash32(s: &str) -> Result<[u8; 32], CliError> {
    let v = decode_hex(s)?;
    v.try_into()
        .map_err(|_| CliError::InvalidHex("expected 32 bytes".into()))
}

pub fn encode_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}
