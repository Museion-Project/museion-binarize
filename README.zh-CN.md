[English](README.md) | 简体中文

# Museion Binarize

**Museion Binarize** 是一款开源、跨平台的应用程序，用于将扫描的学术书籍转换为
干净、紧凑的双色（bilevel）PDF 文件。

**当前状态：Phase 1 —— 早期开发阶段。** 已具备完整的本地命令行处理流程
（`inspect`、`analyze`、`estimate`、`process`、`preview`、`benchmark`，
支持带版本号的 JSON 报告），桌面 GUI 现已接入同一处理流程（打开、预览、
配置、实验性的输出大小预估、转换、取消——详见 [`docs/desktop.md`](docs/desktop.md)）。
`estimate` 会基于抽样生成实验性的输出体积预测——详见
[`docs/size-estimation.md`](docs/size-estimation.md)；这并非保证值。
`benchmark` 是一套可复现的、基于像素级标准答案（ground truth）的二值化
保真度基准测试框架——详见 [`docs/benchmarking.md`](docs/benchmarking.md)；
其内置的合成测试集仅用于验证框架本身，**并非**真实扫描文档的代表性语料，
也不构成对历史多音调希腊语版本保真度的证明。端到端行为目前仅在受控配置的
macOS 环境中验证过；桌面 GUI 的原生应用验收记录详见
[`docs/desktop-testing.md`](docs/desktop-testing.md)。请参阅
[`docs/limitations.md`](docs/limitations.md) 了解本仓库当前能做什么、
不能做什么。

## 核心原则

- **处理过程透明，结果可复现，不对源文档进行生成式改写。**
- 优先采用确定性、可解释的算法，而非黑箱模型。每一次二值化决策都应可追溯到
  一个有文档记录的方法和参数。
- 本地优先：您的扫描件和输出文件始终保留在您自己的设备上。
- 在处理大型扫描书籍时，具有可预测、有边界的资源占用。
- 默认跨平台：macOS、Windows 和 Linux 都是一等目标，而非事后添加的支持。

Museion Binarize 不会将自身描述为“AI 驱动”。Phase 1 使用的是经典、确定性的
图像处理方法，而非机器学习模型。

## 计划中的 Phase 1 功能

- 支持 macOS、Windows 和 Linux 桌面平台。
- 完全本地化的 PDF 处理——转换过程无需上传、无需依赖网络。
- 确定性的 **Otsu**、**Sauvola** 以及**手动**阈值化二值化方法。
- 从扫描页面图像重建真正的 1-bit（双色）PDF。
- 采用 **CCITT Group 4** 压缩，输出紧凑的双色文件。
- 提供图形化桌面应用与命令行界面，二者共享同一处理核心。
- 提供可复现的基准测试框架，用于评估输出质量。

以上功能目前均尚未在本仓库中实现；Phase 1 才刚刚开始。里程碑规划详见
[`docs/roadmap.md`](docs/roadmap.md)。

## 研究方向

在后续、以基准测试为驱动的研究阶段，项目将评估能够更好地保留**多音调古希腊语
（polytonic Ancient Greek）**、**校勘说明（critical apparatuses）**以及其他
容易被激进二值化破坏的细小印刷细节的方法。目前项目**不**声称 Museion
Binarize 能够保留此类排版细节——这一结论只有在可复现的基准测试数据出现之后
才会给出。详见 [`docs/roadmap.md`](docs/roadmap.md) 与
[`docs/benchmarking.md`](docs/benchmarking.md)。

## 当前的非目标（Non-goals）

Phase 1 **不包括**：

- OCR（光学字符识别）。
- 保留源 PDF 中隐藏的 OCR 文本层。
- 任何形式的 AI 或机器学习模型。
- 对损坏或缺失内容的生成式修复（inpainting）。
- 页面去扭曲（dewarping）或几何校正。
- 注释或表单字段的保留。

完整列表及原因说明请见 [`docs/limitations.md`](docs/limitations.md)。

## 隐私

Museion Binarize 的设计目标是完全在您自己的设备上处理文件。核心处理流程不会
将扫描件、页面图像或输出文件上传到任何网络服务。桌面应用与命令行工具均仅处理
您本地选择的文件。

## 仓库架构

