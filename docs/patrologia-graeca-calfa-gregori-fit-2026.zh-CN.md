# Patrologia Graeca Corpus / Calfa–GREgORI（2026）项目适配度评估

**评估日期：** 2026-08-29
**对象：** Calfa–GREgORI Patrologia Graeca（CGPG）语料、OCR ground truth、公开布局权重与 Calfa 托管识别管线
**项目基线：** 本仓库当前的本地优先 OCR、MDP 坐标证据、可搜索 PDF、可替换 sidecar provider 与 200 页真实古希腊文评测目标

## 结论

CGPG **高度适合作为本项目的真实 PG 压力测语料和页面布局研究资产（8.5/10）**，**适合作为可选的搜索/词形分析语料（7/10）**，但 **截至 2026-08-29 不适合直接替换本项目的本地 Tesseract 生产 provider（3/10）**。

建议采用“**先语料，后引擎**”路线：

1. 立即将 Zenodo v2 导入独立的外部 OCR 评测流程，先重测当前 `tessdata_best grc+deu+eng` 在真实 PG 扫描上的 CER、变音符错误和版面恢复。
2. 不把公开语料的发布误解为可嵌入引擎的发布：公开仓库只附布局检测权重，没有可下载的 PG CRNN 识别权重、本地推理代码或稳定 API 契约。
3. 只在获得识别模型、商用/再分发条款、引擎依赖许可、官方 train/test 切分和可固定的权重哈希后，再开发 Calfa provider 候选。

## 对象盘点

### 1. 论文与管线

LREC 2026 论文报告了一条专为 19 世纪 PG 设计的管线：YOLO 页面布局检测 + CRNN 文字识别 + GREgORI 词形/词性分析。论文在 30 页手工转录测试集上报告：

| 模型 | CER | WER |
|---|---:|---:|
| Tesseract Greek | 11.57% | 39.65% |
| Transkribus 19th-century Greek | 6.14% | 14.82% |
| 人工噪声预训练 CRNN | 8.12% | 11.23% |
| CGPG PG-specific CRNN | **1.05%** | **4.69%** |

这是很强的相关性信号，但不能直接与本仓库当前的合成 gold 数字横向比较：两者页面、评分单位、语言组合和扫描退化都不同。相反，论文的 Tesseract 11.57% CER 表明，本项目的合成样本不足以支撑 PG 级质量声明。

### 2. OCR ground truth

Zenodo record `10.5281/zenodo.20008699` 于 2026-04 发布 v2，记录元数据标明 **CC BY 4.0**。实际下载并审计 `data-v2.zip` 后得到：

- 304 张图像 + 304 个 PAGE XML（2013-07-15 namespace）；
- 266 个 XML 含非空行转录，合计 11,058 条非空 `TextLine/TextEquiv/Unicode`；
- 38 个 XML 是布局导向页，有 region/line polygon 但转录为空；
- 常见类别包括 `MainText_ColGreek`、`MainText_ColLatin`、`MainText_Title`、`Marginalia`、`Marginalia_Footnote`、`Marginalia_PageNumber`、`Title_RunningTitle`；
- 没有 PAGE `ReadingOrder`、`Word` 元素或置信度属性；
- Zenodo 发布的 MD5 是 `20bf8621dfb4dfe4d9270eee26cfc90e`；本次下载后计算的 SHA-256 是 `2ee5d79f3c781dc1b64fa386f0f194a762ab183cd36b97f5d873ce0a3004e1f7`。

论文写的训练统计是 445 images / 11,096 text lines，而 Zenodo v2 明确发布 304 pairs；公开包与论文内部训练集不完全同构。因此本项目应将 v2 视为“可用的公开压力测语料”，而不宣称它能精确复现论文数字。

### 3. OCR 文本与语言标注语料

当前 GitHub 仓库标记 33 volumes / 约 600 万词，仓库的原始文本用 `$0`（卷）、`$8`（PDF 页）、`$9`（起始行）标记。Zenodo record `10.5281/zenodo.19915273` 提供带 lemma/POS 的 `.vert` 语料，亦为 CC BY 4.0。

这是“silver corpus”：项目网页和论文均明示仅有最少人工校订，使用者必须回到扫描校核。它适合搜索、词频、候选排序和校订界面辅助，不适合作为无条件的 OCR 真值或临界版本替代品。

### 4. 可执行模型的发布状态

公开 GitHub 在审计时只有 `models/REG-YOLOv12s.pt`（18,993,171 bytes；固定 commit `a415fcae253cb2d7ec69a1176cfef1008c252215`；本次计算 SHA-256 `47a69c4eae86e765aeb907f170a227c9f64f491e4843729c97f2e6fe06cec5b0`）。它是布局检测权重，不是 CRNN 文字识别器。

直接接入该 `.pt` 存在三个门禁：

- 需要确认实际推理代码与权重的分别许可；YOLOv12 参考实现为 AGPL-3.0，与本项目 MIT/Apache-2.0 发布策略有强开源/商业许可风险；
- PyTorch `.pt` 常为 pickle 容器，不应在未隔离的构建机或客户端直接加载不可信权重；
- 仓库没有固定运行时、导出的 ONNX/safetensors、类别映射版本、阈值契约或离线安装说明。

Calfa Vision 可在网页上免费进行标注和使用 `Greek printed (Patrologia Graeca)` 布局模型，但 Calfa 的 Research Plan 明示定制 OCR/HTR 模型不开源、不可下载。这与本项目“本地离线、可固定、可再分发”的生产默认线不相符，但可作为明确同意后的外部研究服务或手工标注工具。

## 与本项目的契合矩阵

