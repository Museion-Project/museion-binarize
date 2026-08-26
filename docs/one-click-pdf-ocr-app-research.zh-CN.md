# 近半年“一键 PDF OCR → 机器可读产物”成品项目调研

**日期：** 2026-08-26
**窗口：** 2026-02-26 至 2026-08-26
**对象：** 面向最终用户的网站、本地 Web、桌面 App 或轻量 CLI；不是 Docling、PaddleOCR、
Marker、MinerU 这类上游框架本身。
**证据原则：** 功能与实现只采信公开仓库 README、源码、提交与许可证；“可优化点”与已观察
事实分开表述。

## 1. 结论

这轮找到的真正同类不是一种产品，而是三种壳：

1. **轻量脚本/工具箱**：几十到数百行代码，一键把 PDF 变成 TXT、Markdown 或简单 JSON；
2. **本地 OCR 工作台**：浏览器或桌面三栏界面，逐页预览、校对、导出；
3. **自托管转换服务**：上传、持久队列、GPU worker、REST/MCP、下载产物。

最重要的发现不是“大家用了哪个 OCR 模型”，而是：

- 多数产品把 Markdown、Word 或 searchable PDF 当终点，**没有稳定、版本化、可迁移的中间层**；
- 能保存 bbox 的产品，往往直接保存某个模型的原始 JSON，缺 source digest、坐标空间、变换链、
  provider 版本和 revision；
- 好用的 GUI 已经普遍具备逐页进度、取消、历史、编辑与导出，但只有少数做到重启恢复；
- agent/API 接口开始出现，但通常只能拿最终文件，不能按页/块查询有证据的 Document IR；
- 没有一个样本把“自动书签所需的目录项、正文标题、印刷页码与对齐证据”作为一级产物。

因此 M PDF 处理器不应复制一个“PDF 转 Markdown 页面”。更有价值的定位是：

> **本地优先的一键 OCR 工作流 + 可验证的 AI-ready 文档包；Markdown、可搜索 PDF、Word、
> chunks 和自动书签材料都由同一份逐页事实层派生。**

## 2. 样本与边界

### 2.1 核心样本

