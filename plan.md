# Plan：M PDF 处理器分阶段实施与 GitHub CI 门禁

**状态：** Active
**日期：** 2026-08-30
**当前分支：** `main`

## 2026-09-06 Surya正式选择与版本冻结

当前状态已先提交为`2f73d03`。随后按`intent.md` §8.6执行0.17.0 vs 0.22.1各25页真实推理，adapter/规则不变；raw和最终geometry全部25/25一致。正式选定Surya，冻结0.17.0 + 2025_05_07 detector + r3.0 adapter。新版默认detector仍是同一checkpoint，无几何升级收益。开发选择/probe为`aligned`；生产接线、打包和独立验收为`unverified`。

[ADR0015](docs/adr/0015-selected-surya-geometry.md) · [probe报告](docs/evidence/surya-upgrade-probe-2026-09-06/report.md) · [精确冻结清单](docs/evidence/surya-upgrade-probe-2026-09-06/provider-freeze.json)

## 2026-09-06 Surya-only apparatus bounded repair完成

依据 `intent.md` §8.5，Burnet1100四条、1400三条恢复独立分行；apparatus匹配21/27→27/27，无新增over-split。两遍25页adapter replay确定性通过，23个非目标页及目标页非apparatus行未变，14项测试通过。原始框与split provenance完整保留。未新增模型推理，未进入holdout/语义D/生产验收。

[修复报告](docs/evidence/surya-apparatus-repair-2026-09-06/report.md)，状态 `SURYA_APPARATUS_ADAPTER_REPAIR_COMPLETE`，意图 `aligned`。

## 2026-09-06 当前结果：Paddle / Surya adapter 决赛第2轮

最新授权见 `intent.md` §8.4。Tesseract已退出本轮预选；两家各完成25页×2遍真实
模型推理，并将各遍原始输出分别转成保留fragment/layout/hierarchy/provenance的
`museion-geometry-evidence/1`。评分前r2.2仅修正split子polygon表示，未改行框、
分组、列或顺序；原始输出和旧表示均保留。

Winner为Surya：logical micro-F1 96.9455%（Paddle 96.9316%），顺序99.93%
（95.85%），逆序对27（1620），本轮定义的灾难性遗漏0页（1页）。Paddle在strict
IoU与apparatus上仍更好，但Brisson发生分段column hierarchy/grouping回归。
列指标基于现有dev字段整理的空间分区，不等同独立D Gold。

两家raw与最终geometry分别25/25两遍一致，10项测试通过。报告与逐页可视化见
[`第2轮报告`](docs/evidence/geometry-finalists-r2-2026-09-06/report.md)。
`GEOMETRY_FINALISTS_R2_COMPLETE`，意图 `aligned`（§8.4）；未冻结provider，
未生成D层最终语义，未接线生产或运行E2E，未读取/修改16页holdout图像与标注。

## 2026-09-06 第1轮范围：实验性 geometry-provider bake-off

以 `intent.md` §8.3 用户澄清为准，下面的产品接线阻塞记录属于被本次范围修订
取代的上一轮任务。Burnet已完成，25页reference现已全部通过核验。此次只使用
这25页比较 Tesseract PSM3、Surya local detection、PaddleOCR local detection/layout，
报告winner与局限；不冻结provider、不接线生产路径、不运行Gemini，不接触16页holdout。
证据目录：`docs/evidence/geometry-dev-bakeoff-2026-09-06/`。

已完成：三家各25页×2遍，150次有效本地推理，无运行失败，逐页有序坐标两遍完全
一致。主指标IoU≥0.30行框F1：Paddle 95.18%、Surya 95.08%、Tesseract 91.20%。
Paddle为微弱winner；IoU≥0.50与逐页等权F1均由Surya领先，不能宣告全面优势。
两家神经检测候选的双栏排序适配器仍有明确缺陷。详情见
[`bake-off报告`](docs/evidence/geometry-dev-bakeoff-2026-09-06/report.md)。
本轮状态 `GEOMETRY_DEV_BAKEOFF_COMPLETE`，与 `intent.md` §8.3 `aligned`；
未冻结provider、未进入生产接线/E2E、未读取16页holdout图像或标注。

## 2026-09-06 上一轮执行状态（已由 §8.3 范围修订取代）