| 维度 | 适配度 | 判断 |
|---|---:|---|
| 真实多音调古希腊文扫描 | 10/10 | 正中本项目尚未完成的真实史料基准空白 |
| PG 双栏、页眉、边注、脚注布局 | 9/10 | PAGE polygon 和语义 region 类别很有价值 |
| 本项目坐标证据模型 | 6/10 | 有 page/region/line polygon 和 baseline，但没有 word bbox、confidence 和显式 reading order |
| 希腊/拉丁/德语/英语完整保留 | 4/10 | CGPG 生产输出主动仅抽取希腊文，并人工删除拉丁文；不能替代本项目的混合语言保存路径 |
| 可离线生产推理 | 2/10 | 识别权重/推理代码未发布；只有布局 `.pt` |
| 许可与再分发 | 5/10 | 数据为 CC BY 4.0，但布局运行时可能引入 AGPL，识别模型条款未公开 |
| 词形/词性/搜索增强 | 8/10 | `.vert` 和约 600 万词可作可选的派生层，不得覆写原始 OCR 证据 |
| 泛化到所有古希腊文版本 | 5/10 | 对 PG/Migne 特异性强，不等于手稿、现代临界本或其他 19 世纪铸字的普遍性 |

## 建议的适配方案

### P0：建立 CGPG 外部 OCR 压力测（建议现在做）

不要把 PAGE XML 塞进现有“输入图 + 二值 ground-truth mask”数据集 schema。应为 `scripts/ocr/gold` 增加一个权利清晰的外部 PAGE 语料加载器，不把 406 MB 数据直接提交进仓库。

最小验收条件：

1. 使用 DOI + 版本 + SHA-256 的外部 manifest，显式记录 `CC-BY-4.0`、作者和归属文本。
2. 校验 ZIP 路径逃逸、XML 外部实体、图像尺寸、polygon/baseline 边界和资源上限。
3. 将 266 张有转录的页用于行级 OCR 压力测，38 张布局页用于 region 检测；不伪造 word boxes 和 confidence。
4. 同时报告 raw Unicode CER 和经明确规则正规化后的 CER；原始字符永远保留。变音符、Greek↔Latin 形近混淆、大写标题和栏误检需分项报告。
5. 在任何对 Calfa 模型的比较中，先取得论文 30 页 test IDs 和官方 train/test 切分，避免用训练页验证其自身。

### P1：将 CGPG 语料用于审校路由（可选）

可以从 CC BY `.vert` 构建带来源的词形频率、lemma/POS 候选和 PG 专有名词词典，用于：

- 将低频/疑似异常的 OCR token 路由到人工审校；
- 在界面中显示不覆写原文的建议；
- 增强搜索和词形归并。

不建议用它静默“修复” OCR；它本身也含 OCR 错误，而且本项目的不可变原始证据原则必须继续保持。

### P2：Calfa provider 可行性 spike（有条件）

只有在下列问题获得书面答复后才开始：

- PG CRNN 识别权重是否可离线下载、商用和随应用再分发？
- 推理代码的完整许可树是什么？能否提供 ONNX/safetensors 或不触发 AGPL 的推理路径？
- 能否固定模型版本、字符集、输入尺寸、类别映射、预/后处理和 SHA-256？
- 能否输出 word/line bbox、confidence、reading order 和完整混合语言，而不是仅希腊文？
- 是否允许用本项目自己的 200 页独立评测集公开结果？

若答案为否，Calfa 应保留为显式同意的外部研究工具，不进入默认产品路由。

## 不建议的做法

- 不要用论文的 1.05% CER 宣传本项目的识别质量；本项目尚未运行该模型，也没有复现测试切分。
- 不要把 CGPG raw/silver OCR 当作临界文本、全量校对真值或无错词典。
- 不要把仅希腊文抽取的输出写回为整页的完整 OCR 证据；它会静默丢失拉丁文、脚注和标题。
- 不要在本地构建或应用进程中直接 `torch.load` 未经隔离的 `.pt` 文件。
- 不要在缺少模型许可、哈希、SBOM 和完整运行时条款时捆绑权重。

## 最终决策

**Go：** 将 CGPG Zenodo v2 纳入外部、可固定、不入 Git 的 PG OCR/布局压力测；将 CC BY 语料评估为可选的搜索和审校辅助层。
**Conditional Go：** 对公开布局权重做隔离、不发布的技术 spike，前提是先理清运行时许可和安全加载方式。
**No-Go（现阶段）：** 将 Calfa/CGPG 作为本地默认 OCR provider 或用它取代 `tessdata_best`。

## 主要资料

- Vidal-Gorène, C. & Kindt, B. (2026), [The Patrologia Graeca Corpus: OCR, Annotation, and Open Release of Noisy Nineteenth-Century Polytonic Greek Editions](https://arxiv.org/abs/2603.09470)，LREC 2026。
- UCLouvain, [Calfa-GREgORI Patrologia Graeca](https://uclouvain.be/fr/instituts-recherche/incal/ciol/calfa-gregori-patrologia-graeca)。
- Zenodo, [Patrologia Graeca (OCR ground truth), v2](https://zenodo.org/records/20008699)，CC BY 4.0。
- Zenodo, [Patrologia Graeca (OCRized and analyzed texts)](https://zenodo.org/records/19915273)，CC BY 4.0。
- Calfa/GREgORI, [Patrologia-Graeca GitHub repository](https://github.com/calfa-co/Patrologia-Graeca)。
- Calfa Vision, [annotation platform and plans](https://vision.calfa.fr/)。
- Calfa, [Research Plan](https://calfa.fr/research-plan)。
- YOLOv12 reference implementation, [license and runtime](https://github.com/sunsmarterjie/yolov12)（AGPL-3.0）。
