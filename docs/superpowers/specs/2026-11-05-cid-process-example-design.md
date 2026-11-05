# CID Process Example Design (2026-11-05)

## Goal

样例 Process 合约（带 SSRI）、独立离线 SSRI executor crate，以及通过 **Process SSRI Calculator**（而非 CID calculator 的 digest）消化 Intention 的测试。

## Decisions

- **Business**：fixed-beneficiary escrow（payload 编码 beneficiary `Script`）；digest 时直接放行，无 arbiter
- **Offline path**：SSRI host（ckb-ssri-std 风格），不是仅 FakeRpc fixture
- **Layout**：同仓三部分 —
  - `contracts/cid_process_example`
  - Process Calculator（对应目录）
  - `crates/cid-process-executor`（后续 server 可复用）
- **Boundary**：CID calculator 继续负责 create / refund / timeout；Intention consume 由 Process SSRI Calculator 调用 executor
- **Batch bounds**：Process 合约定义 **最小 / 最大 Intention 批处理数量**（`batch_min` / `batch_max`）；样例给出具体常量，链上与 SSRI 均按此校验
- **Approach A**：竖切最小可测 — 真实二进制 + executor + 测试一刀落地

## Architecture

```
Intention create / refund / timeout  →  CID calculator（现有；不动 digest 业务）
Intention digest_ok                  →  Process SSRI Calculator
                                         └─ cid-process-executor
                                              └─ 离线 SSRI 跑 cid_process_example
On-chain                             →  CID lock Process pair 检查
                                         + Process type verify（beneficiary 输出 + batch 边界）
```

## Components

1. **`contracts/cid_process_example`** — Type ID Process，SSRI 方法，链上 verify（含 batch_min/max）
2. **Process Calculator** — 组 digest 骨架，调用 executor
3. **`crates/cid-process-executor`** — 加载 RISC-V，调用 SSRI，返回 partial tx 或 `Incompatible`
4. **`tests/`** — FakeRpc：真实 Process 二进制 + executor + Process Calculator

## SSRI surface (sample)

- `SSRI.get_methods`
- `Process.batch` → 返回本 Process 的 `(batch_min, batch_max)`（样例常量，例如 min=1、max≥2，具体值实现时定）
- `Process.assemble` → 接受 `[batch_min, batch_max]` 范围内的 Intention 集合，返回 partial execution tx；数量越界或坏格式 → `Incompatible`
- 链上 type：beneficiary 输出 + Process pair 连续性（Type ID）+ 本 tx Intention 数量落在 `[batch_min, batch_max]`

方法名可按 ckb-ssri-std 惯例在实现时微调，语义不变。

## Data flow

1. 用 CID calculator 创建 1 个或多个 Intention（每个 payload = beneficiary Script；多样例可同 beneficiary）
2. Process Calculator → executor `assemble`（传入本批 Intention）
3. Executor 校验数量 ∈ `[batch_min, batch_max]`，返回 partial tx：destroy 本批 Intention、资产落到各自 / 约定 beneficiary、Process consume+recreate
4. FakeRpc / 节点校验 CID + Process

## Testing

- **Unit**：executor 的 `batch`（min/max）、`assemble`、越界与坏格式 → `Incompatible`
- **Integration — 单 Intention**：真实二进制 digest 1 个 Intention，资产在 beneficiary
- **Integration — 多 Intention**：真实二进制一次 digest `n` 个 Intention（`batch_min < n ≤ batch_max`，至少覆盖 n=2 或样例 max），资产均正确落地
- **Negatives**：错误 beneficiary、缺失 Process pair、Intention 数量 < min 或 > max
- 保留现有 CID create / refund / timeout；旧 fixture `digest_ok` 可选保留（仅测锁）

## Out of scope

- Official executor daemon / server
- Arbiter / state-machine escrow
- 修改 CID lock 语义
- 超出样例 `batch_max` 的超大批处理（仅测到 max 边界）

## Notion alignment

对齐 [Common Intention Dispatch 协议设计](https://www.notion.so/3de9f5bf382080249b91d32ad72639d9) 中的角色约定（Process Type ID、SSRI 客户端执行器、`batch` / `Incompatible`）。Notion 尚未写死 SSRI 方法签名；本样例给出最小接口填补该 TBD，并明确 Process 侧 `batch_min` / `batch_max`。
