# CID lock deployment notes

This directory holds **historical** testnet deployment metadata
(`testnet/*.json`). Those JSON files are **not** automatically updated by CI and
may list a stale `data_hash` after source changes.

## Expected RISC-V `cid_lock` data hash (verify before enabling bounded refunds)

Bounded DigestIncompatible fee haircuts require Intentions locked to a binary
that includes fee-binding Verify. **Do not enable shaved refunds against an older
deployed cell.** TimeoutBurn remains **exact capacity** on the new binary as well.

| Field | Value |
|-------|--------|
| Binary | `build/release/ckb_common_intention_dispatcher` |
| CKB `data_hash` (blake2b-256, personalization `ckb-default-hash`) | `0x2edb827f3050ee44ceb4d493a1ba097be92a949db98adb34442b1569c61fed2d` |
| SHA-256 (artifact checksum only) | `7c76b304b930265706c039b1e2198f97c74b9946271653fb1c56db981b03e5c6` |

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
