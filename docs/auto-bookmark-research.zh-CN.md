# M PDF 处理器自动 Bookmark：研究结论与独立实现方案

**日期：** 2026-08-26
**状态：** 方案研究完成，尚未实现
**目标：** 在不融合第三方项目代码的前提下，为 M PDF 处理器设计“足够简单、足够准确”的自动 PDF 书签管线，并分别支持电脑本地模式与电脑/手机 API 模式。

## 1. 结论先行

M PDF 处理器不应把自动 Bookmark 做成“让大模型通读整本 PDF，自由创作一个目录”。最可靠、也最简洁的产品定义是：

> **把文档已经表达出来的结构编译成可验证的 PDF outline。**

推荐的 v1 只有一条主路径：

1. 有效的原生 PDF outline 直接保留；
2. 否则从印刷目录页取得候选标题、层级与印刷页码；
3. 在正文中寻找标题证据，联合求解“印刷页码 → PDF 页码”；
4. 只有通过确定性验证的条目才自动写入；不确定项默认跳过，不要求用户逐条确认；
5. 写入标准 PDF outline 后重新打开并验证。

本地版和 API 版必须共用同一个确定性 `OutlineEngine`。二者只在“证据如何取得”上不同：本地使用 PDF 文字层或本地 OCR，API 可使用服务器 OCR/VLM；模型只能生成带坐标的候选，不能越过验证器直接决定最终书签。

不建议在 v1 做“没有目录页时根据全文主题自动创作章节”。那是主观的语义摘要，不是可客观核验的书签恢复，应当以后作为独立的“语义目录”实验功能处理。

## 2. 术语与产品边界

本报告中的 **bookmark** 指 PDF 标准 outline：每项至少包含标题、树深和可跳转的 PDF 目标页/位置。

它不等同于：

- 阅读器内部的虚拟目录；
- 用户标记“稍后回来”的页面书签；
- 根据内容摘要生成的主题列表；
- Markdown 标题或 RAG chunk 的标题路径。

