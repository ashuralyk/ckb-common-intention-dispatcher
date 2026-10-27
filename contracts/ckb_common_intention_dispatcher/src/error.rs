use ckb_cinnabar_verifier::{define_errors, CUSTOM_ERROR_START};

define_errors!(CidError, {
    ArgsInvalid = CUSTOM_ERROR_START,
    UnknownBranch,
    ProcessMismatch,
    PartialDigest,
    RefundMissing,
    RefundAssetMismatch,
    TimeoutNotReached,
    HeaderMissing,
    ZeroAsset,
    UnsupportedType,
});
