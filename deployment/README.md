# CID lock deployment notes

This directory holds **historical** testnet deployment metadata
(`testnet/*.json`). Those JSON files are **not** automatically updated by CI and
may list a stale `data_hash` after source changes.

## Expected RISC-V `cid_lock` data hash (verify before enabling bounded refunds)

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
already deployed on any public chain.

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

Before configuring a backend / CLI `cid_lock_dep` for bounded refunds, confirm the
printed `data_hash` equals the table above (or update this table in the same PR
that changes Verify). After a real redeploy, refresh `testnet/*.json` with the
on-chain tx/outpoint/`data_hash`.
