# 25 页 development geometry-provider bake-off

**本轮主指标 winner：PaddleOCR local detection/layout，优势很小。**
按运行前写定的 IoU ≥0.30 行框 micro-F1，Paddle 为 **95.18%**，Surya 为
**95.08%**，Tesseract PSM 3 为 **91.20%**。Paddle 与 Surya 都匹配了
1,324/1,378 条参考行；Paddle 仅少输出 3 个未匹配框。这个差距不足以支持
“Paddle 已全面优于 Surya”：更严格的 IoU 门槛和逐页等权平均都会改变排名。

三家均完成 25 页 × 2 遍，第二遍的有序坐标与第一遍逐页完全相同，无运行失败。
这次交付符合 `intent.md` §8.3 的几何比较范围。**不冻结 provider，不接生产路径，
不运行 Gemini、production E2E 或 16 页 frozen holdout。**

[逐页交互查看器](viewer.html) · [机器可读结果](results.json) ·
[测量协议](protocol.md) · [逐组/第二遍补充结果](supplemental-results.json)

## 实测结果

以下为第一遍；第二遍各项几何分数完全一致。顺序准确率仅针对成功匹配的行对，
并非所有参考行的顺序正确率。未匹配框既可能是噪声，也可能是一个参考行被拆成
多个框；“漏行”计数同样是未匹配参考行数，不等同于完全没有检测到任何文字。

| 指标 | Tesseract PSM 3 | Surya + XY-cut | Paddle 检测 + 布局排序 |
|---|---:|---:|---:|
| **行框 F1，IoU ≥0.30** | 91.20% | 95.08% | **95.18%** |
| Precision | **95.55%** | 94.10% | 94.30% |
| Recall | 87.23% | **96.08%** | **96.08%** |
| 未匹配参考行 | 176 | 54 | 54 |
| 未匹配候选框 | 56 | 83 | 80 |
| 匹配框 mean IoU | **0.9391** | 0.8386 | 0.7800 |
| 匹配行的平均参考框覆盖率 | **94.97%** | 89.12% | 92.64% |
| 全部参考行的平均框覆盖率，未匹配记 0 | 82.84% | 85.63% | **89.01%** |
| 匹配行对顺序准确率 | **99.38%** | 93.92% | 93.57% |
| 可比较行对 / 全部参考行对 | 80.24% | 93.85% | 95.19% |
| CPU 中位秒/页 | 7.13 | **5.25** | 5.37 |
| CPU P95 秒/页 | 11.32 | **6.71** | 6.95 |
| 两遍有序坐标完全一致 | 25/25 | 25/25 | 25/25 |

覆盖率使用每行参考矩形被其匹配候选矩形覆盖的面积比例，再按行平均；它不是
像素级墨迹覆盖、字符覆盖或页面面积加权覆盖。mean IoU 仅对已匹配行计算，
因此 Tesseract 的高贴合度不能补偿漏检。参考中有 503 个框与 Tesseract 输出
坐标完全重合，Surya/Paddle 为 0；这说明框边界并非独立盲测条件，不能仅凭
0.9391 宣告 Tesseract 对新页面的框定位更准确。本轮未改参考来消除这种差异。

耗时在同一 macOS arm64 主机上顺序执行候选，以 4 个主要 CPU 线程运行。
统计包含图像读取、模型预处理/推理/后处理及排序，不含模型初始化。Surya 初始化
2.52 秒，Paddle 初始化 6.61 秒；Tesseract 的进程和模型加载包含在每页耗时中。
没有做多次独立冷启动、GPU/MPS、跨机器或打包运行时测试。

## winner 的稳健性

| 不改变模型输出的诊断 | Tesseract | Surya | Paddle |
|---|---:|---:|---:|
| 主指标 micro-F1，IoU ≥0.30 | 91.20% | 95.08% | **95.18%** |
| 更严格 IoU ≥0.50 的 micro-F1 | 89.91% | **93.72%** | 92.38% |
| 各页等权平均 F1，IoU ≥0.30 | 90.01% | **94.90%** | 94.52% |

