# 书签语义约束：20 份 DeepSeek 检测

已完成全部 20 次授权调用（28 张目录图、439 个固定候选），无重试。保守费用折算 **1.553319 元 / 3 元上限**；这是 API usage 按既定峰值单价及 10 CNY/USD 记账系数的估算，并非账户账单。中位耗时 29.59 秒。

规则已加入实验模块：目录页标题（目录、Contents 等）、栏目标签和页面杂项不能成为书签或父节点；真实 Part/Book/Section 分组即使无页码也保留。依据上下文判断，不使用字串黑名单。原始文字、图像、几何、页码和历史参考不改，只生成派生书签投影。当前桌面产品仍是 Apple-only，本次未接入自动云端产品路径。

## 完成情况及限制

| 结果 | 份数 |
|---|---:|
| 严格协议通过并生成投影 | 9 |
| 8192 token 上限截断，结果正文为空 | 9 |
| 仅 layout_reason 超过 400 字符 | 2 |

两份长度失败的其余角色/父节点通过离线诊断校验，但没有补写投影或计入正式成功。高推理模式与本轮为控制 20 次最坏费用而降低的输出上限共同限制了可用率；不能据此断言 DeepSeek 不会判断语义。没有中途调参或追加付费调用。

## 原图核对

1. **Bonaventure 已解决已知伪根问题。** PDF 第 19 页的 CONTENTS 和 Chapter / Page 均正确排除，18 个真实条目的层级及父节点为 18/18。与上次云端把 CONTENTS 当父节点的结果相比，本轮修复；与原有本地基线相比则是维持 18/18，而非从零提高。原始 OCR 候选标题为 CONNS，模型使用图像角色判定。
2. **Amsterdam Circle 发生实质误删。** toc-98dafa3cd1ff5338 第 8 页显示它是第三章标题的续行；模型标为 page_furniture 后直接移除，未并回上一行，导致派生书签丢失标题内容。它不是独立导航条目，但也不是可丢弃的杂项。旧参考没有匹配该碎片，因此标准匹配评分看不见这个错误。
3. **SoLMSEN 也被误归为页面杂项。** toc-391b3958fcefcb13 第 9 页显示它是前一篇条目的作者续行。去掉独立伪条目有合理性，但未合并就删除会丢失源条目内容；应记录为续行归属/内容保留缺口。原始证据仍在，未被覆盖。
4. **真实分组没有被当作目录标题删除。** 同书第 8–9 页 SECTION I–VII 得到保留，文章挂在相应 SECTION 下。Physics 第 3 页 BOOK I–VIII 也正确挂在 PHYSICS 下，与原图缩进吻合。

本次逐图审查覆盖全部四项删除，以及旧参考计为层级退步的两份文档；不是全部 28 张图的独立人工 Gold 审核。

## 评分应如何解读

严格通过的 9 份，共同匹配分母为 133 个条目。旧参考下，层级与父节点均为 **110/133（82.71%）→108/133（81.20%）**：22 个由错变对、24 个由对变错、1 个仍错。删除不缩小原匹配分母；失败的 11 份没有用基线补作模型成功。

**这不是可信的“模型整体退步”结论。** 24 个所谓退步全部来自上述 SECTION 分组（16 个）和 PHYSICS→BOOK 分组（8 个）；原图支持模型的分组，旧参考把这些结构设成平级。旧参考因此存在可定位的层级缺陷；保留正式分数，不改 Gold、不另报一个事后修正的高分。先前 Spinoza 参考全平级争议也仍未解决。

预先声明的检测信号为 NO_GO_OR_INCOMPLETE：20 份可用的条件未满足，且原图审查发现续行内容丢失。检测本身完成；自动书签质量及产品接入仍未通过。下一版应明确区分“续行需归并”和“真正页面杂项”，并对需要归并但协议禁止改标题的样本弃权；同时调整推理/输出预算与非关键说明字段约束。这些是后续建议，本轮冻结结果未重跑。

## 逐份记录

