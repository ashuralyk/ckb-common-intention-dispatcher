# CID lock deployment notes

This directory holds **testnet** deployment metadata (`testnet/*.json`). Those
JSON files are **not** automatically updated by CI. Prefer Type ID identity for
live Intentions: `lock.code_hash = type_id`, `lock.hash_type = type`.

## Live testnet (verified read-only via `https://testnet.ckb.dev/rpc`)

Checked against the node (live cells + blake2b-256 data hash + Type ID script
hash). Use these when configuring `cid_lock_dep` / Process deps — not the
historical first-deploy rows alone.

### `ckb_common_intention_dispatcher` (CID lock)

| Field | Value |
|-------|--------|
| Live code cell | `0x5d9e44fca3beb712c9efe848c24f83fb8dba3ff35cfb23353a89631322a155fc:0` |
| `data_hash` | `0x110f6f793b0a22ccc3942791573933206c5d6b24a8f5ddfe9749ff04f8646dca` |
| Type ID `type_id` (script hash → Intention `code_hash`) | `0xf3f8a56c2594030f99f05b09ff1b6a1b07e6e07ed04309c1c4e246561faad326` |
| Type ID `type_id_args` | `0xdfe0a1416c7373e8aaae5257445e940b6ff08e9cbc62ebca8541d28997370f25` |
| Type script `code_hash` | `0x00000000000000000000000000000000000000000000000000545950455f4944` |
| Type script `hash_type` | `type` |
| Occupied capacity (shannons) | `8835000000000` |

CLI `cid_lock_dep.type_script` must be that Type ID type script. Intention locks
must use `hash_type=type` and `code_hash=type_id` so they survive Type-ID
upgrades that consume the previous code cell.

### `cid_process_example`

| Field | Value |
|-------|--------|
| Live code cell | `0x2d8634134dea382d2361427dfcbce642b4092782ec4d6ab1b62acfd959f0c79f:0` |
| `data_hash` | `0xfab29bfe0b150e10c36bc64bfb08589100b07f8e27ddc5213d79429dc7965ed2` |
| Type ID `type_id` | `0x9efaaa094bdbdc463813d26c82021a2bcc2fc9cf886a3c2cfd04612e44c5f16d` |
| Type ID `type_id_args` | `0x1666a7d877fcabcf28bbc0903df791742a4c83a85dfb666a6c5db7adde76ace7` |
| Occupied capacity (shannons) | `10963000000000` |

Process **state** cells still use their own type script (often Data1 on this
sample); do not confuse the Process code-cell Type ID with Process state identity.

## Expected local RISC-V `cid_lock` data hash (rebuild oracle)

Bounded DigestIncompatible fee haircuts require Intentions locked to a binary
that includes fee-binding Verify (including `MixedCidLock` fail-closed checks).
**Do not enable shaved refunds against an older deployed cell.** TimeoutBurn
remains **exact capacity** on the new binary as well.

| Field | Value |
|-------|--------|
| Binary | `build/release/ckb_common_intention_dispatcher` |
| CKB `data_hash` (blake2b-256, personalization `ckb-default-hash`) | `0x9897167bd9ff000c3ddbd25cd1f7a362b8b959a452aa51aa6ea69e6c54ab7750` |
| SHA-256 (artifact checksum only) | `eedabd7460c1154e8d472e997964866034a752e9e220bb868fba64949acefd4e` |

This documents the hash produced by a clean `make build` in CI-equivalent
environment (clang-18, `--locked`). It is **not** a claim that this binary is
already deployed on any public chain. The live testnet `data_hash` above may
differ until a matching rebuild is redeployed under the same Type ID.

## Reproduce / verify

```bash
# From repo root (matches CI: clang-18 + locked deps)
make prepare
make build CLANG=clang-18 CARGO_ARGS=--locked

# CKB data hash (what Script.code_hash uses with hash_type Data1/Data2)
python3 - <<'PY'
import hashlib, pathlib
data = pathlib.Path("build/release/ckb_common_intention_dispatcher").read_bytes()
h = hashlib.blake2b(digest_size=32, person=b"ckb-default-hash")
h.update(data)
print("0x" + h.hexdigest())
PY

# Optional artifact checksum
sha256sum build/release/ckb_common_intention_dispatcher
```

Before configuring a backend / CLI `cid_lock_dep`, confirm the live Type ID
fields (table above) and that `cid_lock_dep.type_script` matches the live code
cell. After a migrate that keeps Type ID args, refresh `testnet/*.json`
tx/outpoint/`data_hash` only.