主排名保持为 **Paddle → Surya → Tesseract**，没有事后换指标选 winner。
但从上述敏感性可见，Paddle 只是本轮主指标的微弱胜者；Surya 的框贴合度和跨页
平均表现更好。建议下一阶段仍保留 **Paddle 与 Surya 两个预选**，Tesseract
保留为原生排序/既有集成对照；本报告不修改当前生产默认值或 ADR 0014。

| 来源分组 | 页数 / 参考行数 | Tesseract F1 | Surya F1 | Paddle F1 |
|---|---:|---:|---:|---:|
| Brisson | 4 / 304 | 80.69% | 95.30% | 89.44% |
| Burnet | 4 / 177 | 84.51% | 85.89% | 85.55% |
| Dillon | 3 / 126 | 100.00% | 100.00% | 100.00% |
| Menn | 4 / 202 | 91.33% | 100.00% | 99.75% |
| Primavesi | 7 / 316 | 94.72% | 98.88% | 98.23% |
| Ueberweg–Krämer | 3 / 253 | 98.03% | 90.00% | 99.41% |

分组样本很少，且来自同一部书的页面相关；这里是定位开发失败类型，不是独立
统计显著性或对未见历史材料的质量认证。

## 失败页与架构含义

**目录的点线与页码：Brisson PDF013。** 参考为 36 行。Surya 匹配全部 36 行，
但输出 66 框；Paddle 输出 71 框，仅 20 个达到一对一 IoU 门槛。图上能看到
两家将右侧页码独立检测，Paddle 还将若干短条目和正文词组拆开，且较少覆盖点线。
这不是简单的“没看到页码”，而是检测单元与参考行定义不一致。Tesseract 只输出
15 行，页面下半部分大面积漏检。[四列对照图](figures/brisson-le-meme-pdf013.png)

**双栏阅读顺序：Ueberweg PDF143 / PDF145。** Surya 在两页分别产生 989 / 1,444
个逆序行对；Paddle 为 1,164 / 1,430。它们贡献了两家绝大多数顺序错误。
Paddle 的 raw layout 已包含左右栏区域，但本轮采用“最小包含区域分组 + XY-cut”
的适配器，未保留完整的栏层级。在 PDF145，Surya 正文的横向空隙为 38 px，
低于适配器的 45 px 门槛；Paddle 为 41 px，低于 58.5 px 门槛，因此落入逐行
按 y 排序的分支。**这是可定位的适配器局限，不应直接称作 Surya/Paddle 模型
本身不懂两栏。** [带顺序号对照图](figures/ueberweg-kraemer-pdf145.png)

相同框换成公共 XY-cut 的诊断中，Tesseract 顺序从 99.38% 变成 99.99%，
Paddle 从 93.57% 变成 93.60%，Surya 不变。这里仅改变已保存框的排序，不是
重新推理，也未选作新的主候选。它说明本轮 Paddle 布局适配并未带来顺序收益，
也说明共同 orderer 仍需处理窄栏缝和区域层级。

**希腊正文、边码与 apparatus：Burnet PDF1100。** 43 条参考行中，Tesseract /
Surya / Paddle 分别匹配 34 / 33 / 37 行。图上可见边码被并入正文，以及底部
apparatus 被拆成短块或漏检；普通正文质量不能代表这些小字与边码已受保护。
[对照图](figures/burnet-platonis-opera-pdf1100.png)

**跨页扫描漏区：Brisson PDF170。** 91 条参考行中，Tesseract 只匹配 26 条，
主要集中在左叶上部；Surya 和 Paddle 均检测并匹配 91 行。这是大片区域遗漏，
不能用总体平均掩盖。[对照图](figures/brisson-le-meme-pdf170.png)

在 `deterministic geometry → geometry-bound Gemini transcription → strict compositor`
架构中，几何层决定可转录的行集合与顺序。Gemini 没有权限补行、拆合行或改序，
合成器验证一致性也不能恢复未提供的几何。因此，下一项最有价值的几何工作是：
保留布局栏层级、改进与行高脱钩的窄栏缝处理，以及对目录/边码/apparatus 明确行
合并与分离规则。先在现有 development 上做这一有界比较，再判断两家胜负。
这些是后续建议，本轮没有据此调参、重写参考或启动额外候选迭代。

