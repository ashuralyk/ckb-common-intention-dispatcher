# Cinnabar Patches

This repository currently uses a locally-patched version of ckb-cinnabar to enable `AddSecp256k1SighashSignatures` to work with FakeRpc test environments.

## Required Patch

**Base**: ashuralyk/ckb-cinnabar@635e32b (includes PR #4 Data-hash ScriptId support)

**File**: `calculate/src/shell/operation/basic.rs`

**Issue**: The operation uses `DefaultTransactionDependencyProvider::new("", 0)` which attempts network RPC access even though it's not needed for secp256k1 sighash signing.

**Fix**: Use `OffchainTransactionDependencyProvider` populated with skeleton input cells instead:

```rust
// Replace this:
use ckb_sdk::{
    traits::{DefaultCellDepResolver, DefaultTransactionDependencyProvider, SecpCkbRawKeySigner},
    // ...
};
// ...
let tx_dep_provider = DefaultTransactionDependencyProvider::new("", 0);

// With this:
use ckb_sdk::{
    traits::{
        offchain_impls::OffchainTransactionDependencyProvider, DefaultCellDepResolver,
        SecpCkbRawKeySigner,
    },
    // ...
};
// ...
let mut tx_dep_provider = OffchainTransactionDependencyProvider::default();
for (idx, input_ex) in skeleton.inputs.iter().enumerate() {
    let out_point = tx.inputs().get(idx).unwrap().previous_output();
    let tx_hash: ckb_types::H256 = out_point.tx_hash().unpack();
    let index: u32 = out_point.index().unpack();
    tx_dep_provider.cells.insert(
        (tx_hash, index),
        (input_ex.output.output.clone(), input_ex.output.data.clone().into()),
    );
}
```

## Status

This patch is pending submission to upstream cinnabar. Once merged or a published version is available, update `Cargo.toml` workspace dependencies to point to it.

See the full diff in `/tmp/ckb-cinnabar-fix` on the build system.