```
museion-binarize/
├── crates/
│   ├── museion-binarize-core/   # 不依赖 Tauri 的 Rust 处理核心
│   └── museion-binarize-cli/    # 基于核心库构建的命令行界面
├── apps/
│   └── desktop/                 # Tauri 2 + React + TypeScript 桌面应用
├── docs/                        # 架构、路线图、算法、基准测试文档
├── benchmarks/                  # 可复现的基准测试框架（规划中）
├── test-data/                   # 合成数据与来源明确的测试素材
└── .github/                     # CI 工作流、Issue 与 PR 模板
```

完整设计说明（包括为何处理核心独立于 Tauri，以及规划中的 PDF 处理流水线）请见
[`docs/architecture.md`](docs/architecture.md)。

## 开发说明

### 前置依赖

- Rust（版本由 [`rust-toolchain.toml`](rust-toolchain.toml) 固定）
- Node.js（版本固定于 [`.nvmrc`](.nvmrc)）
- [pnpm](https://pnpm.io/)（`corepack enable pnpm`）

### 构建与测试 Rust 工作区

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

### 运行命令行工具

```bash
cargo run -p museion-binarize-cli -- --help
```

### 提供 PDFium

PDF 渲染需要 PDFium 动态库。本项目不捆绑该库、不将其提交到仓库，也绝不在运行时下载——需要您自行提供一次。详见 [docs/pdfium.md](docs/pdfium.md)。

```bash
export MUSEION_PDFIUM_LIBRARY=/path/to/libpdfium.dylib
```

### 命令行用法

```bash
# 检查文档：页数、页面几何、旋转、各 DPI 下的渲染尺寸
museion-binarize inspect input.pdf

# 通过真实处理流程测量文档，不写出转换后的 PDF——适合在正式转换前挑选参数
museion-binarize analyze input.pdf --dpi 300 --method otsu --json --pretty

# 通过真实处理流程对少量抽样页面进行处理，推算出实验性的输出体积预估，
# 同样不写出转换后的 PDF
museion-binarize estimate input.pdf --dpi 400 --method sauvola --samples 8

# 转换为双色（bilevel）CCITT Group 4 PDF
museion-binarize process input.pdf --output output.pdf \
  --method sauvola --dpi 400 --validate render-all

# 将处理后的某一页保存为 PNG 预览（页码从 1 开始）
museion-binarize preview input.pdf --page 12 --output preview.png

# 基于像素级标准答案，对二值化保真度进行基准测试
# （光栅级基准测试无需 PDF/PDFium）
museion-binarize benchmark run \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml
```

常用选项：`--method otsu|sauvola|manual`、`--threshold`、`--sauvola-k`、`--sauvola-window`、`--contrast`、`--median-denoise`、`--background-normalization`、`--despeckle off|conservative|strong`、`--overwrite`、`--pdfium-library`、`--pages`（仅 `analyze` 支持，例如 `1,3,8-12`）、`--json`/`--pretty`/`--quiet`/`--report`。进度信息输出到 stderr；最终结果（人类可读或 `--json`）输出到 stdout。完整命令、退出码与 stdout/stderr 约定见 [`docs/cli.md`](docs/cli.md)，JSON 报告结构见 [`docs/reporting.md`](docs/reporting.md)。

源文件绝不会被修改；只有在生成并通过校验的完整文档就绪后，才会写入目标路径。

### 运行桌面应用

```bash
pnpm install
pnpm --filter museion-binarize-desktop tauri dev
```

## 贡献

欢迎参与贡献。在提交 Pull Request 前，请阅读
[`CONTRIBUTING.md`](CONTRIBUTING.md)——其中说明了代码格式、测试要求，以及
关于测试素材来源和未经证实结论的相关规则。如需报告安全漏洞，请参阅
[`SECURITY.md`](SECURITY.md)。

## 引用

如果您使用本软件，请依据 [`CITATION.cff`](CITATION.cff) 中的元数据进行引用。

## 作者与维护者

Museion Binarize 由 **Pei Haoran（裴皓然）** 在 **Museion Project** 组织下
创建并维护。详见 [`AUTHORS.md`](AUTHORS.md)。

## 许可证

本项目采用双重许可，您可任选其一：

- MIT License（[`LICENSE-MIT`](LICENSE-MIT)）
- Apache License, Version 2.0（[`LICENSE-APACHE`](LICENSE-APACHE)）