## 实际候选与运行修正

| 候选 | 本地实测配置 | 本轮能力边界 |
|---|---|---|
| Tesseract | 5.5.3；OEM 1；PSM 3；固定 grc+eng traineddata；TSV level 4 | 执行内部识别以取得行框；文字丢弃，采用原生顺序 |
| Surya | 0.17.0；text_detection/2025_05_07；CPU float32；batch 1；Transformers 4.56.2 | 独立行检测器，未调用 recognition/layout VLM；顺序由几何适配器给出 |
| Paddle | PaddleOCR 3.3.2 / PaddlePaddle 3.2.2；PP-OCRv5_server_det + PP-DocLayout_plus-L；CPU、MKLDNN 关闭 | 检测框全部保留，布局仅辅助空间分组；未测语义 D |

Surya 提供独立行检测接口，与完整 OCR/布局能力分开；本轮使用固定 0.17.0，
不能把最新版本文档或其他后端的能力算入此次结果。
[Surya 0.17.0 官方说明](https://github.com/datalab-to/surya/blob/v0.17.0/README.md)
Paddle 的独立布局检测模块用于空间区域识别；本文的最终行顺序来自本轮适配器。
[Paddle 官方布局模块说明](https://www.paddleocr.ai/main/en/version3.x/module_usage/layout_detection.html)

Surya 首次使用预装 Transformers 5.8.0 时出现错误加载：276 个模型张量中
116 个不等于 checkpoint。该次部分运行被中止，原始输出保留且不计入排名。
隔离改用 Transformers 4.56.2 / huggingface-hub 0.35.3 / tokenizers 0.22.2 后，
276 个张量全部吻合，随后完成正式两遍。没有改模型、检测阈值、评分规则或
全局 Python 安装。[完整诊断](runtime-diagnostics/README.md)

## 验证、证据与停止点

25 页、1,378 行参考通过本轮输入核验；每家运行前后和报告生成前均核对了
development 图像与参考文件 SHA-256。隔离检查只使用已有 frozen manifest 的
页 ID、图像摘要、源 PDF 摘要＋页号元数据；本次 bake-off 没有读取或修改
16 页 holdout 图像/标注，也没有运行冻结评测。

6 项几何测量/排序单元检查通过，覆盖重复框惩罚、漏页分母、少量匹配时顺序为
null、双栏顺序及未归入布局区域的行保留。三家各 50 次有效推理全部成功；
同一评测进程中的二次完整运行逐页严格相同（Tesseract 每页启动独立子进程）。
可视化报告经本地 Chromium 实际渲染，
核对 25 页选择器、SVG 框和首张 Paddle 指标，并查看渲染截图；未做全部交互的
自动浏览器测试。四张失败图已人工式视觉检查，不是重新标注 Gold。

- 输入：[reference-manifest.json](reference-manifest.json)
- 实际模型版本/哈希/逐次输出索引：[Tesseract](tesseract-run.json)、[Surya](surya-run.json)、[Paddle](paddle-run.json)
- 原始坐标：`raw/{tesseract,surya,paddle}/`，每家 50 个页面 JSON
- 测量与配置：[protocol.md](protocol.md)、[environment-validation.json](environment-validation.json)、[surya-effective-settings.json](surya-effective-settings.json)
- 执行/复现说明：[geometry README](../../../scripts/ocr/geometry/README.md)

**状态：`GEOMETRY_DEV_BAKEOFF_COMPLETE`；意图对齐：`aligned`（`intent.md` §8.3）。**
本轮授权内的本地比较与报告已完成。完整 GeometryProvider 的稳定 line ID、digest、
生产绑定、Gemini 转录和合成路径仍未在此实验中验证；A/B/C/E 完整 milestone、D
语义及生产准备状态均不能据此记为通过。本轮终点是报告与微弱 winner，未创建
provider freeze，也未推进 production wiring / production E2E。