以下记录对应当时 `intent.md` §8.2 的授权与停止点，保留为历史；当前授权见 §8.3。
测量定义已写入并按 SHA-256 锁定于
`docs/evidence/ocr-geometry-dev-2026-09-06/measurement-contract.json`。
当前 reference 草稿 schema 25/25 PASS，人工 OCR/geometry 完成状态及有效凭据
24/25；Burnet PDF0050 第44行仍缺 transcription 核验，已向用户核实是否漏点完成，
未修改人工文字。三重 holdout 隔离和16页冻结摘要检查 PASS。

真实产品路径 **BLOCKED**：CLI `build_cloud_factory` 与桌面 `final_pdf_cloud`
拒绝 Cloud OCR；`PRODUCTION_BACKEND=None`。几何绑定 `GeminiTransport` 只有
测试实现，broker factory 仍连接旧 `CloudOcrProvider`。不以这些测试实现运行评分。
完整25页 development run **未运行**，产品修复迭代 **0/3**，冻结评测 **未运行**。
46项 reference/工作台相关测试 PASS，仅证明这些检查，不代表产品 regression suite。

当前终态：**MILESTONE_BLOCKED — PRODUCT_GEOMETRY_TRANSPORT_NOT_CONNECTED**。
下一步产品修复是实际 broker 的几何绑定请求/响应实现、共享 factory 接线与CLI/桌面
相同配置路径的真实验证；不得用 BYOK、旧整页接口或 fake backend 替代。
Dev exit 后必须报告 **DEV_EXIT_READY** 并停止，等用户确认才运行 frozen holdout。

本文把当前 OCR + AI-ready 中间层方案拆成可以独立审查、测试、回退的 milestones。
每个 milestone 必须先通过本地检查，再提交 GitHub Pull Request；只有远端 CI 全绿并合并后，
才创建下一个 milestone 分支。

## 2026-09-05 OCR/geometry 验收契约

来源：产品负责人本轮明确批准的验收契约；意图修订见 `intent.md` §8。
适用于 deterministic GeometryProvider + Gemini transcription + synthesis。
当前 16 页 frozen Gold 仅判定本模块是否可结束优化，不替代长期 200 页产品验收。

当前用户批准的晋级门槛为 A/B/C/E，必须全部满足。D 按用户后续批准的阶段修订
保留到结构阶段验收，不计作当前 PASS，也不阻塞当前晋级：

| ID | 条件 |
|---|---|
| A | 总体 normalized CER ≤5%；Greek 子集 ≤8%；Latin-script 子集 ≤4%。不得删除困难字符类别或扩大 normalization。 |
| B | line recall ≥99%；不得系统性漏整行或出现页面整块区域遗漏。产品负责人在同轮澄清：“允许不超过 1% 的孤立整行遗漏；系统性或整区遗漏仍 FAIL”。该澄清取代把任一孤立整行遗漏均解释为硬失败的读法。 |
| C | 对可匹配 Gold line，line/region coverage recall ≥98%，reading-order accuracy ≥99%；不得出现正文、脚注、apparatus、页眉页脚大范围串序。轻微 box 松紧不阻塞，但不得损害文本归属、顺序或 PDF 合成。 |
| D（后续结构阶段） | Gold/validator 定义的 semantic isolation 100% PASS：marker/definition 隔离，无 definition-number self-link，continuation/external-target 例外正确，apparatus 非引用 superscript 不误连脚注，无灾难性文本类别错配。延期不等于通过；Gold 及其校验语义不变。 |
| E | 同一冻结 implementation/prompt/model configuration 连续两次 clean-cache dev regression 核心结构稳定；不重复 frozen evaluation 来证明稳定。 |

Dev exit：主要文字指标达到上述目标；没有已知系统性整行/整区遗漏、严重顺序或
大规模 merge/split 问题；已知 footnote/apparatus/superscript/mixed-script 的文字、
几何和顺序问题已处理，语义配对与跨页关系按 D 留到后续结构阶段；同一实现连续两次
clean dev regression 规范化结果稳定。满足即停止
development loop，验证冻结集完整性后进入 frozen evaluation。

最多 3 次 substantive development iterations；达到 dev exit 即停止，不能用剩余预算
继续优化。仅允许补充缺失的测量能力，不修改 frozen Gold、schema、validator、既有
scorer 语义或阈值，不以 holdout 失败实例制作 heuristic、prompt patch 或训练样例。
若指标语义冲突、架构修改必需或发现污染，停止升级；未决测量语义不能算作 PASS。

