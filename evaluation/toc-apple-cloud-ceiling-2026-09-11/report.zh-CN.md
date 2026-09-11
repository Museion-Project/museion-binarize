# Apple 接口精简与云端目录层级测试

2026-09-11。意图基线：`intent.md` §8.42 AC1–AC3。产品范围与获授权的一轮实验 **aligned**；质量验收 **未通过**。

## 已交付应用

应用：`target/release/bundle/macos/Museion Binarize Apple.app`。只保留 Apple 的主动可用性检测和原图层级建议接口。MiniCPM / 本地 Qwen 选择器、下载命令、模型目录配置和 llama.cpp 启动代码均已撤下。旧实验与已下载的约 6.42 GB 权重保留，无当前产品入口；没有继续修复或运行它们。

Apple 不可用时明确保留基础目录，不修改文本、条目归属/顺序或页码目标。保存仍须人工核对。云端实验没有接入 GUI 或后台自动回退。

## 固定一轮云端实验

使用项目已有 DeepSeek 凭据，用户另明确回复“授权外发”。8 份旧开发目录、11 张原图、176 个固定恢复条目，各调用一次；不重试、不换样本、不进入 holdout。原图 JPEG 字节与旧实验一致，几何编号输入一致。开启 high thinking，输出上限 16384 tokens，使用 JSON object 模式及一个无关四条目格式例子。与本地量化模型的协议/推理设置不同，因此不是严格的单变量 provider 比较，也不是理论最大能力证明。

