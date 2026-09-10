# TOC compiler 三项修复：四页 regression

**三项修复完成；固定 60 条源目录中，层级＋printed page＋PDF target 同时正确由 23/60 提升为 55/60（91.7%）。** 这是相同四页上的开发回归结果，不是新书验证或完整 GUI 发布验收。产品目标按 `intent.md §8.34 TG1–TG4` 改为 **Local automatic bookmark generation with editable review**。

## 结果与书级修改负担

| 固定目录范围 | 源条目 | 独立条目 | parent＋level | printed page | PDF target | 三项同时正确 | 需要修改／处理的节点 |
|---|---:|---:|---:|---:|---:|---:|---|
| Horn PDF6/7 | 18 | 18 | 18 | 17 | 17 | **17/18** | 1：Introduction 缺页码／目标 |
| Menn PDF7 | 11 | 11 | 11 | 9 | 9 | **9/11** | 2：第一、第二章缺 1／6 及目标 |
| Brisson PDF13 | 31 | 31 | 31 | 30 | 29 | **29/31** | 2 个源条目：85 缺号／目标，64 缺目标锚点；另 1 个无页码章题候选需保留并定位或删除 |
| 合计 | **60** | **60** | **60** | **56** | **55** | **55/60** | **5 个源条目＋1 个额外候选** |

上一轮对应值为：独立条目 58/60、层级 27/60、printed 54/60、target 53/60、三项全对 23/60。分母、parent/level 标准与上一轮 `assessment-final.json` 完全一致：Brisson 的无页码章题不计入31源条目，1.1/1.2/1.3规范化为顶层；旧合并的85/86仍是两个独立源条目。本轮按原始 observation ID 分别定位二者，没有因分组变化改写参考数据。

书级判断：所选范围分别需要 **1／2／3 个关键节点操作**，已经接近“看到基本正确的树，改少量位置再保存”的目标。这里计的是代理复核后的**最少数据修改节点**，不是实际用户计时或 UI 点击次数，也不包含可辨认标题中的拼写美容。Brisson 是章节局部目录页，不能外推其整本书已达到同样可用率；没有新增500页/40书签样本。

证据：`assessment.json` 有逐源条目对照，`assess_regression.py` 是仅在运行结束后使用的计分器。它读取旧代理源图复核记录而非 human Gold；运行时 grammar、grouping 和 numeric lane 不导入它。没有新 Gold、holdout 或书籍。

## A. Whole-book hierarchy grammar

新增 Rust `local_grammar.rs`，仅用于本地 editable TOC 入口；旧0.4引擎及其测试保持原合同。

- 解析 PART/BOOK/CHAPTER/SECTION、Roman、Arabic、decimal path、大小写字母和 Preface/Introduction/Conclusion/Notes/Bibliography/Index 等命名角色。
- 先收集同书编号 inventory，再以路径前缀、兄弟序列和容器作用域确定 parent。1→1.1/1.2→2→2.1，以及省略后续2.1的情况都有合同测试。
- `l.l.I` 保留为带疑似标志的 `1.1.1`；原 title/raw 不覆盖。多点号可形成规范化路径；破损编号只能在前后明确 decimal siblings 的约束下形成候选。A/4、a/短破损前缀也必须有字母兄弟证据，不能仅靠单行字形自动确认。
- 命名前后文项回到顶层；Index/Indices 之后的连续数字条目形成同一组子项。无可靠编号但明确缩进的条目可形成带疑义的 parent 候选；不把字体大小当作已验证层级。
- 不再统一附加 `hierarchy_requires_inspection` 来否定每个节点。局部规范化、缺号、映射未确认等仍留下 review reasons，不以降低这些标记来刷正确率。

当前 review 标记数量仍为 Horn14、Menn11、Brisson32，主要含数字置信度、native header映射和规范化提示。它们表示需要在书级复核中留意的证据，不等于14/11/32个节点实际错误；上述修改负担以源图语义核对为准。旧 writer 的确认/导出合同未改，书级一次确认的 GUI 交互也不在本轮范围内。

