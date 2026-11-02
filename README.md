# Common Intention Dispatch (CID) v0.1

CID is a CKB **lock** that holds a single Intention cell until it is destroyed.

An Intention is a promise of assets (pure CKB or standard xUDT) that a later
Process / SSRI executor can propose consuming, with explicit User Lock authorization. This repository implements the **first
slice only**: the CID lock, Calculate recipes, and FakeRpc tests.

Design notes: [Common Intention Dispatch](https://app.notion.com/p/ckb-devrel/Common-Intention-Dispatch-3de9f5bf382080249b91d32ad72639d9).

**Out of scope for v0.1:** Process contract, SSRI client, and the executor
daemon. Those land in later slices.

## How it works

Intention cell:

- Data empty (CKB) or the standard 16-byte xUDT amount
- Lock = this CID script
- Type empty (pure CKB) or official xUDT

Lock args: `process_type_hash[32] || molecule(CidEntry { version, user_lock, payload })`
via `serde_molecule` (defined in `protocol/`).

Lock Create does **not** run Verify. Digestion is **destroy only** (group 1 in /
0 out). The three Burn hops are inferred from the transaction, not a witness
enum:

| Hop | When | What CID checks |
|-----|------|-----------------|
| `digest_ok` | Matching Process CellDep **and** User Lock in inputs | Destroy + transaction authorization by that User Lock; partial user proceeds allowed |
| `digest_incompatible` | User Lock refund present, User Lock absent from inputs | Destroy + exact CKB capacity and token refund |
| `timeout_burn` | User Lock in inputs, no matching Process, headers prove `judge - pack >= 10800` | Destroy + exact refund + User Lock authorization |

**Authorization change:** a Process CellDep only identifies a Process; it does
not execute it. `digest_ok` now requires an input protected by the exact User Lock
encoded in the Intention. CKB validates that lock in the same transaction. Use a
lock that authorizes all transaction outputs (such as secp256k1 sighash-all), and
review those outputs before signing. An always-success User Lock offers no
protection. This is a user-coauthorized protocol, **not autonomous Process/SSRI
execution**. A future autonomous mode needs a separately specified and audited
Process authorization proof. Merely consuming a Process input is not sufficient.

The authorized digest branch takes precedence over refund-shaped outputs. A
matching Process without User Lock authorization may only perform a complete
permissionless refund; it cannot redirect assets. `ProcessMismatch` also denotes
missing user authorization on the digest path.

Refunds preserve the Intention's **exact capacity**, and xUDT refunds preserve
both the full type hash and token amount. Fees come from separate funding inputs.
Until aggregate refunds are specified, refund paths accept at most one Intention
per User Lock per transaction, including different payloads and deployment aliases.
The guard conservatively counts any input lock whose args decode as CID v0.1
with that User Lock, even if it belongs to another program.
This prevents two script groups from counting the same refund twice. The first
output to the User Lock must be the exact refund; append fee change afterwards.

Existing deployed binaries are unchanged by this source fix. Rebuild/redeploy
and use the new code hash for new Intentions; existing cells locked to the old
binary do not automatically acquire these protections.

`TIMEOUT_BLOCKS = 10800` is dual-written on-chain and off-chain. Pack height is
the block that committed the Intention; judge height is the newest header dep.

## Build and test

This is a Cinnabar project (not Capsule / `deployment.toml`).

```bash
make prepare   # rustup target add riscv64imac-unknown-none-elf
make build     # RISC-V binary → build/release/ckb_common_intention_dispatcher
make test      # FakeRpc + native CKB-VM, all cases
```

Both `make build` and `make test` must exit 0. Tests load this contract from `build/release/` and the
official xUDT RISC-V binary from `tests/binaries/xudt`.

Cargo may print non-fatal parse errors for `ckb-cinnabar`’s
`templates/contract` placeholders (`{{crate_name}}`). Those files are not
workspace members; the build still succeeds.

Dry-run deploy (needs a reachable indexer):

```bash
cargo run -- --json --dry-run deploy \
  --contract-name ckb_common_intention_dispatcher --tag v0.1.0 \
  --payer-address ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsq28phxutezqvjgfv5q38gn5kwek4m9km3cmajeqs
```

## Layout

| Path | Role |
|------|------|
| `protocol/` | Shared `no_std` args layout + constants |
| `contracts/ckb_common_intention_dispatcher/` | On-chain Verify tree (`cinnabar_main!`) |
| `calculator/` | `Instruction::new` recipes |
| `tests/` | FakeRpc universe |
| `tests/binaries/xudt` | Real xUDT RISC-V (not always-success) |
| `build/release/` | This lock’s RISC-V binary |
| `src/main.rs` | `ckb_cinnabar::dispatch()` runner |

Most unit scenarios use always-success user locks; the signed integration scenarios
use the real genesis secp256k1 lock. Process is a **hash mock** cell whose
type script hash matches args — not a real SSRI binary.

## Transaction construction and finalization

Recipes return **partial, unsigned instructions**, not broadcast-ready transactions.
`create_intention(..., Some((type_script, amount)))` collects existing tokens from
the payer using the exact type script, rejects insufficient balance, and returns
any token remainder to the payer. It does not mint, even when the payer is the
issuer. It supports standard 16-byte xUDT amounts; extension-bearing cells are
not collected. Balance the remaining CKB capacity separately.

`digest_ok` loads xUDT dependencies from inputs even when `asset_output` is absent.
Add the User Lock input explicitly and sign after all outputs/deps/fees are final.
For an incompatible refund, use a **different fee sponsor lock**: adding a User
Lock input deliberately selects timeout semantics when no Process matches.

A standard secp256k1 integration should:

1. Add CID, xUDT (recipes handle this), and the real secp256k1 code/data deps.
2. Apply the recipe and any explicit authorization/sponsor inputs.
3. Balance CKB with a separate change output, preserving exact refund outputs.
4. Sign every input lock group, then check actual fees against the chosen budget
   and the node's fee rate. Do not alter the transaction after signing.
5. Dry-run the final signed transaction against a CKB node before broadcasting.

Example finalization with an explicit fee budget (shannons); `signatures` is an
`AddSecp256k1SighashSignatures` configured for **all** signing input groups:

```rust,ignore
use ckb_cinnabar_calculator::{
    operation::{basic::AddSecp256k1SighashSignatures, Operation},
    re_exports::ckb_types::core::Capacity,
    skeleton::ChangeReceiver,
};

let fee = 10_000u64;       // Explicit total fee, not a fee rate.
let max_fee = 100_000u64;  // Application-approved upper bound.
assert!(fee <= max_fee);
// Append change; do not use the refund output as ChangeReceiver::Output.
skeleton.balance(&rpc, Capacity::shannons(fee), payer_lock.clone(),
    ChangeReceiver::Script(payer_lock))?;
Box::new(signatures).run(&rpc, &mut skeleton, &mut Vec::new())?;
let input_capacity: u64 = skeleton.inputs.iter()
    .map(|i| i.output.capacity_shannons()).sum();
let output_capacity: u64 = skeleton.outputs.iter()
    .map(|o| o.capacity_shannons()).sum();
let paid_fee = input_capacity.checked_sub(output_capacity).expect("capacity deficit");
assert_eq!(paid_fee, fee);
assert!(paid_fee <= max_fee);
// Use the final signed size, rounding the node's shannons/kB rate up.
let tx = skeleton.clone().into_transaction_view();
let size = tx.data().serialized_size_in_block() as u64;
let required_fee = size.checked_mul(rpc.min_fee_rate()?).unwrap().div_ceil(1000);
assert!(paid_fee >= required_fee);
// If too low, rebuild and re-sign with a higher approved fee; never shave refunds.
// Dry-run and broadcast `tx` using your node/client only after all checks pass.
```

`BalanceTransaction` is also available from Cinnabar for rate-based assembly,
but it estimates before final signatures/change; always check the final signed
size and actual fee. A failed assembly must be discarded or retried from a clean
skeleton, since instructions may have already inserted earlier operations.

## Security regression coverage and CI

GitHub Actions runs formatting, clippy, protocol tests, RISC-V build, CKB-VM
regressions, and a pinned SHA-256 check of the bundled official xUDT binary. The
built CID binary and its SHA-256 are uploaded together. Configure the repository's
branch rules to require the `verify` job before merge; workflow files alone cannot
enforce branch protection.

Tests include CellDep-only theft, xUDT capacity theft on both refund paths,
non-issuer token collection/change/insufficient balance, missing output with xUDT
input dependency, timeout boundaries 10799/10800/10801, partial user proceeds,
and cross-Intention refund reuse. `tests/src/signed.rs` executes the real genesis
secp256k1 script and real signatures for create/digest/refund/timeout, checks fee
and change, and rejects missing signatures and post-signing output mutations.
FakeRpc still supplies chain state; these are not live-network broadcasts or
an implementation of Process/SSRI. Live-node dry-run/broadcast acceptance remains
an integration/deployment gate.
