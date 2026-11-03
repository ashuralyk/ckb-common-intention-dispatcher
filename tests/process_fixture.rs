#![no_main]
#![no_std]

//! Test-only executable Process. Reject a forbidden recreated state to prove
//! that matching identity cannot bypass Process validation. Not production code.

use ckb_std::{ckb_constants::Source, high_level::load_cell_data};

ckb_std::entry!(main);
ckb_std::default_alloc!();

fn main() -> i8 {
    match load_cell_data(0, Source::GroupOutput) {
        Ok(data) if data.as_slice() != b"reject" => 0,
        _ => 42,
    }
}
