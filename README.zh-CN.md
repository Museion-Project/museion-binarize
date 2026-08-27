[English](README.md) | 简体中文

# M PDF 处理器

**M PDF 处理器** 是一款开源、跨平台的应用程序，用于将扫描的学术书籍转换为
干净、紧凑的双色（bilevel）PDF 文件。

**当前状态：Phase 1 功能已完成——当前公开发行候选版为
`v0.1.0-rc.2`。** 已具备完整的本地命令行处理流程
（`inspect`、`analyze`、`estimate`、`process`、`preview`、`benchmark`，
支持带版本号的 JSON 报告），桌面 GUI 已接入同一处理流程（打开、预览、
配置、实验性的输出大小预估、转换、取消——详见 [`docs/desktop.md`](docs/desktop.md)）。
`estimate` 会基于抽样生成实验性的输出体积预测——详见
[`docs/size-estimation.md`](docs/size-estimation.md)；这并非保证值。
`benchmark` 是一套可复现的、基于像素级标准答案（ground truth）的二值化
保真度基准测试框架——详见 [`docs/benchmarking.md`](docs/benchmarking.md)；
其内置的合成测试集仅用于验证框架本身，**并非**真实扫描文档的代表性语料，
也不构成对历史多音调希腊语版本保真度的证明。目前仅 **macOS（Apple
Silicon）** 完成了人工端到端运行验收——桌面 GUI 的原生应用验收记录详见
[`docs/desktop-testing.md`](docs/desktop-testing.md)；Windows 与 Linux
安装包可正常构建打包，但尚未完成人工运行验收（详见下方"下载"一节）。
请参阅 [`docs/limitations.md`](docs/limitations.md) 了解本仓库当前能做
什么、不能做什么。

MDP 0.1 证据包切片也可通过 CLI 使用：
`mpdf package create book.pdf --output book.mdp` 和
`mpdf package validate book.mdp`。它保存确定性的来源/页面几何证据和
SHA-256 引用，不执行 OCR，也不复制源 PDF；详见
[`docs/document-package.md`](docs/document-package.md)。

M2 本地持久任务库可在不接入 OCR provider 的情况下验证：

```bash
mpdf job create --db .mpdf/jobs.sqlite --job-id demo --pages 500
mpdf job status --db .mpdf/jobs.sqlite --job-id demo
mpdf job cancel --db .mpdf/jobs.sqlite --job-id demo
```

M3 本地 OCR 最小入口会优先复用 PDF 原生文字层，只对空白、极少文字或乱码页逐页
渲染并写入 typed block/line/word 证据；默认使用需显式提供本地可执行文件和模型目录的
RapidOCR，`reference` provider 仅用于开发和测试。每页结果与 raw artifact 在 SQLite
checkpoint 前持久化，支持取消后保留已完成页并安全重跑：

```bash
mpdf ocr scan.pdf --output scan.mdp --jobs-db .mpdf/jobs.sqlite --job-id scan-1 --provider reference
```

M4 提供确定性的 AI-ready 派生导出和本地校对队列：
`mpdf export scan.mdp --format all --output scan-derived`、
`mpdf review scan.mdp --json` 以及 `mpdf revision`。派生 IR 保留页、bbox
和不可变 OCR 原文，不调用云服务；详见 [`docs/derived-document.md`](docs/derived-document.md)。

## 核心原则

- **处理过程透明，结果可复现，不对源文档进行生成式改写。**
- 优先采用确定性、可解释的算法，而非黑箱模型。每一次二值化决策都应可追溯到
  一个有文档记录的方法和参数。
- 本地优先：您的扫描件和输出文件始终保留在您自己的设备上。
- 在处理大型扫描书籍时，具有可预测、有边界的资源占用。
- 默认跨平台：macOS、Windows 和 Linux 都是一等目标，而非事后添加的支持。

M PDF 处理器 不会将自身描述为“AI 驱动”。Phase 1 使用的是经典、确定性的
图像处理方法，而非机器学习模型。

## Phase 1 功能

- 支持 macOS、Windows 和 Linux 桌面平台。
- 完全本地化的 PDF 处理——转换过程无需上传、无需依赖网络。
- 确定性的 **Otsu**、**Sauvola** 以及**手动**阈值化二值化方法。
- 从扫描页面图像重建真正的 1-bit（双色）PDF。
- 采用 **CCITT Group 4** 压缩，输出紧凑的双色文件。
- 提供图形化桌面应用与命令行界面，二者共享同一处理核心。
- 桌面应用支持原生单 PDF 拖入打开。
- 提供可复现的基准测试框架，用于评估输出质量。

