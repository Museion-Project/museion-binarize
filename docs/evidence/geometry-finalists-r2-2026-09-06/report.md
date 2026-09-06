# Paddle / Surya geometry adapter 决赛 · 第2轮

**Winner：Surya。** 主指标差距只有约 **0.014 个百分点**（96.9455% 对 96.9316%），单凭 F1 仍不能称作明显胜出；但本轮 Surya 的顺序、跨栏关系和灾难性遗漏表现更好，因此是更合理的下一阶段预选。Paddle 在 stricter IoU 和 apparatus 上仍有优势。本轮没有冻结 provider 或接入生产路径。

本次按 `intent.md` §8.4 实现并评测两家 provider-specific adapter，Tesseract 不参赛。输入始终是原来的25页、1,378条参考行；原始标注未修改。每家真实本地模型推理50次，分别对两遍独立模型输出完成最终normalization，没有从一次模型输出复制出“两遍”。

[逐页可视化与fragment溯源](viewer.html) · [全部指标JSON](results.json) · [逐页CSV](per-page.csv) · [运行前协议](protocol.md)

## 总体结果

| 指标 | Paddle | Surya |
|---|---:|---:|
| micro-F1（logical envelope，IoU≥0.30） | 96.93% | 96.95% |
| stricter IoU≥0.50 F1 | 94.79% | 94.18% |
| 逐页等权 macro-F1 | 96.93% | 96.91% |
| 行召回率 | 95.14% | 96.73% |
| 阅读顺序（已匹配行对） | 95.85% | 99.93% |
| 可比较行对覆盖率 | 90.71% | 94.26% |
| fragment并集覆盖：全部参考行 | 87.48% | 86.34% |
| 全部匹配行的逆序对 | 1620 | 27 |
| 灾难性遗漏页数（本轮诊断定义） | 1 | 0 |
| 原始输出/完整geometry两遍相同 | 25/25 · 25/25 | 25/25 · 25/25 |

一次匹配现在对应一条预测logical line；该logical line引用一个或多个保留的fragments。主评分的一对一关系发生在logical lines之间，而不是要求每个detector fragment单独覆盖完整reference。参考行、IoU门槛及匹配规则没有改；这是用户明确要求的评分单位修订。旧轮原始框F1仍作为对照：Paddle 95.18%，Surya 95.08%。原始API返回框顺序不作为阅读顺序基线。

外接框分数与并集支持必须一起看：Paddle的TOC logical envelopes全部匹配，但原始fragments并集仅覆盖对应参考框的45.41%；点线几何cue被保留，未把未检测到的完整leader区域自动算作fragment支持。这里的“并集覆盖”仍是矩形面积指标，不是逐字符墨迹准确率。

## 列检测与阅读顺序

| 列与顺序诊断 | Paddle | Surya |
|---|---:|---:|
| 同栏/异栏关系准确率（已匹配行） | 91.46% | 99.82% |
| 该列分区指标的参考行对覆盖率 | 93.66% | 95.71% |
| within-column order | 99.97% | 99.97% |
| cross-column order | 86.02% | 100.00% |
| 栏内逆序对 | 8 | 8 |
| 跨栏逆序对 | 1440 | 0 |
| 空间track数量一致页数 /25 | 22 | 25 |
| 已匹配行的分区完全一致页数 /25 | 21 | 24 |
| 参考行全部匹配且分区完全一致 /25 | 14 | 14 |

列分区评测覆盖1,292条非furniture、非margin参考行。真值分区由现有dev的leaf/column字段整理为空间tracks（普通正文和同列注释属于同一track，局部左右栏单列）；它是可检查的评测侧分区，**并非另行人工验收过的D Gold**。某些结构标签仍为candidate-unverified，因此上述准确率以这份分区为条件，不能宣称独立列Gold认证。完整分母、分组和source见[evaluation-partition.json](evaluation-partition.json)。adapter从未读取这份分区、参考文字或类别。

“列关系准确率”是匹配行对是否同栏的判断正确率，已明确报告覆盖；“完整分区”还要求所有相关参考行均匹配。局部双栏上下的单栏正文可构成第三个空间track，不能把它误说成页面全高有三栏。

### Ueberweg 143/145

| 页面 | 候选 | F1 | 整页逆序对 | 栏内 / 跨栏逆序对 | 列关系准确率 | 列指标行对覆盖 |
|---|---|---:|---:|---:|---:|---:|
| 143 | paddle | 100.00% | 1 | 0 / 0 | 100.00% | 100.00% |
| 143 | surya | 89.20% | 1 | 0 / 0 | 100.00% | 86.42% |
| 145 | paddle | 99.10% | 1 | 0 / 0 | 100.00% | 98.17% |
| 145 | surya | 98.20% | 1 | 0 / 0 | 100.00% | 96.35% |

两家均恢复了已匹配正文的先左栏、后右栏次序；两页整页各剩1个逆序对，正文栏内/跨栏均为0。Paddle仍有更好的检测完整性，尤其143页：Paddle全部100条列评测行匹配，Surya为93条，故不能把Surya的100%列关系分数理解成整页全部正确。

