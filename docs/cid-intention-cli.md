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
| Asset | CKB only, with no output type script and empty output data |
| Intention capacity | Defaults to the example Process minimum (**600 CKB**). The CLI sizes the CKB output to `max(requested, occupied + 1 shannon)` so the CID lock can consume it. When `enforce_example_process_deposit_minimum` is `true` (default), it rejects an effective capacity below the **example** Process 600 CKB floor. Set that flag to `false` for other Processes that own a lower (or zero) deposit minimum. |
| Inputs | Explicit funding outpoint + deps in offline request JSON (pinned by outpoint; not address scan) |
| Process identity | Process state cell’s built-in **TYPE_ID type script** `{TYPE_ID, type, args}` → `calc_script_hash` (= Intention `process_type_hash`); Process contract is the state-cell **lock** |
| CID lock identity | Intention lock always references the CID code cell by **Type ID**: `lock.code_hash = hash(cid_lock_dep.type_script)`, `lock.hash_type = type`. Data-hash (`data1`) referencing is rejected. |
| Payload | Required beneficiary lock → `Payload::from_script` (Process example semantics) |
| User lock | Funding lock (= payer), using `data`, `type` or `data1` |
| Fee / change | Explicit `fee_shannons` (Intention) or rate **1000** (Process); sized on `serialized_size_in_block` (= molecule + 4) **after** change + sighash witness placeholders; change to payer via skeleton `balance` |
| Signing | `CID_INTENTION_PRIVKEY` env (32-byte hex); never CLI flags / files / `.env` / logs / JSON |
| Broadcast | `send` subcommand only; requires `--network testnet` + `--genesis-hash`; strips `hash`/`process_type_hash` then validates as RPC `Transaction`; fail-closed on mainnet / missing-empty-unknown chain (whitelist `ckb_testnet`) / genesis mismatch; `safe_message` keeps structured genesis hashes actionable while omitting hex-flooded guard/RPC strings |
| Coin selection | Not in v1 (no online UTXO gather) |

## Request JSON (create)

`create` accepts objects or positional arrays for both the full request and
`funding`. Arrays follow the field order shown in the example below.

The `create` command checks asset field names in the top-level request object
and in the funding object. For positional requests, it checks the fifth
element when that element is a funding object. The Rust request types use
standard Serde deserialization and ignore unknown fields.

Use `intention_capacity` for the requested output capacity. `funding.capacity`
and the optional `funding.data` describe the existing input cell. For each
extra key in a checked object, `create` lowercases the name and removes
non-alphanumeric characters, including underscores and hyphens. It rejects
the key only if the result exactly matches the normalized form of one of the
following names, even when the value is `null`.

- Capacity and cell output fields: `capacity`, `cell_capacity`,
  `intention_capacity`, `output_capacity`, `output`, `outputs`, `cell_output`,
  `data`, `cell_data`, `intention_data`, `output_data`, `outputs_data`,
  `raw_outputs_data`, `type`, `type_script`, `cell_type`, `cell_type_script`,
  `intention_type`, `intention_type_script`, `output_type`, `output_type_script`.
- Amounts and asset or token fields: `amount`, `total_amount`, `asset`,
  `assets`, `asset_amount`, `asset_data`, `asset_type`, `asset_type_script`,
  `token`, `tokens`, `token_amount`, `token_balance`, `token_data`, `token_type`,
  `token_type_script`.
- UDT fields: `udt`, `udt_amount`, `udt_data`, `udt_type`, `udt_script`,
  `udt_type_script`, `sudt`, `sudt_amount`, `sudt_data`, `sudt_type`,
  `sudt_script`, `sudt_type_script`, `xudt`, `xudt_amount`, `xudt_data`,
  `xudt_type`, `xudt_script`, `xudt_type_script`, `xudt_args`, `xudt_code_hash`,
  `xudt_hash_type`, `xudt_type_args`, `xudt_type_hash`, `input_xudt_args`.

For example, `intentionCapacity`, `intention-capacity`, `outputCapacity`, and
`XUDT_AMOUNT` all fail before `create` writes a transaction. When a rejected
key matches a declared asset field at that level, the diagnostic suggests
the canonical name, such as `intention_capacity` or `funding.capacity`. The
diagnostic escapes the key into a single line and displays at most 48 bytes
of the escaped key, including the `...` suffix when truncated.

Other extra keys such as `feeShannons`, `funding.txHash`, `tokenizer`,
`typescript_version`, `request_id`, and `note` are ignored. Their values can be
any valid JSON, including `1e400` or an object like `{"value": 1e400}`. Use
`fee_shannons` and `funding.tx_hash` for the actual fee and input transaction
hash.

See the CID v0.1.4 asset and User Lock rules in the
[README](../README.md#intention-creation-with-cid-v014).

```json
{
  "network": "testnet",
  "fee_shannons": 10000,
  "intention_capacity": 60000000000,
  "enforce_example_process_deposit_minimum": true,
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

For an example Process request (`enforce_example_process_deposit_minimum: true`),
the signed bundle also carries `example_process_type_script`. Before broadcast,
`send` resolves that TYPE_ID with the node's indexer, checks that the state is
live, and compares its CID binding with the signed Intention lock. Keep the
metadata in the bundle and use an RPC with the Indexer module enabled. Legacy
example states with short opaque lock args have no CID binding to check.
For other Processes use `enforce_example_process_deposit_minimum: false`.

## Process state create (`create-process`)

Request JSON (no secrets):

```json
{
  "network": "testnet",
  "process_capacity": 300000000000,
  "process_lock_salt": "0x01",
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

Cell layout: Process lock = Process code Type ID (`hash_type=type`) with
lock args `cid_code_hash[32] || cid_hash_type[1] || instance_salt`.

By default **omit** `process_lock_args`: the CLI auto-fills
`cid_code_hash = hash(cid_lock_dep.type_script)`, `cid_hash_type = type` (1),
and `instance_salt` from optional `process_lock_salt` (omit / `"0x"` = empty).
Pass explicit `process_lock_args` only when you already have the bytes; they are
decoded and **must** match that CID identity (`cid_code_hash` + `hash_type=type`),
or create fails. Do not set both `process_lock_args` and `process_lock_salt`.
Args must not decode as a CID Intention. The signed JSON echoes the resolved
`process_lock_args` hex. The state uses a built-in TYPE_ID derived from the
first input and output index, with empty data. Both calculator entry points
check the Process code dep, lock-args layout, and state capacity before adding
the input.
The state must reserve at least 1,000 shannons plus one shannon per lock-arg
byte above occupied capacity for future digest fees. Reserve more for larger
digest transactions.

The mint fee is calculated from `serialized_size_in_block` at rate 1000
after change and the 65-byte sighash placeholder are present. Funding must
cover the state, occupied change, and the measured fee.

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

Keep the Process creation metadata when passing the artifact to `send`.
The CLI checks the CID and Process code Type IDs against the first and
second signed `cell_deps` outpoints, respectively, and rejects dead
dependencies. It strips CLI metadata before submitting. Ordinary raw
transactions continue to use the generic send path, including other
TYPE_ID mints with a secp256k1 `dep_group`.

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
