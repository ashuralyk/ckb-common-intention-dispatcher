#![no_std]
#![no_main]

mod deposit;
mod host;
mod ssri;
mod verify;

pub use deposit::MIN_DEPOSIT_CAPACITY;

use ckb_cinnabar_verifier::{cinnabar_main, define_errors, CUSTOM_ERROR_START, TREE_ROOT};

const ERROR_PROCESS_PAIR_INVALID: i8 = 30;
const ERROR_BATCH_COUNT_INVALID: i8 = 31;
const ERROR_BENEFICIARY_MISSING: i8 = 32;
const ERROR_BENEFICIARY_AMOUNT_MISMATCH: i8 = 33;
const ERROR_UNSUPPORTED_ASSET: i8 = 34;
const ERROR_TYPE_ID_INVALID: i8 = 35;
const ERROR_HOST_QUERY_FAILED: i8 = 36;
const ERROR_CAPACITY_INSUFFICIENT: i8 = 37;
const ERROR_UNSUPPORTED_FUNDING_LOCK: i8 = 38;
const ERROR_DEPOSIT_BELOW_MINIMUM: i8 = 39;

define_errors!(ProcessError, {
    ReadFailed = CUSTOM_ERROR_START,  // 20
    IncompatibleBatch,                // 21
    ProcessPairInvalid = ERROR_PROCESS_PAIR_INVALID,    // 30
    BatchCountInvalid = ERROR_BATCH_COUNT_INVALID,      // 31
    BeneficiaryMissing = ERROR_BENEFICIARY_MISSING,     // 32
    BeneficiaryAmountMismatch = ERROR_BENEFICIARY_AMOUNT_MISMATCH, // 33
    UnsupportedAsset = ERROR_UNSUPPORTED_ASSET,         // 34
    TypeIdInvalid = ERROR_TYPE_ID_INVALID,              // 35
    HostQueryFailed = ERROR_HOST_QUERY_FAILED,          // 36
    CapacityInsufficient = ERROR_CAPACITY_INSUFFICIENT, // 37
    // The example Process only spends its state and Intention inputs.
    UnsupportedFundingLock = ERROR_UNSUPPORTED_FUNDING_LOCK, // 38
    // CKB deposit Intention capacity below Process MIN_DEPOSIT_CAPACITY.
    DepositBelowMinimum = ERROR_DEPOSIT_BELOW_MINIMUM, // 39
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
