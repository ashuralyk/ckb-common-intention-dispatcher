use ckb_cinnabar_verifier::{
    re_exports::ckb_std::{
        ckb_constants::Source,
        ckb_types::{
            core::ScriptHashType,
            packed::Script,
            prelude::{Builder, Entity, Pack, Unpack},
        },
        error::SysError,
        high_level::{
            load_cell_capacity, load_cell_data, load_cell_lock_hash,
            load_cell_occupied_capacity, load_cell_type, load_header, QueryIter,
        },
    },
    Result,
};
use cid_protocol::{is_standard_xudt, CidArgs, Script as CidScript, XUDT_AMOUNT_LEN};

use crate::error::CidError;

pub fn packed_script(script: &CidScript) -> Result<Script> {
    let hash_type = match script.hash_type {
        0 => ScriptHashType::Data,
        1 => ScriptHashType::Type,
        2 => ScriptHashType::Data1,
        _ => return Err(CidError::ArgsInvalid.into()),
    };
    Ok(Script::new_builder()
        .code_hash(script.code_hash.pack())
        .hash_type(hash_type)
        .args(script.args.pack())
        .build())
}

pub fn script_hash_bytes(script: &Script) -> [u8; 32] {
    script.calc_script_hash().unpack()
}

pub fn has_input_lock_hash(lock_hash: &[u8; 32]) -> bool {
    QueryIter::new(load_cell_lock_hash, Source::Input).any(|hash| &hash == lock_hash)
}

pub fn find_output_by_lock_hash(lock_hash: &[u8; 32]) -> Option<usize> {
    QueryIter::new(load_cell_lock_hash, Source::Output)
        .enumerate()
        .find_map(|(i, hash)| if &hash == lock_hash { Some(i) } else { None })
}

pub fn has_matching_process_celldep(process_type_hash: &[u8; 32]) -> bool {
    let mut i = 0;
    loop {
        match load_cell_type(i, Source::CellDep) {
            Ok(Some(type_script)) => {
                let hash: [u8; 32] = type_script.calc_script_hash().unpack();
                if &hash == process_type_hash {
                    return true;
                }
                i += 1;
            }
            Ok(None) => i += 1,
            Err(SysError::IndexOutOfBound) => return false,
            Err(_) => return false,
        }
    }
}

pub fn has_typed_celldep() -> bool {
    let mut i = 0;
    loop {
        match load_cell_type(i, Source::CellDep) {
            Ok(Some(_)) => return true,
            Ok(None) => i += 1,
            Err(SysError::IndexOutOfBound) => return false,
            Err(_) => return false,
        }
    }
}

pub fn load_pack_block_number() -> Result<u64> {
    let header = load_header(0, Source::GroupInput).map_err(|_| CidError::HeaderMissing)?;
    Ok(header.raw().number().unpack())
}

pub fn load_judge_block_number() -> Result<u64> {
    let mut max = None;
    let mut i = 0;
    loop {
        match load_header(i, Source::HeaderDep) {
            Ok(header) => {
                let number: u64 = header.raw().number().unpack();
                max = Some(max.map_or(number, |m: u64| m.max(number)));
                i += 1;
            }
            Err(SysError::IndexOutOfBound) => break,
            Err(_) => return Err(CidError::HeaderMissing.into()),
        }
    }
    max.ok_or_else(|| CidError::HeaderMissing.into())
}

pub fn parse_xudt_amount(data: &[u8]) -> Result<u128> {
    if data.len() < XUDT_AMOUNT_LEN {
        return Err(CidError::UnsupportedType.into());
    }
    let mut buf = [0u8; XUDT_AMOUNT_LEN];
    buf.copy_from_slice(&data[..XUDT_AMOUNT_LEN]);
    Ok(u128::from_le_bytes(buf))
}

pub fn load_intention_asset(index: usize, source: Source) -> Result<IntentionAsset> {
    let capacity = load_cell_capacity(index, source)?;
    let occupied = load_cell_occupied_capacity(index, source)?;
    let data = load_cell_data(index, source)?;
    match load_cell_type(index, source)? {
        None => {
            if data.is_empty() && capacity > occupied {
                Ok(IntentionAsset::Ckb { capacity })
            } else if !data.is_empty() {
                Err(CidError::UnsupportedType.into())
            } else {
                Err(CidError::ZeroAsset.into())
            }
        }
        Some(type_script) => {
            let code_hash: [u8; 32] = type_script.code_hash().unpack();
            let hash_type: u8 = type_script.hash_type().into();
            if !is_standard_xudt(&code_hash, hash_type) {
                return Err(CidError::UnsupportedType.into());
            }
            let amount = parse_xudt_amount(&data)?;
            if amount == 0 {
                return Err(CidError::ZeroAsset.into());
            }
            Ok(IntentionAsset::Xudt {
                amount,
                type_hash: type_script.calc_script_hash().unpack(),
            })
        }
    }
}

#[derive(Clone, Debug)]
pub enum IntentionAsset {
    Ckb { capacity: u64 },
    Xudt { amount: u128, type_hash: [u8; 32] },
}

pub fn refund_assets_match(index: usize, expected: &IntentionAsset) -> Result<bool> {
    let capacity = load_cell_capacity(index, Source::Output)?;
    let data = load_cell_data(index, Source::Output)?;
    let type_script = load_cell_type(index, Source::Output)?;
    match (expected, type_script) {
        (IntentionAsset::Ckb { capacity: want }, None) => Ok(capacity == *want && data.is_empty()),
        (IntentionAsset::Xudt { amount, type_hash }, Some(type_script)) => {
            let got_hash: [u8; 32] = type_script.calc_script_hash().unpack();
            if &got_hash != type_hash {
                return Ok(false);
            }
            let got = parse_xudt_amount(&data)?;
            Ok(got == *amount)
        }
        _ => Ok(false),
    }
}

pub fn parse_args(raw: &[u8]) -> Result<CidArgs> {
    CidArgs::decode(raw).map_err(|_| CidError::ArgsInvalid.into())
}
