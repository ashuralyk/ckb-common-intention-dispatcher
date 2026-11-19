# `cid-intention-cli` (v1)

Lean host binary in this contract repo that:

1. **`create`** — builds and **offline-signs** a CKB-only CID Intention create tx
2. **`create-process`** — builds and **offline-signs** a Process state cell create tx
3. **`send`** — **explicitly** broadcasts a previously signed tx JSON to a **testnet** RPC

`create` / `create-process` never broadcast. `send` never reads a private key.

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
| Process identity | Process state cell’s built-in **TYPE_ID type script** `{TYPE_ID, type, args}` → `calc_script_hash` (= Intention `process_type_hash`); Process contract is the state-cell **lock** |
| CID lock identity | Intention lock always references the CID code cell by **Type ID**: `lock.code_hash = hash(cid_lock_dep.type_script)`, `lock.hash_type = type`. Data-hash (`data1`) referencing is rejected. |
| Payload | Required beneficiary lock → `Payload::from_script` (Process example semantics) |
| User lock | Funding lock (= payer) |
| Fee / change | Explicit `fee_shannons` (Intention) or rate **1000** (Process); sized on `serialized_size_in_block` (= molecule + 4) **after** change + sighash witness placeholders; change to payer via skeleton `balance` |
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
    "data": "0x…",
    "type_script": {
      "code_hash": "0x00000000000000000000000000000000000000000000000000545950455f4944",
      "hash_type": "type",
      "args": "0x…"
    }
  },
  "sighash_deps": [
    { "name": "secp256k1_blake160_sighash_all", "tx_hash": "0x…", "index": 1, "data": "0x…" },
    { "name": "secp256k1_data", "tx_hash": "0x…", "index": 3, "data": "0x…" }
  ],
  "process_type_script": {
    "code_hash": "0x00000000000000000000000000000000000000000000000000545950455f4944",
    "hash_type": "type",
    "args": "0x…"
  },
  "beneficiary_lock": {
    "code_hash": "0x…",
    "hash_type": "data1",
    "args": "0x…"
  }
}
```

`process_type_script` must describe the **live Process state cell's TYPE_ID type
script** (Process Type ID), not the Process code cell and not the Process lock.

`cid_lock_dep` must point at the **current live** CID code cell outpoint
(`dep_type` code) and include:

- `data` — the CID lock binary (seeds FakeRpc / offline assembly)
- `type_script` — the code cell's **Type ID** type script (`TYPE_ID` code hash,
  `hash_type=type`, Type ID args). **Required.** Without it the CLI fails closed
  (no silent `data1` / data-hash fallback). That Type ID script hash becomes the
  Intention `lock.code_hash` with `lock.hash_type=type`, so Intentions survive
  Type-ID code-cell upgrades that consume the previous binary cell.

After a migrate that keeps the same Type ID, keep `type_script.args` (and thus
`type_id`) and refresh only `tx_hash` / `index` / `data` to the new live code
cell. Intention args encoding is unchanged. This CLI only creates Intentions —
it does not assemble refunds.

`send` (after chain/genesis guards) fetches each `cell_deps` live cell and
requires every **CID Intention** output lock (`hash_type=type` and args that
decode as CID v0.1) to match a live dep cell whose type script is TYPE_ID and
whose script hash equals the lock `code_hash`. Other `hash_type=type` locks
(e.g. genesis secp256k1/blake160 with a sighash `dep_group`) are ignored so
raw-transaction send stays usable. The CID check catches a correct live
outpoint paired with mistyped Type ID args in the offline request (create
alone cannot see the chain).

## Process state create (`create-process`)

Request JSON (no secrets):

```json
{
  "network": "testnet",
  "process_capacity": 300000000000,
  "process_lock_args": "0x01",
  "funding": { "tx_hash": "0x…", "index": 0, "capacity": 500000000000,
    "lock": { "code_hash": "0x…", "hash_type": "type", "args": "0x…" }, "data": "0x" },
  "cid_lock_dep": {
    "name": "ckb_common_intention_dispatcher",
    "tx_hash": "0x…", "index": 0, "data": "0x…",
    "type_script": {
      "code_hash": "0x00000000000000000000000000000000000000000000000000545950455f4944",
      "hash_type": "type", "args": "0x…"
    }
  },
  "process_code_dep": {
    "name": "cid_process_example",
    "tx_hash": "0x…", "index": 0, "data": "0x…",
    "type_script": {
      "code_hash": "0x00000000000000000000000000000000000000000000000000545950455f4944",
      "hash_type": "type", "args": "0x…"
    }
  },
  "sighash_deps": [
    { "name": "secp256k1_blake160_sighash_all", "tx_hash": "0x…", "index": 1, "data": "0x…" },
    { "name": "secp256k1_data", "tx_hash": "0x…", "index": 3, "data": "0x…" }
  ]
}
```

Cell layout: Process lock = Process code Type ID (`hash_type=type`, distinct
`process_lock_args`); type = built-in TYPE_ID from first input + output index;
data empty. Fee is `floor(serialized_size_in_block × 1000 / 1000)` after change
and 65-byte sighash placeholders (not pre-balance molecule length). Creation
validates Process lock + TYPE_ID off-chain (Process lock Create does not run on
output-only mint). `send` live-verifies **both** CID and Process code Type ID
celldeps and fails closed on any dead celldep.

Signed JSON includes `state_outpoint`, `identity_type_id` (TYPE_ID script hash =
Intention `process_type_hash`), `process_type_script` (for Intention `create`),
and `registry_post` for `POST http://127.0.0.1:9101/v1/processes`:

```json
{
  "identity_type_id_args": "0x…",
  "identity_type_id": "0x…",
  "code_type_id_args": "0x…",
  "code_type_id": "0x…"
}
```

`send` live-verifies the Process lock Type ID and the exact `cid_code_type_id`
recorded in create-process metadata (not any other live TYPE_ID) before
broadcast, and fails closed on dead celldeps.

## Usage

```bash
# Offline sign (never talks to a node)
export CID_INTENTION_PRIVKEY=0x…   # 32-byte hex; never commit
cargo run -p cid-intention-cli -- create --request req.json --output tx.json

# Offline Process state create (never talks to a node)
cargo run -p cid-intention-cli -- create-process --request process-req.json --output process-tx.json

# Explicit broadcast (no key). Point rpc-url at YOUR endpoint; CI uses localhost mocks only.
cargo run -p cid-intention-cli -- send \
  --tx tx.json \
  --rpc-url http://127.0.0.1:8114 \
  --network testnet \
  --genesis-hash 0x…   # must match get_block_hash(0x0)

# Register the new Process with a local backend (after send confirms)
jq .registry_post process-tx.json | curl -sS -X POST http://127.0.0.1:9101/v1/processes \
  -H 'Content-Type: application/json' -d @-
```

## Unavailable in v1 / documented gaps

- No automatic live-cell / indexer coin selection
- `create` does not validate cells against a node (live Type ID check is on `send`)
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