## B. Multiline grouping contract

合法 page number 默认结束条目。下一行只有无页码、无新编号/命名节、缩进和小间距相符且有明显短续行或断词证据，才允许并入。没有页码的行也不会直接变成新条目；它可继续承接标题或作者行。作者＋Introduction 作为必要边界处理，避免修复命名节时把作者另拆为节点。

- Menn `The Journal ... / Monograph Series` 在 Vision 与真实 native glyph 两条输入上均合为一项。其 native候选由前轮12变为11。
- Brisson 85/86 成为两个独立条目；85仍没有可靠识别值，单独留空，而不是把86借给它。
- 数字 lane 第二读裁剪可能看到相邻视觉行的号码。因此先按返回数字的真实 bbox 重新关联最近的标题行，再 grouping；不会把同一个84同时挂到长标题的两行上。
- 同一旧native raw重放，Horn仍18条、Menn11条；没有新增native抽取或PDF语料。

## C. Mandatory numeric-lane reread

保留四份冻结的 full-page Vision raw，只对其标题视觉行的右侧数字lane做正常第二读。这里的“全部”包括作者、续行及可能为空的数字位置，而非只读最终条目或低置信度数字。

- **84 个 crop**：Horn PDF6 19、PDF7 16、Menn 13、Brisson 36；只执行一轮，没有循环重试。
- 原 Vision fast Swift 源码和现有可执行文件没有修改；仍用同一 `.fast` 设置，无模型下载、云调用、费用。
- 第二读 OCR 合计 **0.827 秒**；一次300dpi raster/页合计约 **0.169 秒**。没有重跑完整页OCR；累计阶段时间不冒充连续GUI E2E时间。逐页数据在 `performance.json`。
- 正常 intake 的 Vision 分支已串入 `numeric_lane.py`；native glyph 分支不额外识别。`retry_numbers.py` 仅保留历史用途，新的正常路径不再使用8个低置信度ROI上限。
- 空结果、不合pattern、多候选、与完整页冲突均保持不确定。层级grammar不参与“猜页码”。本轮补回Menn的ix等字段，但仍有上述4个缺号，另有1个缺目标锚点。

Surya 本轮运行 **0 页**。TOC compiler 的请求边界收窄为双栏阅读顺序异常、明显bbox碎裂、无法形成稳定物理行组；语义hierarchy、数字漏识别不触发它。Surya、adapter与历史几何结果保持冻结。

## 修改范围、验收和停止

主要改动：`bookmarks/local.rs` 与新增 `local_grammar.rs`；Python `project/compile_pages`；新增 `numeric_lane.py`；`intake.py` 只接入必经的数字阶段。未修改 Vision worker、Surya/adapter、native抽取逻辑、页码映射逻辑、editor UI 或 bookmark-only writer；旧证据目录全部保留。

测试：51个书签单元测试、26个既有编译器集成测试、13个Python分组/编辑/PDF合同测试通过。新合同覆盖decimal兄弟、易混编号、Roman/字母、命名前后文、PART/BOOK/CHAPTER/SECTION、已找到页码后的续行、漏号后的新标签、作者＋Introduction、数字crop跨行归属。Rust测试与Python回归使用合成合同输入；真实语料仍只有原四页。最终编译器重放确认交付表与代码一致。

意图对齐：**TG1–TG4 aligned**。本轮三项修改与固定回归完成，超过用户提出的50+/60观察目标；未声明全部节点自动confirmed。更广泛书级可用率、完整书籍覆盖、用户实际编辑时间、GUI一键保存和分发仍未验证。按用户范围停止，不继续扩模型、语料或GUI开发。

可编辑产物：`horn-editor.html`、`menn-editor.html`、`brisson-editor.html`，以及对应 `*-editable.json`。本轮未生成新的修订PDF来替代自动结果；旧PDF和旧代理修订补丁均未覆盖。
