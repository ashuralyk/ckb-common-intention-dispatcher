#![no_std]
#![no_main]

mod ssri;
mod verify;

use ckb_cinnabar_verifier::{cinnabar_main, define_errors, CUSTOM_ERROR_START, TREE_ROOT};

const ERROR_PROCESS_PAIR_INVALID: i8 = 30;
const ERROR_BATCH_COUNT_INVALID: i8 = 31;
const ERROR_BENEFICIARY_MISSING: i8 = 32;
const ERROR_BENEFICIARY_AMOUNT_MISMATCH: i8 = 33;
const ERROR_UNSUPPORTED_ASSET: i8 = 34;
const ERROR_TYPE_ID_INVALID: i8 = 35;

define_errors!(ProcessError, {
    ReadFailed = CUSTOM_ERROR_START,  // 20
    IncompatibleBatch,                // 21
    ProcessPairInvalid = ERROR_PROCESS_PAIR_INVALID,    // 30
    BatchCountInvalid = ERROR_BATCH_COUNT_INVALID,      // 31
    BeneficiaryMissing = ERROR_BENEFICIARY_MISSING,     // 32
    BeneficiaryAmountMismatch = ERROR_BENEFICIARY_AMOUNT_MISMATCH, // 33
    UnsupportedAsset = ERROR_UNSUPPORTED_ASSET,         // 34
    TypeIdInvalid = ERROR_TYPE_ID_INVALID,              // 35
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
