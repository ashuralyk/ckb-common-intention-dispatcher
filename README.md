# Common Intention Dispatch (CID) v0.1

CID is a CKB **lock** that holds a single Intention cell until it is destroyed.

An Intention is a promise of assets (pure CKB or standard xUDT) that a later
Process / SSRI executor can consume by spending and recreating the committed Process cell. This repository implements the **first
slice only**: the CID lock, Calculate recipes, and FakeRpc tests.

Design notes: [Common Intention Dispatch](https://app.notion.com/p/ckb-devrel/Common-Intention-Dispatch-3de9f5bf382080249b91d32ad72639d9).

**Original v0.1 lock slice** did not include a production Process. This repo
also ships a **sample Process** (see [Process Example](#process-example)).

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
| `digest_ok` | Committed Process Type ID (state cell TYPE_ID type hash) present in **both inputs and outputs** | Destroy; Process **lock** enforces execution; no User Lock input/signature required |
| `digest_incompatible` | No matching Process pair, User Lock refund present, User Lock absent from inputs | Destroy + User Lock refund; token amount/type exact; CKB capacity within fee window |
| `timeout_burn` | User Lock in inputs, no matching Process pair, headers prove `judge - pack >= 10800` | Destroy + exact capacity/token refund + User Lock authorization |

**Delegated execution:** the user chooses `process_type_hash` when creating
an Intention. That hash is the script hash of the Process state cell’s built-in
**TYPE_ID** type script (Process Type ID). The state cell’s **lock** is the
Process contract (referenced by its code Type ID). For `digest_ok`, CID finds
that exact type-script hash on an input AND an output. The live Process cell is
consumed and recreated, not added as a CellDep. CKB executes the Process lock
(and TYPE_ID type) as part of normal validation; CID only checks the matching
pair. The user does not need another input or signature for execution. The
encoded User Lock is the refund destination and authorizes the timeout reclaim
path.

Code cells needed to execute the Process lock still belong in CellDeps; these
are distinct from the live Process state cell. A dependency-only, input-only,
output-only, or mismatched Process cannot authorize `digest_ok`. TYPE_ID
provides identity continuity; the Process lock requires the **same** TYPE_ID
type-script hash on its lock-pair input and output and rejects that Type ID on
non-Process-lock outputs. Use distinct Process lock args per instance (shared /
empty args merge lock groups). CID does not implement application-specific
payload rules. A permissive Process lock delegates permissive execution; choose
and audit the Process accordingly.

Off-chain, `Process.assemble` returns a complete molecule transaction (see
`docs/ssri-backend-interface.md`); the backend submits it without rewriting
structure. Capacity/fee balance is Process-owned (example deducts fee from the
state cell).

The matching Process branch takes precedence over refund-shaped outputs, so
partial user proceeds are allowed. Multiple distinct Intention lock groups for
the same User Lock may be digested together with the matching Process pair;
the one-input/no-output rule still applies to each CID lock group. The shared
refund guard below applies only to refund paths. Without a matching Process
pair, exact refund and timeout rules remain available; arbitrary spending fails.

Refunds must go to the exact User Lock. xUDT refunds preserve both the full type
hash and token amount.

**DigestIncompatible (keyless) fee haircut:** refund CKB capacity may be at most
`MAX_REFUND_FEE` (= `0.01 CKB` = `1_000_000` shannons) below the Intention, and
must stay `>=` the refund cell's occupied capacity. That ceiling alone is not
enough: Verify also requires `tx_fee >= Σ shortfalls` over **every**
DigestIncompatible Intention in the transaction (same aggregate in each lock
group). Equivalently, non-refund outputs must be funded by inputs that are **not**
counted in `S` (non-DigestIncompatible inputs — sponsors, TimeoutBurn/user cells,
DigestOk Process cells, etc.), so a shortfall cannot be redirected to a third
party and cannot be double-counted across groups. `MAX_REFUND_FEE` is only a
per-Intention ceiling on how much of that fee one refund may contribute — never
a skim allowance.

**TimeoutBurn** keeps **exact** capacity refunds (user can add fee inputs).
**DigestOk** is unchanged. Process example still requires exact beneficiary
capacity. **Do not co-bundle shaved DigestIncompatible refunds with DigestOk**
(CID rejects shortfall > 0 when any DigestOk Intention is present); backends
must build separate transactions. Exact incompatible refunds may still share a
tx with DigestOk as today.

**Mixed CID lock identities forbidden on DigestIncompatible:** a DigestIncompatible
spend rejects the tx if any other input lock decodes as CID args but has a
different `code_hash` or `hash_type` than the executing script (`MixedCidLock`).
That covers two deployments, Data1/Data2 aliases, and foreign locks that only
embed CID-shaped args — they cannot share a DigestIncompatible transaction
(otherwise each group could accept `F >= S_local` while `S_A+S_B > F`). Same
identity (same code_hash+hash_type) multi-Intention batches remain OK when
`F >= Σ shortfalls`.

No Intention-args / encoding change: fixed dual-written `MAX_REFUND_FEE`.

Until aggregate refunds are specified, refund paths accept at most one Intention
per User Lock per transaction, including different payloads and deployment aliases.
The guard conservatively counts any input lock whose args decode as CID v0.1
with that User Lock, even if it belongs to another program.
This prevents two script groups from counting the same refund twice. The first
output to the User Lock must be the refund; with a fee sponsor, keep
`sponsor_in >= sponsor_change` so fee still covers the Intention shortfall.

**Redeploy / Type ID:** changing refund matching changes the `cid_lock` binary /
data hash. Rebuild and migrate under the **same Type ID**; new Intentions must
lock with `hash_type=type` and `code_hash=type_id` (see Intention CLI) so they
track the live code cell across upgrades. Existing cells locked to an older
**data hash** (`data1`) stay bound to that binary until spent — but spending
them still requires a **live CellDep** that contains those exact old bytes
(CKB code locating). Before a Type-ID migration that consumes the only live
copy of the old binary: settle (digest/refund/timeout) those Intentions first,
**or** keep the exact old binary available in a separate live code cell that
spends can CellDep. **TimeoutBurn stays exact** after redeploy (only
DigestIncompatible gains the fee-bound haircut). See `deployment/README.md`
for the expected RISC-V `data_hash` and how to verify it; `deployment/testnet/`
JSON may still list a stale deployed hash — do not treat that as current until
the owner updates it post-redeploy.

`TIMEOUT_BLOCKS = 10800` and `MAX_REFUND_FEE = 1_000_000` are dual-written
on-chain and off-chain. Pack height is the block that committed the Intention;
judge height is the newest header dep.

**Intention capacity / Process deposits:** Intention creates size capacity to
`max(requested, occupied)`. Protocol / CID impose **no** Intention or deposit
floor — each Process owns its own minima. The example Process uses a
Process-owned `MIN_DEPOSIT_CAPACITY` = **600 CKB** (single source in
`crates/cid-process-example-policy`, used on-chain + calculator/CLI) and rejects
smaller CKB DigestOk Intentions for **its own** process_type_hash
(`DepositBelowMinimum` = 39). CLI defaults to that 600 CKB deposit minimum and
rejects a smaller effective capacity before signing. See the
[upgrade steps](docs/process-example-usage.md#upgrading-an-existing-process-deployment)
before replacing the example Process binary at the same code Type ID.

**Intention filter (Sequencer):** Host crate `cid-intention-filter` (kept out of
`cid-protocol` so the CID lock binary stays byte-stable) provides
`intention_process_type_id` for routing and `Process.filter` wire framing.
Each Process exposes SSRI `Process.filter` to classify a batch (ownership +
Process rules) with stable reason codes. Ownership is checked at **both**
layers (Sequencer route + Process filter) using the same helper. See
[docs/ssri-backend-interface.md](docs/ssri-backend-interface.md).

## Build and test

This is a Cinnabar project (not Capsule / `deployment.toml`).

### Host crates (default workspace members)

From a clean clone, the root default members are host libraries only
(`cid-protocol`, the two calculators, and `cid-process-executor`). RISC-V
contracts and the optional deploy runner are workspace members but **not**
defaults, so plain Cargo commands succeed on the host target:

```bash
# System prerequisite for host crates that pull ckb-cinnabar-calculator → reqwest:
#   Debian/Ubuntu: sudo apt-get install -y libssl-dev pkg-config
#   Fedora:        sudo dnf install openssl-devel pkgconf-pkg-config
#   macOS:         usually covered by the system LibreSSL / Xcode CLT
cargo check --locked
cargo build --locked
cargo test --locked
```

### RISC-V contracts and FakeRpc tests

```bash
make prepare   # rustup target add riscv64imac-unknown-none-elf
make build     # RISC-V binaries → build/release/
make test      # FakeRpc + native CKB-VM (needs make build first)
```

Both `make build` and `make test` must exit 0. Tests load contracts from
`build/release/` and the official xUDT RISC-V binary from `tests/binaries/xudt`.

To build CID and the Process example into `build/release/` without a full
`make build` clean:

```bash
./scripts/build-all-contracts.sh
```

Build a single contract (used by downstream CI), e.g.:

```bash
make CONTRACT=cid_process_example build
```

## Process Example

In-tree sample Process (not a production SSRI daemon):

- RISC-V contract `contracts/cid_process_example` with `cinnabar_main!` SSRI
  (`Process.batch`, `Process.assemble`) and on-chain Create/Transfer Type ID,
  batch, and beneficiary Script/capacity/data/asset checks
- Greenfield offline host `crates/cid-process-executor` (ckb-vm, argv method id)
- Calculator `calculator/process` — `process_digest_ok` rebuilds the skeleton
  from the full assembled molecule `Transaction` (inputs, deps, witnesses,
  outputs)
- FakeRpc tests: n=1, 2, 4, TYPE_ID continuity theft negatives
  (`tests/src/process_example.rs`)
- **BATCH_MIN=1, BATCH_MAX=4**
- Example assemble deducts Process-owned fee from the state cell recreate

Usage: [docs/process-example-usage.md](docs/process-example-usage.md)

```bash
./scripts/build-all-contracts.sh
cd tests && cargo test process_example -- --nocapture
```

## Optional Intention CLI

CKB-only offline Intention creator (non-default member). See
[docs/cid-intention-cli.md](docs/cid-intention-cli.md).

Intentions are locked to the CID code cell by **Type ID** (`lock.hash_type=type`,
`lock.code_hash` = hash of the code cell's Type ID type script). The create
request's `cid_lock_dep` must include that `type_script` and point at the live
code outpoint; missing type script fails closed (no `data1` / data-hash fallback).

```bash
cargo run -p cid-intention-cli -- create --request req.json --output tx.json
cargo run -p cid-intention-cli -- send --tx tx.json --rpc-url http://127.0.0.1:8114 \
  --network testnet --genesis-hash 0x…
```

Signing uses `CID_INTENTION_PRIVKEY` (env only). `create` never broadcasts;
`send` never reads a key. Do not point CI/agents at public RPCs.

## Optional deploy runner

The Cinnabar deploy/migrate CLI lives in `runner/` (package
`ckb_common_intention_dispatcher-runner`). It is **not** a default member, so
it is not built by plain `cargo check/build/test`. It needs the same OpenSSL
headers as above, plus a reachable indexer for live deploy commands.

```bash
cargo run -p ckb_common_intention_dispatcher-runner -- --json --dry-run deploy \
  --contract-name ckb_common_intention_dispatcher --tag v0.1.0 \
  --payer-address ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsq28phxutezqvjgfv5q38gn5kwek4m9km3cmajeqs
```

Cargo aliases `deploy` / `migrate` / `consume` / `list` in `.cargo/config.toml`
forward to this package.

## Upstream ckb-cinnabar notes

Template Cargo manifests under cinnabar’s `templates/contract/` are shipped as
`Cargo.toml.liquid` (pinned `8cd49b0`), so Cargo no longer emits non-fatal
`error:` diagnostics while fetching that git dependency.

Contract ELF hashes are **not** guaranteed reproducible across machines,
`CARGO_HOME` layouts, or checkout path differences; treat
`deployment/testnet/*.json` `data_hash` values as metadata for already-deployed
artifacts, not as a local rebuild oracle.

## Layout

| Path | Role |
|------|------|
| `protocol/` | Shared `no_std` args layout + constants |
| `contracts/ckb_common_intention_dispatcher/` | On-chain Verify tree (`cinnabar_main!`) |
| `contracts/cid_process_example/` | Sample Process (SSRI + on-chain verify) |
| `crates/cid-process-executor/` | Offline SSRI host for the sample Process |
| `calculator/intention/` | CID `Instruction::new` recipes |
| `calculator/process/` | `process_digest_ok` skeleton assembly |
| `tests/` | FakeRpc universe (CID + `process_example`) |
| `tests/binaries/xudt` | Real xUDT RISC-V (not always-success) |
| `build/release/` | This lock’s RISC-V binary |
| `runner/` | Optional `ckb_cinnabar::dispatch()` deploy CLI (non-default member) |
| `tools/cid-intention-cli/` | Offline CKB Intention create + env-key sign; explicit testnet `send` (non-default) |

Most unit scenarios use always-success user locks; the signed integration scenarios
use the real genesis secp256k1 lock. Process cells use an executable test-only
type script (`cid_process_fixture`), not a production Process/SSRI contract.
The fixture rejects a forbidden output state to prove that Process validation
runs even when CID finds a matching pair. Do not deploy the fixture.

## Transaction construction and finalization

Recipes return **partial, unsigned instructions**, not broadcast-ready transactions.
`create_intention(..., Some((type_script, amount)))` collects existing tokens from
the payer using the exact type script, rejects insufficient balance, and returns
any token remainder to the payer. It does not mint, even when the payer is the
issuer. It supports standard 16-byte xUDT amounts; extension-bearing cells are
not collected. Balance the remaining CKB capacity separately.

`digest_ok` loads xUDT dependencies from inputs even when `asset_output` is absent.
The recipe consumes and recreates the Process unchanged; customize its output
state as permitted by the Process scripts, then add their code deps/witnesses.
There is no Intention User Lock input/signature requirement for digest.
Sign any server/sponsor input locks after all outputs/deps/fees are final.
For an incompatible refund, use a **different fee sponsor lock**: adding a User
Lock input deliberately selects timeout semantics when no Process pair matches.

A standard secp256k1 integration should:

1. Add CID, xUDT (recipes handle this), and the real secp256k1 code/data deps.
2. Apply the recipe, Process code deps/witnesses, and any server/sponsor inputs.
3. For keyless `digest_incompatible` without a sponsor: set refund capacity to
   `intention_capacity - fee` with `fee ∈ (0, MAX_REFUND_FEE]`, `>=` occupied,
   and ensure the tx has no other outputs that would make `tx_fee < Σ shortfalls`.
   With a sponsor, fund change so `tx_fee >= Σ shortfalls` still holds. Never
   co-bundle shaved refunds with DigestOk. TimeoutBurn / signed refunds keep
   exact capacity and pay fees from user/sponsor inputs. Do not change xUDT amounts.
4. Sign every input lock group (when any), then check actual fees against the
   chosen budget and the node's fee rate. Do not alter the transaction after signing.
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
// Keyless DigestIncompatible: shave refund by exactly the miner fee (≤ MAX_REFUND_FEE)
// with tx_fee covering the tx-wide shortfall aggregate — never divert into other outputs.
// Dry-run and broadcast `tx` using your node/client only after all checks pass.
```

`BalanceTransaction` is also available from Cinnabar for rate-based assembly,
but it estimates before final signatures/change; always check the final signed
size and actual fee. A failed assembly must be discarded or retried from a clean
skeleton, since instructions may have already inserted earlier operations.

## Security regression coverage and CI

GitHub Actions runs the root default host `cargo check` / `cargo build` /
`cargo test`, formatting, clippy, protocol tests, RISC-V build, CKB-VM
regressions, and a pinned SHA-256 check of the bundled official xUDT binary. The
built CID binary and its SHA-256 are uploaded together. Configure the repository's
branch rules to require the `verify` job before merge; workflow files alone cannot
enforce branch protection.

**CID lock binary guard (dual):** Filter / Sequencer work must not change the
CID lock. Locally (and in the known deploy / agent environment), run
`make check-cid-hash` against the absolute pin `CID_LOCK_SHA256`
(`569f4446…8c3e`). GitHub Actions never uses that absolute pin — runners
produce a different hash for the same source — and instead runs
`make check-cid-unchanged` with the same toolchain against the PR base SHA (on
`pull_request`) or `github.event.before` (on `push` to `main`). A missing or
all-zero `before` SHA fails closed rather than skipping the guard.

Tests include CKB/xUDT delegation without a User Lock input, Process pair
presence/mismatch checks, a failing executable Process, authorized batch digest,
CellDep-only theft, xUDT capacity theft on both refund paths, DigestIncompatible
fee-binding (exact / within-cap / beyond-cap CKB+xUDT / wrong lock / token mutation /
multi-user aggregate fee / third-party siphon / fee reuse / sponsor change /
occupied floor / shaved+DigestOk co-bundle / MixedCidLock for foreign CID-shaped
locks and cross-identity Data1/Data2 fee reuse / TimeoutBurn shaved rejection),
non-issuer token collection/change/insufficient balance, missing output with
xUDT input dependency, timeout boundaries 10799/10800/10801, partial user proceeds,
and cross-Intention refund reuse. `tests/src/signed.rs` executes the real genesis
secp256k1 script and real signatures for create/digest/refund/timeout, checks fee
and change, and rejects missing funding signatures and post-signing output mutations. Digest
uses a server/sponsor signer distinct from the Intention owner.
FakeRpc still supplies chain state; these are not live-network broadcasts or
an implementation of Process/SSRI. Live-node dry-run/broadcast acceptance remains
an integration/deployment gate.