请求及返回型号均为 `deepseek-flash`。官方当前把它映射为 DeepSeek-V4.1-Flash，旧 `deepseek-v4-flash-vision-exp` 名称也转接新版；响应未提供更细版本 pin。见[官方型号与价格](https://api-docs.deepseek.com/quick_start/pricing/)、[视觉接口](https://api-docs.deepseek.com/guides/vision/)、[推理设置](https://api-docs.deepseek.com/guides/thinking_mode/)。

按官方峰值未缓存价格 $0.30/M 输入、$1.20/M 输出保守计价，并用 10 CNY/USD 作预算换算，8 次最坏预留 1.8366 元、硬上限 3 元；实际 usage 保守估计 **0.861018 元**。不是账户账单核验，未计算缓存或非高峰优惠。费用未知或外部请求异常会停止；本轮没有未知费用。

## 结果（旧冻结参考计分）

| 指标 | 结果 |
|---|---:|
| 请求完成 | 8/8 |
| 可应用的合法层级 | 7/8 |
| 七份配对匹配条目 | 157 |
| 层级准确率：基础 → 模型 | 52.23% → 80.25% |
| 直接父节点准确率：基础 → 模型 | 52.23% → 79.62% |
| 层级错误改正 / 原正确改错 | 62 / 18 |
| 父节点错误改正 / 原正确改错 | 61 / 18 |
| 请求耗时中位数 | 28.59 秒 |

七份可应用文档包含 161 个固定恢复条目，与参考成功匹配 157 个；完整八份为 176 个恢复条目、171 个参考匹配。失败文档没有用基础结果冒充模型结果，完整八份模型准确率留空。

| 文档 ID | 分组 | 基础层级 | 模型层级 | 状态 |
|---|---|---:|---:|---|
| toc-0eaa6eb9a3fe17c5 | imperfect | 31.11% | 97.78% | applied |
| toc-753b4ec16d0660ac | imperfect | 45.00% | 45.00% | applied |
| toc-59f94f410be1d1ac | imperfect | 35.00% | 100.00% | applied |
| toc-0dc0ffe13a609af6 | imperfect | 25.93% | 96.30% | applied |
| toc-98dafa3cd1ff5338 | control | 100.00% | 100.00% | applied |
| toc-e325ad25f4faa3f1 | control | 100.00% | 100.00% | applied |
| toc-8fb23ef1ddf5f9c0 | control | 100.00% | 未测得 | failed |
| toc-847a87593090cae5 | control | 100.00% | 0.00% | applied |

DeepSeek 明显改进了三份困难目录。Bonaventure 控制样本（847a…）却把印刷 CONTENTS 标题作为总父节点，把其他条目全部降一级：18 个匹配条目从全对变全错。另一份控制样本（8fb2…）耗尽 16384 个输出 tokens，finish_reason=length，最终 content 为空；没有重试。预先规定的“八份均可应用、实际指标提升、正确 controls 不退化”未满足。

**参考本身存在一处已发现争议。** Spinoza（753b…）的原图有三个粗体 Part 及各自编号章节，基础结果和模型都给出相应父子关系；旧 agent Gold 却全部标成顶层，导致两者均只得 45%。此处只是本次源图复核意见，不是独立人工 Gold。原 Gold 和原评分保持不动，详见 `reference-dispute.json`。因此上述绝对准确率必须称为“旧冻结参考分数”，不能称为已经确认的真实准确率。控制样本的全树降级错误与另一份无输出仍足以阻止当前自动接入。

**判断：更强视觉模型在这个任务上有实际潜力，但还不能自动覆盖基础目录。** 此次仅完成用户要求的一轮试验，保持 Apple-only 产品决定；不以新模型实验替代产品验收，也不自动追加调参或云端调用。

## 验证与边界

- 前端 61 项测试通过；TypeScript 与修改文件 ESLint 通过。Apple 接口/投影 9 项、原图层级合同 8 项通过；Rust cargo check --offline 通过。
- 本机优化应用构建成功，五个 Apple/合同资源与源码 hash 一致；包内不含旧模型 catalog、下载器或 llama 网络运行代码。
- 真实 release 应用通过 macOS 原生文件对话框打开固定 Spinoza PDF，目录入口只显示 Apple，主动检测 modelNotReady，生成明确保留基础层级。此原生文字路径产出 18 个导航条目，人工确认和保存按钮仍禁用；没有保存 PDF。当前 GUI 编译结果与旧 pilot 恢复条目不同，不混入模型计分。
- 较早 debug 自动注入测试未发出日志，未计为通过；最终使用真实 release UI 完成上述验证。没有据此宣称 Apple 可用时的推理路径已跑通。
- 旧原图、请求、参考、基础预测、评分器与原始关系合同的冻结 hash 在运行前后均核对一致。已存在的其他脏树工作未清理。未 commit/push、未发布、未触碰 PCC 或 holdout。

证据：`plan.json` / `seal.json`、8 个固定 payload、`cloud/*` 原始响应/usage/receipt、`evaluation.json`、`reference-dispute.json`、`native-ui-summary.json`、`bundle-verification.json`、`before/` 与验证日志。

## 固定外发范围

- toc-0eaa6eb9a3fe17c5：Lloyd P. Gerson. Aristotle and Other Platonists.pdf；PDF 页 8, 9。
- toc-753b4ec16d0660ac：Spinoza_ A Guide for the Perplexed (Guides for the Perplexed) - Charles Jarrett.pdf；PDF 页 8。
- toc-59f94f410be1d1ac：(2007) Raffaele Peluso - Heidegger interprete del Sofista [FedOA thesis].pdf；PDF 页 3。
- toc-0dc0ffe13a609af6：Theophrastus of Eresus- Sources for his Life, Writings, Thought and Influence. Commentary, Volume 2- Logic (Texts 68-136).pdf；PDF 页 8。
- toc-98dafa3cd1ff5338：Spinoza's Radical Cartesian Mind - Tammy Nyden-Bullock.pdf；PDF 页 8。
- toc-e325ad25f4faa3f1：[9783050055398 - Die benediktinische Klosterreform im 15. Jahrhundert] Die benediktinische Klosterreform im 15. Jahrhundert.pdf；PDF 页 5, 6。
- toc-8fb23ef1ddf5f9c0：Interpreting Spinoza - Charlie Huenemann.pdf；PDF 页 6, 7。
- toc-847a87593090cae5：Philosophy of St. Bonaventure, The - Étienne Gilson.pdf；PDF 页 19。