终态仅为 `OCR_GEOMETRY_MILESTONE_PASS`、`MILESTONE_BLOCKED`、
`FROZEN_GOLD_FAIL` 或 `HOLDOUT_CONTAMINATED`。Frozen FAIL 后停止开发；
PASS 后剩余字符错误进 backlog，不继续 OCR/geometry 优化或自行启动下一 milestone。
目录系统优化不在本轮范围；后续按用户计划迁移既有结构管线、独立输入域验证、
完成扫描 PDF 到 searchable PDF 的 E2E，再进行 release hardening。

修订来源：用户在“只做宏观结构设计”讨论后明确回复“允许做这一修正”，批准上述
D 阶段归属，见 `intent.md` §8.1。当前 OCR 输出须保留文字、标记、位置与顺序证据，
不得删除困难符号或制造未经验证的语义链接；这不是要求本轮新增完整语义识别。

后续职责：几何层负责空间证据；Gemini 提出局部语义候选并绑定行 ID／字符范围；
结构管线解析 marker/definition 和跨页关系；确定性验证／合成检查并写出。
Gold 中的 D 标注保留，不能为了后续迁移把 holdout 转为调参数据。

历史 preflight 终态 **MILESTONE_BLOCKED** 保存在
`docs/evidence/ocr-geometry-contract-preflight-2026-09-05.json`，不重写历史证据。
其中 D 阶段归属阻塞已由本次用户决定解除；这不表示质量通过或其余 preflight 完成。
A–C 操作定义及 E 的开发稳定性证据仍需核对。D 归属修订本身只修改文档。

随后用户以“开始下一步”授权恢复本 milestone。新增 reference preflight 仅检查隔离
和开发参考准备度，不运行 OCR：16 页冻结集摘要完整；按 page ID、实际图像摘要及
来源 PDF 摘要＋页号排除 holdout 后，剩余 25 页均无穷尽可见行核验，24 页存在未核验
几何／转录行，当前可用整页开发参考为 **0**。结构与 typography 的 D 核验不作为
该检查的前置条件。12 项新增及相邻测试通过。结果见
`docs/evidence/ocr-geometry-dev-reference-preflight-2026-09-05.json`。

当前终态：**MILESTONE_BLOCKED — DEV_REFERENCE_EVIDENCE_MISSING**。
Substantive iterations **0/3**；frozen evaluation **未运行**；模型调用 **0**。
已有合成文本与干净原生几何控制不能替代真实开发页的整页 A/B/C 证据或当前完整
管线的两次 E 回归。下一准备项是非 holdout 页的独立文字、行框、顺序与穷尽覆盖核验；
不要求本轮提前完成 D。当前提议的 CER 子集归属、行匹配和顺序操作定义尚未获用户
答复，不把提议记为批准，且未用于评分。真实几何绑定 Gemini transport 的运行接入
仍需验证，旧 whole-page 云端或 fake transport 不构成当前完整管线质量证据。

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

### M0 — 命名解耦与 CI 基线（已合并）

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
- 当前 `package create` 复用 `inspect` 的会话路径生成最小 MDP，不接真实 OCR；`process` 联动导出推迟；
- 为后续书签预留 page label、existing outline、typography 和 region evidence 字段。

实现进度（本分支）：`mpdf-core::document_package` 已提供 MDP 0.1 的
manifest/source/pages/assets/provenance/validation 类型、确定性 SHA-256 ID、
顶左母版坐标与 PDF point affine transform、安全目录读写和 validator；CLI
已加入 `package create <PDF> --output <DIR>` 与 `package validate <DIR>`。
当前 `create` 复用 `PdfDocumentSession` 取得真实页几何和源摘要，不复制外部源 PDF；
JSON Schema 位于 `schemas/mpdf-document-package-0.1.schema.json`。完整本地门禁、真实 PDFium
集成测试、CLI 正负向冒烟测试和 GitHub CI 均已通过，并由 PR #14 合并。

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

实现进度（本分支）：`mpdf_core::jobs` 已提供 SQLite WAL job/page 状态库、租约心跳、
有界重试、逐页 checkpoint、取消与崩溃恢复；并提供版本化 `mpdf-job` NDJSON sidecar、
reference/fake provider 及完整失败模式测试。CLI 的 `job create/status/cancel` 仅操作本地
任务状态，桌面侧提供可恢复的任务进度 DTO；provider attempt provenance 已与成功
checkpoint 或失败结果原子写入 SQLite；真实 OCR provider 和 process 联动仍等待后续里程碑。
完整本地门禁、500 页恢复验收、CLI 正负向冒烟测试和 GitHub CI 均已通过，并由
PR #15 合并。