MarginNote 4 是重要真实案例，但其官方手册说明，AI 生成的是可编辑、可重置的**虚拟目录**，不会修改源 PDF 的目录数据。因此它证明了“一键生成 + 可选编辑 + 跨端消费”的产品需求，却不能直接证明标准 PDF outline 的生成方法或准确率。[MarginNote 4 用户手册](https://manual.marginnote.com.cn/mn4/en/manually-automatically-generating-document-table-contents/)

## 3. 外部证据

### 3.1 近期 GitHub release 与实现

| 项目/版本 | 与本项目相关的事实 | 可借鉴而不融合的原则 | 不直接采用的原因 |
|---|---|---|---|
| [Docling v2.117.0](https://github.com/docling-project/docling/releases)（2026-07-30） | release 专门修复深层 PDF outline 遍历的递归溢出；当前 heading hierarchy 实现采用 `bookmarks > numbering > style` 的优先级 | 原生 outline 是最强证据；遍历必须防环、防超深 | 完整 Docling 管线过重，且其目标是通用文档转换，不是可靠写回 PDF |
| [PaddleOCR v3.5.0](https://github.com/PaddlePaddle/PaddleOCR/releases)（2026-04-21）及 PP-StructureV3 | 已形成 OCR、布局、阅读顺序和结构化导出的完整管线；PP-StructureV3 能识别标题等布局区域 | OCR、布局和 outline 应通过中间结构解耦 | 运行时和模型体积大；“识别标题区域”不等于“正确恢复整本书层级和目标页” |
| [MinerU 3.x](https://github.com/opendatalab/MinerU) | 证明复杂 PDF 的 OCR、布局与结构化解析可以统一服务化 | API 端可把解析作为异步任务 | 功能面过宽，当前自定义许可证也需要单独法律评估 |
| [docling-hierarchical-pdf](https://github.com/krrome/docling-hierarchical-pdf) 0.0.1 | 使用 bookmark、编号和字体样式推断层级；作者报告在 60+ 文本 PDF 上结果满意，但图像数据效果受上游 VLM 限制 | 多信号的优先级应固定，不能让弱样式覆盖强证据 | 公开评测规模小，扫描件能力不足，且依赖完整 Docling |
| [pdf.tocgen](https://github.com/Krasjet/pdf.tocgen) | 将元数据抽取、规则生成、PDF 写入拆成三个程序；适合软件生成 PDF | “抽取—求解—写入”分层非常正确 | 明确不适合扫描 PDF；GPL/AGPL 依赖不适合直接融合进当前 MIT/Apache 产品 |
| [pdf-bookmarks](https://github.com/dillettante/pdf-bookmarks) | 近期真实原型覆盖多页/双栏目录、页码偏移、旋转和 OCR，并始终输出新文件 | 目录页优先、自动 offset、原件不覆盖 | 项目仍早期；Windows OCR 明示未在真机验证；包含大量平台与脚本耦合 |

Docling 2026 年的实现变化尤其有参考价值：它把原 PDF outline 作为权威信号，再退到编号，最后才使用字体样式；低置信匹配不覆盖其他证据。[当前实现](https://github.com/docling-project/docling/blob/main/docling/models/stages/heading_hierarchy/heading_hierarchy_model.py) 这与本项目应采用的保守策略一致。

### 3.2 arXiv 与公开数据集

- [HiPS: Hierarchical PDF Segmentation of Textbooks](https://arxiv.org/abs/2509.00909)（2025）直接比较了基于 TOC、开源结构解析器和无显式 TOC 的 LLM 方法。其结论支持：高质量 heading 元数据存在时，TOC 路径表现尤其好；LLM 需要结构感知预处理来降低假阳性。
- [Multimodal Tree Decoder for Table of Contents Extraction](https://arxiv.org/abs/2212.02896) 提供 HierDoc（650 份科学文档）并报告 87.2% TEDS、88.1% F1。它证明端到端树解码可行，也同时说明单靠模型的公开结果距离本项目的 99% 目标页准确率仍很远。
- [HRDoc](https://arxiv.org/abs/2303.13839) 将问题正式定义为跨页文档层级重建，数据含 2,500 份文档、近 200 万语义单元。它适合验证标题分类和父子关系，但主要来源仍不是历史扫描专著。
- [DocLayNet](https://arxiv.org/abs/2206.01062) 含 80,863 个人工标注页面和 11 个布局类别，适合预训练/验证标题区域检测；它不提供完整 bookmark 目标真值。
- [OmniDocBench](https://arxiv.org/abs/2412.07626) 覆盖九类文档、19 个布局标签，适合防止只在论文 PDF 上优化；它同样不能代替本项目自己的书签金标准。
- [A Scalable Framework for ToC Extraction](https://arxiv.org/abs/2310.18073) 的“先构树、再逐节点 Keep/Delete/Move”说明：对长文档，局部候选加全局树约束比所有标题两两比较更可扩展。

### 3.3 已落地产品和用户工作流

- [MarginNote 4](https://manual.marginnote.com.cn/mn4/en/manually-automatically-generating-document-table-contents/)：原生目录、手工目录、AI 目录三条路径并存；AI 结果是初稿，支持重命名、缩进和删除；目录属于应用内部虚拟数据。
- [Foxit PDF Editor 2026.1](https://cdn01.foxitsoftware.com/pub/foxit/manual/phantom/en_us/foxit-pdf-editor-user-manual-2026.1.pdf)：可以按文本内容和文本样式自动建 bookmark，但需要用户设置层级规则。它适合格式统一的数字 PDF，不符合 M PDF 处理器对异质扫描书“一键完成”的默认要求。
- [PDF-XChange](https://help.pdf-xchange.com/pdfxe10/bookmarks2_ed.html)：分别提供“从目录”“从页面文本”“从文本文件”生成 bookmark，证明将证据入口分开比一个万能 AI 按钮更稳定。
- [EverMap AutoBookmark](https://evermap.com/autobookmark.asp)：本地依据文本格式、缩进和内容生成书签，并声明不与服务器通信。它证明纯本地商业工具可行，但其可配置规则仍把大量判断交给用户。

这些案例的共同点不是“模型越大越好”，而是：优先使用现成结构，保留可编辑结果，把 PDF 写入与候选识别分离。

## 4. M PDF 处理器方法论：Evidence-Constrained Outline Compilation

### 4.1 证据优先级

固定优先级如下，弱证据不得覆盖强证据：

1. **已存在且通过验证的 PDF outline**；
2. **印刷目录行 + 正文标题匹配 + 页码映射共识**；
3. **正文编号体系**（`I`、`1`、`1.1`、`§`、`A` 等）+ 标题位置；
4. **字体/字重/缩进/留白等版面样式**；
5. **语言模型的结构判断**。

第 5 级只能消歧，不能凭空增加无页面证据的章节。

### 4.2 最小管线

```text
PDF
 └─ inspect: page geometry / existing outline / text availability
     ├─ valid existing outline ───────────────────────────┐
     └─ evidence blocks                                   │
         └─ detect TOC pages                              │
             └─ parse TOC lines                           │
                 └─ align titles + printed pages to body  │
                     └─ confidence gate                   │
                         └─ write outline ────────────────┤
                                                          └─ reopen + validate
```

只有六个核心阶段：检查、取证、目录识别、目录解析、正文对齐、写入验证。OCR、VLM、PDF text extraction 都是 `EvidenceProvider`，不进入求解器核心。

### 4.3 目录页检测

只扫描前置页区域，默认 `min(40 页, 全书 15%)`，特征包括：

- `contents / table of contents / 目录 / sommaire / indice / inhalt` 等词；
- 多行以阿拉伯/罗马页码结尾；
- 点线连接符；
- 相似的左右列边界；
- 连续 2–5 页的目录型版面。

检测器输出页号、区域和分数。不要让模型直接返回“第几页是目录”而不给 bbox/文本证据。

### 4.4 目录行解析

每行解析为：

```text
raw_title, match_key, level_hint, printed_page,
toc_pdf_page, bbox, evidence_provider, evidence_confidence
```

- `raw_title` 保留原始 Unicode，写入 bookmark 时使用它；
- `match_key` 仅用于匹配：NFKC、空白/连字符折叠、去点线和尾页码、大小写折叠；
- 古希腊语重音和附加符号不能从 `raw_title` 删除；去音符版本只能作为次级匹配键；
- 层级先取显式编号，再取目录缩进；字体大小只能作最后的 tie-breaker；
- 双栏先按 x 聚类成列，再按列内 y 排序，禁止简单按整页 y 排序。

### 4.5 页码映射与正文验真

不能只猜一个全书常量 offset。推荐使用**单调序列对齐 + 分段常量偏移**：

1. 在可能的正文页顶部区域搜索目录标题的规范化文本；
2. 从多个高置信标题匹配得到锚点 `(printed_page, pdf_page)`；
3. 用 RANSAC/多数共识估计主要偏移；
4. 在罗马前言、阿拉伯正文、插页或缺页处允许新的分段；
5. 用动态规划选择全局单调路径，禁止后一个目录项跳到前一个目录项之前；
6. 目标页默认定位到标题 bbox 的顶部，而不只是页面左上角。

对每个候选，综合以下可解释证据：标题相似度、页码残差、标题区域/字号、编号一致性、相邻条目的单调性。最终 `confidence` 必须能分解为这些字段，不能只是模型给出的一个浮点数。

### 4.6 Fail closed，而不是人工兜底

默认用户体验应当是：

> “已自动加入 47 条可靠书签；3 条证据不足，已跳过。”

用户可以展开可选详情，但不应被强迫逐条确认。`needs_review` 可保存在 MDP 中，却不能阻塞导出，也不能在未确认时写入正式 PDF。这样既满足准确率，也避免把模型失败成本转嫁给用户。

## 5. 两种实现

### 5.1 电脑本地版

**定位：** 零业务网络请求、一次点击、结果可复现。

本项目现有依赖已经具备关键 PDF 原语，不需要引入另一套 PDF 引擎：

- `pdfium-render 0.9.3` 可读取 bookmark 树、目标和带坐标的页面文本；
- `pdf-writer 0.15` 已支持 `/Outlines`、`OutlineItem` 和 destination；
- 当前 `BilevelPdfBuilder` 已掌握稳定的 page object refs，只需在 finish 前分配 outline 对象并让 catalog 引用它；
- 输出后继续使用 PDFium 重开验证，可沿用现有 validation 架构。

建议新增的自有模块：

```text
outline/
  model.rs       # OutlineEntry、Evidence、PrintedPage、ConfidenceState
  extract.rs     # PDFium outline/text → 统一 EvidenceBlock
  toc_detect.rs  # 目录页候选
  toc_parse.rs   # 行、列、编号、页码
  align.rs       # 锚点、分段 offset、单调 DP
  validate.rs    # 树、目标、标题与证据验证
```

本地 OCR 不应藏在 bookmark 模块中。它消费未来 OCR/MDP 管线产生的带坐标文本；若 PDF 无文字层且用户只运行 Bookmark，可由统一 OCR provider 只处理前置目录候选页和待验证目标页顶部裁剪。没有目录且没有全文 OCR 时，v1 应明确报告“未找到足够结构证据”，而不是扫描全书后猜测。

**本地默认界面：** 一个“自动添加目录书签”按钮；完成后只显示加入数、跳过数和“打开结果”。高级证据面板存在但默认折叠。

### 5.2 API 版（电脑 + 手机）

**定位：** 与本地输出同构，但服务器可使用更强 OCR/VLM，手机无需下载模型。

最简 v1 API 工作流：

1. 客户端创建 job，分块/断点上传完整 PDF；
2. 服务端安全解析、渲染并运行 `EvidenceProvider`；
3. VLM 若启用，只把目录页图像转为严格 schema 的目录行和 bbox；
4. 服务端仍使用同一个确定性 `OutlineEngine` 对齐和验真；
5. 返回新 PDF、MDP outline JSON 和结构化报告；
6. 到期删除原文件和中间页，保留策略在上传前显示。

不要在 v1 做客户端—服务器两轮“先传前言、服务器再索要若干正文 crop”的协议。它节省带宽，但增加状态机、失败恢复和移动端边界条件。先用可恢复的完整文件上传保持产品简单；只有真实成本数据证明必要时再优化为按页证据上传。

建议最小接口：

```text
POST /v1/outline/jobs
PUT  /v1/outline/jobs/{id}/content   # resumable upload
GET  /v1/outline/jobs/{id}
GET  /v1/outline/jobs/{id}/result    # PDF + outline.json + report.json
DELETE /v1/outline/jobs/{id}
```

API 不应暴露“选模型、选 OCR、调 temperature”。用户只选择“本地”或“云端增强”。provider、重试和降级是服务实现细节，但具体引擎版本、调用成本和数据保留必须写进报告。

## 6. 准确性定义与评测

### 6.1 金标准语料

先建 80–120 本真实文档的小而难语料，而不是追求海量页面：

| 分层 | 建议占比 | 必须覆盖 |
|---|---:|---|
| 有原生 outline | 15% | 正确、部分缺失、错页、循环/过深恶意树 |
| 数字 PDF + 印刷目录 | 25% | 多栏、长标题、不同字体、罗马/阿拉伯页码 |
| 扫描书 + 印刷目录 | 45% | 历史字体、倾斜、污渍、缺页、插页、希腊/拉丁/中英混排 |
| 无目录 | 15% | 编号清楚与结构含混各半，用于验证安全拒绝 |

每本由两人独立标注标题、层级、PDF 目标页、目标 y 坐标和印刷页码，冲突再仲裁。按“书”切分 train/dev/test，绝不能把同一本书的页面拆到不同集合。

### 6.2 指标

- **目标页准确率：** 自动写入条目中，PDF 页完全正确的比例；发布门 ≥ 99%。
- **标题精确率：** 自动写入标题与金标准规范化匹配的比例；发布门 ≥ 95%。
- **层级 edge-F1 / TEDS：** 父子关系是否正确；建议发布门 edge-F1 ≥ 95%。
- **自动覆盖率：** 金标准条目中有多少被自动写入；必须与 precision 同时报告，防止“全部跳过”作弊。
- **零编辑文档成功率：** 用户无需修改即可接受的整本文档比例；建议 v1 ≥ 80%（限“存在可读目录”的支持范围）。
- **幻觉数：** 无正文证据却写入的条目数；发布门为 0。
- **成本指标：** 本地 wall time/RAM；API 每 100 页成本、上传量和 P95 延迟。

阈值只能在 dev 集校准一次，然后冻结到 test 集。每次 provider/model 更新都必须完整回归，不能只展示 OCR 自带 benchmark。

## 7. 实施顺序

### P0：先固定契约和基准

- 实现 `OutlineEntry`、`OutlineEvidence`、页码与置信状态 schema；
- 建 20 本最小金标准和恶意/畸形 outline fixtures；
- 固定指标脚本和版本化报告。

### P1：原生 outline 读、验、写

- 使用 PDFium 安全读取并规范化现有树；
- 检查循环、深度、标题长度和目标范围；
- 扩展 `BilevelPdfBuilder` 写 outline；
- 在 Preview、Acrobat、PDF.js、Foxit/iOS 阅读器矩阵中验证。

这一步不含 AI，却立刻解决“当前重建 PDF 会丢掉原书签”的确定问题。

### P2：数字 PDF 的印刷目录路径

- 页面文本/坐标抽取；
- 目录检测、双栏解析、页码 token 和标题规范化；
- 分段 offset + 单调对齐；
- 仅写入通过 gate 的条目。

### P3：扫描 PDF

- 接入统一 OCR evidence；
- 先 OCR 前置候选页，再验证目标页顶部 crop；
- 建真实历史书扫描回归集。

### P4：API 与手机

- 将相同 schema、aligner、validator 部署到 job 服务；
- 加可替换的 OCR/VLM provider；
- 实现断点上传、删除和成本报告；
- 手机端只负责选择文件、显示进度和保存结果。

### P5：无目录 fallback（有条件）

只有 P2/P3 已满足发布门后才评估“编号 + 样式”的正文标题路径。语义生成目录仍保持独立实验，不并入默认 Bookmark。

## 8. 明确不做

- 不复制或嵌入 `pdf.tocgen`、Docling、MinerU 等代码；只用其公开方法和失败案例设计测试。
- 不让 LLM 直接输出最终 PDF 页号。
- 不把“需要用户修 20 条”包装成 AI 成功。
- 不为本功能另造一套 OCR、PDF parser 或云端专有 outline 格式。
- 不覆盖原文件；始终输出新 PDF，并保留可导出的 MDP/JSON 证据。
- 不用“准确率”单指标掩盖低覆盖率。

## 9. 最终建议

**立项，但把范围锁死在“证据化目录恢复”。**

第一笔开发投入应放在 P1 + P2，而不是模型采购：现有 Rust 依赖已经能读取原 outline/文字并写标准 outline，核心缺口是自有的证据 schema、页码对齐器和验证器。API 也不应成为另一套算法，而只是同一引擎的强证据 provider 与跨端执行环境。

判断功能是否值得付费的标准不是“AI 生成了多少”，而是：用户点击一次，得到可以直接交付的 PDF；不确定项被安静、诚实地跳过；绝大多数支持范围内的书不需要再打开编辑器。
