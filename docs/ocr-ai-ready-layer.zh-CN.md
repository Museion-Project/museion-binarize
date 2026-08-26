# M PDF 处理器 OCR + AI-ready 中间层：近期项目调研与本地优先方案

**日期：** 2026-08-26
**调研窗口：** 2026-02-26 至 2026-08-26
**状态：** 可行性方案，尚未实施
**与现有决策链的关系：** 本文细化 [`intent.md`](../intent.md)、[`spec.md`](../spec.md) 和
[`plan.md`](../plan.md) 中的 MDP/OCR 方向，不替代三者；实现前仍需批准 MDP 与 provider ADR。

## 1. 结论先行

建议把这个层定义为 **MDP 0.1 的 Document IR**，而不是再造一个独立的“AI 格式”：

> **一份可验证、可追溯、坐标稳定的文档事实层；Markdown、RAG chunks、可搜索 PDF、
> 自动书签材料和未来 API 都是它的派生视图。**

具体判断如下：

1. **可行，且应先做本地。** 当前 Rust/PDFium 管线已经拥有页序、几何、渲染资产、摘要、
   版本化报告和重新打开验证等基础原语，缺的是稳定 IR 与 provider 边界，不是另一套 PDF 引擎。
2. **Markdown 不能做唯一事实源。** 它适合阅读和投喂模型，但会丢失字符/词坐标、原始 OCR、
   多候选、置信来源、处理变换和结构证据。
3. **IR 必须由项目自有。** 本地 OCR、上游解析框架或 API 响应都经 adapter 进入相同
   DTO；provider 升级不得迫使 MDP、书签或 PDF exporter 改 schema。
4. **本地首版采用 sidecar 边界。** Rust 核心拥有 job、schema、坐标、验证和派生物；OCR
   运行时通过版本化 NDJSON 协议接入。这样既能先接 RapidOCR 与专项 OCR 候选，也不把 Python、模型
   框架或供应商 SDK 塞入现有纯二值快速路径。
5. **自动书签材料从第一版即产出。** IR 必须保留 `toc_entry`、`section_heading`、页码观测、
   阅读顺序、编号/缩进、原生 outline 和证据引用；书签模块不再重复 OCR 或解析 PDF。
6. **“AI-ready”指可接地、可选择、可复算，不等于先调用大模型。** AI 修订是引用源节点的
   proposal，不能覆盖原始 OCR；只有接受后的 revision 才能成为 preferred view。

## 2. 近半年成品项目，而非上游框架

详细源码复核见
[`one-click-pdf-ocr-app-research.zh-CN.md`](one-click-pdf-ocr-app-research.zh-CN.md)。这次把“同类”
严格限定为最终用户可以上传/打开 PDF、执行 OCR，并取回机器可读产物的网站、本地 Web、桌面 App
或轻量工具；Docling、PaddleOCR、Marker、MinerU 只作为这些产品的实现依赖或技术参考。

### 2.1 代表样本