出口条件：模拟 500 页任务中止后能从最后已提交页恢复；取消不会删除已确认产物；sidecar
崩溃不会形成伪成功任务。

### M3 — 本地 OCR 最小闭环（已合并）

**目标：** 本地优先完成 PDF 导入、逐页路由、OCR、MDP 写入和基础导出。

交付：

- born-digital 页面优先抽取原生文字；仅对缺字、乱码或扫描页运行 OCR；
- 首个本地 adapter 使用 RapidOCR/ONNX；保留专项古希腊文 provider 插槽；
- 分页渲染和有界并发，不把整本 PDF 图片一次性驻留内存；
- block/line/word 文本、bbox、confidence、reading order 和原始 provider artifact 写入 MDP；
- 原文、Unicode 规范化文本和人工/AI revision 分层保存；
- CLI 一键处理路径，以及最小桌面设置、进度、取消和逐页错误显示。

实现结果（PR #16 已合并）：`mpdf ocr` 已接入 PDFium 原生文字优先路由、逐页
300 DPI 渲染、typed `ocr/` MDP 扩展记录、离线 reference provider，以及显式 argv
调用的 RapidOCR/ONNX sidecar runner；单个 PDF session 按页写入 M2 SQLite job
checkpoint，支持重跑跳过已校验页、取消保留已提交页，失败页保留为可诊断错误而不伪造
成功。桌面已有 provider 设置、持久状态/取消/逐页错误查询 wiring（实际启动仍由 CLI
控制）；真实 RapidOCR fixture 仍是可选门禁（不下载模型），而 reference provider
已覆盖基于 checkpoint 的重启跳过。
page JSON 是提交标记，raw artifact 先落盘并可幂等校验；崩溃留下的合法 page+raw 可被
同源新 job adopt，临时 provider 失败按 M2 retryable 语义在后续运行重试。RapidOCR
指纹包含协议、DPI、配置和三个 ONNX 模型内容摘要；原生文字的行/词框明确是近似几何。
该里程碑保留为 v1 历史与兼容基线。根据 ADR 0011，当前发行契约不再把本地
OCR 运行时当作基础包必需组件；它以独立可选插件交付。

出口条件：扫描 PDF、混合 PDF 和 born-digital PDF fixture 全部通过；重跑可复用已完成页；
内存随并发页数有界；没有模型时基础二值化仍可用。

### M4 — AI-ready 派生物与校对工作台（已合并）

**目标：** 把证据 IR 转为机器和人都能消费的产物，但不以 Markdown 取代 IR。

交付：

- 由 MDP 可重复生成 JSON/JSONL、Markdown、TXT、HTML 和 hOCR/ALTO 候选导出；
- 按页、区域和 token 的稳定引用，支持 RAG chunk 与原页反查；
- 三栏校对界面：页面图像、结构文本、属性/置信与修订；
- 低置信、阅读顺序冲突、Unicode 差异和疑似漏识别的审核队列；
- 修改日志和派生产物失效/重建机制。

实现结果（PR #17 已合并，merge `181265f`）：`mpdf-core::derived` 已提供版本化 typed IR、稳定
page/region/block/line/word/chunk 引用、母版坐标反查、四类审核问题、append-only human/AI
revision overlay，以及 JSON、JSONL、Markdown、TXT、HTML、hOCR、ALTO 七种确定性导出。
`mpdf export/review/revision` 已完成二进制级闭环；全格式 bundle 使用精确白名单、内容摘要、
输入/修订摘要和原子目录替换来检测 stale/corrupt 状态。桌面加入三栏本地校对工作台；没有
预览资产时明确显示 page/bbox 证据，不伪造页面图像。本地 Rust、前端、分发、cargo-deny
和真实 PDFium → OCR → derived → 七格式集成门禁均已通过，GitHub CI 全绿。

出口条件：所有导出均能定位回源页与 bbox；人工修订不会覆盖原始 OCR；从同一 MDP 重建
派生物结果确定。

### M5 — 证据化自动书签与可搜索 PDF（已合并）