| 文档 | 状态 | 旧参考层级：基线→本轮 |
|---|---|---|
| Lloyd P. Gerson. Aristotle and Other Platonists.pdf (`toc-0eaa6eb9a3fe17c5`) | failed | 无模型分数 |
| Spinoza_ A Guide for the Perplexed (Guides for the Perplexed) - Charles Jarrett.pdf (`toc-753b4ec16d0660ac`) | failed | 无模型分数 |
| (2007) Raffaele Peluso - Heidegger interprete del Sofista [FedOA thesis].pdf (`toc-59f94f410be1d1ac`) | applied | 35.00% → 100.00% |
| Theophrastus of Eresus- Sources for his Life, Writings, Thought and Influence. Commentary, Volume 2- Logic (Texts 68-136).pdf (`toc-0dc0ffe13a609af6`) | failed | 无模型分数 |
| Spinoza's Radical Cartesian Mind - Tammy Nyden-Bullock.pdf (`toc-98dafa3cd1ff5338`) | applied | 100.00% → 100.00% |
| [9783050055398 - Die benediktinische Klosterreform im 15. Jahrhundert] Die benediktinische Klosterreform im 15. Jahrhundert.pdf (`toc-e325ad25f4faa3f1`) | applied | 100.00% → 100.00% |
| Interpreting Spinoza - Charlie Huenemann.pdf (`toc-8fb23ef1ddf5f9c0`) | failed | 无模型分数 |
| Philosophy of St. Bonaventure, The - Étienne Gilson.pdf (`toc-847a87593090cae5`) | applied | 100.00% → 100.00% |
| Descartes on Causation - Tad M. Schmaltz.pdf (`toc-b1b08847839aad76`) | failed | 无模型分数 |
| Spinoza's Ethics A Collective Commentary - Ursula Renz & Robert Schnepf & Michael Hampe.pdf (`toc-7685b49228910629`) | failed | 无模型分数 |
| Trans. Robin Jackson, Kimon Lycos and Harold Tarrant. Olympiodorus. Commentary on Plato’s Gorgias.pdf (`toc-297d0fa842ffe69e`) | failed | 无模型分数 |
| Philosophie au Moyen Âge, La - Étienne Gilson.pdf (`toc-6bfc00896b2afb6e`) | failed | 无模型分数 |
| F. W. J. Schelling. Timaeus(1794).pdf (`toc-64b5ae719214a3cd`) | failed | 无模型分数 |
| [9783110299076 - Das Buch Henoch] Das Buch Henoch.pdf (`toc-849c7ae23cd9cabe`) | failed | 无模型分数 |
| Pierre Duhem. To save the phenomena. An essay on the idea of physical.pdf (`toc-4143daaac170eeec`) | applied | 91.67% → 100.00% |
| Ed. Düring and Owen. Aristotle and Plato in the Mid-Fourth Century. Aristotelicum Symposium.pdf (`toc-391b3958fcefcb13`) | applied | 100.00% → 15.79% |
| Pierre Bayle's Cartesian Metaphysics - Todd Ryan.pdf (`toc-bc7447817172f53c`) | applied | 92.31% → 100.00% |
| Ed. Yury Arzhanov. Porphyry in Syriac.pdf (`toc-530ae9e4e807ebde`) | failed | 无模型分数 |
| 1.07_physics.pdf (`toc-a3f7480a42c9d938`) | applied | 100.00% → 33.33% |
| Saint Augustine. De Trinitate. (Bücher VIII-XI, XIV-XV, Anhang Buch V). Lateinisch-Deutsch.pdf (`toc-92372e2d6f33fcb0`) | applied | 33.33% → 91.67% |

## 对齐、授权与验证

- 基线 intent.md §8.43 SB1–SB3；约束实施与 20 次检测执行 aligned，结果层面的语义保留存在 drifted（上述续行误删），全体语义正确性及 PDF 跳转行为 unverified。
- 操作授权已满足：20 次既定 DeepSeek 外发、3 元上限内；无重试、无新 holdout、无发布、无产品云端接入。
- 新语义模块 5 项测试通过，原层级契约 8 项测试通过；冻结请求、图像、来源证据、评分器与代码哈希复核通过。 retained 字段不变及删除/保留集合完整性检查通过。
- 证据：plan.json / seal.json、cloud/ledger.json、evaluation.json、failure-diagnostics.json、samples/ 原图和冻结输入；逐次原始响应与投影保存在 cloud/ 对应文档下。
- 仅验证目录图上的角色和结构投影，未验证 PDF 实际跳转目标、最终 PDF 输出或独立人工验收。
