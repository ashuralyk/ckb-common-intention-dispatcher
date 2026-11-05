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
- **Approach A**：竖切最小可测 — 真实二进制 + executor + 测试一刀落地

## Architecture

```
Intention create / refund / timeout  →  CID calculator（现有；不动 digest 业务）
Intention digest_ok                  →  Process SSRI Calculator
                                         └─ cid-process-executor
                                              └─ 离线 SSRI 跑 cid_process_example
On-chain                             →  CID lock Process pair 检查
                                         + Process type verify（beneficiary 输出）
```

## Components

1. **`contracts/cid_process_example`** — Type ID Process，SSRI 方法，链上 verify
2. **Process Calculator** — 组 digest 骨架，调用 executor
3. **`crates/cid-process-executor`** — 加载 RISC-V，调用 SSRI，返回 partial tx 或 `Incompatible`
4. **`tests/`** — FakeRpc：真实 Process 二进制 + executor + Process Calculator

## SSRI surface (sample)

- `SSRI.get_methods`
- `Process.batch` → `1`（v1）
- `Process.assemble` → partial execution tx；坏格式 → `Incompatible`
- 链上 type：beneficiary 输出 + Process pair 连续性（Type ID）

方法名可按 ckb-ssri-std 惯例在实现时微调，语义不变。

## Data flow

1. 用 CID calculator 创建 Intention（payload = beneficiary Script）
2. Process Calculator → executor `assemble`
3. Executor 返回 partial tx：destroy Intention、资产落到 beneficiary、Process consume+recreate
4. FakeRpc / 节点校验 CID + Process

## Testing

- **Unit**：executor 的 `batch` / `assemble` / `Incompatible`
- **Integration**：真实二进制 digest，资产在 beneficiary
- **Negatives**：错误 beneficiary、缺失 Process pair
- 保留现有 CID create / refund / timeout；旧 fixture `digest_ok` 可选保留（仅测锁）

## Out of scope

- Official executor daemon / server
- Multi-intention batch > 1
- Arbiter / state-machine escrow
- 修改 CID lock 语义

## Notion alignment

对齐 [Common Intention Dispatch 协议设计](https://www.notion.so/3de9f5bf382080249b91d32ad72639d9) 中的角色约定（Process Type ID、SSRI 客户端执行器、`batch` / `Incompatible`）。Notion 尚未写死 SSRI 方法签名；本样例给出最小接口填补该 TBD。