**目标：** 使用 M1–M4 已保存的材料生成可审查书签，而非让模型自由猜目录。

实现结果（PR #18 已合并，merge `42619f0`）：已接受 ADR 0007，冻结版本化候选/证据/append-only review 契约、
确定性离线规则、旋转与页面坐标映射、嵌入式 Unicode Type-0 字体、不可见文字层、outline
目的地、源摘要绑定和原子 no-clobber 输出规则；core、CLI、桌面与真实 PDFium 闭环已经完成，
本地全量门禁、真实 PDFium 重开验证和 GitHub CI 全部通过。

严格出口复核（2026-08-27）：自动化 conformance fixture 与 PDFium、PDFKit、qpdf、Poppler、
Ghostscript、固定 PDF.js 5.4.624 引擎矩阵通过；Foxit PDF Editor 2026.1 UI 的打开、
书签层级、目标页跳转和希腊文搜索也通过。翻译 agent 2 的 20 份真实 PDF（4,869 页）
已加入外部语料 manifest：19 份共 574 条原生 outline 的标题、层级、物理目标页全部精确
保留，1 份无效目标按预期 fail closed。另已从相同 input 固定 12 份互不重复的人工验收样本，
分为 digital TOC、scanned TOC 与 safe-refusal 各 4 份；单一权威标注者的冻结模板、校验器和
Acrobat/Preview/iOS 固定 fixture 已制作。按产品负责人决定，本轮不再要求两名标注者或
标注者间一致性指标；但人工标注表和三端 UI 结果尚未填写，故严格出口门仍未完成。详见
`docs/benchmark-results/m5-exit-acceptance-2026-08-27.md`。
验收补充 PR #21 已合并，merge `a4ce00a`。
单一权威标注与三端人工验收包 PR #23 已合并，merge `2ff7001`。
M5 人工 PDF 阅读器结果由产品负责人后补；2026-08-28 产品负责人明确授权该结果不再阻塞
M6。M5 的待填验收包仍保留并继续如实标记为 pending，不把未完成的人工测试写成通过。

交付：

- 证据信号：原 PDF outline、目录页、印刷页码、标题区域、字体/字号、编号、重复页眉页脚、
  阅读顺序和用户修订；
- bookmark candidate 包含标题、层级、目标页、证据引用、置信状态和生成者；
- 书签只消费经验证的规范原生文字/OCR 证据和用户修订，由确定性核心完成
  目录检测、页码映射、层级、评分和 safe refusal；provider/enhancer 不直接写 outline；
- 书签审阅 UI；
- 写入不可见文字层和 PDF outline，并重新打开验证页数、坐标、搜索和跳转目标。

出口条件：目标页准确率、层级树指标和 Unicode 标题指标达到基准门；低置信条目不自动冒充
已确认结果；专业 PDF 在目标阅读器矩阵通过。

### M6 — API provider 与跨设备任务

**目标：** 在不改变 MDP 和 UI 语义的前提下增加 API 路径。

状态（2026-08-28）：代码已合并（PR #25，merge `3dd3389`）。M7A distribution
foundation 与 M7B1 MAS technical path 已存在；本轮开始 rc.3 release-candidate
hardening。生产构建、签名、公证、发布状态仍分别记录，不作合并声称。
ADR 0008 已冻结
network-free core、独立 HTTPS client、显式摘要绑定上传许可、原生 credential store、整数预算、
append-only audit、portable task receipt、可见 retention 与显式 fallback 契约。首个远程操作仅为
OCR；不在 M6 引入云端 LLM bookmark 生成或具体厂商 SDK。

交付：

- API provider 复用 M2 契约；
- 内容摘要去重、幂等 request ID、重试/限流、成本上限和审计记录；
- 明确的本地/API 路由、上传前确认、数据保留策略和密钥存储；
- API 原始响应作为 provider artifact 保存，统一映射进 MDP；
- 服务不可用或预算触顶时可退回本地或暂停，不静默丢页。

出口条件：关闭云端时产品完整成立；相同任务可切换 provider；隐私、失败语义、成本和删除
策略均有自动化测试与用户可见说明。

上述 M6 是已合并的历史基线。ADR 0011 已取代 ADR 0010 的默认产品决策：文本转写
不再伪装成完整 OCR，Gemini BYOK 已禁用，托管 `mpdf-credits` 在无生产完整坐标
OCR 服务时报告 `unavailable`。

