# M5 单人权威标注与阅读器验收包

这个语料包由产品负责人作为唯一权威标注者。它用于 M5 产品验收，
不声称具有标注者间一致性指标。源 PDF 不进入 Git；manifest 只保存安全
相对路径、SHA-256、页数和待核对的 TOC 提示页。

仓库已有的 20 份语料只验证原生 outline 保留。本包另外选择 12 份互不重复的
PDF（digital TOC、scanned TOC、safe-refusal 各 4 份），用于补齐前一批没有覆盖的
人工判断。两批合计 32 份真实 PDF；不把同一份材料重复计数。

## 准备工作包

在仓库根目录运行；输出目录必须尚不存在：

```sh
MPDF_PDFIUM_LIBRARY='/absolute/path/to/libpdfium.dylib' \
python3 scripts/m5/human_acceptance.py prepare \
  '/absolute/path/to/翻译agent 2/input' \
  '/absolute/path/to/new-acceptance-pack' \
  --annotator pei-haoran
```

命令会核对 12 份源 PDF 的 SHA-256 与页数，并生成 `source-files.csv`、三个待填表格
和固定阅读器 PDF。它默认 no-clobber，失败时删除 staging，不留下半成品。

## 索引约定

- 所有 `physical_page_index` 都从 **0** 开始；PDF 阅读器显示的第 1 页对应索引 0。
- `toc_hint_page_indices` 是自动预检提示，不是金标准；请以你看到的 PDF 为准。
- `target_y_fraction_from_top` 和 bbox 使用可见页归一化坐标：左上角是
  `(0, 0)`，右下角是 `(1, 1)`。例如页面上方约 20% 处的标题写 `0.20`。
- 无法精确确定 y/bbox 时可留空，并在 `notes` 说明原因；校验会将其记为
  未完成坐标证据，不会伪造数值。

## 第一部分：文档级决策

编辑 `document-decisions.csv`，每份 PDF 一行。

1. 填写 `annotator`。
2. 核对或修正 `toc_physical_page_indices`，多页用分号分隔。
3. 从下列值选择 `expected_behavior`：
   - `bookmarks_required`：证据足够，应生成书签；
   - `needs_review`：可提候选，但不应自动确认；
   - `safe_refusal`：证据不足，不应猜测目录。
4. 完成后将 `review_status` 改为 `complete`。

manifest 的 `evidence_class` 只表示抽样分层。如果你发现所谓“安全拒绝”文档
确实有充足标题证据，应如实改为 `bookmarks_required` 或 `needs_review`。

## 第二部分：书签金标准

编辑 `bookmarks.csv`，每个应有或明确排除的书签一行。

- `bookmark_id`：文档内稳定且易读的 ID，如 `digital-proclus-001`。
- `decision`：`include` 或 `exclude`。
- `level`：从 0 开始。子项的 `parent_bookmark_id` 必须指向同一文档的上级。
- `target_physical_page_index`：标题正文所在的物理页，不是印刷页码。
- `printed_page_label`：书页上印刷的页码，可为罗马数字或空值。
- `evidence_kind`：`digital_toc`、`scanned_toc`、`heading_region`、`typography`、
  `numbering` 或 `other`。
- `confidence`：`high`、`medium` 或 `low`。

`safe_refusal` 文档不能有 `include` 行。`bookmarks_required` 文档至少有一个
`include` 行。

## 第三部分：Acrobat / Preview / iOS

打开 `reader-fixture/searchable-rotations.pdf`，在 `reader-results.csv` 中填写实际应用与
版本。每项只用 `pass`、`fail`、`not_run` 或 `not_applicable`。

逐项验证：

1. PDF 可打开，共 4 页。
2. 与 `source-rotations.pdf` 对比，黑色图形与页面方向没有可见变化。
3. 能搜索 `Ἀρχὴ` 与 `Πολιτείας`；搜索命中不应出现可见黑色文字层。
4. 书签树为：
   - `1. Ἀρχὴ`
     - `1.1 Πολιτείας`
   - `2. Appendix`
5. 子书签跳到阅读器第 2 页，Appendix 跳到第 4 页。
6. 0/90/180/270 度页面中，跳转后页面方向和目标位置合理。
7. 在 `evidence_notes` 记录所见结果；失败时写明页码、书签和具体现象。

## 校验

填写完成后运行：

```sh
python3 scripts/m5/human_acceptance.py validate /path/to/acceptance-pack
```

成功时会在验收包中写入 `validation-report.json`。任何缺页、空标题、越界坐标、
断裂父子关系、不完整阅读器检查或伪造的安全拒绝都会非零退出。