## 三类专项

| 专项 | 参考行 | Paddle：raw→logical 匹配 | Surya：raw→logical 匹配 | Paddle / Surya fragment并集覆盖 |
|---|---:|---:|---:|---:|
| TOC | 36 | 20 → 36 | 36 → 36 | 45.41% / 89.79% |
| apparatus | 27 | 24 → 27 | 20 → 21 | 86.24% / 68.28% |
| marginalia | 35 | 13 → 13 | 6 → 25 | 30.80% / 59.57% |

TOC共有36条参考行。Paddle有32条以多fragment logical line实现，Surya为30条。残余fragmentation诊断为Paddle 1条、Surya 0条；Paddle另有1条logical envelope覆盖其他参考行的风险。Surya的TOC分组及子框实际覆盖较稳。两家均保留title/点线/右端短框之间的几何关联，未写入最终TOC membership。

apparatus共有27条参考行。Paddle从24个匹配提升到27，10条以多fragment实现；Surya从20提升到21，仅2条为多fragment。Paddle仍有2条残余fragmentation记录，Surya有1条；Surya还存在1条覆盖其他参考行的风险。**Paddle在本专项更好，Surya仍漏6条**。这类多框组成只建立逻辑行关系，原始小框不会被删除或粗化。

marginalia共有35条参考行。本轮Paddle产生9次拆分，Surya产生24次（含全部dev上的候选拆分，不等于真实边码命中数）。Surya边码匹配由6增至25；Paddle仍为13，拆分实现的净收益为0，尚未解决22条未匹配边码。两家各有1条边码匹配的跨参考行覆盖风险。重复margin lane + whitespace valley能够工作，但也会把其他缩进或小符号视作候选；这些hint与provenance均保留，不能把它们当作已经确认的语义角色。

专项类别取自已有dev字段，只用于诊断分组；参考tag尚未等同D验收。逐页未匹配line ID、残余fragmentation和交叉覆盖的原始列表保存在results.json中。

## 灾难性遗漏与回归

本轮预先定义的诊断触发条件：连续至少5条参考行未匹配；或至少5行的页面/参考空间track召回低于80%。这不是额外制造的产品验收阈值。

**Surya：0页触发。Paddle：Brisson PDF030触发1页，连续5条参考行未匹配。** 受影响ID：candidate-0047、human-0096、candidate-0048、candidate-0049、candidate-0050（完整前缀为brisson-le-meme-pdf030-）。

该回归发生在logical grouping层，原始框并未丢失。Paddle原始96框可匹配95条参考行，normalization变成83条logical lines，仅80条匹配。其layout包含分段的正文/注释区域，本轮adapter把两对局部区域各自当成独立column band，未拼成贯穿整叶的两条空间track；band间未分配片段进入single-track后又发生跨叶分组。Paddle在Brisson030/110/170分别产生414/440/740个逆序对，三页占总体绝大部分逆序。

这定位了Paddle adapter的第一处可观察偏离：原始区域/框仍完整，局部region-pair选择与whole-leaf hierarchy重建不足。**不能归因成Paddle detector本身漏掉这些文字。** Surya基于持续gutter推断，在这些spread上得到完整两列，因此避免了这次分组回归。

[Brisson030回归图](figures/brisson-le-meme-pdf030.png) · [TOC图](figures/brisson-le-meme-pdf013.png) · [Burnet边码/apparatus图](figures/burnet-platonis-opera-pdf1100.png) · [Ueberweg145图](figures/ueberweg-kraemer-pdf145.png)

## D-ready信息保留与共同schema

共同输出为 `museion-geometry-evidence/1`，定义在[JSON schema](../../../schemas/museion-geometry-evidence-1.schema.json)。这是实验几何证据schema，不改Rust生产GeometryPage/compositor。

- `provider_raw`完整保留本次API实际输出，包括boxes/polygons/confidence、Paddle layout区域和原生labels；原生labels只存档，不提升为Museion最终role。
- `source_fragments`保存每个原始detector box和polygon；`fragments`保存细粒度派生子框、源ID、source pointer、拆分证据、source polygon与几何特征。原框和split子框之间可逆追溯。
- Paddle保留所有layout regions及几何containment边，以受支持的区域对优先建立columns；Surya根据相对x分布、crossing-line barriers、持续y支持与空白带建立adaptive column bands。区域证据、column bounds、support IDs、gutter和选择来源都在输出中。
- logical lines仅引用fragment IDs，并保存row alignment、dotted-row、small-dense-row、recurring-margin-lane等低层hint。没有生成heading level、role、semantic parent或最终TOC membership。
- 完整性验证要求每个源框都可恢复、每个最终fragment恰好属于一条logical line、布局region数量和源payload一致、原polygon/confidence不变、split子polygon位于子框内，且所有内容均纳入确定性摘要。

## 确定性、版本与证据边界

两家模型均沿用上一轮有效配置：PaddleOCR3.3.2 / PaddlePaddle3.2.2（PP-OCRv5_server_det + PP-DocLayout_plus-L）；Surya0.17.0 text_detection/2025_05_07、Transformers4.56.2、CPU float32。模型文件哈希逐次记录，未改检测阈值。

