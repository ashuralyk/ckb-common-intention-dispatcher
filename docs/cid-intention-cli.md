# `cid-intention-cli` (v1)

Lean host binary in this contract repo that:

1. **`create`** — builds and **offline-signs** a CKB-only CID Intention create tx
2. **`send`** — **explicitly** broadcasts a previously signed tx JSON to a **testnet** RPC

`create` never broadcasts. `send` never reads a private key.

## Workspace

- Path: `tools/cid-intention-cli`
- Workspace **member**, **not** in `default-members` (same policy as `runner/`)
- Depends on `cid-protocol`, `ckb_common_intention_dispatcher-calculator`,
  `ckb-cinnabar-calculator` (pinned git), `reqwest` (send only)

## Design (approved v1)

| Concern | Choice |
|---------|--------|
| Asset | CKB only (no xUDT) |
| Inputs | Explicit funding outpoint + deps in offline request JSON (pinned by outpoint; not address scan) |
| Process identity | Full Process **state type script** `{code_hash,hash_type,args}` → `calc_script_hash`; never assume code-cell Type-ID hash equals state identity |
| Payload | Required beneficiary lock → `Payload::from_script` (Process example semantics) |
| User lock | Funding lock (= payer) |
| Fee / change | Explicit `fee_shannons`; change to payer via skeleton `balance` |
| Signing | `CID_INTENTION_PRIVKEY` env (32-byte hex); never CLI flags / files / `.env` / logs / JSON |
| Broadcast | `send` subcommand only; requires `--network testnet` + `--genesis-hash`; strips `hash`/`process_type_hash` then validates as RPC `Transaction`; fail-closed on mainnet / missing-empty-unknown chain (whitelist `ckb_testnet`) / genesis mismatch; `safe_message` keeps structured genesis hashes actionable while omitting hex-flooded guard/RPC strings |
| Coin selection | Not in v1 (no online UTXO gather) |

## Request JSON (create)

```json
{
  "network": "testnet",
  "fee_shannons": 10000,
  "intention_capacity": 200000000000,
  "funding": {
    "tx_hash": "0x…",
    "index": 0,
    "capacity": 500000000000,
    "lock": { "code_hash": "0x…", "hash_type": "type", "args": "0x…" },
    "data": "0x"
  },
  "cid_lock_dep": {
    "name": "ckb_common_intention_dispatcher",
    "tx_hash": "0x…",
    "index": 0,
    "data": "0x…"
  },
  "sighash_deps": [
    { "name": "secp256k1_blake160_sighash_all", "tx_hash": "0x…", "index": 1, "data": "0x…" },
    { "name": "secp256k1_data", "tx_hash": "0x…", "index": 3, "data": "0x…" }
  ],
  "process_type_script": {
    "code_hash": "0x…",
    "hash_type": "data1",
    "args": "0x…"
  },
  "beneficiary_lock": {
    "code_hash": "0x…",
    "hash_type": "data1",
    "args": "0x…"
  }
}
```

`process_type_script` must describe the **live Process cell's type script**, not
the code cell alone. `cid_lock_dep.data` must be the CID lock binary so
`ScriptEx::Reference` can resolve Data1. After a CID lock redeploy (e.g.
DigestIncompatible fee-binding), refresh `cid_lock_dep` tx/outpoint/`data` to
the new binary; Intention args encoding is unchanged. This CLI only creates
Intentions — it does not assemble refunds.

## Usage

```bash
# Offline sign (never talks to a node)
export CID_INTENTION_PRIVKEY=0x…   # 32-byte hex; never commit
cargo run -p cid-intention-cli -- create --request req.json --output tx.json

# Explicit broadcast (no key). Point rpc-url at YOUR endpoint; CI uses localhost mocks only.
cargo run -p cid-intention-cli -- send \
  --tx tx.json \
  --rpc-url http://127.0.0.1:8114 \
  --network testnet \
  --genesis-hash 0x…   # must match get_block_hash(0x0)
```

## Unavailable in v1 / documented gaps

- No automatic live-cell / indexer coin selection
- `create` does not validate cells against a node
- ELF / deployment `data_hash` reproducibility across machines is out of scope
- Deployment JSON is not rewritten by this tool
- Agents/CI must **not** call public testnet/mainnet RPCs; use mocks or a private local node when exercising `send`

## Tests

```bash
cargo test -p cid-intention-cli
```

Covers offline create (fee/witness/args/leakage/wrong key/duplicates,
outpoint pin vs same-lock decoy, TransactionSimulator sighash VM verify) and
`send` against a local HTTP JSON-RPC mock (create→send metadata strip,
`ckb_testnet` whitelist / empty-chain refusal, mainnet refusal, genesis
mismatch with process-level hash diagnostics, redacted credential errors).
