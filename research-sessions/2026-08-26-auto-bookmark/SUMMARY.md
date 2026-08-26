# Research Query: M PDF 处理器自动 Bookmark 独立实现方案

**日期：** 2026-08-26
**状态：** Complete
**关键词：** PDF bookmark, outline, table of contents extraction, heading hierarchy, page-number alignment, OCR, scanned books

## 研究问题

如何在不融合第三方项目代码的前提下，为 M PDF 处理器设计一个简单、准确、低人工干预的自动 PDF bookmark 管线，并同时提供电脑本地模式和电脑/手机 API 模式？

## 检索策略

- GitHub：近期 release、官方 README、实现代码与已知限制；
- arXiv：TOC extraction、hierarchical document reconstruction、layout benchmark；
- 产品案例：MarginNote、Foxit、PDF-XChange、EverMap 的官方手册；
- 项目内证据：`spec.md`、`intent.md`、`plan.md`、当前 `pdfium-render` 与 `pdf-writer` 能力。

## 核心发现

1. 最可靠架构是 `existing outline > printed TOC + body verification > numbering > style > model`。
2. LLM/VLM 应是 evidence provider，不应是最终 page destination 的权威。
3. 目标页必须由多锚点页码映射和正文标题共同验证；单一 offset 不够。
4. 默认应跳过不确定项，而不是要求用户逐项复核。
5. 本项目现有 Rust 依赖已具备 outline 读取、文本坐标读取和 outline 写入原语。
6. MarginNote 的 AI 功能生成虚拟目录而非改写 PDF，本项目的标准 PDF 输出要求更严格。

## 完整方案

见 [`docs/auto-bookmark-research.zh-CN.md`](../../docs/auto-bookmark-research.zh-CN.md)。

## 文件清单

- `SUMMARY.md`：检索方法与结论；
- `papers-reviewed.json`：论文/软件/案例筛选记录；
- `relevant-papers.json`：高相关论文；
- `citations/citation-graph.json`：本次使用的关系记录；
- `docs/auto-bookmark-research.zh-CN.md`：项目级实现建议。
