//! Asset field preflight for the CLI's `create` JSON input.

use std::fmt;

use serde::{
    de::{DeserializeSeed, Error, IgnoredAny, MapAccess, SeqAccess, Visitor},
    Deserializer,
};

pub(super) fn preflight_create_request(text: &str) -> Result<(), serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    match text.trim_start().chars().next() {
        Some('{') => deserializer.deserialize_map(AssetFields::Request)?,
        Some('[') => deserializer.deserialize_seq(AssetFields::Request)?,
        _ => return Ok(()),
    }
    deserializer.end()
}

#[derive(Clone, Copy)]
enum AssetFields {
    Request,
    Funding,
}

impl<'de> DeserializeSeed<'de> for AssetFields {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for AssetFields {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a funding object or positional array")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if matches!(self, Self::Request) && key == "funding" {
                map.next_value_seed(Self::Funding)?;
            } else {
                self.check_key::<M::Error>(&key)?;
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        if matches!(self, Self::Request) {
            // CreateRequest's positional form puts funding after four fields.
            for _ in 0..4 {
                if seq.next_element::<IgnoredAny>()?.is_none() {
                    return Ok(());
                }
            }
            seq.next_element_seed(Self::Funding)?;
        }
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }
}

impl AssetFields {
    fn check_key<E: Error>(self, key: &str) -> Result<(), E> {
        let normalized: String = normalize_field_name(key).collect();
        if !is_asset_field(&normalized) {
            return Ok(());
        }

        let canonical = match (self, normalized.as_str()) {
            (Self::Request, "intentioncapacity") => Some("intention_capacity"),
            (Self::Funding, "capacity") => Some("capacity"),
            (Self::Funding, "data") => Some("data"),
            _ => None,
        };
        if canonical == Some(key) {
            return Ok(());
        }

        let prefix = match self {
            Self::Request => "",
            Self::Funding => "funding.",
        };
        let key = escaped_field_name(key);
        let hint = canonical
            .map(|field| format!(" (use `{prefix}{field}`)"))
            .unwrap_or_default();
        Err(E::custom(format!(
            "unsupported asset field `{prefix}{key}`{hint}"
        )))
    }
}

fn normalize_field_name(key: &str) -> impl Iterator<Item = char> + '_ {
    key.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
}

fn is_asset_field(normalized: &str) -> bool {
    // Exact normalized names shared by the request and funding objects.
    // See docs/cid-intention-cli.md for the same names in snake_case.
    matches!(
        normalized,
        // Capacity and cell output fields.
        "capacity"
            | "cellcapacity"
            | "intentioncapacity"
            | "outputcapacity"
            | "output"
            | "outputs"
            | "celloutput"
            | "data"
            | "celldata"
            | "intentiondata"
            | "outputdata"
            | "outputsdata"
            | "rawoutputsdata"
            | "type"
            | "typescript"
            | "celltype"
            | "celltypescript"
            | "intentiontype"
            | "intentiontypescript"
            | "outputtype"
            | "outputtypescript"
            // Amounts and asset/token fields.
            | "amount"
            | "totalamount"
            | "asset"
            | "assets"
            | "assetamount"
            | "assetdata"
            | "assettype"
            | "assettypescript"
            | "token"
            | "tokens"
            | "tokenamount"
            | "tokenbalance"
            | "tokendata"
            | "tokentype"
            | "tokentypescript"
            // UDT, sUDT and xUDT fields.
            | "udt"
            | "udtamount"
            | "udtdata"
            | "udttype"
            | "udtscript"
            | "udttypescript"
            | "sudt"
            | "sudtamount"
            | "sudtdata"
            | "sudttype"
            | "sudtscript"
            | "sudttypescript"
            | "xudt"
            | "xudtamount"
            | "xudtdata"
            | "xudttype"
            | "xudtscript"
            | "xudttypescript"
            | "xudtargs"
            | "xudtcodehash"
            | "xudthashtype"
            | "xudttypeargs"
            | "xudttypehash"
            | "inputxudtargs"
    )
}

fn escaped_field_name(key: &str) -> String {
    const MAX_ESCAPED_LEN: usize = 48;
    const ELLIPSIS: &str = "...";
    let mut escaped = String::new();
    for character in key.chars() {
        let fragment = character.escape_default();
        let escape_delimiter = character == '`';
        let fragment_len = fragment.clone().count() + usize::from(escape_delimiter);
        if escaped.len() + fragment_len > MAX_ESCAPED_LEN - ELLIPSIS.len() {
            escaped.push_str(ELLIPSIS);
            break;
        }
        if escape_delimiter {
            escaped.push('\\');
        }
        escaped.extend(fragment);
    }
    escaped
}
