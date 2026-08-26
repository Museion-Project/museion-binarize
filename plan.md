# Plan：M PDF 处理器分阶段实施与 GitHub CI 门禁

**状态：** Active
**日期：** 2026-08-26
**当前分支：** `codex/m-pdf-m0-naming-ci`

本文把当前 OCR + AI-ready 中间层方案拆成可以独立审查、测试、回退的 milestones。
每个 milestone 必须先通过本地检查，再提交 GitHub Pull Request；只有远端 CI 全绿并合并后，
才创建下一个 milestone 分支。

## 1. 临时命名约定

- 中文展示名：**M PDF 处理器**；
- 英文展示名：**M PDF Processor**；
- 技术占位前缀：`mpdf`；
- CLI：`mpdf`；
- 中间包名称：**Machine-readable Document Package（MDP）**；
- M 只是临时占位符，不代表任何既有品牌，也不是最终名称；
- 展示名必须集中配置。IR、数据库和 provider 契约不得依赖未来品牌名称；
- 仓库根目录和 GitHub 仓库名的迁移属于独立远端操作，恢复 GitHub 授权后再执行。

## 2. 固定交付循环

每个 milestone 严格执行：

1. 从最新 `main` 创建 `codex/m-pdf-mN-<topic>`；
2. 只实现该 milestone 的验收范围，并补齐相邻单元/契约测试；
3. 运行本地门禁；
4. 提交并推送分支，创建 PR；
5. 等待 GitHub CI 全部通过；失败则只在当前 milestone 修复；
6. CI 全绿后 squash merge；
7. 更新本文件中的状态和实际测试证据，再开始下一 milestone。

不得把多个 milestone 堆进同一个 PR，也不得以“本地通过”代替 GitHub CI。

### 2.1 每个 PR 的最低本地门禁

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/distribution/check_version_consistency.py
python3 scripts/distribution/test_distribution.py
pnpm install --frozen-lockfile
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

涉及 PDFium、sidecar、真实 OCR 或输出 PDF 的 milestone，必须额外运行该阶段列出的集成检查。

### 2.2 GitHub 必需检查

- `Rust (fmt, clippy, test)`；
- `cargo-deny (licenses, advisories)`；
- `Version consistency`；
- `Frontend (lint, typecheck, test, build)`；
- milestone 新增的契约/fixture 检查。

发布和多平台安装包 workflow 不作为日常 PR 的替代品；涉及打包的 milestone 必须额外触发
Windows、macOS、Linux 分发构建。

## 3. Milestones

### M0 — 命名解耦与 CI 基线（本地完成，等待 GitHub CI）

**目标：** 删除当前产品代码和构建配置对旧品牌的依赖，建立后续开发的可靠门禁。

交付：

- 展示名统一为 M PDF 处理器 / M PDF Processor；
- Rust package/lib/binary、npm package、Tauri identifier、事件 namespace、环境变量和构建产物
  统一采用 `mpdf` 技术占位名；
- 分发脚本、workflow、版本一致性检查和现有测试同步迁移；
- PR CI 覆盖 Rust、前端、许可证和分发脚本；
- 文档明确区分临时展示名、稳定技术标识和未来正式品牌。

出口条件：仓库代码/配置中不存在无兼容理由的旧产品名；本地门禁和 GitHub CI 全绿；现有
二值化功能无回退。

### M1 — MDP 0.1：无 OCR 的证据包垂直切片

**目标：** 先冻结机器可读中间产物，而不是先绑定某个 OCR 引擎。

交付：

- `manifest/source/pages/assets/provenance/validation` 的 Rust 类型和 JSON Schema；
- 稳定 document/page/asset ID、SHA-256 摘要、相对路径与版本拒绝规则；
- 左上角原点的归一化母版坐标空间，以及 pixel/PDF point 显式变换；
- 目录容器、原子写入、validator 和损坏 fixture；
- 当前 `inspect/process` 可导出最小 MDP，不接真实 OCR；
- 为后续书签预留 page label、existing outline、typography 和 region evidence 字段。

出口条件：同一输入和参数产生可验证、可追溯的包；路径逃逸、摘要错误、未知主版本和残缺
资源均被拒绝；现有 PDF 输出测试继续通过。

### M2 — 持久任务系统与 OCR provider 契约

**目标：** 在真实模型进入前解决长文档恢复、取消和 provider 隔离。

交付：

- SQLite WAL 任务库，含 job/page 状态、heartbeat、lease、retry 和 crash recovery；
- 每页 checkpoint、原子状态转换、取消与重启恢复；
- 版本化 NDJSON sidecar 协议；
- reference/fake provider，覆盖成功、部分失败、超时、崩溃、乱序和协议版本不兼容；
- provider 运行记录包含引擎、模型、版本、参数、输入资产摘要和执行位置；
- CLI 开发入口和桌面任务进度状态，不接真实 OCR。

出口条件：模拟 500 页任务中止后能从最后已提交页恢复；取消不会删除已确认产物；sidecar
崩溃不会形成伪成功任务。

