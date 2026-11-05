#![no_std]
#![no_main]

mod ssri;
mod verify;

use ckb_cinnabar_verifier::{cinnabar_main, define_errors, CUSTOM_ERROR_START, TREE_ROOT};

// Error code constants for verification (30-32)
const ERROR_PROCESS_PAIR_INVALID: i8 = 30;
const ERROR_BATCH_COUNT_INVALID: i8 = 31;
const ERROR_BENEFICIARY_MISSING: i8 = 32;

define_errors!(ProcessError, {
    ReadFailed = CUSTOM_ERROR_START,  // 20
    IncompatibleBatch,                // 21
    // Reserved: 22-29 for future SSRI errors
    ProcessPairInvalid = ERROR_PROCESS_PAIR_INVALID,    // 30
    BatchCountInvalid = ERROR_BATCH_COUNT_INVALID,      // 31
    BeneficiaryMissing = ERROR_BENEFICIARY_MISSING,     // 32
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
