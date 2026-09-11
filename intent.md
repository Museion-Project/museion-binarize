# Intent: M PDF 处理器的共享核心与双端产品

**状态：** Accepted，产品负责人已确认

**负责人：** Pei Haoran

**日期：** 2026-08-26

**范围：** `museion.me` 与独立软件产品线的品牌边界，以及本仓库从 Binarize 子项向电脑端
和手机端 PDF 产品的演进

本文按照 [The AI-Native SDLC playbook](https://claude.com/blog/the-ai-native-sdlc-playbook)
保存原始产品意图。它描述“为什么做、为谁做、什么算成功”，不规定具体实现。目标行为和
技术决策见 [`spec.md`](spec.md)，实施文件和验证步骤见 [`plan.md`](plan.md)。本 intent
已经确认；下游 spec 和 plan 必须先按本版本重新同步，不能直接据此前草稿实施。
当前已经实现的能力仍以 [`README.zh-CN.md`](README.zh-CN.md) 和
[`docs/limitations.md`](docs/limitations.md) 为准。

## 1. 问题

### 1.1 古典学文献处理工作流是割裂的

学者把纸书或扫描件转为可研究材料时，通常需要在多个互不相连的工具之间切换：手机
扫描、裁剪与矫正、PDF 二值化、OCR、目录书签、文本校订、词典和古代文本工作站。
中间结果往往只有一份视觉上可读的 PDF，没有可靠的文字坐标、版面结构、处理来源和
可复用的数据出口。

这种割裂会造成：

- 多次有损转换，原始页面与处理结果之间失去对应关系；
- OCR、页码、目录和书签彼此独立，错误难以追踪和校订；
- 通用工具对多音调古希腊文、小号脚注、apparatus 和校勘符号支持不足；
- 同一份材料很难继续进入 `museion.me` 或其他数字人文工作流；
- 学者依赖闭源云服务，却无法长期保存完整、可迁移的中间数据。

### 1.2 开源研究工具与可持续收入之间缺少清晰边界

纯开源研究工具有助于积累学术信誉、用户反馈和真实问题经验，但难以长期负担移动端
适配、云模型、服务器、支持和机构部署。反过来，如果把 OCR、数据格式或本地出口锁在
付费墙后，会损害这套软件作为开放研究基础设施的可信度。

这套软件需要一条长期稳定、用户可以预期的边界：

> 开放研究方法、数据格式和本地自主权；对移动体验、托管计算、协作管理和服务保障收费。

### 1.3 当前产品名称和范围不再匹配长期目标

当前旧版 Binarize 工具已经能把扫描 PDF 转为真正的 1-bit CCITT
Group 4 PDF，但
“Binarize”只描述一个已经完成的底层能力，不适合作为未来所有 PDF 产品的共同展示名。
它可以保留为共享开发组件和仓库子项；在它之上继续开发自动 Bookmark、含古希腊语 OCR
和机器可读中间产品。

电脑端和手机端应共用这套逐步开发的组件，但形成不同产品入口和使用重点：

- 电脑端暂称 **M PDF 处理器**，强调本地、完整和专业处理；
- 手机端暂称 **M PDF OCR**，保留更适合应用商店搜索的功能词，
  强调从扫描/导入到专业 PDF 的完整移动任务；
- 两端必须使用相同的文档模型和处理能力，不能演变为两套互不兼容的实现。

这里的 **M 只是可替换的品牌占位符，不表示 Museion，也不是已经确定的正式名称**。最终
名称确定后，应能在不改变功能名、文档格式、CLI/schema 兼容标识和用户数据的情况下集中
替换 M。

### 1.4 会写作和研究，不等于会配置技术工具

大量古典学、人文学和一般文本工作者的电脑技术接近零。他们不会因为 AI 可以辅助编程，
就突然具备自己搭建 OCR、PDF、命令行和 API 工作流的能力。为这类用户持续制作工具仍然
有直接价值。

这些工具必须在足够专业的同时保持简单、清楚：默认路径不要求理解阈值算法、OCR
引擎、模型部署、JSON schema 或命令行；高级能力可以存在，但不能成为完成基本任务的
前置知识。

## 2. 用户

### 2.1 核心用户

- **古典学研究者与学生：** 处理古希腊文、拉丁文、apparatus、脚注和历史版本，希望
  得到可搜索、可引用、带目录的 PDF。
- **低技术熟练度的文本工作者：** 能够判断文本与研究质量，但不会安装模型、配置环境、
  使用命令行或自己借助 AI 编写工具，需要清楚、可靠、无需技术背景的完整工作流。
- **数字人文学者与开发者：** 需要坐标化 OCR、版面结构、处理来源和开放格式，以便
  继续分析、校订、索引或接入其他工具。
- **图书馆、研究项目和编辑团队：** 需要可复现的批量数字化、质量评测、多人校订、
  权限和长期数据出口。

### 2.2 次级用户

- 只需要把扫描 PDF 清理、压缩和增加搜索文本层的一般 PDF 用户；
- 通过 `museion.me` 阅读古代文本、查阅 apparatus 和中文 LSJ 的现有用户；
- 希望嵌入开放处理核心或机器可读中间格式的第三方研究工具作者。

### 2.3 付费者

- 为完整手机扫描与专业 PDF 制作体验付费的个人用户；
- 通过电脑端或手机端购买 API、服务器算力、批处理、协作、部署和支持的机构；
- 为持续发生的可选云端增强或云模型成本付费的个人或机构用户。

付费者与研究工具使用者可能不是同一个人，因此开放数据出口和本地自主权不能依赖
机构订阅是否继续。

## 3. 目标

### 3.1 品牌与产品组合目标

向同一批古典学、人文学和文本工作者提供两组相互连接、但品牌清楚分离的产品：

1. **`museion.me`：** 集成 AI、apparatus、术语、语义单元和中文 LSJ 的古代文本与
   数字人文工作站；
2. **独立 PDF 软件产品线：** 不再使用 Museion 作为产品名称，以用户会直接搜索和理解的
   功能词命名：
   - **电脑端（占位名：M PDF Processor / M PDF 处理器）：** 免费开源、本地优先，提供完整
     PDF 处理能力，并可接入 API 与机构订阅；
   - **手机端（占位名：M PDF OCR）：** 付费应用封装，强调扫描/导入、
     OCR、书签和专业 PDF 导出的完整移动工作流，也可接入 API 与机构订阅。项目已经取得
     Apple Developer 资格，可以将 iOS 应用作为正式付费产品提交 App Store。

软件名称优先表达“它能做什么”，而不是表达“它属于哪个网站”。`museion.me` 可以在网站、
文档或 About 页面推荐这些软件，软件也可以导出到 `museion.me`，但双方不共享产品主名称。

**不存在独立 Institution 产品。** 机构能力作为电脑端和手机端都能使用的 API、托管
算力、批处理、协作、部署和支持层存在，不再建立第三个机构客户端或独立品牌。

### 3.2 PDF 产品目标

共享开发组件按以下阶段演进：

1. **Binarize（已完成）：** 确定性二值化、真正 1-bit PDF 与可复现基准；后续可将当前
   仓库或 crate 收为完整 PDF 工程中的子项；
2. **含古希腊语 OCR + 机器可读中间层（待分析与开发，同一阶段）：** 检测并评估已有
   PDF 文字层；需要时运行本地 OCR；保存文字、坐标、版面、引擎、模型版本、置信度和
   provenance；同时定义开放、版本化、可验证的文档中间层，连接两个 PDF 客户端、
   `museion.me` 和第三方数字人文工具，并生成可搜索 PDF；
3. **自动 Bookmark（待分析与开发）：** 在稳定的文本、坐标和中间层之上，完成印刷页码
   与 PDF 页序映射，从目录页、页码和正文标题证据生成可核验、可编辑的 PDF 书签。

两个未完成阶段都遵循同一研究顺序：

1. 先研究本地、开放、可复现的实现及其质量上限；
2. 建立真实材料基准，明确失败类型和默认行为；
3. 再研究 API 或云模型能补充什么、成本和数据边界是什么；
4. API 始终是可替换能力，不反过来定义开放核心的数据格式。

两端在共享组件之上形成不同侧重：

- **电脑端：** 免费开源、本地优先、适合长文档和专业控制；默认界面仍须简单，进阶设置
  按需展开；API 和机构订阅提供托管 OCR、批处理、协作和支持；
- **手机端：** 对原生产品体验收费，优先完成“夸克/系统扫描或文件导入 → 处理 → 专业
  PDF”的单一任务，减少参数和文件管理负担；API 和机构订阅可以提供云端加速与受管能力；
- **共享层：** 两端必须输出兼容的 PDF、OCR、书签和机器可读中间产品，并共享质量基准。

### 3.3 已确认的开发路径

```text
Binarize 基线与共享组件边界
        ↓
本地 OCR + 机器可读中间层 + 可搜索 PDF
        ↓
页码映射 + 证据化自动 Bookmark
        ↓
电脑版易用性、兼容性与发布加固
        ↓
手机端导入、处理与付费工作流
        ↓
内置扫描
        ↓
可选夸克或其他 API 增强
```

- OCR 阶段必须先检测 PDF 是否已有可靠文字层，避免无条件重新 OCR；
- 机器可读中间层与 OCR 同时设计和实现，不作为 OCR、书签完成后的补充导出；
- Bookmark 阶段必须处理 PDF 页序、印刷页码、罗马数字前言、缺页和插页等映射问题；
- OCR/AI provider 接口可以在本地阶段设计，但商业 API 的实际接入不阻塞电脑版发布；
- 电脑版完成真实用户测试和发布加固后，再封装手机端；手机端先导入，后内置扫描；
- 夸克 API 保持可选，只有在质量、合同、隐私和单位成本评测通过后才接入。

### 3.4 易用性目标

- 一个不理解 OCR 引擎、二值化算法或文档 schema 的用户，也能通过合理默认值完成任务；
- 基本流程使用任务语言，例如“让 PDF 可搜索”“自动生成目录书签”，而不是首先展示
  Sauvola、provider、坐标层级等实现术语；
- 高级用户仍能检查参数、证据、置信度和中间结果，简单不等于隐藏质量问题；
- 错误信息必须告诉用户发生了什么、文档是否安全、下一步可以做什么；
- CLI 和机器可读报告继续服务开发者与批处理，但普通用户不需要接触它们。

### 3.5 学术与信任目标

- 不把生成式恢复伪装成原始文献内容；
- 不在没有可复现基准的情况下宣传古希腊文保存或 OCR 准确率；
- 原图、处理结果、OCR、AI 修订和人工修订之间保持可追溯关系；
- AI 书签必须有可定位到页面和文本区域的证据；
- 用户始终能导出完整数据，不通过闭源格式制造锁定。

### 3.6 可持续目标

- 让开源研究工具持续吸收学者反馈和真实材料问题；
- 让移动端销售，以及双端提供的 API 与机构订阅承担产品化、算力和维护成本；
- 不让一次性买断承诺覆盖持续发生且不可控的第三方 API 成本；
- 不让软件的核心能力永久绑定夸克、Gemini 或任何单一供应商。

### 3.7 开源与 API 商业边界

预计采用以下开源与封装方式：**所有能够在用户设备上完成的本地功能及其共享核心保持
开源；托管 API 的服务端实现、商业 provider、任务编排、计费和运营系统不开源。封装后的
电脑端和手机端 App 可以包含 API 使用入口，但用户需要通过付费订阅获得服务额度，或购买
额外点数；本地开源功能不因未订阅而失效。**

API 客户端协议和调用入口即使存在于开源 App 中，也不得包含服务端或供应商密钥。消费端
iOS 应用中的数字订阅和点数购买必须按适用的 App Store In-App Purchase 规则实现；通过
IAP 单独购买的点数不得设置有效期。机构通过合同购买账号、额度和服务后从客户端登录使用，
必须符合届时适用的企业服务或多平台服务条款，不因此形成独立 Institution 产品。具体支付
实现以 [Apple App Review Guidelines 的 Payments 条款](https://developer.apple.com/app-store/review/guidelines/#payments)
和实际审核要求为准。

## 4. 非目标

- 不在第一版手机产品中完整复刻夸克扫描王的所有相机与图像恢复能力；
- 不把电脑端和手机端做成两套文档格式、两套 OCR 结果或两套不可复用的核心；
- 不建立独立 Institution 产品；
- 不把本项目发展为通用 Acrobat 替代品、通用 PDF 编辑器或全格式办公套件；
- 不保证保留源 PDF 的表单、签名、脚本、附件、批注、图层或所有交互对象；
- 不以“支持 20+ 语言”之类宽泛口号代替古希腊文专项评测；
- 不让视觉模型无证据地发明文字、章节、页码或缺失图像细节；
- 不在单机格式、质量和失败语义稳定前建设大规模机构服务器；
- 不在本仓库直接实现 `museion.me` 或中文 LSJ；本仓库只定义可供它们消费的
  文档接口与导出；
- 不因为移动端、API 或机构订阅收费而削弱已经承诺给开源桌面端的本地能力；
- 不以简单“可选择多个文件”冒充机构 batch 的价值。

## 5. 约束

- 当前 Rust 核心、CLI、Tauri 桌面端和 JSON 报告已有使用与发布兼容性，重命名必须分期；
- `M` 必须被视为集中管理、可一次替换的展示品牌变量，不得写进稳定文档格式、JSON schema、
  数据 ID 或不可迁移的用户目录结构；
- 电脑端和手机端必须复用同一组开放处理组件与文档模型，但可以有不同界面、默认值、
  发布方式和商业重点；
- 自动 Bookmark、古希腊语 OCR 和机器可读中间层都必须先完成本地方案研究，再决定 API
  的角色；
- 默认工作流必须服务没有命令行、模型配置或 API 知识的用户；专业性不得以复杂用法为代价；
- Apple Developer 账户已经取得，开发者注册不再是 iOS/App Store 发布的阻塞项；具体
  应用仍需完成签名、隐私披露、商店材料、审核和持续版本维护；
- 开源客户端不能依靠隐藏代码保护 API；所有商业密钥、配额裁决、计费和防滥用逻辑必须
  位于受控服务端；
- 当前处理路径是本地、无网络、可复现的；增加云能力不得改变默认隐私行为；
- 当前 PDF 输出会栅格化并丢弃隐藏文本层、书签和其他非图像对象；新增能力必须明确
  选择“重建”而不是暗示无损保留；
- PDF、图片、机器可读中间包和 OCR 文本均是不可信输入；解析、解包和 AI 调用必须有
  资源与权限边界；
- 基准材料必须具有可记录的版权或使用许可；
- 手机端受内存、电量、温度、后台执行和应用商店政策约束；
- 商业 OCR、夸克增强和视觉模型受价格、地区、许可、数据处理条款和服务可用性约束。

## 6. 成功指标

### 6.1 不可妥协的发布门

- **本地隐私：** 桌面本地模式和手机本地模式处理文档时产生 0 次业务网络请求；
- **可验证出口：** 所有正式生成的机器可读中间包均通过对应 schema 版本的验证器，且页面、坐标、
  来源和处理步骤引用完整；
- **无数据锁定：** 100% 用户生成的 OCR、书签、结构与处理记录可导出为开放格式；
- **证据化书签：** 评测集上的书签目标页准确率达到 99%，标题精确率达到 95%；无法
  找到可靠证据的条目必须标记为待确认，而不是静默生成；
- **古希腊文评测：** 在发布“古希腊文 OCR”质量声明前，至少建立 200 页、来源与许可
  清楚的真实古典学评测集；报告基础字符 CER、附加符号错误率和版面区域结果；
- **OCR 改进：** 选定默认管线在该评测集上的总体 CER 相对公开、可复现的基线至少改善
  15%，且不得以清晰现代版本的显著回退换取历史版本的平均改善；
- **二值化不回退：** 现有合成基准、PDF 结构验证、方向、极性和字节确定性测试继续通过；
- **安全：** 云密钥不进入客户端包、日志、机器可读中间包或错误报告；原始文档内容不进入遥测。

质量阈值在首次真实基准完成后可由产品负责人修订，但必须在相关发布开始实现前写回
本文和 `spec.md`，不能在看到结果后只挑有利指标。

### 6.2 用户结果指标

- 封闭测试样本中至少一半是自评电脑技术较低、不使用命令行的文本工作者；
- 至少 80% 的测试用户能在无人工指导下完成“导入/扫描 → OCR → 校对书签 → 导出
  可搜索 PDF”的核心任务；
- 用户不需要理解或选择 OCR provider、模型部署、二值化算法或中间层 schema，也能用默认
  路径得到通过验证的输出；
- 测试用户在 30 天内对第二份文档重复使用该工作流的比例达到 30%；
- 对需要人工校订的文档，工具能报告不确定页和证据位置，使平均人工查找时间低于逐页
  浏览原 PDF 的基线；具体改善幅度在可用性测试前记录；
- 第一项机构试点能处理至少 10,000 页，失败任务可恢复或给出可操作错误，且每页模型
  成本、处理引擎和数据保留状态可审计。

### 6.3 可持续性指标

- 本地功能的运营成本不随每处理一页线性增长；
- 任何按调用计费的云能力都有独立配额、成本可见性和可停止机制；
- 个人买断收入不补贴无限第三方 API 调用；
- API 订阅或点数售价必须覆盖模型、存储、带宽、支付渠道、失败重试和滥用风险；
- 机构收入通过电脑端、手机端或共同 API 对应服务器、协作、治理或支持等持续价值，而
  不是一个独立 Institution 产品、封闭格式或取消本地出口。

## 7. 产品负责人需要确认的意图

在 `spec.md` 获批并进入实施前，以下判断构成本 intent 的已确认边界：

1. Binarize 是已经完成的共享组件，可以收为完整 PDF 工程的子项；
2. 下一阶段同时开发含古希腊语 OCR 与机器可读中间层，并在此阶段完成已有文字层检测、
   坐标化输出和可搜索 PDF；
3. OCR 与中间层稳定后，再开发页码映射和证据化自动 Bookmark；
4. 两个未完成阶段都先研究本地方案，再研究 API 方案；商业 API 不阻塞电脑版发布；
5. `museion.me` 与 PDF 软件面向相近用户，但使用不同产品品牌；软件不再使用 Museion
   作为主名称；
6. 电脑端和手机端共用同一套开发组件与文档模型，但保留不同产品名称和使用侧重；
7. 电脑端暂称 **M PDF Processor / M PDF 处理器**，功能完整、免费开源、本地优先；
   手机端暂称 **M PDF OCR**，对完整移动体验收费；其中 M 只是与 Museion 无关、可集中
   替换的品牌占位符；
8. 不建立独立 Institution 产品；API 和机构订阅能力同时服务电脑端与手机端；
9. 低技术熟练度的文本工作者是核心用户；所有工具在专业的同时必须简单、清楚；
10. 机器可读中间产品是开放中轴，也是 PDF 工具连接 `museion.me` 的核心；
11. 第一版手机端优先接收夸克、文件 App 和相册导出，不以自研相机阻塞上线；
12. 夸克、Gemini 或其他 API 只作为可替换提供者，不是产品定义；
13. 本地功能与共享核心开源；API 服务端、商业 provider、计费和运营系统不开源；封装
    App 通过订阅额度或额外点数提供 API，但不以订阅锁住本地功能；
14. 任何古希腊文质量宣传必须晚于真实、可复现的专项基准。

## 8. 已批准修订：2026-09-05 OCR/geometry milestone

来源：产品负责人在本项目任务中以“补充当前 `museion-binarize` OCR/geometry
milestone 的验收契约”开头，并以“现在恢复此前任务”授权执行的消息。

本 milestone 使用已批准的 `deterministic GeometryProvider + Gemini transcription
+ synthesis` 架构，只判断当前模块是否足以停止优化并进入后续结构迁移与产品 E2E。
16 页 frozen Gold 是 holdout，不是开发数据，也不是本文件 §6.1 的 ≥200 页产品级
certification。该长期要求保持不变。当前 milestone 的用户批准验收条件记录在
[`plan.md` 的“2026-09-05 OCR/geometry 验收契约”](plan.md#2026-09-05-ocrgeometry-验收契约)。
此修订填补当前 milestone 的门槛缺口；不把旧 CGPG 原生坐标实验的阈值或干净原生
PDF 的实测成绩提升为当前产品阈值。指标操作定义中的未决问题必须显式保留，不得以
有利于通过的解释补齐。

### 8.1 D 的阶段归属修订（2026-09-05，用户已批准）

来源：产品负责人先要求“只做宏观结构设计”，随后以“允许做这一修正”批准将完整
D 语义验收移至后续结构阶段的建议。本次仅修正设计和验收归属，不启动实现或评测。

当前 OCR/geometry 阶段保留文字（包括脚注编号、上标字符、apparatus 符号）、
位置证据及阅读顺序，供后续结构管线消费；不得删除标记或制造未经验证的链接。
完整 D（角色分类、marker/definition 配对、跨页续注、外部目标及语义隔离）由后续
结构阶段验收，不再作为当前 OCR/geometry milestone 的晋级门。D 的 100% 硬不变量
要求保留，不将延期记为通过。当前 A/B/C/E 数值条件、3 轮上限和 holdout 保护不变。

职责分配：几何层提供空间证据；Gemini 可提出绑定行 ID／字符范围的局部语义候选；
结构管线解析跨行、跨页关系；确定性验证／合成层检查关系一致性并写出。Gemini 不
修改几何，也不直接裁定最终跨页链接。具体接口和实现留待相应阶段设计。
Gold 中的 D 信息保持原样；后续使用仍遵守 holdout 隔离，不转为开发或调参数据。

### 8.2 Dev exit 与测量契约锁定（2026-09-06，用户明确指令）

来源：产品负责人以“25 页非-holdout development reference 已完成人工核验。
现在继续执行 A–C OCR/geometry milestone。”开头的本任务消息，第 1–7 节。

25 页为主 development set；先核验 reference 和三重 holdout 隔离，不因产品失败
扩充标注，不补 D。只允许机械性 schema/consistency 修复，涉及人工内容或语义判断
必须停止该分支。测量采用 NFC＋既有 whitespace normalization；Greek/Latin
从同一次字符 alignment 计错；一对一行匹配 IoU ≥0.30；顺序准确率为正确比较对数
除以全部可比较对数。定义与既有阈值锁定于
`docs/evidence/ocr-geometry-dev-2026-09-06/measurement-contract.json`。

必须执行真实产品入口、provider binding 和前后处理路径。测试 transport、reference
注入或 benchmark-only 替代路径不算产品证据。若真实路径不存在或无法运行，报告
architecture/blocker。最多 3 次产品修复迭代，不得改 reference、holdout、指标或阈值。

两次完整 25 页开发回归达标、稳定性和 regression suite 通过、所有版本与摘要冻结后，
仅报告 `DEV_EXIT_READY` 并停止；须等用户另行确认才可运行 16 页 frozen holdout。
这取代 §8 原计划中 dev exit 后自动进入 frozen evaluation 的执行顺序。未满足则报告
`MILESTONE_BLOCKED`，定位产品修复，不要求重做大批 reference。

### 8.3 当前阶段改为 geometry-provider bake-off（2026-09-06）

来源：产品负责人澄清“产品接线留空是故意的”，因为分离架构新颖且实验性；明确要求
“现在需要的就是用25页 development reference 做 geometry-provider bake-off”。
本轮仅比较 geometry providers，必测 Tesseract PSM3 geometry、Surya local line
detection、PaddleOCR local detection/layout。交付 winner 与报告，不冻结 provider，
不进入 production wiring 或 production E2E，不接触16页 frozen holdout。

此明确指令取代 §8.2 对本轮要求真实生产路径和完整 A–C OCR 回归的执行范围。
生产路径未接通是当前实验阶段的有意安排，不能继续将其作为本次几何比较的阻塞。
先前关于历史规划遗漏的归因属于代理解释，已由本次用户澄清纠正。生产质量、OCR
字符准确率和 production E2E 均不能由此次 geometry bake-off 宣告通过。

### 8.4 Paddle / Surya adapter 决赛修订（2026-09-06）

来源：用户以“Tesseract 已经出局。Paddle和Surya进入决赛圈，调整后再比赛一轮”
开头的本任务消息，第1–4节。排除Tesseract，仅在现有25页比较两家，经两遍真实本地
运行报告 micro-F1、stricter IoU、逐页F1、灾难性遗漏、阅读顺序、TOC/apparatus/
marginalia专项，以及column detection、栏内/跨栏顺序、逆序对和Ueberweg143/145。

用户明确要求：apparatus logical line可对应多个保留的fragments；TOC同一row的
title/leader/page number归于logical line但保留子框。Paddle优先利用原始layout /
column hierarchy；Surya从line boxes的持续垂直空白、x分布和重叠自适应推断列。
边码拆分结合整页重复margin lane与框内whitespace valley / layout边界，不按具体
文字、页码或字母硬编码。两家输出共同Museion geometry schema。

D-ready信息保留是本轮方法约束：原始boxes、layout regions、空间column hierarchy、
fragment关系、低层margin/apparatus/TOC-row线索、geometry features和provenance
不得丢失。可增加低层结构hint，不生成heading level、role、parent或最终TOC membership。
此修订明确改变上一轮“一个detector box即一条评分行”的粒度；原始参考不改，原始
框评分作为对照，logical-line评分必须报告子框support，避免大外接框掩盖空白。
不冻结provider，不进入production wiring / E2E，不接触16页frozen holdout。

### 8.5 Surya-only bounded apparatus adapter repair（2026-09-06）

来源：本任务用户消息“你能针对性地再做一次Surya-only bounded adapter repair吗？
主要处理apparatus的问题。”本轮仅修Surya适配层，不调整Paddle或评分阈值。
要求：Burnet1100的0042–0045成为四条独立logical lines；Burnet1400的0043–0045
正确分行；无新增apparatus over-split；d0034类小框不再误吸入跨行大框；保留原始
detector boxes、split provenance和全部D-ready信息。随后Surya-only完整25页一次
或两次确定性regression，确认TOC、双栏、marginalia、普通正文没有退化。

执行决定：采用图像墨迹支持的纵向拆分与大小框合并限制，禁止页码/行ID/参考框硬编码。
本次adapter regression重放R2两次独立真实检测的完整raw payload，共25页×2；不重新
运行模型推理。该证据验证适配器路径，不代表新模型推理、语义D或生产验收。沿用§8.4
其余方法与隔离约束，不接触16页frozen holdout，不冻结provider。

### 8.6 Surya provider decision and bounded upgrade probe（2026-09-06）

来源：用户消息“先 commit 当前状态，然后将provider正式确定为Surya”，并要求freeze前
只在现有25页比较“Surya 0.17.0 + 2025_05_07 detector”与“Surya 0.22.1 + 当前默认
detector”，不调adapter、不改规则，比较raw boxes与最终geometry。

Provider选择正式确定为Surya，取代§8.4/8.5的未冻结provider状态和ADR0014的临时
Tesseract选择；detector/runtime的精确冻结在本次probe后记录。原有geometry绑定
Gemini转写和确定性合成的方法保持。Provider选择与生产接线、发布、语义D、独立holdout
验收分别记录，不因选择本身宣告这些路径通过。当前checkpoint为2f73d03。

§8.6执行决定（本轮probe后）：两版本各25页真实推理的完整raw和最终geometry逐页
完全一致；0.22.1默认detector仍为2025_05_07。正式冻结Surya 0.17.0 + 该detector +
r3.0-apparatus-repair，精确哈希见ADR0015所引provider-freeze.json。保留原稳定依赖
路径，无规则调整。该冻结限定开发配置，非生产接线/打包或独立质量验收完成声明。

### 8.7 本机 broker 接入与 Gemini 凭据保管（2026-09-06）

来源：用户消息“根据此provider，接上broker，并且找一个安全的地方，我来填入gemini api”；
随后明确选择“先接通本机 broker”。连接§8.6冻结的Surya到按行ID绑定Gemini转写和已有
Rust确定性合成器；保留原始几何与D-ready溯源。凭据由用户填入项目外私有文件，仅broker
读取；客户端只持有独立本机token。现有旧整页转写/fuzzy-alignment协议不能替代此路径。

本轮交付本机开发入口和真实本地Surya→HTTP broker→Rust compositor联通验证，Gemini端
采用模拟返回验证契约。用户填key不自动授权付费请求：本轮不调用付费API。启动后默认禁用
付费请求，需要用户显式开启并设请求上限；不部署公网、不启用生产Credits支付或改变发布门槛。

### 8.8 Production wiring 至 frozen blind ready（2026-09-06）

来源：用户以“从当前已冻结的 geometry 状态继续推进到 frozen blind ready”开头的
milestone 0–4 指令。先审计并再次 freeze commit；随后完成真实 CLI/Desktop shared
factory → 冻结 Surya → real broker/Gemini → strict compositor → searchable PDF，
以少量 non-holdout 页面证明 PRODUCTION_E2E_PROVEN，再执行既定两次完整25页 A–C。
只有全部 dev exit 条件成立才准备16页 blind 的冻结、ledger、目录和 provenance，最终
停在 FROZEN_BLIND_READY — WAITING_FOR_USER_AUTHORIZATION；不执行 blind。

不得重开 provider/version 比较或调整 geometry adapter；不得修改冻结 provider、adapter、
schema、measurement contract、reference、threshold 或 scorer。产品修复沿用3次 substantive
iteration 上限。任何前置 gate 不成立，停止在该 milestone；触及冻结 geometry 的修复
须由用户决定是否重开。本指令授权任务内真实 development 调用，不授权 holdout 消费、
公网部署、外部发布或把本地模拟计费当生产服务。§8.7 的本机链证据不自动升级为产品 E2E。

### 8.9 独立 broker 测试模式（2026-09-08）

来源：用户明确指令“允许新增独立 broker 模式及相应 capability schema，仅测试使用。
然后继续完成milestone”。这是§8.8 schema冻结的限定例外：允许独立 `broker-test`
模式和对应capability约束，接入真实CLI/Desktop共享处理链，使用已配置本机broker
进行真实Gemini development调用。此模式不进入普通产品picker，不启用商业Credits，
不恢复旧BYOK，不宣称可发布。保持显式上传同意、请求/输出token上限、真实用量和费用
证据；不把请求数当点数或把未知费用写成零。其余冻结项、dev exit和holdout门禁不变。

### 8.10 重新开放 geometry development（2026-09-08）

来源：用户确认九个非文本假框后，明确要求“1. 查这 9 个框为什么被 Surya/后处理留下。
2. 修规则，使透页痕迹不再进入 text geometry。3. 对现有 dev reference / 25 页重新跑
geometry regression。4. 达标后重新 freeze。5. 再执行真实 broker E2E。”

本指令取代§8.8对本次geometry adapter修复的禁止，授权针对墨迹支持修复Surya后处理；
保留Surya provider、runtime与detector版本，保留原始检测和D-ready排除溯源。
不修改reference、scorer、measurement contract或阈值，不访问holdout。
按要求先修复并验证全部25页，再依据既定geometry门槛决定是否重新冻结；未达标不得
以“没有退化”代替dev exit，后续真实broker E2E依赖新的合格冻结。

实施解释：先重放两遍独立检测raw以隔离adapter变化，记录其与新模型推理的证据区别。
墨迹过滤修复作为新增一次substantive迭代计入既有3次上限（累计2/3），不重置预算。

### 8.11 未匹配行与覆盖不足的逐项分析（2026-09-08）

来源：用户要求“保留已完成的 ink-support 修复，不回退”，对36条unmatched和coverage
deficit逐页逐类分析，区分detector miss、merge/split、box geometry、matching artifact、
reference defect；“先输出失败 inventory 和各类型数量，再针对系统性 geometry 原因做
最小修复；每轮修复后重跑完整 25 页”，继续要求recall≥99%、coverage≥98%。

不修改Gold来提高分数，不访问holdout；达标前不重新freeze、不运行broker E2E。
既有ink-support保持，reference/scorer/measurement contract不变。疑似reference缺陷
仅登记证据和待人工裁决状态，不据此改标注、移除分母或另报“修正后通过”。本轮先发布
inventory再实施有证据的最小适配器修复，沿用既有substantive上限，不因重开分析重置。

### 8.12 固定测量与有限 detector × adapter challenge（2026-09-08）

来源：用户确认“ueberweg-kraemer那条疑似reference异常已经人工修复”，要求“先把测量口径和错误类型固定下来，再做一轮有限对照与通用修复”，最后提供Surya vs Paddle报告。此指令重新开放检测器开发比较与一轮有限通用修复，取代§8.6的开发选型限制和§8.11对已由用户修正该条reference的禁止；不追溯改写历史成绩。

方法约束：Surya/Paddle原始检测→通用单位组织→既定转写/合成。先独立编号保护/图像支持拆分与同基线片段组织，再受邻行/邻栏约束补边，最后同检测器有预算局部重检。不得读取reference指导拆分、以评分并集替代逻辑行、全局膨胀或仅凭Otsu删除浅字。原始片段和每步变换保留；新增诊断放证据，不改受保护产品schema。四组对照同25完整开发页、同共享适配器、固定模型/权重/输入/阈值、相近预算，优先复用raw。阅读顺序共同匹配行对回归与新增行错误分开。不得新增Gold或进入holdout；技术选型不是泛化/生产证明。

执行决定（非额外用户要求）：本次有限轮最多首实现+一次纠正，每候选两检测器25页；同检测器每页最多2次局部2倍重检。详见 docs/evidence/detector-adapter-challenge-2026-09-08/protocol.md。未重新冻结前不改生产路由、不进行Gemini调用。

验证纠正记录（代理执行决定，非用户要求变更）：病例与回归复核发现短词/页眉后缀误拆、局部零散像素重新支持透页假框，以及独立单位顺序错误，故在首版及初次纠正后继续做这些安全与顺序纠正，共保留6个规则版本，每版均完整25页、双检测器回放，不扩大模型或局部推理预算。此记录取代上述代理设置的候选次数上限；不改变用户方法、测量或产品验收要求。最终报告为 docs/evidence/detector-adapter-challenge-2026-09-08/report.md，最终候选为该目录 final-delivery/；尚未重新冻结。

### 8.13 空间支持、测量与受限语义关系修订（2026-09-08）

用户来源：本任务中以“继续当前 Surya geometry 工作……三个连续阶段”开头的完整指令。
该指令明确授权 Phase 1 实证之后修改 contract/scorer，以及必要的最小 schema/API；
取代 §8.2/§8.10–12 对**本轮新版本**测量和 schema 修改的禁止，不改写旧证据。
用户明确要求：“logical unit 可以有多个 ordered spatial supports”；这是等价行为
约束，具体表示由实现决定，并非强制 unit 直接拥有 crops。详见 ADR 0017。

本轮要求编号（用于溯源，不另建权威 baseline）：
- SS1：detector fragments/source IDs/原坐标可追踪；真实输入为安全 spatial supports，
  grouping 不生成新的权威 bbox 或扩大 crop。确定性层拥有 canonical order。
- SS2：新旧测量分开；一对一匹配承认真实紧框且防止跨行/超大框；正式 reference-line
  ink 分母独立于 prediction，自动 mask 不是人工 Gold。保留 split/merge、顺序、超大框、
  fragment 完整性及实际转写输入污染诊断。旧 IoU≥.30 成绩继续保留。
- SS3：Gemini 仅提出绑定既有 IDs 的语义关系候选；不改坐标、顺序、source IDs，
  不删除 unit、不搬移原始转写。允许 uncertain，确定性验证关系结构。
- SS4：顺序执行三个阶段；每个实质候选全25页回放（共享抽象同时回放 Paddle）；
  测量后不得根据得分调 adapter；真实语义实验少量、有预算、隔离于生产。
- SS5：没有三阶段证据不得宣布 FROZEN/blind-ready/product-ready；不运行 holdout。

职责决定：geometry 负责物理空间证据、保守 membership 与顺序；1.2./717 的最终
语义关系移交独立候选层。旧5/5整体几何 grouping gate不再是新空间层的单独责任，
但旧 gate 未通过的历史状态不变，原始 residual split 必须继续列出。旧 recall≥99%、
coverage≥98%数值目标不降低；coverage 的正式分母须经独立逐行墨迹归属核验，缺失
则为 BLOCKED，不把新自动诊断转成通过。完整用户行为/E2E仍需空间、文本和结构共同验证。
本轮候选测量锁定不是产品 geometry freeze；生产 /1 factory 保持原来的显式版本限制。

### 8.14 人工 reference 修订与 digest-chain hardening 审计（2026-09-08）

来源：用户在原标注工作台完成 Burnet1400 页边码视觉修订后，明确要求记录 old/new
bbox、理由、reviewer、时间，保留旧 reference 下 oversized 失败，使用修订后的同一
manifest完整重跑25页，“先解决 measurement truth，不再碰 adapter”；另明确要求
Astra审计 raw detector→parent geometry→spatial→实际输入→Gemini response→OcrPage
raw artifact 的 digest chain，作为独立 hardening 检查而非当前 freeze 主 blocker。

此修订仅承认本次人工保存的7个边码bbox更新；source IDs/文字/order和其他24页不变。
旧manifest/评分不可覆盖；新manifest明确父版本并冻结参考快照，两检测器使用相同版本。
scorer与adapter保持不变，不能按新成绩调参。独立逐行墨迹分母的正式要求仍有效，框
修订不等于墨迹Gold完成。digest审计须区分metadata hash、实际重算、可信预期值绑定与
现有数据的一致性；不得把缺少入口hardening直接称为已证实的生产漏洞。

### 8.15 人工墨迹核验工作台与 digest 加固实施（2026-09-08）

用户来源：“下一步做measurement truth人工核验，同步digest hardening。你先自动压缩成最小 review queue，然后在右边打开一个人工核验的小工作台，可以全选核验通过，也可以单个标注。digest hardening这边，按你自己审计出来的缺口进行加固。”

沿用 SS1–SS5：自动草稿不构成人工 Gold；最小队列压缩交互，不抽样省略 reference。
实施决定：25张页卡覆盖1378行，126个重点组覆盖226行，其余1152行在整页图中核验。
独立墨迹归属草稿只依据 reference 和图像，prediction 仅用于核验优先级。允许人类
全选批准、逐页/逐行批准、组件归属修订、补画/擦除及问题标注；墨迹修改撤销该页批准。
审批保留 reviewer、时间、范围及图像/reference/mask身份。正式覆盖率只消费明确人工
批准的像素归属；未完成全部核验时全量正式指标仍为 BLOCKED，不自动冻结。

digest加固采用外部选定并独立保留的预期值，校验 raw/parent/spatial/实际PNG/payload/
response及最终artifact；兼容旧结构读取接口，以增量checked入口提供严格校验。
这是本轮实现决定，不把旧生产/1路由自动改为/2；离线一致性不代替真实转写证据。
证据：docs/evidence/measurement-truth-review-2026-09-08/report.md。

### 8.16 Foreign-ink contacts 的人工严重性分类（2026-09-08）

用户明确要求保存165个foreign-ink contacts的人工classification，不修改geometry、crop、adapter；正式分别报告contact、benign、material contamination、uncertain数量。
用户关闭条件：“若 `material=0` 且 `uncertain=0`，关闭 actual-input contamination blocker”；同时“保留99.97% actual-input ink coverage和所有原始pixel diagnostics”，“不删除165这个数字，也不重新定义成0”。
此明确决定将contact数量与人工严重性判断分开：完整分类满足上述条件时，仅关闭该contamination blocker，原始contact和覆盖率数字不变，不因此自动宣告其他freeze/product gates通过。
当前批准记录未含明确严重性分类；先保存165项待分类台账，pending与人工uncertain分开。不得由代理推断benign或将缺失分类计为零。证据：docs/evidence/measurement-truth-review-2026-09-08/foreign-ink-classification-r62/。

§8.16执行结果：用户对“165项全部为benign，即benign=165、material=0、uncertain=0”明确回复“确认”。已保存逐项批量人工分类；按用户条件关闭actual-input contamination blocker。contact=165、actual-input coverage=99.9718437830315%及全部pixel diagnostics保持不变；不自动关闭其他gate。分类及关闭收据见foreign-ink-classification-r62/。

### 8.17 真实 /2 transcription + PDF E2E（2026-09-08）

用户来源：“进入真实 `/2` transcription + PDF E2E”。在§8.16明确关闭actual-input contamination blocker后，此指令授权进入本轮真实开发转写/PDF验证，取代先前本轮不得运行真实E2E的阶段性暂停；不授予holdout、发布或无上限费用授权。

本次执行决定：先两页non-holdout（Burnet1400、Ueberweg143），每页最多一次真实Gemini请求，不自动重试；既有模型gemini-3.7-flash，每次maxOutputTokens16384。使用已核验锁定的Surya spatial/2支持，经checked准备与实际PNG绑定、真实Gemini、checked Rust composition、既有DerivedDocument/searchable_pdf writer到可检索PDF。

保存开发语料页序号→单页PDF物理页号0的显式映射，只改变新envelope的page_index与其digest；原geometry/spatial/crop/adapter不变，所有实际PNG与历史一致。输入PDF由对应开发PNG无损嵌入生成；不打开holdout或原始整卷。

此次路径从锁定geometry开始，未重新运行detector，也未经过CLI/Desktop factory或商业Credits，不能据此宣布这些产品路径通过。验收检查包括真实response支持ID/顺序/digest、不可变几何、PDF文本保真与落位、源图渲染一致；文本准确率需与源文另核，不能用离线fixture或HTTP成功替代。证据和请求上限见docs/evidence/spatial-transcription-pdf-e2e-2026-09-08/request-plan.json。

### 8.18 Step1真实 multi-support开发验证（2026-09-08）

用户来源：当前对话以“Step 1 — 真实 multi-support /2 transcription → composition → PDF 验证”开头的完整指令，范围仅holdout前Step1。固定前提：geometry/reference已收敛、1378/1378matched、正式coverage通过、inversions0、oversized0及既有checked /2 hardening。

MS1：枚举当前25页全部multi-support units（实际34个），保存IDs/provenance/bbox/实际PNGhash/order/disconnected/masked及重点病例映射。
MS2：根据inventory最小调用集，至少TOC48/55/F，覆盖实际3+supports及masked/single controls；事前请求与token预算、无自动重试、完整vendor响应。
MS3：逐support ID/order/text独立，NFC-only显式规范化保留raw与摘要，不改变其他字段或放宽validator，不重请求。
MS4：核验跨support文字搬移、消费后漏字/重复/错误join；保留support边界，使用已有消费规则而非reference反推join。不增加语义关系。
MS5：专项证明F/body各自safe support、旧1132上行像素未重入，TOC48/55正文及尾页码独立保留和定位。
MS6：必须checked OcrPage→canonical DerivedDocument→既有PDF writer→真实重开；验证文字/顺序/定位/渲染，不能退回generic assembly。
MS7：使用既定NFC+whitespace normalization正式CER，保留空格标点重音同形字；Greek/Latin从同次alignment计错。阈值5%/8%/4%不变；可另报去全部空白诊断。
MS8：失败按最早层分类，geometry/input问题只记录blocker；只允许一般消费者侧最小修复，保留before/after并回归受影响/离线50页契约。
MS9：上述硬门槛全部满足才声明REAL_MULTI_SUPPORT_SPATIAL_TRANSCRIPTION_PDF_E2E_PROVEN — DEVELOPMENT_SCOPE，否则PARTIAL/BLOCKED。通过只说明可进入Step2，不授权自动执行。
MS10：不改detector/adapter/membership/crops/reference/scorer/threshold，不接factory、不改/1路由、不跑完整25页双遍/freeze/blind/holdout，不扩展1.2/717、书签或apparatus语义解析。

执行决定：34个multi units全部位于Brisson13/Burnet300/Burnet1100三页，选这三页整页各1次，159supports含89个single controls，覆盖全部34个；不按响应选取supports。预计输入182850tokens，总输出上限49152；请求与停止预算见docs/evidence/multi-support-real-e2e-2026-09-08/request-plan.json。

后续有界诊断授权（2026-09-08）：用户在只读错误分析及“小批量对照→完整性指令对照”提案后回复“可以，先做对照验证”。允许对已知开发失败选择固定诊断集，在调用前锁定两组相同PNG/IDs/批次，组A仅缩小请求批量，组B仅增加完整性指令。此为已知失败的诊断，不替代MS1–MS10正式验收；生产prompt、geometry、crop、reference、scorer保持不变。原3次调用记录保留；新增预算为6次请求、不重试，详见docs/evidence/support-transcription-controls-2026-09-08/request-plan.json。不得据此宣称Step1通过或自动进入factory/holdout。

后续重复性验证授权（2026-09-08）：用户“很好，验证小批量效果的可重复性”。固定原A组PNG/指令/分批，新增3轮独立请求，每轮3个A批次及1个事前固定的开发长行对照批次，共12次，无自动重试。预算和验收见docs/evidence/support-small-batch-repeatability-2026-09-08/request-plan.json；不改变MS1–MS10门槛，不据此宣称Step1通过。此前6次诊断证据全部保留。

### 8.19 可审计小批量策略与TOC leader语义投影（2026-09-08）

用户来源：先提出“点引线本来就是layout evidence”，要求原图/raw transcription/support geometry/像素证据保留，新语义CER与leader fidelity分开且版本化；在只读方案分析后明确指令：“先把‘小批量原指令’变成真正可审计的 `/2` 转写策略，然后根据这个方案处理点引线保真问题。”本指令授权实现，不是仅分析。

BS1（方法）：原prompt、实际support PNG和membership保持不变；固定、事前可审计的分批→逐批严格ID/order/NFC验证→全量聚合→原完整页/2 validator，不允许混入历史响应、漏批发布或隐式重试。每批最多4supports、保留unit、超限unit显式拒绝是本轮实现决定，不是永久产品不变量。
BS2（保真）：raw响应、NFC派生、plan/请求/输入哈希、完整聚合依赖均保留。真实开发验证采用Brisson13完整67supports/18批，事前预计input82450、累计input超过90000停止下一请求，无自动重试。不是完整25页双遍或factory接入。
LP1（分层）：原始文本/几何不可改写；新增版本化leader像素解释和语义文本投影，只有有TOC上下文及独立像素证据的确定leader可投影为结构间隔。不确定短点串、正文标点原样保留；不重新推断membership。
LP2（审计）：保留leader区域、输入hash、原始字符范围、派生文本/定位及policy版本；leader检测不读取reference答案。参考侧语义标注单独固定，禁止用预测mask同时删参考以改变分母。
LP3（测量）：原始CER及旧失败历史不覆盖；另报semantic CER和leader fidelity。新建语义参考标注若仅代理审阅，明确development candidate，不伪称人工Gold。新验收口径尚未获正式独立验证前，不能据候选semantic CER宣布旧Step1通过。
LP4（消费）：新投影必须显式进入派生文本及searchable PDF；标题不能被拉伸至原点引线宽度。新增像素支持的派生标题框不修改原support geometry；raw全文仍可追溯。范围仅opt-in checked/2开发路径，不接CLI/Desktop生产factory、不扩展书签/apparatus语义、不运行holdout。

### 8.20 Leader fidelity复核与新版Step1验收（2026-09-08）

用户来源：本任务消息“接下来给 leader projection 做最后一次独立 fidelity review”，明确检查“confirmed leader 没有误吞真实文字/省略号/缩写句点”“两个 abstention 保留合理”“uncertain 不自动处理”；要求“正式版本化批准”raw CER、semantic CER、leader fidelity及“semantic CER 是否继续沿用 overall≤5% / Greek≤8% / Latin≤4%”，并指令“这一步通过后，把新的 Step 1 标记为通过”。

FA1：对原图独立审查布局/标题分界，再与固定预测mask、字符删除span、派生框比较。此次为用户委托的代理视觉复核，独立于detector边界，但并非另一位独立人员、盲审或新增人工Gold；此身份不得改写。
FA2：授权本轮正式决定新版开发验收契约。实施决定：semantic CER继续沿用5%/8%/4%；raw CER保留原算法、数值和旧门槛比较，作为独立必报指标，不再用layout点数不一致单独否决新版语义交付。leader fidelity独立硬门槛为误删真实内容0、保护标题墨迹被mask/派生框截断0、uncertain自动处理0、非leader修改0；miss/abstention完整报告并保留在semantic CER中，不按预测删参考，不设置事后贴合28/30的recall门槛。
FA3：这是已知开发样本上的显式验收修订，取代§8.18 MS7在新版Step1中的raw-only总门槛及§8.19 LP3的“新验收尚待决定”状态，不追溯改变旧失败。只有复核和上述新门槛实际通过，才记录Step1/2 PASS — DEVELOPMENT_SCOPE。新语义参考仍为代理源图标注，不升级为人工Gold。
FA4：新版Step1沿用全部34 multi units/159 supports的原开发覆盖；Brisson13采用§8.19完整18批的新真实结果，Burnet300/1100使用原已通过的整页真实证据，逐页明确策略和版本。不是三页都跑过小批量、不是新的同一批次复现。保留旧34-unit失败快照。无新增模型请求，不接factory、不自动执行Step2/holdout/freeze。

验收契约及新结果：docs/evidence/leader-fidelity-acceptance-v2-2026-09-08/。

### 8.21 真实factory接入small-batch /2及四页smoke（2026-09-09）

用户来源：本任务消息“把 small-batch /2 接入真实 factory”，指定产品路径“PDF → frozen Surya → spatial supports → deterministic small batches → Gemini → checked aggregate → semantic projection → DerivedDocument → searchable PDF”；要求factory实际checked /2、operational digest/expected-parent hardening、正式NFC normalization、checked canonical order，以及leader仅进入semantic/index/bookmark consumer、不反改raw OCR/geometry。另要求先跑Brisson13、Burnet1100、Burnet1400、Ueberweg143四类production-path smoke，比较support texts/order/projection/PDF文字/定位/hashes/provenance；“如果不一致，准备先修 factory wiring，不回头碰 geometry”。

FW1（方法/行为）：将已验证的空间算法和小批量策略挂入CLI/Desktop共享factory和真实orchestrator。禁止选中此路径后回退/1或通用line assembly；缺少checked绑定必须失败。
FW2（证据）：detector/runtime→parent→spatial→实际PNG→每批vendor raw/NFC→完整聚合→checked OcrPage→语义DerivedDocument/PDF全链保存与核验。原始响应和OCR/geometry不因投影改写；仅派生消费者读取semantic。
FW3（范围）：四页smoke先做不付费的实际PDF渲染/Surya接线预检及契约比较，再按固定计划真实转写，每批一次、无自动重试。旧开发响应只用于明确标记的离线对照/比较，不填充真实聚合。不得直接全25页或holdout。
FW4（差异处置）：先区分源PDF渲染、provider/raw detector、adapter调用顺序、身份/provenance重命名、真实模型非确定性和消费者差异。已有geometry算法/阈值/参考不改；接线错误可直接修复。真正的geometry差异若不能由接线修复则记录阻塞，不偷偷调参。
FW5（边界）：本指令授权实际共享factory路径实现与smoke，不是商业Credits/公开发布授权。现有显式用户管理broker模式可以承载真实入口；不得把其商业可用性标为production_ready。TOC适用上下文沿用独立选择和源绑定，未确认页面作identity projection，不把点串检测推广到全部正文。

### 8.22 固定产品路径上的 Lite 文本质量/成本比较（2026-09-09）

用户来源：本任务消息“不是新的 smoke test、不是 Step 2 重验、也不是重新验证 geometry / multi-support / factory / PDF consumer”；仅问固定路径上将 gemini-3.7-flash 换成 gemini-3.5-flash-lite / gemini-3.1-flash-lite 的文字质量与成本，以及能否作为后续开发主模型候选。

LM1：复用§8.21真实产品路径已保存的四页68批实际payload，除请求模型标识外保持输入PNG/prompt/分批/参数一致；3.7复用已完成响应，不再调用。
LM2：仅比较文本及成本。读取现成参考与对应关系；沿用既有NFC/空白/Greek-Latin计分及固定leader像素解释来获得产品语义文本，不重跑detector、geometry验收、factory、DerivedDocument或PDF。新结果不修改既有Step状态/Gold/评分器。
LM3：保留逐模型raw/NFC文本、失败、用量和标准在线价格依据。小批量不是Batch API；包含thinking费用。模型默认thinking随模型固有变化，显式参数保持原样，不另外调参。
LM4：结论仅为已知四页上的开发候选判断；不自动替换正式默认模型，不推断全25页/holdout/上线。若固定请求不兼容或质量不足，报告实际限制，不修prompt/改geometry以迁就某模型。

### 8.23 Brisson actual-raster 安全关闭与25页真实产品开发验收（2026-09-09）

用户来源：本任务消息“先保留3.7，之后再加入3.1作为替代选择。目前不需要”；“只确认真实 PDFium raster 下那31条TOC leader的安全性：25个confirmed没误删、3个abstention合理、没有meaningful punctuation被吞。不要再调leader detector”；“完整25页 /2 product-path开发验收”，明确路径“PDF → Surya → spatial supports → small-batch Gemini → semantic projection → DerivedDocument → searchable PDF”，并列举八类正式指标。

BR1：只核对已完成Brisson actual PDFium raster的内容安全：全部31条TOC候选、25条confirmed及3条新增abstention；同时完整记录2条既有abstention与1条短点线uncertain，不把31改成28。检查源图、删除span、保护内容和框、非leader及页码保持；不修改或调优detector。关闭范围是这份已存raster的安全，不等同原始整书PDF验收或CER通过。
PA1：3.7 Flash继续作为当前模型；3.1替代选项延期，本次不实现。固定既有detector/adapter/leader和small-batch /2产品链，从25页对应的原source PDF进入factory；不能把既有PNG封装PDF、旧detector输出或响应replay冒充该链。
PA2：全部25页留在分母，按§8.1、§8.13–8.16及§8.20已批准口径报告line recall、geometry/input coverage、semantic overall/Greek/Latin CER、leader fidelity、omission/duplication、multi-support、canonical order、PDF text/position；raw CER继续必报。实际raster与人工mask坐标/像素绑定须验证，不把旧PNG人工分母静默标成新raster Gold。未知或缺少有效分母的项报告unverified/BLOCKED，不删页或调整阈值。
PA3：授权完整25页真实产品路径开发验收，取代§8.21 FW3和§8.22 LM4对各自旧任务的四页范围限制；不授权holdout、自动freeze、生产发布或3.1实现。一次25页结果不替代既有两次clean回归的DEV_EXIT_READY条件。前景执行、无自动重试；单页失败仍完整报告并继续可执行页。
执行准备记录：原始source registry的六份PDF均指向当前未挂载的/Volumes/Haoran。先完成独立Brisson边界核验及固定验收计划；源PDF可用后核对SHA/page身份与holdout三重隔离并计算实际请求预算，再执行真实链。原始PDF文件本身包含其他页不构成读取holdout内容的许可，只渲染/提取固定development页。

§8.23源文件恢复/坐标执行记录：用户“已挂载 继续”后，六份原PDF SHA验证一致。旧首批参考为pdftoppm/MediaBox渲染，后批为PyMuPDF/CropBox；25页参考均可由原PDF原renderer逐像素复现。Brisson原CropBox各边缩进9pt；评价时用已验证PDFium f32渲染矩阵及PDF box偏移，把实际region足迹换算到原人工mask坐标，不重采样/修改mask，不改变产品geometry或匹配阈值。先前拟用的PyMuPDF抽页在Burnet改变渲染，改用PDFium原生页对象导入；25页原件/抽页PDF渲染逐像素一致。未用PNG封装替代，未进入holdout页。

§8.23执行结果：25/25原PDF页已完成真实链，364次gemini-3.7-flash调用、无重试/历史填补，usage计价约$2.778352。正式line recall1376/1378，geometry/input宏覆盖99.0406%/99.8064%，semantic CER overall/Greek/Latin为0.7606%/0.3909%/0.1002%，25页逐页CER通过；25PDF/1413supports文本顺序与定位核验通过。实际32个multi units，Brisson当前原PDF28confirmed内容安全通过。仍保留1整行与局部内容遗漏、模型漏词/加词、公式LaTeX表示及2个prefix split；172个当前foreign-ink contacts实质分类未验证。交付FULL25_MEASUREMENT_COMPLETE_WITH_DEFECTS，意图aligned；不宣布两次clean或DEV_EXIT_READY，不自动追加回归/holdout。详见docs/evidence/product-path-dev25-2026-09-09/report.md和real-01/acceptance.json。

### 8.24 Full25缺陷修复与两轮开发回归（2026-09-09）

用户来源：本任务在对§8.23报告只读分析后，用户“可以，根据你的思路执行1-4”；针对已披露的旧3/3额度与新费用上限，用户明确回复“授权新增1次，合计封顶15美元”。

FR1（范围与方法）：补全172个当前foreign-ink contacts诊断，复核D. Peinture页码47及数字、单字符、标点/学术符号错误；保留原参考、评分器、测量合同、阈值和历史结果。诊断中的agent图像复核不作为新增human Gold。
FR2（修复授权）：在既有3/3之后新增一次有界substantive修复迭代，针对输入遗漏、模型漏词/加词/非请求纠错及公式表示漂移进行局部产品修复与验证。本条取代PA1对本次必要adapter/转录修复的冻结限制；Surya模型/权重、gemini-3.7-flash、真实source PDF→factory→/2→projection→PDF的路径保留。不以reference文本指导产品裁片、转录或补字；不全局膨胀裁片、不以文本去重规则静默删词。实现与集成纠错属于同一修复；新的实质方案不自动获得额外额度。
FR3（验证与预算）：先做固定开发案例的小范围受控验证，再做两次完整25页真实产品回归及相关regression suite，全部页留在分母；新模型调用合计费用上限15美元，包含thinking，预留未知usage并设停止规则；无自动重试或历史响应填补，前景执行。局部probe、离线回放与真实产品回归的证据分开报告。
FR4（验收与边界）：沿用既有dev exit和稳定性要求，不用平均分掩盖内容缺陷，不重新解释clean来晋级。两次结果/所有已知缺陷及未关闭事项均报告；不访问holdout、不自动freeze或发布、不实现3.1选项。全局准入未满足时交付具体未达项，不将测量完成写成产品完成。

代理实施约束（非新用户要求）：本轮原证据不可覆盖，新产物位于docs/evidence/product-path-dev25-repair-2026-09-09；付费前固定本轮请求与实现摘要。公式修复优先约束转录为源忠实的Unicode搜索文本，不增加复杂公式的新产品结构。

§8.24实施记录（候选，非完成声明）：完整组件与既有局部检测双重确认恢复C.、47、脚注5及führt.；验证器重算来源证据，原始detector输出不改。25页旧detector输出离线回放为1377/1378匹配、0倒置，新增Dillon6邻行20像素接触，总contact单元173，仍待人工分类。39项相关测试和CLI编译通过；提示词候选尚未应用。四组A/B及两次真实模型回归受自动审批阻塞：审查要求明确授权这些固定学术文献裁剪图像发送至Google Gemini目的地，已向用户提出。首次沙箱内尝试DNS失败，无模型请求发出；未绕过拒绝。纯离线CLI预检不消耗模型调用，不作为CER/clean证明。详见docs/evidence/product-path-dev25-repair-2026-09-09/report.md。

§8.24联网授权补充：针对明确列出的固定25页学术文献裁剪图像、目的地generativelanguage.googleapis.com、模型gemini-3.7-flash、四组A/B及两次25页回归、总上限15美元，用户回复“授权上述图像发送与验证”。该明确授权解除上述联网分支的授权缺口；人工contact验收、无自动重试和其他FR4边界继续保留。

§8.24候选固定记录：四组原完整batch相同PNG的A/B完成8次调用，全部16supports的固定参考编辑53→17；字面候选改善法语词尾/Unicode公式，但worden在候选中仍遗漏，不能宣称漏词全部解决。依预先声明的整组非劣条件将同一通用字面提示词作为回归候选，不追加调参或择优试跑。提示词不含参考词答案，除提示词以外AB输入/分批/模型参数不变。25页纯离线CLI新推理通过（25全页+7局部），图像/父几何/spatial与已测量候选一致；提示词修改后39项相关测试再次通过并重编译CLI。重新固定payload计划及实现用于两次真实25页回归；每次实际运行必须重新推理并在调用前匹配独立固定输入/计划，不能重放旧检测或旧响应作为实际回归。

§8.24补充回归授权：首次真实回归在Menn414遇到已成功返回但省略thoughtsTokenCount的响应；totalTokenCount恰等于prompt+candidates，费用账本误判未知并阻止下一调用，后续页由既有unknown-usage保护拦截。原响应、错误账本快照、失败页和费用核对均保留；不重试单批或改写失败为通过。费用计算只在总数等式严格成立时认定省略thinking为0，针对性7项测试通过。用户针对补充1次完整25页回归、保持总15美元及失败记录的具体提问回复“授权补充1次完整回归”。因此可执行剩余已授权一轮加补充一轮，以取得两次完整结果；产品几何/提示词保持不变，不增加实质产品方案。

§8.24最终回归记录：保留首次12/25失败尝试后，literal-accounted/pass-02与pass-03两轮各25/25页、364批执行完成。相同输入/几何/完整请求逐页哈希一致，产品5文件与固定候选一致；125个历史固定文件、6个原PDF、25个选页PDF核对未改。两轮匹配1377/1378、0倒置、173contact；semantic整体CER为0.7594%和0.7346%。四处输入目标两轮均恢复并转录，但essayerons de、worden等漏词随轮次变化，342→:312和额外is持续，新增整词遗漏；FR2模型忠实度仍drifted。最终PDF逐字/位置后置核验分别25/25和23/25；后一轮NUL→替换字符及分式换行→空格未通过原逐字断言。两次clean与全局质量准入均未满足，不freeze/holdout。FR1诊断及FR3获批执行完成，human contact实质分类仍待审；FR4保持。费用保护最终9项测试通过，连同39项产品相关测试及CLI编译证据保留。总921次真实调用，usage标价估算$7.2338265、未知预留0，低于15美元。额外一次实质方案额度已用于该候选，不因预算剩余自动再迭代；详见docs/evidence/product-path-dev25-repair-2026-09-09/report.md。

### 8.25 低成本分层转写与确定性fidelity契约（2026-09-09）

用户来源：本任务“处理 Binarize 的下一阶段开发”长指令§1–16；核心原文为“在不牺牲 scholarly literal fidelity 的前提下，大幅降低 Binarize 的 OCR 模型成本，并把生成式模型不可避免的随机错误约束在一个可以检测、升级和拒绝的产品契约内”。另明确“两个api在项目中都可以找到，10刀以内的测试我明确授权。要注意api不要泄露。”

TR1（结果不变量）：不能将自身无法忠实消费的模型响应标记completed。建立response→semantic→compositor→实际PDF提取→completed确定性契约；分类valid/lossless candidate、需明确canonical representation、invalid/fail、unresolved/needs_review。禁止静默删字符或用normalization改判旧PDF失败；精确投射检查必须在最终交付前执行。
TR2（方法与边界）：当前Surya geometry/adapter为candidate freeze，除新的直接可复现geometry根因不改动；173contacts保留human substantive gate，不用agent判断关闭。Gold/reference/scorer/旧失败固定，holdout封存。不切Paddle geometry或调ROI追模型遗漏。
TR3（架构方向）：本地geometry/信号→低成本primary→本地独立验证与风险路由→选择性Gemini3.7升级→本地semantic/PDF gate→accept或needs_review。不是强制实现处方；允许证据否定local verifier/具体provider/router价值。不能将同随机模型二次调用叫独立verifier。路由不得仅依据primary输出中的Greek字符；独立本地recognizer从源crop取信号，分歧不得静默忽略，recognizer不取代最终文本。
TR4（实验）：当前官方文档核实gemini-3.1-flash-lite与deepseek-v4-flash-vision-exp的接口/vision/thinking/accounting/pricing/约束；保留Gemini3.7高质量baseline，不切3.8。复用dev已审来源建30–50 supports challenge、普通controls及严重失败，来源/参考/实际输入可追溯；预定义go/no-go再看结果。必要时对失败子集比较原batch与isolated；不得默认生产一support一次请求。费用与严重漏词/加词/数字/Greek/PDF失真分开计量，不以低CER掩盖严重问题。
TR5（业务目标与晋级）：在fidelity不退化前提下争取blended成本降低约50%以上，不能操纵阈值或用大量needs_review冒充成功。若绝大多数仍需升级则明确无足够商业价值。只有deterministic契约完成、geometry未改、challenge明确winner、费用验证及router停止规则满足后才值得提议Full25；本任务不启动新Full25。holdout、freeze/release/publish仍未授权。
TR6（操作授权）：授权可逆产品实现/测试/离线response replay/local recognizer benchmark/provider-router接线/challenge/harness与并行子任务；本阶段新的Gemini/DeepSeek focused实验共享10美元硬上限，密钥仅从已存在项目外私密配置读取，不显示、不入日志、prompt或证据。该明确新阶段授权取代§8.23/8.24对3.1实现延期及旧有界修复次数对本阶段的限制；不扩大到Full25/holdout/不可逆操作/评测合同变更。无自动重试，无persistent background任务。证据新写docs/evidence/transcription-router-2026-09-09，历史记录不改写。

代理实施决策（非新增用户要求）：以实际PDF重新读取的每个新生成文本对象为exact对齐单位，保留support内部空白、重音及上标；页面提取器插入的段间分隔不等同support内部文本。canonical NFC仅可沿用已有raw→NFC显式来源记录；多行公式无预先批准表示时不自动压平，转requires_representation/needs_review。停止规则将另在该证据目录事先固定，作为实验决策而非更改旧评分器。

§8.25执行结果（2026-09-10）：实现Unicode eligibility、/2及writer前置拒绝、最终安装前实际encoded spans/ToUnicode精确核验，并用PDFium核对对象数量/不可见属性。PDFium文本重排会补空格、折叠连续空格及处理行尾hyphen，故不把该layout字符串冒充literal decoder；原诊断保留，未trim/normalize。历史50份PDF的48份通过，NUL和multiline分式2份拒绝；独立MuPDF同样48/50exact。新协议有效响应171组/252spans的真实PDF probe有168组/249spans通过、3组multiline在build前拒绝；这是synthetic carrier序列化验证，不是新source-geometry E2E或任意阅读器copy/paste保证。

§8.25实验判定：预定义48supports(24风险/24controls)×3模型及10原batch×3，共174次真实调用完成，usage保守计价$0.16623936、未知预留0、无重试。新3.7low/3.1minimal/DeepSeekdisabled的完整分母CER为1.6834%/2.5898%/5.7624%；后者含1次malformedJSON缺失，原batch3.7另有2次schema失败，均保留。Lite新增普通controls整词遗漏，DeepSeek发生Greek转Latin等错误。两个固定策略各有27/48needs_review、12/24control review；matched3.7节省仅5.32%/17.30%，四组合全部NO_GO，不通过降低门槛宣布成功。独立local信号检出了本panel源图代理确认错误，但误升级过高；47模糊polytonic仍uncertain。代理源图review不是humanGold，22/23reference差异未改。TR1与TR2/TR4/TR6授权开发执行aligned；TR3/TR5低成本且fidelity不退化的产品目标未达。保留Lite为后续候选，不接成GUI默认、不晋级Full25。288绑定artifact含36geometry文件校验未改；173contacts人审gate保留，holdout/freeze/release未触碰。统一证据docs/evidence/transcription-router-2026-09-09/report.md与decision.json。

### 8.26 DeepSeek easy-domain focused experiment（2026-09-10）

用户来源：针对§8.25失败结果，用户提出应问“Binarize里面有多大比例的supports属于DeepSeek能稳定廉价解决的easy domain”，而非要求DeepSeek完成全域OCR；随后认可分域实验设计并明确“可以开始实验，授权20元以内的deepseek api使用”。此指令解除前两轮只读限制，仅对本阶段授权实现和执行。

ED1：验证预先可识别的easy domain、域内literal fidelity/重复运行稳定性、困难内容误入、实际可自动完成覆盖率和blended成本。普通Latin正文先分English/French/German，不默认Latin或短数字就是easy。不得按DeepSeek成功结果倒推域定义。典型书75/10/10/5是待验证假设，现有dev25只证明开发分布。
ED2：先在已见48challenge校准source-only分域信号，确认集与其disjoint；计划300unique supports(240Latin候选各语言80+60边界)，复用未改正式references/原图/geometry。不能按历史Gemini正确率筛选。样本配额若实际不可行，先报告并记录具体调整；不接触holdout，不修改Gold或几何。
ED3：本地recognition-only比较Surya/Paddle，不恢复Tesseract为统一否决器。不重新检测、不更改裁图/ROI，不强制全文一致。具体信号先校准再冻结，unknownfailclosed；reference/oracle标签只用于取样和独立评价，不进入runtime路由。
ED4：本次新DeepSeek调用共享20CNY硬上限，保存raw/usage/model identity/call plan，不泄露密钥；每support计划3次固定独立请求(非纠错重试)，暂定900calls。仅授权DeepSeek，不执行设计中的300次新Gemini；改用既有同cropGemini历史对照，明确thinking/batch/prompt差异。未知usage保留最坏预留并停止，不因质量差选择性删样本或重试。
ED5：保留deterministic semantic/PDF gate。任意已确认严重错误被自动放行则当前分域规则不晋级；稳定错误仍是错误，重复执行不增加独立样本量。评估source-only及增量检查的安全/覆盖/误升级成本，不以大量人工review或aggregateCER掩盖失败。未取得代表书页分布前仅报告开发覆盖及情景成本，不宣称典型书70–80%覆盖。不会因本实验通过自动跑Full25/holdout或freeze/publish。

代理执行决定：新产物位于docs/evidence/easy-domain-2026-09-10；旧§8.25报告、代码pins和raw结果保持不改。先固定source分域和样本，再读新DeepSeek结果；不追加新书抽样以绕过既有holdout。

§8.26执行结果：300条新确认样本与旧48在ID/PNG上均disjoint，240普通文献正文候选各语言80、60边界；无新增公式确认。源图规则/代码/900payload先固定，本地300完整分类后才读取新DeepSeek文字。900次调用全部protocol valid、无重试/未知费用，保守费用1.9781124CNY≤20，临时密钥已删，无新Gemini。D1各语言73条、共219/300，60边界均排除；D2各轮210/211/212。87条差异/不稳定样本均经agent源图复核（非humanGold），D1确认10个独立严重误放行样本/26次，D2仍8个/22次，含稳定词语替换；easy125模糊待判、排版/参考表示差异未改判。275/300原文三轮稳定不等于正确。真实encoded PDF probe为896次pass/4次nonNFC构建前拒绝，319不同PDF独立MuPDF0不一致；合成载体不冒充新source E2E。历史Gemini等份分摊的成本情景显示约59–62%算术降幅，但未实现安全节省；本地Surya300 CPU耗时1578s且典型书覆盖未测。ED1–ED5实验执行aligned，当前D1/D2依ED5均NO_GO，可靠低成本自动完成仍未证明。1048输入/参考/geometry/历史文件、16代码配置及900payload核对未变；原173contact人审gate保留，不晋级Full25/holdout/freeze/release。详见docs/evidence/easy-domain-2026-09-10/report.md、decision.json。

### 8.27 Independent page witness / dual-reader feasibility study（2026-09-10）

用户来源：本任务“做两件事”：本地加入临时Mistral API，明确“免费，未充值，你使用免费额度来测试”，用户自行测试后rotate；参考所附C*报告设计测试项目，允许更合理推断，但“各项AI能力、思路可行性都需要先测数据”。前一条明确授权DeepSeek/Qwen密钥复制并保留在本项目本地配置。

IW1：先以数据评价廉价support reader、Mistral整页独立证据、选择性Gemini升级的可行性，不预定DeepSeek必须为primary或C*必须胜出；接口能力、对齐覆盖、共同错误、误报警、费用与耗时分开测量。报告中的价格与能力为待核实输入，不是既定事实。
IW2：复用固定300supports/旧DS三轮/local recognizer结果，先离线测严重误放行捕获与正确controls误报。新增建议限一轮Qwen support OCR及对应unique整页Mistral；不重复900DS、不调用新Gemini、不改router/geometry/Gold/scorer，不启动Full25或holdout。当前已见300仅开发证据，不冒充fresh blind确认。
IW3：Mistral只读取已授权development源页并提供证据，无权修改Surya canonical supports。段落到support对齐不能使用待核验DS文字或Gold选取最相似片段而制造一致；不可唯一定位时UNKNOWN或预声明group-level比较，不能冒充support证据。confidence只作待评价feature，不能直接当正确性标签。共享裁图风险需单独测试，paragraph bbox本身不证明缺字。
IW4：临时Mistral密钥仅保存本地Git忽略的0600配置并保留待用户rotate；Mistral仅免费额度，禁止充值或自动切付费。Qwen先核实免费额度保护和实际费用边界；没有覆盖的新增付费须在具体计划后请求授权。所有请求保留raw/usage/identity/input pins，无自动纠错重试，错误与unknown完整保留。新证据写docs/evidence/independent-witness-2026-09-10，不改旧报告和固定调用计划。

§8.27第一批执行记录：本地保存并保留临时Mistral凭据（Git忽略、0600），只读model-list确认4.1可用。复用300supports对应24张完整原PDFium页PNG，24调用均成功返回284blocks/wordconfidence，原图尺寸一致，调用耗时合计41.70s；依用户未充值免费账户授权，无充值/新DS/Gemini。旧结果离线replay中Surya/Paddle均检出8D2严重ID的22次错误，D2参考正确代理controls误报69/512与312/512；Paddle另有Λ→A三次共同错误。Mistral回溯源图/参考引导诊断8关键短语均正确，但不能称自动召回。冻结的无DS/Gold参与几何aligner在目标300仅20原生定位，280UNKNOWN，另4完整组12supports；原生一致19/19/20每轮，意味着281/281/280理论升级。两处未覆盖警报均源图确认阈值假警报，不改geometry；合成drop单独support能报警、paragraph内缺行仍UNKNOWN。报告Batch价格0.4被核实为缓存价，Batch公告2USD/1000；word/blockconfidence二选一。Qwen300固定payload已准备，最坏2.8536CNY、拟硬上限3元；免费保护只能控制台设置，新增付费授权待用户答复，尚未调用。IW1–IW4获授权免费/离线工作aligned；C*生产可行性未证明，Qwen分支pending，不改默认/Full25/holdout。详见docs/evidence/independent-witness-2026-09-10/report.md。

§8.27 Qwen操作授权补充：针对已披露的300次固定Qwen请求、最坏2.8536元和总额硬上限3元，用户明确回复“授权qwen调用”。授权覆盖该固定计划的新增费用与向阿里云Qwen发送既有学术文献裁图；不追加重试、新模型或扩大其他评测范围。

§8.27最终执行记录：用户授权后Qwen300/300完成，无重试，usage估算0.0102367CNY≤3，累计490.008s。旧DS三轮原文一致99/101/103，全部已审严重77次均分歧，D2的22次均分歧；但Qwen在easy119把formaeque读成formaequa，分歧不等于纠正。第一轮参考正确proxy误报123/216，6条一致参考不同保持表示歧义。Qwen300响应的293份unique合成PDF通过encoded/MuPDF精确核验，不冒充source E2E。Mistral–Qwen原生20/20一致，280UNKNOWN不豁免。第一轮DS+Qwen严格一致99候选、201升级，历史Gemini分摊情景仅约19–21%算术节省；当前Mistral原生策略更贵，非已实现安全节省。IW1–IW4实验执行aligned且已授权范围完成，低成本生产路线仍unverified、TR5商业目标未达；没有新增DS/Gemini、产品默认变更或晋级。详细数据及限制见本轮report.md/final-validation.json。

§8.27段落评估补充（2026-09-10）：用户指出段落/行粒度不同不应直接触发Gemini，架构并非必须行对齐，并明确要求“先测Mistral的完整段落文字准确度，同步分析align是否有可行的策略”。这是授权离线评估与架构分析，不是修改生产输出合同。复用24页284块，276块有完整既有参考映射，173多行组80993字符原始段落CER1.6063%；双方仅换行投为空格的附加诊断1.3927%。源图抽查确认Greek误字、脚注号遗漏和破坏性HTML/LaTeX输出；不是仅对齐问题。既有几何规则1343/1348supports、298/300panel可唯一关联到块，20/300仅逐support文字定位；旧几乎全升级成本结论仅适用于旧逐support一致策略，不是段落方案成本。分析提出完整group比较、canonical unit共同粗化、独立可选精细定位，尚未改变产品实现或准入。本次相对请求aligned；词汇错误全量源图裁决、整页遗漏与运行时核验/精细PDF定位仍unverified。详见docs/evidence/mistral-paragraph-2026-09-10/report.md。

§8.27快速路径验证补充（2026-09-10）：用户明确“下一步做这个验证”，提出“主版面mistral，复杂、fail区域例如apparatus再使用Surya、paddle geometry+gemini的结合”为未验证设想，要求先测数据再给架构方向。本轮离线运行原页+Mistral输入的预固定S结构规则和C保守规则，Surya/Qwen/reference不参与运行时筛选。24页284块S候选220块/81.0%可评分字符，但漏放源图确认réflection→réflexion、konjekturale→konjunkturale、Senocrate→Xenocrate，故S不准入。C候选113块/14.8%字符、0整页全快，11个字母数字差异候选复核未新增确认词汇错误，但源图位置/表示及总体正确性未证；C保持unverified。轻量本地图像规则24页合计1.72s，删除整块影子试验173/173报警，正常0/24；不检测保留框内漏词/稀疏数字，不当作自然遗漏召回。历史同条件apparatusSurya/Paddle均27/27匹配但Surya残余分片较少；历史Gemini在S选中块参考CER约.69% vsMistral3.28%，是同源回溯而非新ROI fallback执行。架构建议分开内容重读与几何恢复，Surya作为空间fallback候选、Paddle针对已证Surya失败；不再只按主版面/复杂版面决定文字正确。新增API0，未改geometry/参考/旧raw/默认，173contacts门槛保留。实施与验证相对本条用户指令aligned；安全高覆盖快路径与真实ROI/PDF交付仍unverified。详见docs/evidence/mistral-fast-route-2026-09-10/report.md。

§8.27真实区域实验本地阶段（2026-09-10）：用户认可内容重读/空间恢复分支并要求“继续实验以分析更具体的可行性”。固定10内容ROI、4空间ROI；Surya/Paddle各4次CPU本地检测已完成，未改旧几何/参考/默认代码。Surya首次加载在推理前因116张量不等被阻止；新wrapper从同一检查点strict恢复并完全校验后另目录运行，不改历史失败。单ROI Surya1.42–1.52s、Paddle0.12–1.93s；墨迹覆盖均>99.7%，但Paddle脚注裁图有邻行接触及编号拆开，页眉标题/页码均合为一单元；Mistral固定框亦有P字顶端3像素裁切。覆盖不是字符忠实度。32份图片请求已离线准备并通过哈希/预算边界检查，新增10CNY硬上限的具体授权已询问，尚未收到答复、云端调用0。local experiment aligned；实际重读质量/端到端路径仍unverified，不作生产提速结论。详见docs/evidence/region-fallback-2026-09-10/local-report.md、cloud-manifest.json。

§8.27区域实验云端授权补充：针对已固定的32次请求（10Qwen、22Gemini3.7）、新增费用硬上限10元、遇错误/截断/预算不足即停且不自动重试的具体提问，用户回复“授权”。该授权覆盖cloud-seal.json固定请求向阿里云Qwen与Google Gemini发送既有学术文献区域图片及新增费用，不扩大至额外调用、Full25或生产切换。

§8.27区域实验云端结果：在32次固定计划中真实发送23次（10Qwen/13Gemini），22协议有效；第23次Gemini对Paddle支持组返回顶层array而非supports对象，HTTP200/STOP且usage已结算，原校验器schema_failure按预定规则停止，余9次未发送、无重试。保守10CNY/USD情景总0.28955580CNY，未知预留0。10内容区域6613字符原NFC CER：历史Mistral7.016%、Qwen3.917%、Gemini1.527%；空白诊断分别6.049/2.752/1.527%，均非代表性总体或准入分数。Gemini恢复3/3已知词替换，Qwen2/3且与Mistral共同réflexion错误、另mathematicals→mathematical及Greek漏误；中位网络时间2.409s vs8.703s。Burnet300唯一部分空间对照whole/Surya均5/125参考编辑，源图确认两者共同κατίδη→κατίδῃ；完整三路0/4，不宣布geometry胜者。整段Gemini11结果含换行均CANONICAL_REQUIRED，单元可序列化不证明读对。支持继续设计直接区域Gemini内容分支及显式段落表示，空间恢复只用于空间问题；安全触发/剩余对照/PDF实际交付仍unverified。实验及停止aligned、费用在授权内；保留旧geometry/原图/reference/Gold/default及173contact门槛，不晋级。详见docs/evidence/region-fallback-2026-09-10/report.md与final-validation.json。

§8.27剩余区域验证及结构保护：用户明确“保护好之前做的surya+museion adaptor结构。完成剩余验证”。在原10CNY上限内执行原未发送9次，不重复第23次失败；9/9协议有效、累计32尝试/31有效/1历史失败，累计标价情景0.82406580CNY，未知0。执行前后558文件hash完全相同，既有Surya/Museion adapter、raw、geometry及代码未改。新三路对照完成Burnet1400/Primavesi11/Menn577：参考编辑6/3/3、0/0/0、4/10/10；Menn Surya较高主要为上标表示，Paddle却实质重复18并添加逗号，源图及两裁图复核确认。Burnet1400 L/Ł源字形留疑，不凭参考判错。阶段时间whole/Surya/Paddle分别2.52/6.30/4.54、1.63/3.54/1.54、2.19/6.26/11.55秒（排除初始化/公共步骤/PDF，检测阶段为本轮固定先前收据）。结论保留原Surya＋adapter空间路径，内容问题可独立区域Gemini；没有依据切换Paddle或宣称自动接受。任务相对保护及剩余实验aligned，风险路由/段落表示/PDF仍unverified。详见docs/evidence/region-fallback-2026-09-10/continuation/report.md。

### 8.28 区域架构四项验证（2026-09-10）

用户来源：指出“你刚不是说验证触发器、裁图完整性、修订接受条件和段落交付吗”；代理承认误将范围退回旧剩余9次模型对照后，用户明确“嗯 那完成这些验证”。本条取代将旧9次调用完成视为本阶段完成的解释，不改变保护Surya＋Museion adapter要求。

RV1：验证触发器对真实错误漏放及正确区域误报，报告文本覆盖与无标签边界；不能用全部升级冒充有效。RV2：系统验证裁图完整性、邻行/重复/区域遗漏，并对独立源图证据检验。RV3：验证修订接受条件，区分协议有效、源文正确、表示可交付；共同错误不得因一致静默放行。RV4：验证段落表示与真实PDF消费，原始文字、换行及定位有可追溯证据，不以合成空载体或旁文件替代实际段落PDF交付。

授权范围：已有原图/真实响应的本地验证、独立实验原型与必要实际PDF生成/重开检查；原Surya/adapter/core写入器保持，旧raw/geometry/Gold及评分不变。不新增API/成本，不启动Full25/holdout/freeze/release。新证据docs/evidence/region-contract-validation-2026-09-10。四项均须有实际测试、结果及限定，不以补齐旧模型调用数替代。具体判据为本轮plan.md预声明的实验决策，不擅自成为生产准入阈值。

§8.28四项验证执行结果：本地完成24页/284blocks固定触发重跑、14ROI及8组实际mask完整性检查、32既有响应接受合同、14真实段落的表示/PDF试验；0新API/成本。S/C/C_source快路径220/113/94，升级原文量20.13%/85.34%/91.58%；偏置源图确认11内容错C及C_source均抓住，但高置信框内删词/删行/删数字/Greek消失注入仍漏，不能准入。源图补检捕获4框外定位问题；55块边界标记及3ROI未决，像素重复不得直接改判文字污染。独立全区域严格原文一致0/10、仅显式空白/NFC内容一致1/10，恢复Seno-/crate为有用候选；两个同错/同漏注入仍通过候选规则，自动语义接受NO_GO。14段落9建立源图行表示、5空间拒绝；13多行旧单span仍被原writer拒绝。9真实原PNG图像载体PDF保留内嵌raw/分隔符映射，实际字符流、MuPDF及core literal/install验证通过，双阅读器渲染不变；MuPDF普通复制均末尾加LF，PDFium4例连字符变U+FFFE/合行，普通复制及词级/上标空间交付未证明。646既有保护文件及48源图/raw pins未变，原Surya＋Museion adapter/core未修改。RV1–RV4验证任务aligned并完成；可靠低成本无人复核产品仍unverified/NO_GO，不启动后续生产或holdout。详见docs/evidence/region-contract-validation-2026-09-10/report.md。

### 8.29 区域架构集成验证（2026-09-10）

用户来源：“按照这个设想推进验证”，承接Mistral整页初读、内容与空间分流、保护Surya＋Museion adapter、显式段落表示及独立PDF消费验收的候选设计。授权本地实验集成及验证，不代表认可既有触发器或自动接受规则，更不构成新增API费用授权。AI1：验证统一状态机能否正确分支和追踪源图、原文、候选、空间与消费证据；Qwen不成为必经步骤，段落粒度本身不构成空间错误。AI2：保留全部needs_review及缓存缺口，不把旧响应回放说成新实时路径，不因schema/一致/字符流通过而自动接受。AI3：原Surya/adapter/core、历史raw/geometry/Gold及人工门槛保护；新增原型与证据位于docs/evidence/regional-architecture-integration-2026-09-10。新增调用先固定具体样本/上限/停止条件，另行请求授权；不进Full25/holdout/release。

§8.29本地阶段结果：284块自动ROI分流109PRIMARY/157CONTENT/4SPATIAL/14BOTH，原文量12.72%/73.88%/1.94%/11.46%；仅12自动ROI精确命中旧缓存。14固定ROI状态机回放9区域Gemini、5空间缓存、7旧实际PDF重开，全部保留未决源文/空间/普通复制问题，不称实时E2E。新4次本地Surya＋既有adapter完成37逻辑单元，目标与上下文像素归属分开审计，仍待复核；803旧文件hash不变。新8个正文自动ROI（4内容风险/4快路径对照）及请求、10元硬上限/出错不重试计划已准备但未发送，新增费用授权pending。AI1–AI3本地执行aligned；整体实调用与源文/PDF验收继续pending，见regional-architecture-integration-2026-09-10/report.md。

§8.29云端授权补充：用户针对8个固定Gemini区域请求、新增费用上限10元、出错/截断/预算不足即停且不重试的明确提问回复“授权”。授权绑定regional-architecture-integration-2026-09-10/cloud-seal.json与cloud-manifest.json，不扩展至额外请求、4个空间区域云端调用、Full25或生产切换。

§8.29授权8区执行结果：8/8Gemini协议有效、0重试/未知账务，usage计价0.17151750CNY≤10，调用合计19.305s/中位2.272s。8源图复核：Burnet确认恢复χρόνου/边注10/ἐν ᾧ，Brisson恢复断词；4快路径对照未新确认实质词误读但断词/引号差异保留，不证明安全路由。固定行投影2/8候选、6失败；6例实际进入同权重Surya＋原adapter，5行数匹配但pixel归属/重叠未决，Burnet39单元对31文字行不硬配。2新源PNG载体PDF通过原core literal/install及双阅读器图像/字流检查，普通复制仍LF/CRLF和Brisson U+FFFE差异；6例空间拒收。803旧保护文件hash不变，0生产改动/额外云端。AI1–AI3及授权8请求验证aligned并完成WITH_DEFECTS，低成本无人复核产品仍未证明；全部8例needs_review，详见regional-architecture-integration-2026-09-10/live/report.md。

### 8.30 离线 paragraph binding 原型（2026-09-10）

用户来源：本任务用户提交六点计划，代理只读review后指出需区分绑定/文字/PDF验收、允许歧义和未匹配、保留小组件、明确文本投影及统计边界；用户回复“接受你的方案，根据它开始下一步工程。完成后仍然回答”四项问题。该明确决定授权下一阶段本地工程，不授权生产切换或新增模型调用。

PB1（范围）：冻结旧14区域记录和新8区域记录的现有源图、Mistral/Gemini文字及可用Surya输出。核对实际独立裁图、重叠、几何缓存缺口；不新增OCR/检测模型推理，不调用API或下载模型，保护既有Surya/adapter/core/raw/geometry/Gold和人工门槛，不进Full25/holdout/release。

PB2（绑定方法）：在新增版本化层实现源图组件/局部lane、多对多单调support graph、raw字符区间、未匹配和歧义状态。candidate-derived文字长度等仅是布局特征，不能冒充独立文字验证；框重叠作为诊断，所有小组件仍保留归属或未决记录。geometry-only边注不能豁免要求转录的内容。精度按实际段落/行组声明，不伪造词级坐标。

PB3（验收）：算法不读取source annotations/Gold；源图观察与运行结果分离，代理开发标注不晋为human Gold。运行全部22记录并报告独立裁图分母、重复运行确定性、实际绑定错误/歧义及错绑反例。没有4–5个转pass的目标；空间绑定通过不代表文字正确、PDF交付或低成本自动路由成功。

PB4（表示与交付边界）：raw_model_text、source_literal_candidate及reading_text分层，默认identity投影；不静默去连字符或改PDF默认合同。现有PDFium异常单独定位底层提取API与真实复制的证据差异，不预设ToUnicode根因。交付新增原型、manifest、运行/测试证据、源图复核页与四项架构判断。计划位于docs/evidence/paragraph-binding-2026-09-10/plan.md。

§8.30工程结果：冻结22记录/21不同裁图/11页、11记录既有同输入Surya；0新增OCR/检测模型/API。44份Gemini/Mistral候选支持图及其确定性重放、源页→ROI像素与全页坐标封装完成，867旧保护/输入文件未改。源图开发观察中Gemini21/22记录行关联一致，Burnet31正文＋8边码匹配；content-05短符号带曾被配错行，最终显式ambiguous而非修成pass，旧两轮输出保留。12/22同时满足布局候选、组件全归属与行观察一致，仍0自动交付。24单元测试通过；实际7项文字反例中4项触发空间问题，等宽Greek错词、等宽边码错数、近等长行交换3项仍可几何匹配，5项图篡改均拒绝。6个旧真实PDF副本表示实验保持图像及内部字符，但未解决Brisson跨API提取一致性；不修改默认writer/reading合同。PB1–PB4工程范围aligned并完成WITH_LIMITS；文字安全、精细定位、语义结构、普通复制及低成本router仍unverified，不晋级。详见docs/evidence/paragraph-binding-2026-09-10/report.md。

### 8.31 Binding消费边界与模型组合分轮实验（2026-09-10）

用户来源：§8.30交付后明确“保留本轮 binding 层，并落实你说的四个边界”，随后要求三轮验证：“1 少量测试or根据已有测试，提出最佳预案和challenger；2 实测为了获得目标 fidelity，到底应该让多少文本进入 Gemini？预算是多少？3. 测challenger是否有能力挑战”，最后回答工程结果、架构支持、调整与独特性四题。

MC1（已批准边界）：保留封存binding；按实际段落/行组精度交付，不假称词级坐标；空间和文字检查独立；根据消费需求安排几何能力，不能假定Surya只在罕见异常触发；PDF互操作独立。新增binding_consumption.py为离线消费规划合同，不接生产、不改原binding/Surya/adapter/core。

MC2（实验目标）：沿用TR1–TR5的scholarly literal fidelity与成本目标，不降低质量以换费用。模型组合及取样属于实施选择，不升格为产品不变量。第1轮复用既有开发证据；第2轮区分实际实验调用量、策略部署升级量、漏词/错词/数字与Greek遗漏、未知案例；第3轮挑战者同源比较。缓存时间、组件时间与真实端到端时间分别报告，人工Gold与正式holdout门槛不变。

MC3（实施决定与范围）：主预案Mistral OCR 4.1＋选择性Gemini 3.7 Flash区域重读；challenger为Gemini 3.7 Flash直接整页读＋相同独立binding/验收。四张已见开发页34区域，固定S/C/全升级三策略，区域各两次独立请求、整页各两次，总76次Gemini请求，计划共享20CNY硬上限；仅是待授权计划，旧8请求授权不扩展。复用Mistral原响应意味着本轮不能证明新实时全链路时延；如有候选胜出，真实整链路计时及新的分层确认集另行准备。详见model-combination-2026-09-10/plan.md及cloud-manifest.json。

MC4（当前证据）：消费合同44图×4需求=176评估，176篡改拒绝检查，旧203封存artifact哈希一致。24页284块回放，S升级20.13%原文字量但已知错词仍静默放行；C升级85.34%原文字量仍未证明fidelity；全Gemini旧响应也不作为真值。第1轮与MC1本地工程aligned；第2–3轮新增调用pending预算授权，产品成本/忠实度目标unverified。无新OCR费用，无生产/Gold/holdout修改。


### 8.32 免费本地二值化与自动书签阶段（2026-09-10）

用户来源：本任务消息“目前先暂时放下ocr路线，先把本地自动书签目录的工程搭完”，目标“纯免费，本地二值化+自动书签目录的工具，进入封装、分发程序”；“高质量ocr为未来选项（几个月以后）”。本条改变当前阶段优先级，暂缓§8.31及此前高质量OCR后续开发，不删除历史结果，不宣告此前OCR验收通过。

LB1（行为）：已有PDF文字层优先复用；缺少文字层时探索Surya局部提取。先核查苹果系统OCR接入是否便捷。只识别目录页、标题及必要页码锚点，避免全书OCR成为基本任务的前置步骤。
LB2（方法/保护）：用户明确“不要动ocr部分的开发，不要改surya+adaptor的架构，不要动已经冻结的部分”。本次只新增独立书签输入入口、必要验证和建议；原OCR模块、Surya/Museion adapter、冻结数据/规则/阈值保持。局部识别耗时实验获授权，不能据此更换默认provider。
LB3（收益目标）：用户提出“结构90%+文字80%”作为值得开发的收益预期，允许容易的人工修改。它不是本次已达到的测量结果，也不改写冻结OCR/geometry验收门槛。需独立定义目录结构/目标页/文字指标及完整分母。
LB4（本次交付）：调研成熟度、最小可运行文字层PDF→书签流程、真实PDF写回重开证据、局部recognition增量证据及架构升级建议。允许本地可逆开发和验证；封装/分发是本阶段方向，本次不执行外部发布、签名购买或后台服务。

实施决定（非额外用户要求）：复用既有确定性书签编译器，新增native-only输入与独立开发入口；无文字的页面明确列入未识别清单，不能填假OCR文字。先验证单栏/明确编号目录，复杂文字流和缩进型层级如无真实坐标证据保持待审。

§8.32本次结果：独立native-bookmarks开发入口及5项真实PDF消费者检查完成；原核心不新增导出，867旧冻结/输入文件和本轮开始788既有代码/配置/Gold文件hash全部不变。无关键词的明确编号六条目录全部正确，有Contents标签对照误并首项；真实Horn/Menn无outline副本仅2/0自动，前者原生字体校验阻止输出。Surya/Paddle同12小图CPU识别增量33.41/2.33秒，加载8.26/23.31秒；Paddle重音缺失。Apple Vision fast约0.45秒但页码/编号错误，accurate本机失败；未测RapidOCR端到端/新layout/扫描书全流程。41+2单元、26编译器、5新消费者通过，旧PDF回归2通过1错误消息断言失败，原门槛/校验不改。LB1–LB4调研、最小流程与建议交付aligned；90%/80%、桌面组合/目标页编辑/分发仍unverified，不发布。报告docs/local-bookmarks-mvp.zh-CN.md，证据docs/evidence/local-bookmarks-2026-09-10/final-validation.json。

### 8.33 Vision fast本地目录MVP（2026-09-10）

用户来源：本任务“[binarize bookmarks] 下一阶段工程任务：收缩到可分发的本地自动目录书签 MVP”及十一节完整指令。取代§8.32以Surya为首个扫描识别候选的实施决定，并明确授权书签专用输入、既有目录语义入口修复、独立bookmark-only输出与编辑合同。
VB1：PDF→TOC页面→Apple Vision fast→provider-independent BookmarkEvidence→既有TOC/Structure compiler→独立页码映射→轻量编辑→bookmark-only PDF。native用真实glyph视觉行；raw与投影分开。
VB2：完整现有TOC页是主实验，按条目/层级/printed/target及人工负担报告，不以CER为主；不做大型Gold或holdout。
VB3：页码lane、pattern、全局顺序/范围/相邻一致性独立；不确定仅允许同Vision放大原ROI或needs_review，不猜号。
VB4：Surya＋既有adapter不改，只在显式几何不足时fallback；不使用Surya recognition，不默认运行；统计页数/增量。
VB5：独立bookmark-only写回核对页面对象/资源/文字/渲染不变及真实outline跳转；旧OCR fidelity gate不弱化。统一编辑支持标题、parent、目标页、增删和证据跳转。
VB6：零云调用/费用/模型下载，不启新OCR路线/训练，不改旧Gold/source observation/历史封存；结果不佳停在诊断，不扩大研究。最终回答用户四问及真正发布阻塞。

实施决定：新增小型书签合同与core消费者，复用既有TOC parser/层级函数；旧0.4 scoring不变，新入口按独立版本记录。先四张已见目录页（Horn两页、Menn一页、Brisson一页）；前置一次性通用规则固定后运行，最多一次整合纠错，不针对单书规则。不自动发布/签名购买。

§8.33本轮结果：新增BookmarkEvidence、实测native glyph投影、Vision fast完整页worker、既有TOC/hierarchy消费者、独立页码/精确锚点与ROI重试、离线编辑补丁和bookmark-only writer。四张现有目录页的60个源条目中，严格独立恢复58，parent＋level正确27；完整页printed正确50，一轮ROI后54，目标53，三者同时23。代理源图开发复核而非human Gold；全部自动候选保留审核。3/4页未触发Surya；Brisson因分组/缩进诊断运行1页既有Surya detection＋原adapter，0 recognition，6组仍歧义，未证明复杂结构改善。代理复核后的Horn325页/18书签、Menn202页/11书签独立写出，全527页对象/流/native文字/渲染不变，MuPDF与PDFium重开29目标通过；不计作自动成功。45书签单元、26既有集成、8新后端测试通过。788起始保护文件仅bookmarks/mod.rs因授权导出变化，旧OCR/adapter/Gold不改。VB1–VB6本轮有限工程与方法边界aligned（详见局部限制）；无人审核结构、通用fallback、桌面封装分发仍unverified/未满足，停在诊断，不开新模型路线。报告docs/evidence/vision-bookmarks-2026-09-10/report.zh-CN.md，计分以assessment-final.json为准。

### 8.34 TOC compiler 三项修复与书级可用率（2026-09-10）

用户来源：§8.33结果后“可以 执行修复”，明确 A hierarchy grammar、B multiline TOC entry、C mandatory numeric-lane reread，并要求“冻结 Vision OCR；冻结 Surya；只修改 TOC compiler…仍然只跑这四页 regression”。
TG1：按同书 PART/BOOK/CHAPTER/SECTION、Roman/Arabic/decimal/letter 序列一致性决定 parent；OCR易混编号保留疑似规范化与来源，不逐行用字号猜。TG2：合法页码默认结束；只允许无页码、缩进/小间距相符且无新编号的明显续行合入；无页码也不自动新建。TG3：数字lane第二读改为正常算法，所有title视觉行对应数字lane均读取，保持原Vision fast与Surya及adapter不变。取代VB3/前轮实施决定的“最多8个低可信ROI”限制；本轮不重跑全文OCR、不新增书/模型/评测集合。TG4：产品承诺改为“Local automatic bookmark generation with editable review”，按书级正确目录与少量修改负担验收，不再要求所有节点无人审核confirmed；40项中36–38正确、2–4可编辑是用户示例，不是假定已通过的新语料；50+/60是本轮希望达到的观察值，不改分母/旧source/旧评估以达标。Surya只用于双栏顺序、明显bbox碎裂、无法稳定形成物理行组，本轮语义问题不触发。独立writer/GUI/打包不在本次修改范围。

§8.34结果：三项TOC compiler修复完成，保持同四页/60源条目参考不变，独立恢复60、parent＋level正确60、printed56、target55、三项同时55/60；上一轮23/60。源图代理开发复核非human Gold。所选目录范围Horn17/18（1关键节点修正）、Menn9/11（2）、Brisson29/31（2源节点＋1额外无页码章题待处理）；不外推Brisson整书或典型500页书。84个正常数字lane crop仅一轮，Vision OCR0.827秒，render0.169秒；完整页OCR与Surya新增调用均0，provider/adapter冻结。Menn native跨行恢复11条、Horn保持18。TG1–TG4 aligned；以editable book review为产品标准，不要求所有节点逐个无人审核confirmed，不把review提示数等同错误数。GUI/writer/分发未扩展。报告docs/evidence/toc-compiler-repair-2026-09-10/report.zh-CN.md。


### 8.35 小型歧义保留与树投影升级（2026-09-10）
用户来源：§8.34后只读review建议保留源树/导航树合同、弱命名线索、有限候选；用户随后明确“可以，完成这一轮小型的升级，然后再测一次”。
TU1（实施决定，承接TG1/TG4）：输出可追溯source_entries与显式导航投影，未识别页码的叶子不删除；无页码编号容器的隐藏仍是待复核导航约定，不认定原图无号码，不作为人工修改负担自动下降的证据。
TU2（实施决定，承接TG1）：使用已有token bbox的标题起点；Notes/Conclusion等保留上下文/根节点解释；疑似编号保留字面未解析与规范化候选，只有书内明确编号支持时才选择疑似decimal解释，仍需复核。
TU3（本轮界限，承接TG2/TG3）：同四页与既有84数字crop重放，旧raw/reference/分数不覆盖；无新OCR/Surya/模型/语料，候选采集与全局搜索不在本次实现。正确页码等参考字段仅用于单独诊断副本，禁止输入正式识别路径。GUI/writer不变。
验收：同60源条目固定参考，层级60/60与联合55/60不得回退；分别报告源树、导航投影、缺页码/目标以及待复核负担。此值是开发回归门槛，不是新书泛化证明。

§8.35结果：源树/导航树投影、命名项布局作用域、有限编号候选与既有token标题起点完成。同四页60源条目仍独立恢复60、层级60、printed56、target55、联合55；Brisson源树32、导航31，1项容器投影仍待复核，不把隐藏算为人工减负。13个源编号节点保留多解释；零新OCR/Surya/模型。独立字段诊断：parent oracle仍55，正确printed＋原映射59（不是自动成绩，剩64缺目标锚点）。56Rust单元＋26既有集成＋15Python合同通过，407保护文件不变。TU1–TU3 aligned；范围止于本地小型升级与既有开发回归。报告docs/evidence/toc-ambiguity-upgrade-2026-09-10/report.zh-CN.md。


### 8.36 统一桌面工作区与可用书签闭环（2026-09-10）
用户来源：§8.35状态说明后明确“可以，做可用闭环。binarize目前的UI一起改…充分的UI设计权限…最核心的就是工具逻辑和使用动线要清晰一致，不用复杂的美学特征。OCR功能的UI入口可以一起做了，但是暂不开放”。接受前述打开→指定目录页→生成→修改→保存新PDF，并用既有三份PDF做实际路径核对的下一轮建议。
DW1（行为）：一个文档工作区，打开PDF后选择黑白处理与目录书签，可单用或合用；预览/修改后从统一保存动作输出新PDF。用户不需要运行命令行、填写工作目录或模型配置。原PDF不覆盖。
DW2（行为）：目录选择明确使用PDF物理页；生成、源页定位、标题/父节点/目标页修改、增删、无页码容器选择、整本复核确认和保存闭环。缺目标或非法树不允许写出；复核提示数不当作错误数。源树/导航解释、raw和修改记录保持可追溯。
DW3（方法）：复用当前local TOC core与Vision fast/native、numeric lane和独立writer，不将旧OCR factory入口冒充新本地路径。先原图目录识别再黑白处理。native与Vision实际输入路径明确记录，旧四页55/60不得移作新路径成绩。OCR工具入口可见但禁用，不触发识别、云配置、模型安装或收费。
DW4（UI授权）：可重构当前主界面和必要前后端连接，简单清晰的工具逻辑优先。高级参数折叠，无复杂装饰。不更改旧OCR算法/Gold/Surya/provider合同，不发布、购买签名或自动新增语料/云调用。
DW5（验收）：前端状态/交互与后端合同测试；现有PDF真实生成/修改/保存/重开跳转、原内容保护（书签单用）与合用效果；记录实际路径/耗时与待处理节点。实际桌面或浏览器连接验证必须分别注明，不能把mock测试当端到端。打包分发非本轮必要发布动作，但必须明确本机运行依赖限制。
实施决定：左侧工具与设置、中间PDF预览、右侧目录复核；OCR工具禁用。本地Python仅承接既有抽取/几何投影/映射和安全writer，Rust桌面直接调用共享core compiler，不调用CLI。当前本机可用闭环优先，独立运行时的可移植分发另行验收。长任务有取消、错误恢复与过期文档隔离。


### 8.37 实操反馈：预览、处理范围与本机运行时（2026-09-10）
用户来源：§8.36交付后的五项实操反馈：“原页预览加载速度比较慢，有卡顿感，找原因看能否优化”；“binarize应该可以设置范围，可以加一个跳过本页按键，避免用户总是一次性黑白化全书”；“binarize耗时还是整体比较久”；Menn目录正确；其他目录测试报“未找到带PyMuPDF和Pillow的Python运行环境”。
DP1（行为）：原页预览应快速响应翻页；检查渲染/处理/排队/传输的实际耗时，避免不必要的处理与重复读取，保留导出质量。
DP2（行为）：可选择黑白处理的PDF页码范围，并可在当前预览页“跳过本页”。实施解释：跳过或范围外页面在输出中保留原样和原顺序，不删除页面；书签目标仍使用完整PDF物理页。该解释依照“避免全书黑白化”的原意，不扩展为抽页工具。
DP3（结果）：诊断黑白处理耗时，优先保持算法、分辨率及输出不变的优化。发布给用户的本机构建需采用优化构建；以固定已有页的实测比较为证据，不把局部速度外推成全书实测。
DP4（行为）：修复Finder启动时无法发现已有Python依赖；使同一本地应用的目录功能不依赖Codex/终端PATH。验证最小GUI环境启动，不将终端成功当作修复完成。
约束：继承DW1–DW5，保留当前已验证的Menn/native与Vision fast/numeric lane目录路径；不更换OCR/provider、降低输出质量、消耗Gold、发布外部版本或新增付费/后台任务。必要的页范围混合输出与对应验证属于本轮授权。

### 8.38 显式工具动作与打开时页码重建（2026-09-10）
用户来源：§8.37实操确认“目前binarize的速度不错”，随后十项修正：处理范围与原预设/分辨率对齐、无分割线；黑白栏靠右“开始”、目录靠右“生成”；“处理预设加入高级（实际上是闲置）设置”；目录可读但页码关联差；拖入后自动判断文字层、重建排版页码；有文字层直接重建，无文字层Apple Vision随机抽几页、只读排版页码，凭规律立即倒推映射，识别多套/重启/罗马与阿拉伯页码；利用打开到点击目录生成前约20秒；底栏小字显示文字层及页码重建状态；“这轮修复完成后可以commit”。
PG1（行为，细化DW1）：工具区排版统一；黑白“开始”与目录“生成”分别触发对应处理，底栏保留统一保存。实施解释：开始后暂存黑白结果，保存复用它；参数/范围变化使旧结果失效，避免按钮仅切预览或保存时重复处理。
PG2（方法与时机，替代本地目录旧映射只收逐页精确锚点的实施限制）：每次打开/拖入PDF立即在当前文档生命周期内自动检测文字层并重建排版页码；有文字层直接抽取，无有效文字层时仅对少量抽样页的页码区使用既有Apple Vision fast。以少量一致、独立页样本确认分段偏移，无须15个精确锚点；识别Roman/Arabic及重新从1编号的独立段。抽样复核、边界不确定性、原始观测与规则来源保留。不是全页正文OCR、常驻监控或云调用。
PG3（行为）：目录生成复用该文档预建规则，自动关联物理页。相同印刷页码在多段重复时采用目录顺序等可验证证据消歧；仍不唯一则保留候选，不默选错误目标。底栏明确文字层检测及重建中/已完成/部分/不可用；旧文档结果不能污染新文档。不把少量样本解释为已逐页证明无插页或所有重启。
PG4（验收与授权）：维持既有黑白速度与质量、原PDF不覆盖、原始OCR/Gold/历史证据不变；验证文字层、无文字层、Roman→Arabic、Arabic重启及含混/取消/换文档，使用既有书做真实GUI生成与跳转复核。修复并验证后进行本地commit，不推送或发布，不纳入无关OCR/Gold工作。对第4项UI文字的两种理解已询问；独立页码工作不等待该可选布局偏好。
第4项布局澄清：用户明确选择“是，预设移入高级设置”；常用区域保留范围、分辨率与开始，预设移入折叠区，不新增“高级”预设。

§8.38结果：PG1–PG4 在本轮本地开发范围 aligned。真实 Start 暂存/Save 复用、范围/DPI对齐和高级预设完成；打开自动文字层/页边规则重建，原生Horn约5.73秒、图像版约4.18秒就绪；原生Horn18/Menn11目标核对一致，扫描Horn17/18正确关联（另一条目录OCR漏读页码，保留待填），Roman/Arabic非目录页锚点有效。Menn单页组合保存其余201页文本/渲染保持一致。少量抽样对未抽中的短段/插页仍无完备保证；不升级为所有书型/独立分发验收。1466保护hash不变，优化本机R2包已构建；本地精选commit，不push。报告docs/evidence/desktop-pagination-2026-09-10/report.zh-CN.md。

### 8.39 Unattended explicit TOC generalization (2026-09-11)
Authoritative source: this task's full user instruction “长时间、无人值守的 Binarize TOC 抽取泛化任务”, sections 0–25. Outcome TGX1: independently evaluate and improve recovery of all explicitly printed TOC groups, entries, titles, page references and hierarchy in complete real PDFs; never add body-only headings or infer a body-derived TOC. TGX2 explicit methods: new corpus from read-only /Volumes/Haoran (user's /Haoran mount), target >=200 books with 150 dev/50 blind, whole-document split; two direct-visual Luna passes covering the entire document for multiple TOCs, no extractor/native-text/OCR/prediction-derived Gold. Agent visual Gold has no human confirmation. TGX3: seal all blind inputs/Gold after split; development-only structure taxonomy and baseline before principled changes; bounded iterations, failure saturation, source freeze without commit, one blind run with unchanged parameters; no post-blind code/tests/config tuning. TGX4: checkpoint first (750be795), preserve unrelated OCR/Gold/holdout/keys/raw PDFs, no cloud costs/publication/push; ALL final task changes remain uncommitted. Detailed protocol/evidence lives under evaluation/toc-generalization-2026-09-10. Errors in individual source files are recorded/skipped; do not stop or fabricate quota/annotation completion. Final report must include micro/macro/exactness, all catastrophic failures, annotation disputes, rationale and heuristic audit, saturation and explicit commit recommendation; no final commit regardless of recommendation.

§8.39 development evidence correction (before freeze/blind): source-image overview and independent Luna audit contradicted three original development annotations (one no-main-TOC inclusion, two page-coordinate errors, one with content discrepancies). Under the existing TGX2 source-faithful Gold requirement and user sections 6/9/22, preserve v1 records/scores, obtain independent visual confirmations, and prepare versioned corrected development Gold. Exactly the first already-eligible, preseal unique reserve, toc-847a87593090cae5, is promoted to development to replace the ineligible positive case; selection does not use predictions. All other reserves and the blind remain sealed. This changes a corpus implementation decision, not the explicit-TOC outcome or minimum eligible count. Record original and corrected results separately; no claim that two agent passes guarantee correct Gold. See evaluation/toc-generalization-2026-09-10/development/corpus-correction-plan.md and gold-disputes/.

### 8.40 Apple SystemLanguageModel hierarchy pilot and human Gold review (2026-09-11)
Authoritative source: follow-up user request after the §8.39 final report: “引入Apple SystemLanguageModel来帮助理解level。vlm不需要读文字，只需要判断层级和排版特征的关系。接好最小可用线，用10份以内的旧gold先看一下情况，是否能提升level准确度”; then shrink Gold to 40–50 quality candidates for human correctness/inclusion review and separately prepare 20–30 holdout candidates with agents for the same human review. The trailing “1)” and sequencing are interpreted as starting with the hierarchy pilot; the two review-workbench requests remain subsequent work, not cancelled.

HFM1 explicit method/outcome: local Apple SystemLanguageModel assists only hierarchy/layout interpretation, not transcription, TOC discovery, entry creation or body-derived structure. HFM2 pilot boundary: at most 10 old development Gold documents, compare level and immediate-parent accuracy on identical fixed entries; preserve all source text, references, ordering, raw evidence and original Gold. No new/previous blind run or holdout consumption. HFM3 preservation: new optional sidecar and separately versioned evidence; §8.39 source/config/test freeze and single-blind evidence remain untouched, with no commit/push/publication. This follow-up authorizes a new experiment, not tuning or overwriting the completed blind evaluation.

Implementation choice: prepare eight old development documents with well-recovered entries, including imperfect-hierarchy cases and accurate controls; this is a conditional hierarchy diagnostic, not corpus-wide generalization. Actual environment: macOS27 with Xcode SDK26.5; SDK26 has no image prompt API, and SystemLanguageModel.default reports modelNotReady in both sandbox and host preflight. Image mode must stop explicitly if unavailable. A feature-only textual mode (no title text or baseline hierarchy) is an optional alternative whose use was asked of the user; it is not silently substituted for the requested visual path. No accuracy claim is possible without successful real model responses. Evidence: evaluation/toc-hierarchy-foundation-models-2026-09-11/.

User decision, 2026-09-11 follow-up: “必须直接看原页图像，保留此路径并等待 SDK”. HFM1 therefore requires actual source-page image input. The previously proposed feature-only alternative is rejected; keep the image adapter pending SDK/model availability without automatically installing assets, replacing the path, or scheduling a background retry.

HR1 user outcome: prepare 40–50 quality old Gold candidates for the user's separate inclusion and correctness decisions. Existing agent-reviewed annotations are candidate evidence, not human approval. Implementation: 49 previously adjudicated old candidates, excluding the source-unsupported document, with immutable old records and separate editable drafts; the user's exclusions determine the final membership. The old blind has already been exposed and cannot be called a new holdout.
HR2 user outcome/method: independently select 20–30 additional holdout candidates and use agents to preprocess them from source images before the same human review. Select by metadata and source eligibility, not extraction performance; retain whole-document identity separation from all previous annotation sets. Partial agent drafts must be corrected before delivery. No extraction, scoring or automatic sealing of the new holdout is authorized by this preparation request.
HR3 user workflow: local workbench with source-page images beside editable TOC groups/entries, including numbering, title, printed reference, page, level, parent, ordering and additions/removals. “确认收录” and “确认无误” are distinct decisions. Correctness requires the user's source-review action; edits or exclusion revoke active confirmation, retaining versioned history. Local preparation and review server are authorized; no agent may click approval on real candidates. Final dataset quality remains unverified until user review, even if schema and UI tests pass.

HFM environment update after user “27 sdk 已安装”: located SDK27.0 in `/Users/theo/Downloads/Xcode.app`, compiled the actual image branch successfully using command-scoped DEVELOPER_DIR. `attempt-02-sdk27` still returns runtime `modelNotReady`; SDK blocker resolved, model inference remains 0/8 and improvement unmeasured. No global Xcode switch, model download or automatic retry.

§8.40 preparation result: HR1–HR3 local preparation/workbench behavior aligned: 49 old candidates plus 23 new candidates (24 TOC groups, 842 entries), with separate human inclusion/correctness actions, original-source preview, versioned edits and revocation. All 72 draft structures validated; 7 backend tests, 8 hierarchy-contract tests and separate synthetic-browser human-flow checks passed. Source-only final review rejected one agent false-positive bibliography and corrected missing continuation pages, numbering, tree collapse and scrambled physical-page mappings. One scrambled-source candidate remains explicitly flagged for human inclusion judgment. Original 275 frozen source/config/test/runtime hashes unchanged. Final quality, membership and any new sealed evaluation remain unverified/pending human decisions; no agent approved a real record or consumed the new holdout. HFM1 original-image method and isolation aligned; HFM2 accuracy comparison blocked by actual modelNotReady despite SDK27 image build success. Evidence: evaluation/toc-human-review-2026-09-11/preparation-report.json and evaluation/toc-hierarchy-foundation-models-2026-09-11/attempt-02-sdk27/report.json. All changes remain local and uncommitted.

### 8.41 Local visual hierarchy model selector (2026-09-11)
User source: “用本机先安装MiniCPM-V 4.0 / Qwen 3B 来尝试吧。自动目录那里做一个苹果/MiniCPM-V 4.0 / Qwen 3B切换器。苹果 主动测试是否可用，MiniCPM-V 4.0 / Qwen 3B可选下载。先测试这两个，后续pcc通过了再接入”. LM1 supersedes HFM1 Apple-only provider choice with three explicit local providers; source images remain required and models change only level/parent, not text, entry membership, order, page targets, OCR or source evidence. Interpretation stated to user: Qwen 3B means Qwen2.5-VL-3B-Instruct, the image-capable 3B model. LM2: Apple active availability check when opening model controls, on focus and manual refresh; no automatic inference or downloads. Other models have optional explicit downloads, visible progress/error and cancellation, pinned revisions and hash validation. User explicitly authorizes installing both models locally now. LM3: evaluate each on the same predeclared eight old development cases; no new holdout, Gold approval, PCC call, cloud cost, publishing or commit. Q4_K_M and fp16 visual projector via pinned llama.cpp b10908 are implementation choices. A bounded attempt per document uses original images and 90-second inference timeout; failures retain baseline and are reported unmeasured rather than successful. GUI keeps the existing compiler/navigation membership and page mapping; model edits are a separately recorded projection. Existing historical snapshots and scores are immutable; current UI/backend may now be extended under this explicit new task.

LM3 bounded protocol repair: the initial generic relation-object schema allowed missing/extra entries, contradictory level/parent, and repeated entries until truncation. Retain attempt-01 unchanged. One consolidated attempt-02 on the same eight documents per model uses an exact-length parent-index array; levels are deterministically derived from immediate parents and the original ordered-tree validator remains mandatory. This is an output-protocol implementation correction, not new Gold or relaxed acceptance. No more automatic prompt/model tuning beyond this second pass; both attempts and failed/abstained cases remain in the report.

§8.41 result: LM1–LM3 local feature and authorized test scope aligned. Both pinned models installed (~6.42 GB), Apple active check remains modelNotReady; GUI switches, explicit downloads/cancellation and model-generation cancellation verified in real native IPC. Two bounded attempts completed on the same eight old development cases. Final MiniCPM 0/8 applicable trees; Qwen 3/8, 3 abstentions, 2 invalid. On Qwen's identical applied subset (65 matched entries), level 52.31%→3.08%, parent 52.31%→7.69%; no improvement or production readiness claim. Models remain experimental, with source evidence/pagination preserved and manual review required. Current UI integration necessarily updates 7 of the old 275 current-source ledger paths under this explicit new authorization; 268 others and historical evidence remain unchanged. No holdout run/Gold approval/PCC/cloud inference/commit. Report: evaluation/toc-local-models-2026-09-11/report.zh-CN.md.

### 8.42 Apple-only product and one cloud hierarchy ceiling probe (2026-09-11)
User source: “切掉吧，只保留apple接口。用项目中的deepseek/qwen级别的视觉模型测一次上限”. AC1 supersedes §8.41 LM1/LM2 MiniCPM/Qwen product selections and download capabilities: remove their UI, IPC download command and bundled runtime/catalog; retain only Apple's proactive availability and original-image hierarchy interface. Preserve base compiler, OCR, entries/text/order/page targets, review/save gates and unrelated dirty work. Previously downloaded weights and historical evidence are retained, with no active product path to them; deletion of multi-GB data is not implied. AC2 authorizes one bounded cloud visual diagnostic using the existing project credential/provider, separate from product UI. Agent choice: DeepSeek Flash vision (existing DeepSeek account; existing Qwen model is OCR-specific), same frozen eight old-development documents/176 entries and original images, fixed parent-index output contract, one request each, no retries or prompt iteration, no holdout or new human Gold. Existing model alias now routes to DeepSeek-V4.1-Flash per live official documentation; requested/returned identity is recorded. Use deepseek-flash, enabled high thinking and max_tokens=16384 to test this stronger route; these differ from non-thinking local quantized runs and are not a controlled provider-only ablation. Peak uncached USD rates 0.30/M input and 1.20/M output, conservative USD-to-CNY accounting multiplier 10, task stop cap 3 CNY; exact payload/image pins and per-call worst reservation before dispatch, unknown transport/usage or budget failure stops remaining calls. Settled invalid/abstained answers remain failures and do not trigger retries. Semantic evidence reports paired level/parent, controls and regressions, not merely JSON legality. AC3: passing this small old agent-reviewed panel may justify later investigation but is not an actual global upper bound, independent human Gold, production acceptance or automatic cloud integration. No PCC, release/publish/commit, new account, model download or background task.

AC2 external-transfer authorization supplement: after automatic approval review blocked the concrete eight-request DeepSeek batch for lack of explicit image-transfer approval, the user replied “授权外发”. This explicitly authorizes sending the fixed eight-document/eleven-image payloads to the existing DeepSeek endpoint under the previously disclosed 3 CNY cap and no-retry plan. No requests were sent before this clarification.

§8.42 result: AC1 Apple-only product path aligned: withdrawn local-model UI, download IPC, catalog and llama.cpp runtime; release package verified, and real release UI/native file dialog on Spinoza PDF confirms only Apple status, modelNotReady basic-tree fallback, unaccepted review and disabled save. No PDF saved. AC2 executed after explicit “授权外发”: eight fixed DeepSeek Flash high-thinking calls, no retries, seven applicable, one output-token exhaustion (16384; empty final content); conservative charge estimate 0.8610180 CNY, below 3 CNY cap. Seven-document/157-matched-entry paired frozen-reference metrics: level 52.2293%→80.2548%, parent 52.2293%→79.6178%; 18 previously correct matched entries regressed on the Bonaventure control because the model made the printed CONTENTS header the tree root. The predeclared no-control-regression/all-eight-applicable criterion was not met. Source review also found a known dispute in the frozen agent Gold for Spinoza (all-flat reference despite bold Part headings); preserve raw scores and Gold, flag absolute metrics as provisional rather than silently regrade. AC3 diagnostic boundary aligned: no stronger-model production integration or reliable theoretical-upper-bound claim. Evidence: evaluation/toc-apple-cloud-ceiling-2026-09-11/.

### 8.43 Semantic bookmark constraints and twenty DeepSeek diagnostics (2026-09-11)
User source: “增加书签语义约束，明确告知例如‘目录’等字眼不能进入。deepseek应该能做好，授权3元20份检测”. SB1 amends the experimental fixed-membership restriction of AC2/LM1: classify each retained candidate by semantic role and exclude true TOC-page headings, column labels and page furniture only in a new derived bookmark projection. Raw candidate text, evidence, order and target/page references remain untouched; real Part/Chapter/section headings survive even without a page number. This is semantic context, not a word-substring blacklist. Excluded items cannot be parents; the document root is implicit. Uncertain roles cause abstention, not silent deletion. SB2 authorizes twenty DeepSeek cloud visual requests at a new 3 CNY hard cap. This continues the explicit source-image external-transfer authorization to the same existing DeepSeek endpoint/account, now for the fixed twenty-sample plan. Eight prior development cases plus twelve seeded additional old-development cases, never new holdout; one request per case, no retries or prompt iteration. Only sampled source images and existing measured geometry enter prompts, never Gold or baseline hierarchy/text. SB3: measure semantic false-entry exclusion, false exclusion of real headings, same-matched-entry parent/level transitions, full denominators and failures separately. Old agent-reviewed Gold remains immutable and its known Spinoza hierarchy dispute stays explicit; no human approval inferred. The predeclared development signal requires twenty usable results, improvement in paired bookmark hierarchy, removal of the known Bonaventure pseudo-root, and no new real-entry deletion or correct hierarchy regressions. Current product remains Apple-only; this experiment does not authorize automatic DeepSeek product integration. No broad cleanups, release/publishing/commit/PCC or additional paid run.
Agent implementation choices: new versioned module scripts/bookmarks/hierarchy_semantics; one semantic-role array plus one parent array indexed to fixed candidates. High thinking, max_tokens=8192 (reduced from prior 16384 so twenty worst-case reservations can fit the authorized 3 CNY); explicit failure if truncated. Rates retain the official pricing verified today, peak uncached USD0.30/M input and USD1.20/M output with conservative accounting multiplier10 CNY/USD. Fixed candidate count<=60, source group<=3 pages, current recovered page set matches old development metadata. Retain eight previous sources; choose twelve more from sorted eligible development IDs with seed2026091105, independent of score and new model output. Frozen inputs/runner/semantic contract/scorer hashes and budget preflight before paid dispatch; stop on HTTP/unknown charge/accounting overflow and preserve the worst reserve, continue planned remaining cases after a settled semantic failure.

Result: all 20 calls completed without retries, conservative usage-based cost 1.553319 CNY. Nine strict projections, nine empty outputs truncated at the output cap, two explanation-length-only protocol failures. Bonaventure excludes CONTENTS and Chapter/Page and preserves 18/18 correct hierarchy. Source audit found destructive projection of title continuation Amsterdam Circle and author continuation SoLMSEN as page furniture (raw evidence remains intact). Old-reference paired parent/level accuracy 110/133→108/133 is not a reliable quality regression: all 24 correct-to-wrong labels arise from source-supported SECTION and PHYSICS/BOOK grouping that the old reference flattened. No reference modified or post-hoc corrected headline score substituted. Execution/constraint implementation aligned to SB1–SB3; semantic content preservation drifted on the continuation cases; full quality and PDF target behavior unverified. Predeclared signal NO_GO_OR_INCOMPLETE, diagnostic task complete, no production readiness. Evidence: evaluation/toc-semantic-bookmarks-20-2026-09-11/report.zh-CN.md, evaluation.json, failure-diagnostics.json and immutable cloud receipts. Follow-up proposals are not additional paid authorization.

### 8.44 English desktop localization (2026-09-11)
User source: “给软件做一个英语适配”. EN1: add English to the active desktop workflow, including processing settings, contents editing, progress, validation and known local-backend notices. Agent implementation choice: retain Chinese, expose a persistent 中文 / English selector, initially use the saved preference or system language (Chinese for zh, English otherwise). EN2: language changes preserve open documents, edits, page targets, processing settings and explicit review state; never translate PDF content, source observations or user bookmark titles. Existing Apple-only/local processing, OCR-unavailable status, new-file saving and review gates remain in force. EN3: validate both locales, English workflow requests and layout; build a separate local bilingual application for review. This does not authorize publication, paid model calls or background tasks.

EN1–EN3 result: bilingual source and separate local app delivered; 65 frontend tests, lint, TypeScript and production/release builds passed. Actual release-window switching and preference persistence verified, final English home layout checked after wrapping fix. Document-state/save invariance is covered by mocked-IPC integration tests; native PDF open/save smoke is incomplete (file dialog Open remained disabled, origin unknown), not claimed as E2E. Evidence: docs/evidence/desktop-english-2026-09-11/report.md. Localization aligned; no change to provider, source content or review gates.

### 8.45 macOS release readiness and traceable commits (2026-09-11)
User source: “做到release ready。本机已经有apple developer certificate，证书在下载文件夹中。预计先只发布 macOS，ocr先关掉，根据这个还原可追溯的commit。” MR1 supersedes the prior cross-platform release target for this release: prepare macOS artifacts only, with body OCR unavailable and no cloud inference product path. Preserve local binarization, editable/reviewed bookmarks, Chinese/English switching and all source/output protections. MR2 authorizes reconstructing traceable local commits from the applicable existing work plus necessary release fixes, while preserving unrelated dirty OCR/research/Gold/evidence work. Use an isolated checkout and verify its actual dependency closure; do not treat a dirty developer build as a release commit. MR3 authorizes local certificate/identity inspection and use for Developer ID signing, and necessary release preparation/validation. Target release-ready artifacts with independently verified signatures, relocation/runtime closure and final app workflows. Notarization requires usable locally stored credentials; never expose/export private keys. Public release/push/upload of source or binaries to a public distribution channel is not authorized by this readiness request. MR4 records honest unmet gates; no human-review checkbox or independent Gold approval may be supplied on the user's behalf. Runtime licensing choice is unresolved pending the user's response; do not silently amend MIT/Apache distribution or purchase a license.

MR5 licensing decision: user source (2026-09-11) “保留现有 MIT/Apache 分发方向，替换 PyMuPDF 依赖（推荐）”. This resolves MR4's licensing question: replace the shipped PyMuPDF dependency, retain project MIT/Apache licensing, and include applicable dependency notices. Implementation choice: pypdf incremental outline writing and PDFium measured text/rendering; validate the new path independently and retain historical extraction evidence unchanged.

MR6 release-profile implementation decision: the first candidate is macOS arm64, with a declared minimum macOS 13 based on the audited mandatory native closure. The optional Apple image hierarchy helper is compiled with SDK 27 and launched only on macOS 27 or later when its model is ready. Actual OS testing is on this macOS 27 host, not an older-OS compatibility lab. The `macos-local` readiness profile traces MR1–MR5: committed build inputs, bundled runtime/notices/SBOM, functional checks, signed relocated GUI workflows, independent readers, replacement/preferences, privacy/accessibility/performance evidence, and Apple accepted/stapled/Gatekeeper validation. The old cross-platform and OCR-plugin profiles retain their original requirements. Cross-platform execution is superseded for this candidate by MR1; remote distribution CI remains not run and is not relabelled as passed. A locally verified, commit-bound build is this candidate's build evidence. Existing unsupported encrypted/signed bookmark inputs remain explicit refusals; synthetic navigation tests do not approve any real source or Gold record.