### OCR provider contract v2（ADR 0011，当前契约）

**目标：** 把确定性核心、完整坐标 OCR 与文字增强拆成三层，并使基础发行包
不依赖 OCR 运行时。

交付：

- `CompleteOcrProvider` 使用 `mpdf-ocr-provider/2`、schema `mpdf-ocr-provider` 0.2，
  必须独立返回 block/line/word 文本、实测坐标、阅读顺序与 provenance；
- `TextEnhancer` 仅对现有规范坐标证据提出可审阅文字修订，不能充当 OCR 或
  书签决策器；历史 Gemini composite 只保留为实验路径；
- 用户可选 mode 仅为 `local` 与 `mpdf-credits`；`local` 是稳定的离线/原生文字路由，
  扫描页 OCR 能力则取决于运行时是否发现并验证可选插件；`mpdf-credits` 是需明确
  同意和点数上限的付费托管模式，当前不可用；
- 历史 `gemini-byok` 值保持可反序列化，但 availability 为 `disabled`、不出现在
  provider list/picker，且不会静默切换模式；
- 基础包无插件时原生文字 PDF 正常工作，需要 OCR 的扫描页明确报告 provider
  不可用；本地 OCR 插件通过独立 overlay/staging/verifier/SBOM/smoke 契约交付；
- 旧 MDP/job/bookmark/provider 记录按已声明 schema 继续读取，历史 composite 证据不重标。

出口条件：基础发行 readiness 不需要 OCR 插件证据；
`optional-local-ocr-plugin` 独立 profile 必须通过结构、许可、安装后四页 OCR smoke 与
人工准确性门禁。

### 自动书签目录 v2（独立功能，非 milestone 编号）

**目标：** 把 M3–M6 已保存的 OCR/原生文字证据，编译成可验证、可自动写入 PDF 的标准
outline；不是让模型自由生成语义目录。

状态（2026-08-28）：Opus 实现已由远端提交 `1a3ccc1` 交付（基于 M6 merge
`3dd3389`）；本轮在本地修复分支 `codex/auto-bookmark-v2-fixes` 做发布前回归，按要求
不提交、不推送、不建 PR。本功能刻意不复用 M7 编号。

交付：

- ADR 0009 冻结三种业务结果（`existing_outline` / `toc_aligned` / `safe_refusal`）、
  provider-neutral 证据、`auto_confirmed` 与人工 `confirmed` 的区别、固定整数评分、
  分段页码映射、单调对齐，以及 PDF 重开验证；
- `mpdf-core::bookmarks` 拆分为 text_index / toc_detect / toc_parse / align /
  hierarchy / scoring / engine / assembly / config 模块，磁盘装配与 PDF 安全写入各只有
  一处实现（`load_auto_bookmark_inputs`、`searchable_output`）；
- bookmark snapshot 0.2 与 `mpdf-bookmark-generation-report` 0.1 两个新 schema；0.1
  快照与 review 继续可读、可审阅、可构建，且不能凭改标签取得自动状态；
- CLI 新增 `mpdf bookmark auto`（一条命令生成并写出已验证 PDF），`bookmark generate`
  改为调用同一 v2 engine；`--overwrite` 与 `--regenerate` 权限分离；
- 桌面新增"自动添加目录书签"单按钮入口、原生路径选择、阶段进度、取消、忙碌门禁、
  安全拒绝结果面板与自动/人工状态区分；
- 新增 23 个 core 契约测试、7 个无 PDFium 的 CLI 集成测试、6 个桌面 Rust 测试、
  10 个 React 测试，以及自动书签评测入口 `scripts/bookmarks/auto_bookmark_eval.py`
  （含公式自测）。

未完成/未运行（如实记录）：

- 当前 Mac 已有经 manifest 校验的 PDFium：
  `target/pdfium/aarch64-apple-darwin/libpdfium.dylib`。
  本轮用它实际运行了 `auto_bookmarks_pdf`（ignored 3/3）、`searchable_pdf`（1/1）及
  相关 PDFium 门；ignored 测试只有显式命令才计入上述计数。
- `scripts/m5/check_reader_matrix.sh` 已使用全新的输出目录实际通过（qpdf、pdfinfo、
  pdftotext、Ghostscript、Swift PDFKit、Node PDF.js 均可用并通过）；商业阅读器真机
  交互未运行，不写成通过；
