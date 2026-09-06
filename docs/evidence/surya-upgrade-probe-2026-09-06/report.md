# Surya upgrade compatibility probe and provider freeze

状态：`SURYA_UPGRADE_PROBE_COMPLETE`；正式provider为 **Surya**。意图 `aligned`，依据 `intent.md` §8.6。当前状态先提交为 `2f73d03`，然后执行本次probe。

## 结论

**0.17.0与0.22.1在现有25页上完全一致。** 每边一次真实detector推理，共50页次；适配器、评分器、参考标注和规则未变。冻结 **Surya 0.17.0 + text_detection/2025_05_07 + r3.0-apparatus-repair**，保留已完成两遍确定性regression的基线。

0.22.1官方发布wheel的默认detector仍为 `s3://text_detection/2025_05_07`。因此本次不是新旧detector权重比赛，而是库及所需Transformers/Hub升级的本地detector路径兼容性比较。两边均逐一确认276个加载张量与checkpoint一致。新版使用官方`DetectionPredictor.local()`入口；其默认共享服务生命周期没有运行。

## 对比结果

| 比较层 | 完全一致页数 |
|---|---:|
| Raw bbox坐标和polygon，含顺序 | 25/25 |
| 完整raw payload，含confidence及其他字段 | 25/25 |
| 最终geometry字段，含fragments、columns、lanes、derivations、hints | 25/25 |
| Logical lines及reading order | 25/25 |
| 旧版本次复跑与已修复R3基线 | 25/25 |

最终geometry比较剔除版本不同所必然造成的顶层provenance/digest差异，比较字段明列在评分脚本与results.json；并非要求不同版本的整份JSON哈希相同。置信度没有差异。

| 指标 | 0.17.0 | 0.22.1 |
|---|---:|---:|
| Raw line F1 ≥0.30 | 95.0808% | 95.0808% |
| Logical line F1 ≥0.30 | 97.2757% | 97.2757% |
| Logical line F1 ≥0.50 | 94.5877% | 94.5877% |
| Raw boxes | 1407 | 1407 |
| Logical lines | 1375 | 1375 |
| Apparatus匹配 | 27/27 | 27/27 |
| TOC匹配 | 36/36 | 36/36 |
| Marginalia匹配 | 25/35 | 25/35 |
| 全局逆序对 | 27 | 27 |

所有逐页指标一致；双栏、栏内/跨栏顺序、TOC、apparatus、marginalia、正文无升级差异。50份geometry全部通过schema及原始box/fragment分区溯源校验。适配器SHA与checkpoint一致，原R2/R3证据未覆盖，无holdout资产访问或转写调用。旧版实际推理也重现了R3修复结果。

## 冻结决定与证据范围

正式选择Surya，取代ADR0014的临时Tesseract选择。升级没有提供几何收益，因此按probe前记录的决定规则保留0.17.0。精确model/config/adapter哈希、CPU float32/4 threads/batch 1和实际依赖版本见[provider-freeze.json](provider-freeze.json)，决策见[ADR0015](../../adr/0015-selected-surya-geometry.md)。

这是受控本地detector路径probe：新版隔离加载surya-ocr 0.22.1、Transformers 5.12.1、Hub 1.30.0，图像处理依赖与基线共用。共享Pillow 12.2.0超出新版声明的<11，cv2来自opencv-python 4.12.0.88而非其声明的headless 4.11.0.86，未安装此路径不需要的torchvision。故结果不宣称完整新版依赖闭包兼容；精确差异见[package-evidence.json](package-evidence.json)。没有修改包代码或adapter以补救输出。

冻结的是已验证开发配置，未替换已有Tesseract生产接线，未通过打包、真实消费路径、独立holdout、转写或语义D验收；这些路径状态为`unverified`。原有边注遗漏和Burnet0300残余apparatus碎片化仍在，不因provider选择而成为pass。

## 可复核工件

- `0.17.0/`、`0.22.1/`：各25页完整raw、geometry和运行清单。
- `results.json`、`pages.csv`：逐页、总体、分项指标和精确字段比较。
- `release-source/`、`package-evidence.json`：0.22.1发布包默认配置、loader/local API、metadata和来源SHA。
- `run_surya_upgrade_probe.py`、`score_surya_upgrade_probe.py`位于项目geometry脚本目录，完成输出有防覆盖保护。
- 两个脚本语法检查和`git diff --check`通过。适配器没有修改；不新增调参或测试轮次。