| 项目 | 产品形态 | 中间/最终产物 | 对本方案的直接意义 |
|---|---|---|---|
| [guji-tools](https://github.com/Ivan666jjj/guji-tools) | 小型 CLI | TXT/MD；page/block/bbox/confidence JSON | 证明最小链路很短；也暴露全书 300-DPI 驻内存、无 schema/provenance/恢复的问题 |
| [xiaohuan-tools](https://github.com/Ivan666jjj/xiaohuan-tools) | 桌面工具箱 | 原生文字或 RapidOCR → TXT | mixed PDF 会因一个扫描页重做全书 OCR；应按页 probe/selective OCR |
| [octo-ocr](https://github.com/1ampa55ag3/octo-ocr) | localhost 校对工作台 | 版本化项目 JSON、MD/TXT/DOCX/PDF | 最接近目标；需补 durable job、逐页 checkpoint、source digest、显式坐标空间与书签 evidence |
| [Folio-OCR](https://github.com/vorojar/Folio-OCR) | FastAPI 三栏工作台 | SQLite text/regions、MD/TXT/DOCX/EPUB | 三栏校对、SSE、逐页缓存与编辑体验值得借鉴；内部 DB 不是交换 IR |
| [local-llm-pdf-ocr](https://github.com/ahnafnafee/local-llm-pdf-ocr) | Web + CLI | searchable PDF、HTML overlay、MD/TXT | Surya + VLM + DP 对齐很有价值；几何结果应持久化，而非只用于 writer |
| [pdf-converter](https://github.com/cyanyux/pdf-converter) | 自托管 Web/API/MCP | PDF、MD ZIP、DOCX、job JSON | durable SQLite queue、heartbeat/reaper/watchdog 最成熟；agent 仍只能拿最终文件 |
| [paddle-ocr-ui](https://github.com/egore4606/paddle-ocr-ui) | FastAPI + Docker | 原始 Paddle JSON、MD/TXT/images/ZIP | 原始 provider artifact 应保留；但每 job 启容器和内存队列不适合默认路径 |
| [MDFlux](https://github.com/ibrahimqureshae/mdflux) | Tauri + Svelte 桌面 App | AI-ready Markdown | 证明 Tauri + optional Python sidecar + 本地/API cleanup 可发行；当前会丢 bbox/provenance |

另复核了 DeepSeek-OCR-Dashboard 与 pdf-ocr-llm：前者把整页 base64 图像写进 result JSON 并另存
PNG/PDF，后者一次加载全部 PIL pages 且输出 schema 随模型变化。两者适合模型体验，不适合直接
作为长期中间层模板。

### 2.2 共同趋势与空档

- 用户体验正在收敛到 drag/drop、逐页进度、三栏校对、历史与多格式导出；
- 实现上以 localhost Web 或 Tauri + sidecar 为主，本地隐私是明确卖点；
- “机器可读”多数仍等于 Markdown 或 vendor JSON，canonical model 与消费 view 尚未真正分离；
- bbox overlay 已常见，但 coordinate space、旋转/裁剪 transform、source digest 和 provider run 不完整；
- durable queue、取消、页级 checkpoint、重启续跑只零散出现，没有一个样本全部具备；
- REST/MCP 开始成为产品面，但通常只提交 job、下载最终文件，不能查询 page/region evidence；
- 没有样本把 TOC entry、正文 heading、印刷页码及对齐证据作为自动书签的固定交付物。

这意味着 M PDF 处理器的竞争点不是再包装一个 OCR 模型，而是把现有产品中分散的优点组合起来：
**OctoOCR 的结构项目、Folio 的校对体验、pdf-converter 的持久任务、MDFlux 的桌面发行，再加上
项目自有的可验证坐标、revision 和 outline evidence。**

标准基线仍可作为 import/export target：[PAGE XML](https://ocr-d.de/en/spec/page)、
[ALTO](https://www.loc.gov/standards/alto/)、[IIIF Presentation](https://iiif.io/api/presentation/3.0/)
与 [W3C Web Annotation](https://www.w3.org/TR/annotation-model/)；它们不必成为内部对象模型。

## 3. Document IR 0.1

### 3.1 分层

```text
源 PDF / 页面图像
        │
        ▼
Source + Page + Asset + Transform        不可变事实层
        │
        ├── PDF native text adapter
        ├── local OCR sidecar
        └── future API/VLM adapter
        ▼
OCR observations + Layout hypotheses     provider 观测层
        │
        ▼
Preferred text + Structure + Evidence    可审计选择层
        │
        ├── searchable PDF text layer
        ├── automatic bookmark evidence
        ├── Markdown / HTML / ALTO / PAGE
        └── grounded chunks for AI/RAG
```

四条硬规则：

- source、provider observation、AI proposal、human revision 分层，永不就地覆盖；
- 所有可见文字和结构都能反向定位到 page + polygon + producer run；
- 所有派生视图都可丢弃后由 canonical IR 重建；
- provider 原始响应可选择性保存用于调试，但不能成为唯一事实源。

### 3.2 最小实体

| 实体 | 关键字段 | 说明 |
|---|---|---|
| `Document` | schema/version、document id、source digest、page ids | 不存本机绝对路径 |
| `Page` | stable id、physical index、printed label observations、master space、asset refs | `physical_index` 从 0 开始；UI page number 单独派生 |
| `Asset` | kind、MIME、digest、width/height、generation step | `source_render`、`master_gray`、`bilevel`、`thumbnail` 等 |
| `Transform` | from/to space、kind、parameters、inverse status | rotation/crop/projective；未来 dewarp 用 mesh asset |
| `TextNode` | level、raw text、text variants、language spans、polygon、confidence parts、run ref | v0.1 保证 block/line/word；glyph 仅 provider 可靠时提供 |
| `Region` | kind、polygon、text refs、reading-order position、parent/children | heading、paragraph、TOC、page number、footnote、apparatus 等 |
| `ProviderRun` | provider/model/version、capabilities、asset+params digest、platform、timing、errors | run id 由输入、provider 描述与参数摘要决定，可幂等重跑 |
| `Revision` | target refs、operation、before/after、actor kind、evidence refs、status | actor 为 machine/AI/human；proposal 与 accepted 分离 |
| `OutlineEvidence` | candidate title、level hint、target、TOC/heading/page-label refs、confidence parts | 只保存证据和候选；最终 outline 由确定性引擎编译 |

### 3.3 坐标约定

建议 v0.1 固定以下约定，避免复制近期项目的坐标漂移：

1. 每页建立方向已校正、左上角为原点的 `master` space；
2. canonical polygon 使用整数 `ppm` 单位，横纵轴均为 `0..1_000_000`，避免持久化浮点舍入差异；
3. 同时保存 provider 原始 polygon、原始 `space_id` 和该 space 的像素宽高；
4. adapter 必须把原始 polygon 映射为 master polygon；越界、NaN、倒置或不可逆变换直接报错；
5. bbox 始终由 polygon 派生，不反过来丢失倾斜/透视信息；
6. PDF points 映射记录为显式 transform，不能靠调用方猜 DPI 或 `/Rotate`；
7. v0.1 支持 affine/projective；真正非线性 dewarp 以 `mesh_ref` 扩展，不伪装成单矩阵。

### 3.4 文本与 Unicode

- `raw` 原样保留 provider 输出；
- `nfc` 是默认可读规范化视图；
- `search_key` 可以做 NFKC、断词/连字符折叠和大小写折叠，但不能回写 `raw`/`nfc`；
- 多音调古希腊文使用 BCP 47 `grc`，现代希腊文为 `el`，不得混同；
- combining mark、上标、脚注标记和校勘符号必须可单独评测；
- alternative 不塞进一个字符串：每个候选包含文本、producer、score kind 和 score value；
- provider 不提供词级分数时用 `null`，不得把 page confidence 复制到每个词制造虚假精度。

### 3.5 结构角色

首版枚举建议包括：

```text
document_title, section_heading, paragraph, list, table, figure,
caption, equation, code, header, footer, page_number, footnote,
apparatus, bibliography, toc, toc_entry, marginalia, unknown
```

枚举允许 `vendor:<name>:<label>` 扩展，但 canonical role 只能由 adapter 显式映射。阅读顺序
用有向边/序列引用表达，不依赖 JSON 数组的偶然顺序。父子结构与阅读顺序分开，因为多栏、
脚注和 apparatus 常常“视觉嵌套”但“阅读次序”不同。

### 3.6 AI-ready 派生视图

IR 提供四种可重建视图，而不是一个万能 Markdown：

- `document.md`：供人和普通 LLM 阅读，带稳定 page/region anchor；
- `chunks.jsonl`：每条含 text、heading trail、page ids、region refs、master polygons、token estimate、
  content hash；
- `outline-evidence.json`：供确定性 OutlineEngine 使用；
- `searchable-text.jsonl`：按 page/line/word 顺序供 PDF invisible text layer writer 使用。

chunk 只能引用 canonical nodes，不能复制后失去关联。默认按结构边界切分，超长段落才按 token
预算二次切分；每个 chunk 必须能完整追溯到原页区域。

## 4. 自动书签预备材料

Document IR 0.1 应直接产出以下材料，使
[`auto-bookmark-research.zh-CN.md`](auto-bookmark-research.zh-CN.md) 的 OutlineEngine 无需再次 OCR：

1. `native_outline`：原 PDF outline 的标题、层级、目标与安全验证结果；
2. `toc_regions`：疑似目录页/区域及检测特征；
3. `toc_entries`：原始标题、规范化匹配键、显式编号、缩进/列、印刷页码 token、bbox；
4. `heading_candidates`：正文标题文本、编号、role、字体/版面提示、page + y 位置；
5. `page_label_observations`：页眉/页脚中的罗马或阿拉伯页码及区域；
6. `alignment_anchors`：TOC entry ↔ heading 的文本相似度、页码残差和证据 refs；
7. `outline_candidates`：层级、目标页/位置、可分解 confidence 与 `confirmed/suggested/needs_review`。

这里的 AI/VLM 只允许补充 `toc_entry` 或 `heading_candidate`，而且必须返回文字与 polygon。
最终页码映射、单调性、层级约束、门限和 PDF 写入仍由本地确定性代码完成。

## 5. 本地优先实现

### 5.1 代码边界

建议 ADR 0003 评估将纯数据层从一开始放入独立 crate：

```text
crates/
├── mpdf-document/                # 无 PDFium/Tauri/模型依赖
│   ├── model/                    # page/asset/text/layout/outline/provenance
│   ├── coordinate/               # polygon/space/transform
│   ├── package/                  # directory container + validator + migration
│   └── views/                    # md/chunks/outline/searchable projections
├── mpdf-core/                    # 现有 PDFium、图像、1-bit PDF 路径
└── mpdf-ocr-provider/            # trait、reference provider、sidecar client
```

若短期仍按 `plan.md` 把类型放在现有 core，也必须保持模块不引用 PDFium 类型，确保之后可机械拆出。

### 5.2 Provider 契约

Rust 内部接口只接收项目自有类型：

```text
capabilities() -> languages / levels / layout_roles / confidence_kinds / limits
recognize(PageAssetRef, RecognitionOptions, Cancellation) -> PageObservation
```

真实模型默认运行在 sidecar 进程。v0.1 协议用一行一个 JSON object：

```text
hello -> capabilities -> recognize_page* -> page_result|page_error -> done
                                      \-> cancel
```

- 图片通过受限 job 目录中的相对路径 + SHA-256 传递，不把多 MB base64 塞进 stdout；
- stdout 只允许协议消息，日志走 stderr；
- 每条消息含 `protocol_version`、`job_id`、`request_id`；
- 核心验证 schema、坐标、页号、大小和引用后才写入 MDP；
- sidecar 崩溃只使对应 run/page 失败，不破坏现有二值化能力；
- API 将来复用相同 request/result DTO，只把 transport 换成异步 HTTP job。

### 5.3 本地 provider 顺序

1. `reference`：fixture 驱动的假 provider，先验证 schema、取消、部分失败和坐标；
2. `pdf-native-text`：通过现有 PDFium seam 获取原生文字/位置，建立 per-page selective OCR 基线；
3. `rapidocr-onnx`：作为轻量、CPU、跨平台的首个真实 sidecar，验证安装、page checkpoint、bbox
   归一化、低置信校对与无模型降级；
4. `tesseract-grc` / `kraken`：作为古希腊文与历史印刷候选，在 200 页语料上盲测后决定默认；
5. PaddleOCR/Docling/Marker 等 adapter：仅作实验 provider，跑同一契约和 benchmark，不让其原始
   schema 泄漏进 MDP；
6. Mistral/Gemini 等：本地路径稳定后再接 API，定位为疑难页、结构或校订增强。

默认引擎不能由本报告提前指定；仍按 `spec.md` 的决策门，以古希腊文、apparatus、速度、RAM、
模型分发和许可证的综合评测决定。

### 5.4 MDP 目录形态

开发期先用目录，交换期再 ZIP：

```text
book.mdp/
├── manifest.json
├── source.json
├── pages/
│   ├── p000001.json
│   └── p000002.json
├── runs/
├── revisions/
├── assets/
└── views/
    ├── document.md
    ├── chunks.jsonl
    ├── outline-evidence.json
    └── searchable-text.jsonl
```

按页分片能逐页落盘、取消后恢复并限制内存；manifest 只索引页与摘要，不内联全书 OCR。ZIP
必须在封包时排序条目并固定元数据，读取时先验证路径、条目数、声明大小和解压总量。

## 6. 分阶段落地与出口条件

### M0 — ADR 与 schema fixture

- 批准 MDP 逻辑模型、坐标单位、目录容器和 provider sidecar；
- 建 JSON Schema、Rust types、validator、迁移拒绝策略；
- 增加旋转页、多栏、双页扫描、古希腊 combining marks、畸形 polygon fixtures。

**出口：** 同一 fixture 在 macOS/Windows/Linux 序列化一致；未知 minor 字段可忽略，未知 major 拒绝。

### M1 — 无真实 OCR 的垂直切片

- 当前 `inspect/process` 结果写入 page/asset/provenance；
- reference provider 生成带坐标的 block/line/word；
- 导出 Markdown、chunks、outline evidence 和 searchable-text 四种视图。

**出口：** 每个派生 token 都能反查 source page 和 polygon；删除 views 后可字节稳定重建。

### M2 — 本地文字与 OCR

- 接 `pdf-native-text`，只对无文字或文字质量不合格页面 OCR；
- 跑通 Tesseract `grc` sidecar，再接 Kraken 候选；
- 实现逐页超时、取消、失败恢复、模型摘要和能力检查。

**出口：** 200 页许可清楚语料出具字符、附加符号、reading order、wall time/RAM 报告；无模型时旧流程仍完整可用。

### M3 — 自动书签材料

- 实现 TOC/page-number/heading role 与 evidence projection；
- 接既有 OutlineEngine 方案的分段页码映射和 fail-closed gate；
- 先导出 `outline-evidence.json`，暂不急于写 PDF。

**出口：** 每个候选均有可视区域和 provenance；不存在无正文证据的自动确认条目。

### M4 — 专业 PDF

- 写 invisible text layer 和标准 outline；
- 重新打开验证页数、几何、搜索、选择、Unicode、目标页和 y 位置；
- 保留现有纯二值输出 profile 与字节确定性测试。

**出口：** Preview/Acrobat/PDF.js/Foxit/iOS 阅读器矩阵通过；视觉页像素不因文字层改变。

### M5 — API

- 将 provider DTO 映射到 job API；上传、保留、地区、费用与模型版本进入 provenance；
- API 结果仍经过同一个 adapter、validator、selector 和 OutlineEngine；
- 手机和桌面只选择“本地”或“云增强”，不暴露供应商内部参数。

**出口：** 同一 fixture 的 local/API canonical IR 语义一致；云端失效可回退到本地或给出明确部分结果。

## 7. 评测

除现有二值化指标外，新增：

- OCR：CER/WER；基础希腊字母与附加符号分开；
- geometry：word/line polygon IoU、目标页与坐标误差；
- layout：region macro/micro F1、reading-order edit distance/edge F1；
- structure：heading precision/recall、hierarchy edge-F1；
- bookmark：目标页准确率、层级 edge-F1、覆盖率、幻觉数；
- AI-ready：chunk source coverage、失去 provenance 的 token 数（发布门为 0）；
- 系统：pages/s、P50/P95、峰值 RSS、模型磁盘、冷启动、失败/恢复率；
- API：每 100 页成本、上传字节、保留删除验证、P95 延迟。

公开项目的分数只作候选筛选，不作为 M PDF 处理器发布门。所有门限按“书”划分 train/dev/test，
不能把同一本书的页面拆到不同集合。

## 8. 主要风险与判断

| 风险 | 判断 | 缓解 |
|---|---|---|
| IR 过早做成通用文档平台 | 高 | v0.1 只支持 PDF/页面图像与必要实体；Office/音视频不进入范围 |
| 坐标在 OCR、裁剪、旋转后漂移 | 高 | canonical master space、整数 polygon、显式 transform、golden overlay |
| 古希腊文模型质量不足 | 高 | Tesseract/Kraken 双基线，自有 200 页评测，raw/alternatives 不丢失 |
| Python/模型破坏现有发行 | 中 | sidecar 可选安装；纯二值 core 不依赖模型运行时 |
| 全书 JSON 占内存/难恢复 | 中 | page sharding、逐页原子写、manifest 摘要、可恢复 run |
| AI 修订污染事实 | 高 | proposal/accepted 分层；所有修订引用源节点与 provider run |
| provider/license 变化 | 中 | adapter 隔离、模型/代码分别记录 license 与 digest，不把第三方类型持久化 |

总体可行性：**IR/验证器高，PDF 原生文字高，本地 OCR 工程中等，古希腊文质量中等且必须以
基准决定，自动书签中等，API 工程中等。** 最低风险的第一步不是选择“大模型冠军”，而是
冻结 MDP 0.1 的 page/asset/coordinate/text/provenance 契约，并用 reference + native text
完成一条可验证垂直切片。

## 9. 建议立即做的工作

1. 起草 `ADR 0003 — MDP 0.1`：确认独立 crate、整数 master space、page sharding；
2. 起草 `ADR 0004 — OCR provider boundary`：确认 NDJSON sidecar 与 capability negotiation；
3. 制作 8–12 页 contract fixture，不等完整 200 页 benchmark；
4. 实现 Rust model + validator + reference provider，不接真实 OCR；
5. 用 PDFium native text 跑通 `MDP → chunks/outline-evidence/searchable-text`；
6. 再接 Tesseract `grc`，以真实问题修正契约；契约稳定后比较 Kraken 和通用文档 parser。