### M3 — 本地 OCR 最小闭环

**目标：** 本地优先完成 PDF 导入、逐页路由、OCR、MDP 写入和基础导出。

交付：

- born-digital 页面优先抽取原生文字；仅对缺字、乱码或扫描页运行 OCR；
- 首个本地 adapter 使用 RapidOCR/ONNX；保留专项古希腊文 provider 插槽；
- 分页渲染和有界并发，不把整本 PDF 图片一次性驻留内存；
- block/line/word 文本、bbox、confidence、reading order 和原始 provider artifact 写入 MDP；
- 原文、Unicode 规范化文本和人工/AI revision 分层保存；
- CLI 一键处理路径，以及最小桌面设置、进度、取消和逐页错误显示。

出口条件：扫描 PDF、混合 PDF 和 born-digital PDF fixture 全部通过；重跑可复用已完成页；
内存随并发页数有界；没有模型时基础二值化仍可用。

### M4 — AI-ready 派生物与校对工作台

**目标：** 把证据 IR 转为机器和人都能消费的产物，但不以 Markdown 取代 IR。

交付：

- 由 MDP 可重复生成 JSON/JSONL、Markdown、TXT、HTML 和 hOCR/ALTO 候选导出；
- 按页、区域和 token 的稳定引用，支持 RAG chunk 与原页反查；
- 三栏校对界面：页面图像、结构文本、属性/置信与修订；
- 低置信、阅读顺序冲突、Unicode 差异和疑似漏识别的审核队列；
- 修改日志和派生产物失效/重建机制。

出口条件：所有导出均能定位回源页与 bbox；人工修订不会覆盖原始 OCR；从同一 MDP 重建
派生物结果确定。

### M5 — 证据化自动书签与可搜索 PDF

**目标：** 使用 M1–M4 已保存的材料生成可审查书签，而非让模型自由猜目录。

交付：

- 证据信号：原 PDF outline、目录页、印刷页码、标题区域、字体/字号、编号、重复页眉页脚、
  阅读顺序和用户修订；
- bookmark candidate 包含标题、层级、目标页、证据引用、置信状态和生成者；
- 确定性规则先行，可选 AI 只做结构建议和疑难匹配；
- 书签审阅 UI；
- 写入不可见文字层和 PDF outline，并重新打开验证页数、坐标、搜索和跳转目标。

出口条件：目标页准确率、层级树指标和 Unicode 标题指标达到基准门；低置信条目不自动冒充
已确认结果；专业 PDF 在目标阅读器矩阵通过。

### M6 — API provider 与跨设备任务

**目标：** 在不改变 MDP 和 UI 语义的前提下增加 API 路径。

交付：

- API provider 复用 M2 契约；
- 内容摘要去重、幂等 request ID、重试/限流、成本上限和审计记录；
- 明确的本地/API 路由、上传前确认、数据保留策略和密钥存储；
- API 原始响应作为 provider artifact 保存，统一映射进 MDP；
- 服务不可用或预算触顶时可退回本地或暂停，不静默丢页。

出口条件：关闭云端时产品完整成立；相同任务可切换 provider；隐私、失败语义、成本和删除
策略均有自动化测试与用户可见说明。

### M7 — 发布硬化与正式命名

**目标：** 在功能和格式稳定后再决定最终品牌及发布迁移。

交付：

- 正式产品名、仓库名、域名、应用 identifier 和商标检查；
- 从 `mpdf` 占位名迁移的兼容矩阵；
- Windows/macOS/Linux 安装包与升级验证；
- SBOM、第三方模型许可、签名、公证、release manifest 和回滚演练；
- 性能、OCR、书签、隐私和无障碍发布报告。

出口条件：三平台分发 CI 和安装实测通过；正式命名迁移不会破坏已有 MDP、设置或自动化脚本。

## 4. 状态表

| Milestone | 状态 | 分支/PR | 下一门禁 |
|---|---|---|---|
| M0 命名与 CI | 本地完成，等待 CI | `codex/m-pdf-m0-naming-ci` | 恢复 GitHub 授权后提交、推送并创建 PR |
| M1 MDP 0.1 | 未开始 | — | M0 CI 全绿并合并 |
| M2 任务与 provider | 未开始 | — | M1 CI 全绿并合并 |
| M3 本地 OCR | 未开始 | — | M2 CI 全绿并合并 |
| M4 AI-ready/校对 | 未开始 | — | M3 CI 全绿并合并 |
| M5 自动书签/PDF | 未开始 | — | M4 CI 全绿并合并 |
| M6 API | 未开始 | — | M5 CI 全绿并合并 |
| M7 发布/正式命名 | 未开始 | — | M6 CI 全绿并合并 |

## 5. 当前阻塞

`gh auth status` 于 2026-08-26 显示两个已配置 GitHub 账号的 token 均失效。M0 可以继续在
本地实现和测试，但在重新执行 `gh auth login -h github.com` 前，不能推送分支、创建 PR、
确认仓库写权限或等待 GitHub CI。