以上功能均已在本仓库中实现。里程碑演进历史详见
[`docs/roadmap.md`](docs/roadmap.md)；如何获取打包版本请见下方"下载"一节。

## 下载

当前的公开发行版本是
[**v0.1.0-rc.2**](https://github.com/Museion-Project/museion-binarize/releases/tag/v0.1.0-rc.2)
——第二个公开发行候选版。它是一个**预发行版（prerelease）**：请使用下方
的直接下载链接或 Release 页面本身，**不要**使用 `/releases/latest`
（该链接只会指向正式稳定版本，不会列出本预发行版）。所有平台的
桌面应用与命令行工具打包版均内置了固定版本的 PDFium 库——**下载发行版
无需单独安装 PDFium。**（从源码运行则不同，详见下方"提供 PDFium"一节。）

| 平台 | 下载 | 人工运行验收 | 签名情况 |
|---|---|---|---|
| macOS（Apple Silicon / arm64） | [`.dmg`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-0.1.0-rc.2-macos-arm64.dmg)（桌面应用）· [CLI `.tar.gz`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-cli-0.1.0-rc.2-macos-arm64.tar.gz) | 已完成——主要验证平台 | ad-hoc 签名，**未**经 Developer ID 签名或公证（见下） |
| Windows x64 | [`.msi`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-0.1.0-rc.2-windows-x64.msi) 安装包 · [CLI `.zip`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-cli-0.1.0-rc.2-windows-x64.zip) | 尚未完成——仅为发行候选构建 | 未签名 |
| Linux x86_64 | [`.AppImage`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-0.1.0-rc.2-linux-x86_64.AppImage) · [`.deb`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-0.1.0-rc.2-linux-x86_64.deb) · [CLI `.tar.gz`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/mpdf-cli-0.1.0-rc.2-linux-x86_64.tar.gz) | 尚未完成——仅为发行候选构建 | 不适用 |

[Release 页面](https://github.com/Museion-Project/museion-binarize/releases/tag/v0.1.0-rc.2)
还提供 [`SHA256SUMS`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/SHA256SUMS)
与 [`release-manifest.json`](https://github.com/Museion-Project/museion-binarize/releases/download/v0.1.0-rc.2/release-manifest.json)
用于校验上表中的每一个文件，以及表格中列出的全部 CLI 压缩包。

**macOS Gatekeeper**：桌面应用采用完整、有效的 ad-hoc 签名（并非
Developer ID 证书签名，也未经公证），因此首次启动时会出现 macOS 标准的
"无法验证开发者"提示。请右键（或按住 Control 点击）应用图标，选择
"打开"并确认——这是未签名/ad-hoc 签名发行版本在每台机器上首次运行时的
正常、预期流程。**请勿**为此关闭系统级 Gatekeeper（`sudo spctl
--master-disable`）——这会关闭整台机器的一项真实安全防护，而非仅针对
本应用，且从来没有必要这样做。

**Windows SmartScreen**：安装包未签名，Windows 可能相应给出提示，本项目
不对其可信度或信誉做任何声明。

签名/公证的技术细节详见 [`docs/releasing.md`](docs/releasing.md)；各
平台的完整验证状态记录详见
[`docs/desktop-testing.md`](docs/desktop-testing.md)——"已构建打包"与
"已完成人工运行验收"并非同一件事，本仓库不会将二者混为一谈。

## 研究方向

在后续、以基准测试为驱动的研究阶段，项目将评估能够更好地保留**多音调古希腊语
（polytonic Ancient Greek）**、**校勘说明（critical apparatuses）**以及其他
容易被激进二值化破坏的细小印刷细节的方法。目前项目**不**声称 M PDF 处理器能够
保留此类排版细节——这一结论只有在可复现的基准测试数据出现之后
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

## 分发

M PDF 处理器 的源代码保持开源（MIT OR Apache-2.0），GitHub 构建版本
保持功能完整；未来计划推出的付费 Mac App Store 版本是一种便利性的
分发渠道，而非另立的闭源功能层级——该路径的技术沙盒就绪工作已完成
（详见 [`docs/mac-app-store-readiness.md`](docs/mac-app-store-readiness.md)），
但尚未向 Apple 提交任何内容，也不存在 App Store 商品页面。完整分发模型
详见 [`docs/distribution.md`](docs/distribution.md)。打包版本自
`v0.1.0-rc.1` 起发布于 GitHub Releases——详见上方"下载"一节。也可以
从源码自行构建打包版本，详见 [`docs/releasing.md`](docs/releasing.md)。

## 开源与支持项目

M PDF 处理器 是免费且开源的软件，GitHub 提供的官方版本功能完整并
免费提供——不存在任何为付费版本保留的功能。

如果 M PDF 处理器 对您有所帮助，欢迎通过
[GitHub Sponsors](https://github.com/sponsors/pei-haoran) 支持项目的持续开发。

未来也计划提供付费的 Mac App Store 版本。它主要作为更方便的安装、
更新渠道，并用于支持项目的持续开发，而不是通过功能限制来替代 GitHub
免费版。目前尚未设定价格，也尚未向 App Store 提交任何内容（详见
[`docs/distribution.md`](docs/distribution.md)）。

## 隐私

M PDF 处理器 的设计目标是完全在您自己的设备上处理文件。核心处理流程不会
将扫描件、页面图像或输出文件上传到任何网络服务。桌面应用与命令行工具均仅处理
您本地选择的文件。

## 仓库架构

```
mpdf/
├── crates/
│   ├── mpdf-core/   # 不依赖 Tauri 的 Rust 处理核心
│   └── mpdf-cli/    # 基于核心库构建的命令行界面
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
cargo run -p mpdf-cli -- --help
```

### 提供 PDFium

本节仅适用于**从源码运行**的情况。如果您下载的是打包发行版（见上方
"下载"一节），PDFium 已内置其中——可跳过本节。

PDF 渲染需要 PDFium 动态库。从源码构建时，本项目不捆绑该库、不将其提交到仓库，也绝不在运行时下载——需要您自行提供一次。详见 [docs/pdfium.md](docs/pdfium.md)。

```bash
export MPDF_PDFIUM_LIBRARY=/path/to/libpdfium.dylib
```

### 命令行用法

```bash
# 检查文档：页数、页面几何、旋转、各 DPI 下的渲染尺寸
mpdf inspect input.pdf

# 通过真实处理流程测量文档，不写出转换后的 PDF——适合在正式转换前挑选参数
mpdf analyze input.pdf --dpi 300 --method otsu --json --pretty

# 通过真实处理流程对少量抽样页面进行处理，推算出实验性的输出体积预估，
# 同样不写出转换后的 PDF
mpdf estimate input.pdf --dpi 400 --method sauvola --samples 8

# 转换为双色（bilevel）CCITT Group 4 PDF
mpdf process input.pdf --output output.pdf \
  --method sauvola --dpi 400 --validate render-all

# 将处理后的某一页保存为 PNG 预览（页码从 1 开始）
mpdf preview input.pdf --page 12 --output preview.png

# 基于像素级标准答案，对二值化保真度进行基准测试
# （光栅级基准测试无需 PDF/PDFium）
mpdf benchmark run \
  --dataset test-data/benchmark/synthetic-v1/dataset.toml \
  --profile test-data/benchmark/profiles/baseline.toml
```

常用选项：`--method otsu|sauvola|manual`、`--threshold`、`--sauvola-k`、`--sauvola-window`、`--contrast`、`--median-denoise`、`--background-normalization`、`--despeckle off|conservative|strong`、`--overwrite`、`--pdfium-library`、`--pages`（仅 `analyze` 支持，例如 `1,3,8-12`）、`--json`/`--pretty`/`--quiet`/`--report`。进度信息输出到 stderr；最终结果（人类可读或 `--json`）输出到 stdout。完整命令、退出码与 stdout/stderr 约定见 [`docs/cli.md`](docs/cli.md)，JSON 报告结构见 [`docs/reporting.md`](docs/reporting.md)。

源文件绝不会被修改；只有在生成并通过校验的完整文档就绪后，才会写入目标路径。

### 运行桌面应用

```bash
pnpm install
pnpm --filter mpdf-desktop tauri dev
```

## 贡献

欢迎参与贡献。在提交 Pull Request 前，请阅读
[`CONTRIBUTING.md`](CONTRIBUTING.md)——其中说明了代码格式、测试要求，以及
关于测试素材来源和未经证实结论的相关规则。如需报告安全漏洞，请参阅
[`SECURITY.md`](SECURITY.md)。

## 引用

如果您使用本软件，请依据 [`CITATION.cff`](CITATION.cff) 中的元数据进行引用。

## 作者与维护者

M PDF 处理器 由 **Pei Haoran（裴浩然）** 在 **Museion Project** 组织下
创建并维护。详见 [`AUTHORS.md`](AUTHORS.md)。

## 许可证

本项目采用双重许可，您可任选其一：

- MIT License（[`LICENSE-MIT`](LICENSE-MIT)）
- Apache License, Version 2.0（[`LICENSE-APACHE`](LICENSE-APACHE)）
