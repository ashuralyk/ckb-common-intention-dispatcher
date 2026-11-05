#![no_std]
#![no_main]

mod ssri;
mod verify;

use ckb_cinnabar_verifier::{
    cinnabar_main, define_errors, CUSTOM_ERROR_START, TREE_ROOT,
};

define_errors!(ProcessError, {
    ReadFailed = CUSTOM_ERROR_START,  // 20
    IncompatibleBatch,                // 21
});

#[derive(Default)]
struct Context;

cinnabar_main!(
    Context,
    (TREE_ROOT, verify::Root),
    SSRI {
        "Process.batch" => ssri::batch_handler,
        "Process.assemble" => ssri::assemble_handler,
    },
);
