[English](README.md) | 简体中文

# Museion Binarize

**Museion Binarize** 是一款开源、跨平台的应用程序，用于将扫描的学术书籍转换为
干净、紧凑的双色（bilevel）PDF 文件。

**当前状态：Phase 1 —— 早期开发阶段。** 目前尚未实现任何 PDF 处理功能。请参阅
[`docs/limitations.md`](docs/limitations.md) 了解本仓库当前能做什么、不能做什么。

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
