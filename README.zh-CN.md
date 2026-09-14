# Museion PDF 处理器

[English](README.md) · 中文

本地 PDF 黑白处理与可编辑目录书签工具。当前 Beta 面向 **Apple Silicon Mac**，提供中英文界面。

## 下载与安装

[下载 0.2.0-beta.2](https://github.com/Museion-Project/museion-binarize/releases/tag/v0.2.0-beta.2)。打开 DMG，将 **Museion PDF.app** 拖入“应用程序”。发布包使用 Developer ID 签名并经 Apple 公证；无需另装 Python 或 Homebrew。

## 使用

1. 打开 PDF，选择全部页或指定页进行黑白处理，检查预览。
2. 如需书签，指定目录页，生成后编辑标题、层级与目标页。
3. 核对目录并确认后，另存为新 PDF。原文件保留。

正文 OCR 暂未开放。目录页及页码边栏可由本机 Apple Vision 读取；不上传文档。Apple 图像层级建议需要 macOS 27 和可用的系统模型；不可用时保留基础层级，仍须人工核对。不会自动下载模型。

## Beta 限制

- 仅发布 macOS arm64；最低二进制目标为 macOS 13，实机验证在 macOS 27，尚未逐一验证旧系统。
- 黑白处理将选中页转为图像，移除这些页的文字层及交互批注，文件也可能变大。
- 书签生成不保证自动层级及目标页准确；保存前必须核对。
- 仅写书签时不支持加密或数字签名 PDF。
- 当前测试机原有 Gatekeeper 强制检查已关闭；保留独立 Apple 分发检查证据，尚无另一台干净 Mac 的下载启动验证。

[提交问题与反馈](https://github.com/Museion-Project/museion-binarize/issues)。请提供系统版本、操作步骤和去除敏感信息的示例。

## 开发与许可

[本地构建与发布](docs/macos-local-release.md) · [变更记录](CHANGELOG.md) · [第三方许可](THIRD_PARTY_LICENSES.md)。旧 RC 与实验管线的说明见 [历史文档](docs/legacy/README.zh-CN.md)，其中功能不代表本 Beta 已开放。

由 Museion Project 的 Pei Haoran（裴浩然）维护。源代码采用 MIT OR Apache-2.0 双重许可。
