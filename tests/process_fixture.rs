#![no_main]
#![no_std]

//! Test-only executable Process **lock**. Reject a forbidden recreated state to
//! prove that matching identity cannot bypass Process validation. Not production.

use ckb_std::{ckb_constants::Source, high_level::load_cell_data};

ckb_std::entry!(main);
ckb_std::default_alloc!();

fn main() -> i8 {
    // Prefer GroupOutput (recreate); fall back to scanning Output for the same
    // lock-script group semantics some simulators expose unevenly.
    if let Ok(data) = load_cell_data(0, Source::GroupOutput) {
        return if data.as_slice() == b"reject" { 42 } else { 0 };
    }
    // Fallback: first Output cell (GroupOutput unavailable). Same outcomes as
    // scanning from index 0 — Ok always returns, so a loop never iterates.
    match load_cell_data(0, Source::Output) {
        Ok(data) if data.as_slice() == b"reject" => 42,
        Ok(_) => 0,
        Err(ckb_std::error::SysError::IndexOutOfBound) => 42,
        Err(_) => 42,
    }
}