两遍真实模型输出均为25/25相同，最终schema内容（IDs、fragment关系、polygon、hierarchy、hints、provenance、digest）亦为25/25相同，各项评分完全一致。仅排除外部运行时间与attempt编号。未证明跨硬件/版本确定性。

评分前发现r2.1的split child继承了完整源polygon，已用r2.2裁剪子polygon并显式保存源polygon/features；没有改变框、分组、列、line ID或顺序。两遍独立模型输出分别经过最终normalization，先前输出也保留。过程见[修正记录](runtime-diagnostics/normalization-revision.md)。首次评分之后没有继续调参或重跑竞争轮次。

10项synthetic/metric测试通过。参考图像和记录摘要前后不变；两个provider的50次真实推理均无错误。全部原始输出、旧normalization、最终normalization、实现快照和逐次摘要留存。16页holdout图像/标注未读取、未修改，也未运行冻结评测。

## 全部25页结果

| 页面 | Paddle F1 | Surya F1 | Paddle / Surya IoU≥0.50 F1 | Paddle / Surya 逆序对 |
|---|---:|---:|---:|---:|
| brisson-le-meme-pdf013 | 98.63% | 100.00% | 98.63% / 100.00% | 0 / 0 |
| brisson-le-meme-pdf030 | 89.89% | 99.47% | 82.02% / 98.41% | 414 / 0 |
| brisson-le-meme-pdf110 | 98.77% | 100.00% | 96.30% / 100.00% | 440 / 1 |
| brisson-le-meme-pdf170 | 93.79% | 100.00% | 91.53% / 100.00% | 740 / 1 |
| burnet-platonis-opera-pdf0050 | 90.70% | 95.24% | 86.05% / 85.71% | 4 / 5 |
| burnet-platonis-opera-pdf0300 | 93.18% | 95.45% | 86.36% / 79.55% | 5 / 7 |
| burnet-platonis-opera-pdf1100 | 92.50% | 87.80% | 92.50% / 80.49% | 0 / 0 |
| burnet-platonis-opera-pdf1400 | 89.16% | 95.45% | 86.75% / 77.27% | 1 / 0 |
| dillon-phronesis29-pdf004 | 100.00% | 100.00% | 100.00% / 100.00% | 0 / 0 |
| dillon-phronesis29-pdf005 | 100.00% | 100.00% | 97.62% / 100.00% | 0 / 0 |
| dillon-phronesis29-pdf006 | 100.00% | 100.00% | 100.00% / 100.00% | 0 / 0 |
| menn-metaphysics-pdf288 | 100.00% | 100.00% | 100.00% / 100.00% | 0 / 0 |
| menn-metaphysics-pdf414 | 100.00% | 100.00% | 100.00% / 100.00% | 0 / 0 |
| menn-metaphysics-pdf577 | 100.00% | 100.00% | 100.00% / 100.00% | 0 / 0 |
| menn-metaphysics-pdf925 | 100.00% | 100.00% | 100.00% / 100.00% | 0 / 0 |
| primavesi-zwei-elementen-pdf011 | 96.55% | 96.70% | 96.55% / 96.70% | 0 / 2 |
| primavesi-zwei-elementen-pdf012 | 98.70% | 98.70% | 98.70% / 98.70% | 0 / 1 |
| primavesi-zwei-elementen-pdf013 | 97.22% | 97.22% | 97.22% / 97.22% | 0 / 0 |
| primavesi-zwei-elementen-pdf014 | 95.74% | 95.74% | 95.74% / 95.74% | 0 / 1 |
| primavesi-zwei-elementen-pdf015 | 96.15% | 96.15% | 94.23% / 96.15% | 3 / 3 |
| primavesi-zwei-elementen-pdf016 | 97.96% | 98.99% | 97.96% / 98.99% | 3 / 4 |
| primavesi-zwei-elementen-pdf079 | 96.39% | 97.62% | 93.98% / 97.62% | 8 / 0 |
| ueberweg-kraemer-pdf100 | 98.77% | 80.90% | 91.36% / 76.40% | 0 / 0 |
| ueberweg-kraemer-pdf143 | 100.00% | 89.20% | 96.08% / 82.63% | 1 / 1 |
| ueberweg-kraemer-pdf145 | 99.10% | 98.20% | 97.30% / 92.79% | 1 / 1 |

## 本轮结论与停止点

**Surya作为下一阶段geometry预选；Paddle保留为apparatus和strict-IoU对照。** Surya的F1领先极小，推荐主要由顺序、列关系、边码改进及未触发灾难性遗漏支持。它仍有45条参考logical line未匹配，apparatus和marginalia也未全覆盖，不能据此宣布完整OCR/geometry milestone或生产质量达标。

本轮状态：`GEOMETRY_FINALISTS_R2_COMPLETE`；与 `intent.md` §8.4 `aligned`。交付的是这一轮实现和有边界的比较报告，不是全部缺陷已修复。未冻结provider，未进入production wiring / production E2E，未生成D层最终语义。