| 项目 | 最近提交 | 产品面 | 机器可读产物 | 源码/许可 | 定位判断 |
|---|---:|---|---|---|---|
| [guji-tools](https://github.com/Ivan666jjj/guji-tools) | 2026-08-05 | CLI + 安装脚本 | TXT、MD、含 page/text/block/confidence/bbox 的 JSON | 公开，MIT | 最小原型；不是网站/App，但直接命中“一键 OCR + 中间 JSON” |
| [xiaohuan-tools](https://github.com/Ivan666jjj/xiaohuan-tools) | 2026-08-07 | Tk GUI / Electron / Swift 工具箱 | PDF 转 TXT；扫描件可选 RapidOCR | 公开，MIT | 相邻样本；OCR 是转换工具的一条回退路径，没有结构化中间层 |
| [octo-ocr](https://github.com/1ampa55ag3/octo-ocr) | 2026-08-17 | localhost Web 工作台 | `mdun-project-v2` JSON、MD、TXT、DOCX、PDF、XLSX | 公开，Apache-2.0 | 与目标最接近：离线、逐页、可校对、结构 JSON |
| [Folio-OCR](https://github.com/vorojar/Folio-OCR) | 2026-06-18 | localhost 三栏工作台 | SQLite 中 text/regions；MD、TXT、DOCX、EPUB | 公开，MIT | 产品完成度高，IR 仍偏内部数据库 |
| [DeepSeek-OCR-Dashboard](https://github.com/Cross2pro/DeepSeek-OCR-Dashboard) | 2026-04-06 | FastAPI + Vue 本地 Web | history `result.json`、MD、raw TXT、bbox overlay | 公开，MIT | 模型体验台；结果 JSON 可用但不适合长期交换 |
| [local-llm-pdf-ocr](https://github.com/ahnafnafee/local-llm-pdf-ocr) | 2026-07-15 | FastAPI Web + CLI | searchable PDF、HTML overlay、MD、TXT | 公开，MIT | 几何对齐思路强，但结构数据主要留在运行时内部 |
| [pdf-converter](https://github.com/cyanyux/pdf-converter) | 2026-07-07 | 自托管 Web + REST + MCP | searchable PDF、MD+images、DOCX；job JSON | 源码公开，仓库无 LICENSE | 服务可靠性最完整；没有可复用结构 IR，且当前不可默认视为获许可开源代码 |
| [paddle-ocr-ui](https://github.com/egore4606/paddle-ocr-ui) | 2026-07-11 | FastAPI + Docker 本地 Web | Paddle 原始 JSON、MD、TXT、images、ZIP | 公开，MIT | 薄封装范例；保留原始 JSON，但每个 job 启一个短命容器 |
| [MDFlux](https://github.com/ibrahimqureshae/mdflux) | 2026-08-25 | Tauri + Svelte 桌面 App | AI-ready Markdown | 公开，MIT | 安装、sidecar、本地/API cleanup 很值得借鉴；几何与证据层缺失 |
| [pdf-ocr-llm](https://github.com/hz01/pdf-ocr-llm) | 2026-03-15 | Gradio Web + CLI | MD/TXT；OCRFlux 或抽取模式可导出 JSON | 公开，MIT | 多 VLM 试验台；全页驻内存、顺序处理，产物随模型类型变化 |

日期来自各仓库在上述 commit 的 committer timestamp。`pdf-converter` 在所检 commit 的根目录
和 README 中均无许可声明；“public repository”不等于已授权复制、修改或分发。

### 2.2 排除规则

- 上游 OCR/解析框架仅记录为上述产品的依赖，不作为同类产品计数；
- 只有 OCR API、Notebook 或 benchmark，没有“一键输入 → 可取回产物”的项目不进入主表；
- 只有 searchable PDF、没有可被机器直接消费的文本/JSON/HTML 的 GUI 只作为外围参考；
- 半年内无提交的老项目不进入核心样本，即使仍有较高 star。

## 3. 三个种子项目的源码复核

### 3.1 `Ivan666jjj/guji-tools`

**事实。** `scan_pdf.py` 用 PyMuPDF 以默认 300 DPI 渲染 PDF，再调用
`rapidocr_onnxruntime.RapidOCR`；JSON 页对象保存全文、line blocks、四点 bbox、confidence 和耗时。
Markdown 只增加页标题。源码见
[`scan_pdf.py`](https://github.com/Ivan666jjj/guji-tools/blob/79ba7f139a575682f017af65ac1f18aa2d71c106/scan-pdf/scan_pdf.py)。

**肉眼可见的优化点。**

- `pdf_to_images()` 先把全书所有 300-DPI PIL image 放进列表，之后才应用 `page_limit`；长 PDF
  峰值内存随页数线性增长，而且“只识别前 N 页”仍渲染全书。
- OCR 串行、完成后一次性返回，没有 page checkpoint、取消或断点恢复。
- bbox 是渲染像素，但 JSON 没有 page width/height、坐标原点、旋转与 transform，离开这次渲染
  后很难可靠回贴 PDF。
- metadata 没有 schema version、源文件 digest、模型/版本与语言配置。
- README 宣称专门支持竖排、繁体、生僻字，但当前实现只初始化 RapidOCR 默认配置，源码中没有
  竖排 reading-order 或稀有字模型路由；这应视作待验证能力，不宜直接继承为产品承诺。

### 3.2 `Ivan666jjj/xiaohuan-tools`

**事实。** 它不是独立 OCR 中间层。`转换工具/converter.py` 先逐页提取 PDF 原生文字；只要发现
任一空白页，就询问是否安装/调用 RapidOCR，随后重新 OCR **全部页面**，最后只写一个 TXT。
源码见
[`converter.py`](https://github.com/Ivan666jjj/xiaohuan-tools/blob/d38dd84db2e1340cf85394584f7bc3413ee5ab91/%E8%BD%AC%E6%8D%A2%E5%B7%A5%E5%85%B7/converter.py#L276)。

**肉眼可见的优化点。**

- mixed PDF 的一张扫描页会触发全书 OCR，并用 OCR 文本替换已经无损提取的数字页文本；应改为
  per-page probe 与 selective OCR。
- TXT 丢失 bbox、置信、阅读顺序、页尺寸和 provider provenance，无法继续生成可靠文字层或书签。
- 在运行期通过 GUI 执行 `pip install` 使发行可复现性、离线性和失败恢复变差；更适合预打包或
  使用可校验的 optional runtime。

### 3.3 `1ampa55ag3/octo-ocr`

**事实。** 这是三者中最接近目标的实现：Python 标准库 HTTP server + 单 worker queue，
PyMuPDF 逐页流式渲染，RapidOCR/PP-OCRv5，PP-DocLayout、SLANet、公式识别、规则/ONNX 后处理，
项目 JSON 原子落盘。`PageData` 保存 page kind、width/height、paras、lines、low-confidence、tables、
formulas、removed 和 edits；导出 schema 明确为 `mdun-project-v2`。参见
[`pipeline.py`](https://github.com/1ampa55ag3/octo-ocr/blob/f83eb2bc7207acad3de2d0b7852e964c863b5250/src/mdun/pipeline.py#L19) 与
[`export/json.py`](https://github.com/1ampa55ag3/octo-ocr/blob/f83eb2bc7207acad3de2d0b7852e964c863b5250/src/mdun/export/json.py)。

**值得直接借鉴。**

- 数字页直取文字，扫描页才 OCR；逐页释放图像；
- bbox、低置信行和编辑/删除记录能服务图文校对；
- project 用临时文件 + replace 原子保存；本地只监听 loopback，并提供取消；
- JSON 是显式产品产物，不只是调试文件。

**仍可优化。**

- `_jobs` 只在内存；重启后 queued/running 状态消失，worker 不能恢复任务。
- 取消时 `project.pages.clear()`，已完成页不能续跑；全书结果仍到结束才成为持久 project。
- `source` 保存本机绝对路径，却没有 source digest；移动文档、分享 JSON 或判断源文件变化都不稳。
- 数字页坐标是 PDF point，扫描页坐标是渲染 pixel，虽然保存 width/height，但没有显式
  `space_id/unit/transform`；消费者必须按 `kind` 猜语义。
- schema 没有 per-run provider/model digest、reading-order edges、标题层级、目录项、印刷页码观测，
  因而还不能直接作为自动书签证据包。

## 4. 扩展样本的实现模式

### 4.1 Folio-OCR：产品体验成熟，中间层仍是内部状态

FastAPI 把 PDF 以 PyMuPDF 2× 渲染为页面图片，PP-DocLayoutV3 产生 region bbox，GLM-OCR 通过
Ollama 识别；SQLite 只有 documents 与 pages，页表核心是 `ocr_text`、`ocr_regions`、`ocr_time`。
全文 OCR 逐页执行并逐页提交，失败页不会抹掉成功页。它的三栏校对、预取下一页、停止、SSE、
自动保存和多文档历史很适合借鉴。源码见
[`server.py`](https://github.com/vorojar/Folio-OCR/blob/dcf2a16d04e95c3095ae7930a50a010017d64668/folio_ocr/server.py)。

缺口是 regions 只有 label/bbox/text/score，SQLite 没 schema migration/version、source digest、坐标
空间或 provider run；JSON 是 API/数据库内部形态，公开导出仍以 MD/TXT/DOCX/EPUB 为主。

### 4.2 DeepSeek-OCR-Dashboard：可视化好，但 JSON 被页面图片撑大

它把 PDF 全部拆成 PNG，使用全局 `inference_lock` 串行推理；每页结果含 text、rawText、layout、
duration 和**整页 base64 `imageData`**，随后 history 又复制 source 与 page PNG。参见
[`app.py`](https://github.com/Cross2pro/DeepSeek-OCR-Dashboard/blob/97b6d6c81adbd5b9cc83ce4e666051b9941dd87f/web_project/backend/app.py#L254)。

这让 demo 很自包含，但长文档同时保留 JSON base64、page PNG 和原 PDF，磁盘与序列化成本明显重复；
更合适的是 JSON 保存 content-addressed asset ref。progress/task 也只在内存，没有持久队列、取消与
重启恢复。

### 4.3 local-llm-pdf-ocr：最值得借鉴的是“文字—框对齐”

它用 Surya 批量检测框、VLM 做整页转写，再用 Needleman–Wunsch 将文本行绑定到框；低覆盖框会
裁切重识别，密集页自动切为 per-box OCR，bbox-native VLM 则走 grounded fast path。bbox 统一为
0..1，并可写回 searchable PDF 或 HTML overlay。参见
[`pipeline.py`](https://github.com/ahnafnafee/local-llm-pdf-ocr/blob/662b1aa0eac4894d074fa533d897598622b88af2/src/pdf_ocr/pipeline.py) 与
[`geometry.py`](https://github.com/ahnafnafee/local-llm-pdf-ocr/blob/662b1aa0eac4894d074fa533d897598622b88af2/src/pdf_ocr/core/geometry.py)。

它的不足不是算法，而是产品契约：bbox/置信/对齐结果主要停留在运行时，公开输出只有 PDF、HTML、
MD、TXT；Web job 使用临时文件，缺持久 job、checkpoint 和可交换结构 JSON。M PDF 处理器应吸收算法，
但把对齐结果落入 canonical IR。

### 4.4 pdf-converter：任务系统最好，但产物契约偏“下载文件”

Node/Hono server + SQLite WAL queue + Python GPU supervisor/child，提供 heartbeat、crash reaper、
watchdog、取消、流式上传、retention GC、OpenAPI 和 MCP；数字 PDF 走 Docling，扫描 PDF 走
PaddleOCR。参见
[`schema.sql`](https://github.com/cyanyux/pdf-converter/blob/ab932ae519f164ee6b1d5a25859355976d1340b2/db/schema.sql) 与
[`shared types`](https://github.com/cyanyux/pdf-converter/blob/ab932ae519f164ee6b1d5a25859355976d1340b2/packages/shared/src/index.ts)。

它证明了本地 Web 也值得做 durable queue；但 `JobResult` 只描述页数、download id、engine、warning，
agent 拿到的是最终 MD ZIP/DOCX/PDF，而不是 page/block evidence。另一个现实问题是仓库未附 LICENSE，
可研究实现，不能默认复用代码。

### 4.5 paddle-ocr-ui：薄 wrapper 的优点与代价

每个 job 有独立目录和 `job.json`，后端启动一个短命 PaddleOCR-VL Docker container，保留官方
`*_res.json`，再派生 TXT/MD。参见
[`server/ocr.py`](https://github.com/egore4606/paddle-ocr-ui/blob/0d829b96c44f5af069dd3bd635f86dfb1695fbac/server/ocr.py)。

优点是隔离强、原始 JSON 不丢、升级上游容易；代价是每 job 启容器，队列只在内存，重启不会重放
磁盘中 queued job，也没有运行中取消。M PDF 处理器可以保留“原始 provider artifact”，但不应让它
替代稳定 IR。

### 4.6 MDFlux：桌面发行与 local/API 双路径的最佳参考

Tauri + Svelte 管 UI，Python sidecar 做转换；Full 版捆绑不可变 runtime，Lite 版首启事务式安装。
扫描 PDF 由 pypdfium2 逐页渲染、RapidOCR 识别；cleanup 可选规则、本地 OpenAI-compatible 或云 API，
失败会保留原文。参见
[`ocr.py`](https://github.com/ibrahimqureshae/mdflux/blob/00c602534cf3a23174ef48f12d44ba00036d3398/app/src-tauri/resources/sidecar/ocr.py) 与
[`main.py`](https://github.com/ibrahimqureshae/mdflux/blob/00c602534cf3a23174ef48f12d44ba00036d3398/app/src-tauri/resources/sidecar/main.py)。

它直接验证了 M PDF 处理器的 Tauri + optional sidecar 路线。但 OCR worker 只返回 Markdown 字符串，
几何、置信和 provider observation 被丢掉；AI cleanup 以整份 Markdown 为输入，虽有 diff/数据丢失
提醒，却没有 node-level evidence/revision。

### 4.7 pdf-ocr-llm：多模型试验方便，长 PDF 路径不够有界

Gradio 同时支持 GLM-OCR、OCRFlux、Qwen/InternVL，OCRFlux 可输出 per-page JSON，另有 schema prompt
抽取页。其 `pdf2image.convert_from_path()` 一次返回全部 PIL pages，随后串行逐页推理；产物结构随
模型类型切换，缺共同 schema、bbox/transform、checkpoint 与恢复。参见
[`pdf_processor.py`](https://github.com/hz01/pdf-ocr-llm/blob/16abc31a619a09c9f94f5254b0df46d075206b9f/src/processors/pdf_processor.py) 与
[`ocr_pipeline.py`](https://github.com/hz01/pdf-ocr-llm/blob/16abc31a619a09c9f94f5254b0df46d075206b9f/src/pipeline/ocr_pipeline.py)。

## 5. 可见的市场/工程空档

| 已普遍做到 | 尚未普遍做到 | M PDF 处理器应占的位置 |
|---|---|---|
| drag/drop、localhost、逐页预览 | 版本化、provider-neutral 的 IR | MDP 是一级交付物 |
| Markdown/Word/searchable PDF | 所有派生物来自同一事实源 | 一次 OCR，多种可重建 view |
| bbox overlay | 坐标空间、旋转、裁剪变换可验证 | master space + transform chain |
| progress、部分产品可取消 | page checkpoint + 重启续跑 | 每页原子提交，durable manifest |
| 本地模型或 Docker | 同一 DTO 无缝切 local/API | provider adapter 只改变执行端 |
| history/编辑 | raw、proposal、accepted revision 分层 | AI 不覆盖 OCR 事实 |
| REST/MCP 下载文件 | page/region evidence 查询 | agent 可请求 grounded chunks |
| OCR 后猜标题 | TOC/heading/page-label 对齐材料 | 自动书签 evidence 是固定 view |

## 6. 对 M PDF 处理器方案的直接影响

### 6.1 产品流

首版本地流程建议固定为：

```text
拖入 PDF
  → 逐页 probe（原生文字 / 扫描 / 混合 / 旋转 / 空白）
  → 创建持久 job + MDP manifest
  → 只对需要的页渲染、预处理、OCR
  → 每页验证后原子落盘，UI 立即可预览/校对
  → 汇总 reading order、低置信、TOC/heading/page label evidence
  → 导出 Markdown / searchable PDF / outline evidence / chunks
```

这同时吸收了 xiaohuan-tools 与 octo-ocr 的 selective OCR、Folio 的三栏校对、pdf-converter 的 durable job、
MDFlux 的 sidecar 发行，并避开 guji-tools/pdf-ocr-llm 的全书驻内存。

### 6.2 两类交付物必须分开

- **给人的终产物：** searchable PDF、Markdown、DOCX/TXT；
- **给机器继续工作的中间产物：** MDP，包含 page、asset、text/layout observation、provider run、
  revision、坐标 transform、outline evidence 和派生 view 索引。

公开 UI 可以默认只显示终产物，但“导出 AI-ready 包”必须是一等按钮。这样不会要求普通用户理解
IR，也不会让开发者只能反向解析 Markdown。

### 6.3 本地优先、API 后接

本地首版不把某一个模型写死进 schema：

1. `pdf-native-text` 先跑；
2. `rapidocr-onnx` 作为轻量跨平台/CPU 的工程基线；
3. Tesseract/Kraken 或更强本地模型通过同一 page provider 契约加入，以自有古希腊/校勘语料决定默认；
4. API/VLM 只作为另一个 provider，接收 page asset，返回同一 `PageObservation`；
5. 本地失败、低置信或复杂页面才允许用户选择云增强，且记录费用、模型版本和数据驻留。

### 6.4 自动书签材料从第一页就保存

每页除 OCR 文本外，应同步保存：

- `toc_entry_candidate`：标题、编号、缩进/列、印刷页码 token、polygon；
- `heading_candidate`：标题文本、层级特征、page + y、polygon、文字来源；
- `page_label_observation`：页眉/页脚中的罗马/阿拉伯页码；
- `reading_order` 与 region role；
- native outline 与 link/destination 安全检查；
- TOC ↔ heading 的文本相似度、页码残差、单调性与证据引用。

最终书签仍由确定性 OutlineEngine 编译和 fail-closed 验证；VLM 只能增加候选或 proposal，不能直接
把猜测写入 PDF outline。

## 7. 本地 MVP 优先级

1. **P0：** MDP manifest、逐页 JSON、asset ref、source digest、schema validator；
2. **P0：** durable job、逐页 checkpoint、取消与重启续跑；
3. **P0：** native-text + reference OCR provider，先打通 UI/IR/导出；
4. **P1：** RapidOCR 本地 provider 与低置信校对界面；
5. **P1：** Markdown、searchable-text、outline-evidence、chunks 四个确定性 view；
6. **P1：** 自动书签候选与人工确认，不急于自动写回；
7. **P2：** 古希腊专项 provider 盲测与默认引擎选择；
8. **P2：** API provider、疑难页路由和成本/隐私策略。

第一里程碑不应是“接入最多 OCR 引擎”，而应是：关闭应用再打开后，已完成页仍在；删除所有
views 后可从 MDP 重建；任一字符、标题或书签候选都能回到原页区域和 producer run。

## 8. 检索记录与限制

检索入口包括用户给出的三个仓库、GitHub `pdf-ocr`、`searchable-pdf`、`pdf-to-markdown`、
`paddleocr-vl` topics，以及 “PDF OCR Markdown Web UI / local / Streamlit / Gradio / FastAPI /
desktop” 组合词。共深读 10 个仓库的 README、目录、核心 pipeline/server/output/job 源码、最近
commit 与 LICENSE。

限制：没有在本机安装各项目模型并跑统一样本，因此本文评价的是**公开实现与工程契约**，不是 OCR
准确率排名。README 的性能/准确率主张未被独立复现；涉及它们时仅作为项目自述。