- 没有真实人工金标准语料，因此没有任何真实准确率数字；评测脚本在语料缺失时输出
  `not_run`/`pending`；
- 评分阈值是保守冻结基线，未按语料校准。

### M7 — 发布硬化与正式命名（rc.3 第一轮）

**目标：** 在功能和格式稳定后完成 rc.3 可审查分发；正式产品身份已冻结，
法律/owner 清查仍是外部门禁。

交付：

- 产品名、仓库名、bundle identifier、CLI/crate/schema identity 冻结与兼容矩阵；
- Windows/macOS/Linux 安装包与升级验证；
- 基础包 SBOM、签名、公证、release manifest 和回滚演练；可选本地 OCR 插件的
  模型/运行时许可、SBOM 和 smoke 证据独立管理；
- 性能、OCR、书签、隐私和无障碍发布报告。

出口条件：三平台分发 CI 和安装实测通过；正式命名迁移不会破坏已有 MDP、设置或自动化脚本。

## 4. 状态表

| Milestone | 状态 | 分支/PR | 下一门禁 |
|---|---|---|---|
| M0 命名与 CI | 已合并 | [PR #13](https://github.com/Museion-Project/museion-binarize/pull/13) | GitHub CI 全绿；merge `f37a72e` |
| M1 MDP 0.1 | 已合并 | [PR #14](https://github.com/Museion-Project/museion-binarize/pull/14) | GitHub CI 全绿；merge `8179b44` |
| M2 任务与 provider | 已合并 | [PR #15](https://github.com/Museion-Project/museion-binarize/pull/15) | GitHub CI 全绿；merge `72576e3` |
| M3 本地 OCR | 已合并 | PR #16；merge `dfc186c` | 后续回归门禁继续保持绿色 |
| M4 AI-ready/校对 | 已合并 | [PR #17](https://github.com/Museion-Project/museion-binarize/pull/17) | GitHub CI 全绿；merge `181265f` |
| M5 自动书签/PDF | 代码已合并；人工结果后补（不阻塞 M6） | [PR #18](https://github.com/Museion-Project/museion-binarize/pull/18)；[验收 PR #21](https://github.com/Museion-Project/museion-binarize/pull/21)；[人工验收包 PR #23](https://github.com/Museion-Project/museion-binarize/pull/23)，merge `2ff7001` | 产品负责人之后提供 Acrobat/Preview/iOS 与单人标注结果 |
| M6 API | 已合并 | [PR #25](https://github.com/Museion-Project/museion-binarize/pull/25)；merge `3dd3389` | 发布准备留待 M7 |
| OCR provider contract v2 | 当前契约 | ADR 0011 | 基础包与可选本地 OCR 插件独立验收；`mpdf-credits` 保持 unavailable |
| 自动书签目录 v2 | 已交付并保留于 commit `11b0ae1` | `codex/auto-bookmark-v2-fixes` | 真实 reader/人工 gold/真机 Tauri 仍需分别记录，不得伪报通过 |
| M7 发布/正式命名 | rc.3 hardening 进行中 | 本轮源码 | 版本/身份、SBOM、分发、签名、公证与跨平台证据分别过门 |

## 5. 当前阻塞

M5 严格产品出口门已有 20 份真实原生 outline 语料、12 份单一权威标注者验收样本、固定
PDF.js 及 Foxit 证据。产品负责人将之后补充 digital/scanned TOC、safe-refusal、Acrobat、
Preview UI 和 iOS 结果；这些结果仍不得伪报通过，但按 2026-08-28 的明确授权不再阻塞 M6。
自动书签目录 v2 的代码回归和 reader matrix 已使用本机认可 PDFium 完成；商业阅读器真机、
真实人工金标准语料和真机 Tauri/MAS 交互仍未运行，本轮不产出真实准确率或人工验收数字。
另外，可选本地 OCR 插件尚未在所有平台完成 Tesseract、sidecar、固定
`tessdata_best` 模型的独立产物验收；源码环境的合成 gold 通过不能替代这个
分发证据。这是 `optional-local-ocr-plugin` profile 的阻塞项，不阻塞无 OCR 运行时的
基础发行包；基础包中原生文字 PDF 正常工作，需 OCR 的扫描页会明确失败。

M6 当前没有外部阻塞。GitHub 权限不是阻塞：2026-08-26
已确认账号 `pei-haoran` 授权有效，并对 `Museion-Project/museion-binarize` 具有管理员权限。
