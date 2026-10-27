# Common Intention Dispatch (CID) v0.1

CID is a CKB **lock** that holds a single Intention cell until it is destroyed.

An Intention is a promise of assets (pure CKB or standard xUDT) that a later
Process / SSRI executor can consume. This repository implements the **first
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
| `digest_incompatible` | User Lock refund output present **and** User Lock not in inputs | Destroy + refund assets to User Lock |
| `digest_ok` | Matching Process CellDep (`type_script_hash == args` prefix) | Destroy; asset destination is left to Process/SSRI |
| `timeout_burn` | User Lock in inputs + header deps prove `judge - pack >= 10800` | Destroy + User Lock refund |

`TIMEOUT_BLOCKS = 10800` is dual-written on-chain and off-chain. Pack height is
the block that committed the Intention; judge height is the newest header dep.

## Build and test

This is a Cinnabar project (not Capsule / `deployment.toml`).

```bash
make prepare   # rustup target add riscv64imac-unknown-none-elf
make build     # RISC-V binary → build/release/ckb_common_intention_dispatcher
make test      # FakeRpc + native CKB-VM, all cases
```

Both `make build` and `make test` must exit 0 (verified: **build 0**, **test 0**,
19 FakeRpc cases). Tests load this contract from `build/release/` and the
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

User locks in tests are always-success. Process is a **hash mock** cell whose
type script hash matches args — not a real SSRI binary.
