# 更新日志

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [Unreleased]

### 新增（读屏无障碍阶段二：候选窗 ITextProvider，2026-08-19）
- **Windows 候选窗 ITextProvider / ITextRangeProvider**：uia_provider.rs 新增
  CandidateTextProvider 与 CandidateRange（#[implement] 完整 COM vtable）：
  DocumentRange / GetVisibleRanges 覆盖整条候选行文本（"1.你，2.你好…"），
  GetText 支持 maxlength 截断、GetSelection 返回空数组、SupportedTextSelection=None；
  GetPatternProvider(UIA_TextPatternId) 现在返回 ITextProvider。只读"全文范围"
  口径：逐候选偏移/选区/滚动等按 UIA 规范返回 E_NOTIMPL / 0，与 M11 验收报告一致。
- 测试：TSF 56（+5：DocumentRange 全文、GetText 截断、VisibleRanges 单范围、
  Selection 空数组、TextPattern 可达）；workspace 全绿；clippy 0 告警（顺带清理
  shurufa-host 5 项既有告警：asr mut、audio_capture 死代码/vec_box/fn 转型）。

## [1.2.0] - 2026-08-19

### 新增（v1.2-2 读屏无障碍：候选窗 UIA + Android TalkBack，2026-08-19）
- **Windows 候选窗 UIA Provider（阶段一）**：新增 uia_provider.rs，实现
  IRawElementProviderSimple（#[implement] 生成 COM vtable）：Name = 当前候选行
  文本（"1.你，2.你好…"，随候选刷新更新）/ ControlType = Text / IsEnabled /
  IsKeyboardFocusable；候选窗 wnd_proc 处理 WM_GETOBJECT → UiaReturnRawElementProvider，
  NVDA / 讲述人聚焦候选窗即可朗读候选。完整 ITextProvider（逐候选范围朗读/
  导航）按评估口径列为阶段二。
- **Android IME**：候选词 TextView 显式 importantForAccessibility = YES（配合既有
  contentDescription "第 N 候选词：X"），候选刷新发 TYPE_WINDOW_CONTENT_CHANGED，
  TalkBack 可朗读候选与序号；APK 已重建（v1.1.0 code 33）。

### 新增（v1.2-1 语音输入云端转写试点，2026-08-19）
- **真实录音（waveIn）→ 云端转写闭环**：audio_capture.rs（16kHz/16bit/单声道
  waveIn 回调采集，WAV 组装纯函数单测）+ asr.rs（OpenAI 兼容
  /v1/audio/transcriptions，手写 multipart，ureq 60s 超时，响应解析单测）。
- **设置中心语音页**：转写后端下拉（演示 stub / 云端转写）+ Base URL + 模型
  （默认 https://api.openai.com/v1 + whisper-1）；API Key 只从环境变量
  SHURUFA_ASR_API_KEY（回退 AGNES_API_KEY）读取，不落盘。
- **交互**：Ctrl+Shift+S 开始录音 → 再次按下/超时（max_session_secs）收尾 →
  转写 → Partial/Final → 既有书面语化与剪贴板粘贴链路复用；面板标题随后端
  显示（云端转写 / dev-stub）。
- 测试：host 61（+6：WAV 头/组装、multipart、响应解析、配置缺 Key 报错）；
  设置中心后端校验；options SpeechSettings 新字段 serde 兼容。实机麦克风/
  真实 API 留用户验证（无 Key 时面板明确提示）。

### 新增（v1.2-3 常用生僻字词库包 + GB18030 合规清单，2026-08-19）
- **常用生僻字 449 词库包**：从 base/ext/others 词库提取"出现但不在《通用规范汉字表》
  8105 的字"（按 25 亿字语料字频权重排序，437 字）+ 知名扩展 B 常用字补充（龘靐齉爩…
  12 字），拼音来自 rime-ice 41448 大字表。设置中心输入页「专业词场景」下拉新增
  「生僻字」；引擎集成测试 weng→齆 / nang→齉 可打、常规拼音不回归；运行时探针实测
  449 字中 362 个可直接打出。
- **修正 M10-1 机制（重要实测）**：librime 的 import_tables 对这些补充词库**不生效**
  （词条不进编译表，num_entries 不变；原 M10-1"场景词库部署即生效"实际靠 base 词库
  恰好收录场景词才成立）。场景词条与生僻字词条统一**内联进 rime_ice.dict.yaml** 修正；
  删除 schemas/scenario_*.dict.yaml 独立文件。
- **GB18030-2022 合规清单**：[docs/GB18030-2022-合规清单.md](docs/GB18030-2022-合规清单.md)——
  U+ 兜底任意码位 + 8105/449 拼音可打 + 41448 大字表按需启用，合规结论不变。

### 修复（悬浮球白屏，2026-08-19 实机复现）
- **dev 构建白屏根治与自救**：白屏根因 = 运行的是 target/debug/Shurufa.exe
  （dev 构建，devUrl 指向 http://localhost:1420 vite），vite 未启动时
  WebView2 加载失败 → 白屏 + 滚动条（2026-08-12 两次事故同源）。本次：
  - 新增 scripts/start-control-center.ps1：杀残留 dev 实例 →（可选 -Deploy
    提权部署 release）→ 拉起 ProgramData/shurufa/Shurufa.exe（release 内嵌
    ui-dist，无需 vite）；
  - scripts/update-all.ps1 -Cc 部署完成后自动拉起控制中心（-NoStart 仍跳过）；
  - 已实机验证：v1.1.0 release 悬浮球（38px 橙球 F）与菜单/子菜单正常渲染。

### 修复（host worker 启动 panic，2026-08-19 实机复现）
- **Skin::current() RefCell 双重借用**：ai_panel::warm_up()（M9-2 启动预热）在
  新线程首次调用 Skin::current() 时，with_borrow_mut 闭包内嵌套调用 load_with()
  （内部再 borrow 同一线程缓存）→ RefCell already mutably borrowed panic（exit 101）
  → worker 反复崩溃、同步端口 48632 起不来。重构为「先读缓存命中，未命中则释放
  借用后由 load_with 装载再读回」（Skin 为 Copy，无行为差异）；已实机验证 worker
  稳定运行，TSF 与 host 共用该文件同时受益。

### 实机联调（跨设备同步，2026-08-19 手机 23113RKC6C）
- **补齐 M8 遗留「双端实机联调」**：手机安装 v1.1.0（versionCode 33）debug APK
  （adb 安装 + IME 启用并设为默认），同 Wi-Fi mDNS 自动发现 → 配对（确认码比对）
  → 直连 192.168.31.172:48632（协议 v2，启用 msg_id）。
- 验证通过：**文本双端**（PC→手机标记文本手机日志确认；手机复制小说正文→PC
  host list 出现条目 + host 日志「收到 23113RKC6C 的剪贴板（124 字符）」）；
  **图片 PC→手机**（4537 B PNG 写入手机系统剪贴板）；**文件 PC→手机**（剪贴板
  文件投递手机）。活动流 sync-activity.json 正确落盘（direction=in/peer=23113RKC6C/
  status=ok），设置中心「最近同步」数据源闭环。
- **顺带修复部署陈旧**：Program Files\FOX 的 host/algo 为 8月16 旧版（无活动流
  记录/失败重试），已重建并替换部署（stop → 提权复制 → supervise 重启），并清理
  旧安装遗留的僵尸 peer 记录（旧指纹空连 42102 的日志噪音消失）。
- **修复重复投递（联调发现）**：浏览器/Office/微信等会把一次复制拆成多个格式、
  多次事务写入剪贴板（序列号各不相同但内容一致），旧实现每次 WM_CLIPBOARDUPDATE
  都会重复入库并广播（实机 .NET SetText ×2、SetImage/SetFileDropList ×3，手机收到
  多份）。listener 新增「内容指纹 + 1.5s 窗口」去重（覆盖文本/图片/文件，图片按
  捕获字节哈希），已实机复测文本、图片均只收到 1 次；use_count 也不再虚增。

## [1.1.0] - 2026-08-18

### 新增（M10-1 专业词模式，搜狗 16.2 场景词库同类）
- **场景词库包**：`schemas/scenario_{doctor,lawyer,code}.dict.yaml` 三套领域
  词库（医生：白细胞/心电图/处方…；律师：合同/诉讼/乙方…；代码：前端/
  接口/数据库…），部署即生效——实测 librime 编译 schemas/ 全部 dict，
  拼音可直接打出场景词（无需额外挂载）。
- **设置中心入口**：输入页新增「专业词场景」下拉（无/医生/律师/代码），
  保存后写 options.json（`scenario_dict`）并重建词典（deploy）。
- **实现结论（实测）**：librime 1.17 的列表 patch 不支持 `"+item"` 追加
  （会把 `engine/translators` 整体替换、拼音失效），故不做
  rime_ice.custom.yaml 挂载；场景词库以"文件部署即生效"交付。新增
  options 校验/默认单测、设置中心校验单测、引擎端到端测试（场景词可打
  + 常规拼音不回归）。

### 文档（M10 评估：无障碍/语音/GB18030/生僻字）
- 新增 [docs/M10-评估报告.md](docs/M10-评估报告.md)：四项 P2 项现状盘点与分阶段
  建议——生僻字（U+ 输入兜底已闭环 + rime-ice 大字库）、读屏（候选窗 UIA
  Provider 为中等工程，v1.2 计划）、语音（推荐复用 AGNES 通道做云端转写试点，
  本地 Whisper 留 v1.2）、GB18030-2022（不建议全量字库，U+ 兜底已满足输入）。

### 新增（M10-6 节日/节气提醒，搜狗 5.2 节气提示同类）
- **候选栏节气/节日提示**：输入 `jieqi` → 「今日节气：xxx」（无节气日显示
  「今日无节气」，comment 带日期）；输入 `jieri` → 今日公历节日（元旦/
  情人节/妇女节/劳动节/儿童节/教师节/国庆节/圣诞节）。节气日用 C 世纪常数
  近似公式（1901-2100，±1 天）；农历节日（春节/中秋）需农历换算，暂不含。
- **实现要点**：lua_translator 候选必须设 `quality=100` 才能突破 librime
  的候选质量过滤（拼音 initial_quality 2.0 会把低质量 lua 候选挤出/丢弃，
  已实测定位）；新增引擎集成测试（jieqi 必出「今日」候选、jieri 候选格式、
  常规拼音 nihao 不受影响）。

### 新增（M10 调试基础设施：Tauri MCP Bridge 接入）
- 设置中心接入 [hypothesi/mcp-server-tauri](https://github.com/hypothesi/mcp-server-tauri)
  的 `tauri-plugin-mcp-bridge`（feature `mcp-bridge`，默认关闭，不影响生产
  构建与测试）：`cargo tauri dev --features mcp-bridge` 时在
  `127.0.0.1:9223` 起 WebSocket 桥（localhost-only，插件当前版本无 token，
  仅绑回环地址），供 `@hypothesi/tauri-mcp-server` 连接做截图 / DOM /
  模拟输入 / 窗口调试。已实测：dev 启动后端口 9223 正常监听，且与
  tauri-mcp-server 0.12 的 driver 自动连接默认端口一致（自动连接只探测
  默认端口，桥必须对齐到 9223）。

### 新增（M10 困难项：简拼开关，部署期替代方案）
- **无简拼变体方案**：librime 1.17 speller/algebra 不支持条件规则（实测
  `option@jianpin:` 报 Error loading formula #13），无法热开关简拼；
  新增 `scripts/gen-nojianpin-schema.ps1` 生成去掉 abbrev 规则的
  `schemas/rime_ice_nojianpin.schema.yaml`（schema_id/name 同步改写），
  方案页切换 + 重新部署生效。
- **可部署性已验证**：引擎集成测试通过 default.custom.yaml 把变体列入
  schema_list 部署后，select_schema 可加载、全拼 beijing 正常出候选
  （功能不回归）；简拼开关效果依赖 prism 部署增量，留实机验证；设置中心
  方案项接入待 algo 部署管道支持 schema_list 注入后补充。

### 新增（M10 困难项：「？？？」表情触发，TSF 层替代实现）
- **Shift+/ 三连 → 🤔**：中文态、无组合下连续按 Shift+/ 时，TSF 接管
  按键——前两个上屏全角「？」，第三个上屏 🤔（文档呈「？？🤔」），再按
  循环、其他键重置。顺带修正中文态 Shift+/ 被既有直通分支上成半角 "?"
  的瑕疵（现在统一全角）。
- **为何是替代实现**（已实证）：rime-ice 中文标点下 `/` 立即上屏「、」，
  Shift+/ 亦逐键独立处理，librime 组合无法累积标点；TSF 自建组合会与
  引擎抢 composition，回退删除又侵入用户文档——按键级替换是零风险解。
- 新增 TSF 状态机单测（三连替换/重置）与引擎基线集成测试（`/` 上屏
  「、」、无「？？？」组合候选，锁定替代方案的前提事实）。

### 新增（M10 交互式配对 UI，替代终端 CLI）
- **设置中心配对向导**：「跨设备」页新增「发起配对」面板——输入对方 IP
  （默认端口 48632）后 host `pair-ui` 发起端连接对端，确认码经
  `pair-prompt.json` 回传并大号展示；两端确认码一致后点击「确认配对」
  （写 `pair-confirm.json`，token 防串扰），完成后自动刷新设备列表。
  新增 host 单测（配对状态机）与设置中心纯函数单测（结果优先/确认码/
  超时/空闲四态）；冒烟验证连接失败路径正确回写结果并清理临时文件。

### 修复（M10 困难项：文件发送失败重试补全）
- 发送侧新增 msg_id→原路径台账（PENDING_FILE_SENDS）：`send_file_path`
  成功即登记、FileTransferDone 成功即清理；发送失败时取回原路径生成
  `send_file` 重试载荷，设置中心「最近同步」失败条目出现「重试」按钮，
  重发走既有广播通道（文件已不存在/服务未运行会明确报错）。
  新增 host 单测（台账登记/取回/清理）。

## [1.0.0] - 2026-08-18

### 新增（M9-6 AI 划词场景扩展，搜狗 16.4 划词白名单同类）
- **划词应用白名单**：通用页新增「划词应用白名单」编辑器（每行一个 exe
  文件名，如 WINWORD.EXE / chrome.exe；留空 = 所有应用放行，兼容旧配置）；
  host 在 Ctrl+Shift+R / Ctrl+Shift+T 划词热键触发时按前台进程 exe 判定，
  未命中白名单则跳过并写日志（不弹面板、不干扰剪贴板）。白名单保存时
  大写规范化 + 去重 + 上限 50 项；options/设置中心 DTO 全链路单测覆盖。

### 新增（M9-5 AI 光标助手，搜狗 16.1c 光标唤起同类）
- **Word/WPS 光标场景识别**：AI 帮写面板呼出时读取前台进程 exe 路径，
  命中白名单（WINWORD / WPS / WPS Office / ET / WPP）后面板标题显示
  「AI 光标助手 · Word/WPS」，提交后草稿经既有通道粘贴到光标处（与
  输入锚点跟随 M9-4 配合形成"光标处唤起续写"体验）；其余应用保持
  「AI 帮写」标题不变。划词润色/翻译仍走选区热键。

### 新增（M9-4 窗口跟随，搜狗 16.1c/16.4 锚点跟随同类）
- **AI/划词面板随输入锚点移动**：面板可见期间（AI 帮写 / 划词润色 /
  划词翻译）由 200ms 定时器轮询目标窗口光标/插入点位置并重定位（保持
  面板宽高，光标不可用时回退鼠标位置），光标移动面板即跟随；面板隐藏
  自动停止定时器。

### 新增（M9-3 桌面快捷搜索，搜狗 15.4d 快捷搜索同类）
- **悬浮条「桌面搜索」**：主菜单新增入口，二级菜单内嵌搜索框（200ms
  防抖 / Enter 立即搜索）。三类结果：**应用**（用户+公共开始菜单 .lnk、
  注册表 App Paths 直连 exe，点击 ShellExecute 启动）、**文件**（桌面/
  文档/下载有限深度遍历 ≤3000 项，点击资源管理器定位）、**计算器**
  （算术表达式 `meval` 求值，如 1+2*3 / 2^10，结果一键复制到剪贴板）。
- 新增纯函数单测：算式识别与求值边界、.lnk 扫描（临时目录）、文件遍历
  命中（临时目录树）；`desktop_search` 一次 IPC 聚合三类结果。

### 新增（M9-2 AI 工具整合入口，搜狗 15.3/11.0 工具箱同类）
- **悬浮条「AI 助手」菜单**：主菜单新增 AI 助手项，二级菜单聚合
  「AI 帮写」（点击即唤起面板，与 Ctrl+Shift+W 同路）、「划词润色 /
  划词翻译」（热键引导，需先选中文本）、「AI 热键与开关设置」（直达
  通用页开关）。
- **外部唤起通道**：host 新增 `ai show` 子命令——FindWindow 定位
  ShurufaAiPanel 后投递 WM_AI_EXTERNAL_SHOW（WM_APP+73），后台服务
  启动时预热创建隐藏面板窗口，设置中心入口随时可唤起、无需先按过热键。

### 新增（M9-1 设置中心全新升级，搜狗 16.7 设置中心重构同类）
- **导航重构**：设置中心页面态新增左侧分组导航（概览 / 输入 / 效率 / 外观 /
  系统），12 个页面按功能分组、当前页高亮，不再依赖「返回菜单」逐层找入口。
- **全页搜索**：顶栏新增全局搜索框——按页面标签与页内面板关键词（共 12 页
  静态索引）实时过滤，下拉直达目标页（Enter 选首项 / Esc 关闭 / 点击跳转）；
  输入时只刷新下拉不整页重渲染，不丢焦点。
- **未保存提示**：皮肤 JSON、直达快捷、按应用选项、短语行编辑四类带保存按钮
  的编辑区在修改后打脏标记（侧栏圆点提示）；切换页面或收起为悬浮球前弹窗
  确认，避免误丢未保存修改；保存成功自动清除。

## [0.9.0] - 2026-08-18

### 新增（M8-1b 失败重试）
- **失败条目一键重发**：跨设备页「最近同步」中带重试载荷的失败条目（收到
  文本/图片/文件写入系统剪贴板失败）新增「重试」按钮；点击后设置中心写入
  `sync-retry-request.json`，host 常驻进程 2s 轮询执行：按载荷重放写剪贴板
  （文本 ≤512KiB、PNG ≤1MiB、文件按落盘路径），并回写一条「（重试）」新活动
  （成功/失败原因），随后清理请求与载荷文件。超大载荷或入库失败等不可重试
  场景不显示按钮；文件发送失败的重试需 msg_id→原路径映射（数据链路待补）。
  新增单测：活动条目 `retry_id` 往返与旧文件兼容、重试请求体形状；
  设置中心命令 `retry_sync_activity`。

### 新增（M8-5 皮肤包导入导出）
- **导出为文件**：设置中心「皮肤」页支持自定义皮肤一键导出为单文件 JSON
  （导出到 `%USERPROFILE%\Downloads\shurufa-skin-<名称>.json`，文件名自动
  净化防路径注入；导出前复用保存侧同一套 JSON/version 校验，保证导出文件
  一定能被重新导入）。
- **导入文件**：新增「导入文件」入口，本地选择 `.json` 皮肤包后经校验
  落盘为自定义皮肤并立即应用（webview 读文件文本 → save_skin → 刷新编辑器），
  无需手动粘贴大段 JSON；与「保存并应用」「重新加载」「导出」共用
  `validate_skin_json` 单一校验路径。新增后端测试（合法/非法 JSON、
  version 越界、缺少字段均拒绝）。

### 新增（M8-4 应用/网站直达，搜狗 15.2 灵犀候选直达同类）
- **直达候选**：输入触发码（如 weixin / baidu）时，lua 翻译器 `app_direct.lua`
  产出带标记候选（🖥 应用 / 🌐 网址，高质量置顶在拼音候选之前）；选择提交时
  TSF 识别标记并**启动目标且不落文本**（应用直接执行；网址交给默认浏览器）。
- **可配置**：设置中心「输入」页新增「直达快捷」编辑器（每行
  触发码/名称/类型/目标，类型 app|url，网址校验 http(s):// 前缀）；保存后
  写 `app-shortcuts.json` 并生成引擎 lua 快捷表
  （%APPDATA%\shurufa\rime\lua\app_direct_shortcuts.lua），引擎每次按键
  重新加载、改完即生效无需部署。新增单测（规范化去重/裁剪/id 分配、lua
  生成转义）+ 引擎集成测试（标记候选置顶、常规输入不受影响）。

### 新增（M8-3 剪贴板批量整理）
- **历史面板批量选择**：剪贴板历史页新增「选择」模式——行首复选框多选、
  全选（按当前搜索过滤）、批量置顶 / 批量取消置顶 / **批量删除**（两步确认）；
  选择态隐藏单条操作防误触，工具栏显示已选计数。新增 Tauri 命令
  `batch_set_pinned` / `batch_delete_history`（逐条执行并返回实际处理条数）。
  置顶/单删/清空未置顶等既有能力保留。

### 新增（M8-2 设备管理）
- **已配对设备面板**：设置中心「跨设备」页新增设备列表——设备名 / 指纹
  前缀 / **最近在线时间**（Peer 新增 `last_seen_ms`，连接成功即刷新）/ 最近
  地址；支持**重命名**（行内编辑，名称 ≤40 字符）与**移除**（两步确认防误删）。
  数据源为 peers.json（PeerStore），与 host 常驻进程即读即写共享。
- **发起配对暂用终端 CLI**：配对需确认码双向核对（host `pair` 子命令为
  控制台交互），设置中心面板给出操作提示（shurufa-host.exe pair <IP>），
  交互式配对 UI 待设计。新增 sync-core 测试（last_seen 落盘）与设置中心
  命令 list_peers / rename_peer / remove_peer。

### 新增（M8-1 跨设备同步状态可视化）
- **最近同步活动流**：host 记录跨设备收发事件（文本/图片/文件）到
  `%APPDATA%\shurufa\sync-activity.json`——方向（收到/发出）、类型、预览、
  **来源设备名**（对端 from_name）、状态（成功/失败+原因）、时间戳；最多
  保留 50 条（FIFO 裁剪）。覆盖：收到文本/图片/文件（含写入剪贴板失败标记）、
  文件发送应答（对方已接收/发送失败）。设置中心「跨设备」页新增「最近同步」
  面板展示活动流（图标/方向/来源/状态胶囊/相对时间，进入页面即刷新）。
  纯记录+展示层，不阻塞同步主流程；新增 options 单测 3 例（往返/单调 id/
  上限裁剪）。

## [0.8.0] - 2026-08-18

### 新增（功能优化）
- **多时机表情推荐（M7-9，搜狗 15.9「输入 okok/爱你 出表情」同类）**：新增
  `schemas/lua/emoji_timing.lua` lua_translator——精确输入码直接附加 emoji
  候选：okok → 👌、aini（爱你）→ ❤️、wanan（晚安）→ 🌙。与 OpenCC
  simplifier@emoji 互补：emoji.txt 无裸「爱你」，OpenCC 管不到非中文词
  触发，本模块补这两类。新增集成测试 4 断言（三触发码 + nihao 常规输入
  不受影响）。「？？？」触发需拦截 Shift+/ 按键（TSF 层），已记录待评估。
- **上下文调频（M7-8，搜狗 16.6 打字模型同类方向）**：主翻译器开启
  librime `contextual_suggestions: true`——依据上屏上文从 userdb 读取语境
  词频，对紧随其后的候选加权（如刚上屏「中国」则输入 renmin 时「人民」
  更靠前）。MRU（enable_user_dict）已在既有配置；此为语境维度深化。
  新增集成测试锁定：schema 可部署、上屏流程正常、语境加权不劣化候选排名
  （无上文 p0 / 上屏中国后 p1，断言 p1 ≤ p0）。
- **调研结论：简拼开关暂不可行（M7-8，已记录）**：librime 1.17 的
  speller/algebra **不支持**条件规则（实测 `option@jianpin:` 包裹 abbrev
  规则报 `Error loading formula #13`），纯 schema 无法实现「关闭简拼」。
  需引擎侧支持（librime 上游/自定义 patch），标记暂缓；本轮不引入
  UI 空开关。详见 docs/优化灵感库.md。
- **悬浮球不透明度（M7，搜狗 16.1 状态栏不透明度同类）**：设置页「通用」
  新增「外观」面板——悬浮球不透明度滑杆（30%..100%，默认 100%，改动即时
  生效）。存 options.json `general.ball_opacity`（serde 缺省 100、保存钳位到
  [30,100]），设置中心加载后/保存时经 Tauri `setOpacity` 应用（悬浮球与
  控制中心同一窗口，菜单/页面同透明度）。纯设置页 + 选项层改动，无需
  重启输入法。新增 DTO 往返/钳位测试断言（序列化 80、缺省 100、越界钳位）。
- **候选条右键菜单（M7，搜狗 16.3b 候选条菜单入口同类）**：候选窗右键
  候选弹出菜单——复制候选（CF_UNICODETEXT 写系统剪贴板）/ 从候选删除
  （引擎 Control+d 冷词丢弃）/ 降低词频（Control+j）/ 隐藏该词（Control+x）/
  打开设置中心。引擎动作经会话共享钩子走当前组合：service.rs 将 ImeClient
  升级为 Arc<Mutex> 供菜单与 TSF 共用同一会话（新连接会建新会话、空组合
  空转）；键序与真实按键一致（{Down}×index 移动高亮不提交 → {Control+d/j/x}）。
  注意：emoji 影子候选由 simplifier 在冷词 filter 之后附加，对 emoji 的
  删词/降频/隐藏与按热键行为一致地为空操作。新增单测 3 例（命令映射、
  剪贴板往返含宿主并发重试、菜单与多行布局互不影响）+ 引擎集成测试 2 例
  （删词/降频链路，非空防假阳性断言）。
- **修复 cold_word_drop 降频失效（实机排查）**：turn_down_words.lua 为
  空文件时 require 返回 true，processor/filter 的 `(_st and turn_down_words)
  or ...` 会取到布尔值导致 filter 崩溃、候选流清空（降频/隐藏全灭）。
  修复：processor 与 filter 统一优先 reduce_freq_words（实际写入的文件），
  turn_down_words.lua 补齐为合法空表；新增集成测试锁定降频链路。
- **多行候选面板（M7，搜狗 16.3b 候选条/多行候选同类）**：设置页「输入」
  新增候选面板模式下拉（单行候选条 / 多行候选面板），存 options.json
  `candidate_panel_mode`（TSF 每键热读，约 2 秒生效）。multi 模式下候选窗
  每行最多 5 个候选、9 候选排 2 行（5+4），窗口高度 = preedit + 行数×行高；
  命中测试按行映射（第 2 行点击/悬停正常），翻页滚动条 thumb 按多行内容
  高度定长；GDI/D2D/DComp 三渲染后端共享 `Item.row` 行号、零布局漂移；
  模式参与内容指纹，切换单/多行即失效布局缓存。新增真实窗口集成测试
  （9 候选→2 行、第二行命中、单行 90px < 多行 130px）。
- **符号面板搜索（搜狗/微信 emoji 面板搜索同款，2026-08-18 引入）**：
  符号面板顶部新增搜索框，实时跨全部 17 分类过滤——支持三种匹配：
  ① 常用 emoji 关键词索引（中文名/拼音/英文名 → emoji，精选 ~120 条高频
  emoji：微笑/weixiao/smile→😊、谢谢/xiexie/thanks→🙏、咖啡/kafei/coffee→☕）；
  ② 符号字符本身包含（搜"↑"或"心"字面命中的符号）；③ 分类名匹配
  （搜"箭头"列出整类）。搜索态隐藏分类页签与肤色条，平铺去重结果，点击
  复制行为不变；搜索词实时过滤（input 事件 + 重渲染后恢复焦点与光标，
  与历史记录搜索同款）。纯设置页 UI 改动（main.js + styles.css），安装即用。
  搜索逻辑经 node VM 桩测试 10 组断言（中/英/拼音/分类/字符/无结果）验证。
- **按应用 vim 模式（weasel app_options vim_mode 同款，2026-08-18 引入）**：
  按应用选项面板新增「vim 模式」开关——配置了 vim_mode 的应用（如 vim /
  emacs / 终端）下，无组合时按 vim 的「回 normal 模式键」（Esc / Ctrl+C /
  Ctrl+[）自动切英文直输，vim 才能拿到这些键进入 normal 模式（否则输入法
  留在中文态吃掉后续 j/k/l 等 normal 键）。实现镜像 weasel 源码
  （RimeWithWeasel.cpp:274-287）：有组合时跳过（Esc 由引擎取消组合，不抢
  不切）；前端本地判定（AppOption.vim_mode + 前台应用），零额外 IPC。
  设置页按应用面板每行新增开关 + 保存携带字段；纯函数
  `is_vim_normal_mode_key` / `app_vim_mode_enabled` 单测 2 例。兼容老数据
  （vim_mode 缺省 None = 不覆盖）。
- **Emoji 候选注释对齐（rime-ice 同款修正）**：`emoji` simplifier 的
  `inherit_comment` 从 `true` 改为 `false`——emoji 候选不再继承中文词的
  拼音注释（此前 schema 注释已写明意图但值写反，与上游 rime-ice 的
  `inherit_comment: false` 及其"emoji 的 comment 显示为空"设计不一致）。
  已部署到线上 FOX 安装并实机核对（xiexie → 谢谢 + 🙏 行为不变，仅注释
  显示更干净）。
- **Emoji 关键词联想验证 + 回归测试锁定（rime-ice simplifier@emoji 同款）**：
  调研确认 rime-ice **并无**独立"拼音→emoji"词典——emoji 候选来自
  `simplifier@emoji`（OpenCC emoji.json 文本词典）把**中文词候选**转换成
  附加候选（谢谢→🙏、微笑→😊）。该机制我们已完整落地（schemas/opencc/
  emoji.json + emoji.txt 为 rime-ice 全量 4858 行词典 + others.txt，配置与
  上游逐字节一致），实机验证 5 组词族全部命中：xiexie→🙏（紧随 谢谢 之后）、
  weixiao→😊、kaixin→😄、haha→😄+🐸（多 emoji）、zan→👍（单字词同样附加）；
  emoji 开关（switches/emoji，默认开）关闭后无 emoji 候选、恢复后回来（门控
  可逆）。本轮新增集成测试 `core/ime-bridge/tests/emoji_keyword.rs`（7 断言
  锁定上述行为，防止未来改动破坏），无功能改动——按调研如实报告已完整，
  未重复实现。
- **符号面板增强：emoji 分类 + 肤色记忆 + 最近使用 + 颜文字（搜狗 6.24.1
  「emoji 面板优化：分类、肤色多选及记忆、新增颜文字」同类）**：设置页符号
  面板从 11 个文本符号分类扩展为「文本符号 + emoji + 颜文字」三族——
  新增 表情（70+）/手势/动物/生活（食物·旅行·活动·物品）/爱心 五个 emoji
  分类与 颜文字（kaomoji）分类；emoji 分类顶部显示肤色选择条（默认 + 5 档
  肤色修饰符，搜狗「肤色多选」同款），选中的肤色实时应用到手势类 emoji
  （👍🏻👍🏼👍🏽👍🏾👍🏿）并本地持久化（「肤色记忆」）；点击任意符号自动记入
  「最近」页签（去重保留 30 个，本地保存，搜狗「记忆功能」同款），下次打开
  面板直接复用。纯设置页 UI 改动（main.js + styles.css），点击复制行为不变，
  无 TSF/引擎改动，安装即用无需重登录。ZWJ 组合 emoji（如 🧑💻）暂不支持
  肤色（变体规则需按位置插入修饰符，当前只覆盖最高频手势类），已备注在代码。
- **模式切换 toast 提示（微信/搜狗模式提示同类，成熟输入法方向 show_notifications）**：
  Shift 切换中/英、CapsLock 切英文、Shift+空格 全/半角、Ctrl+. 中/英标点
  生效时在输入锚点上方弹出轻量提示条（「英文直输」「中文输入」「全角」「半角」
  「英文标点」「中文标点」），2 秒后自动消失。候选窗不可见（无组合）时这是
  唯一的切换反馈通道——此前 Shift 切换中英文在无组合场景零反馈。实现：
  `platforms/windows/src/toast.rs` 独立小窗（WS_POPUP + TOPMOST|NOACTIVATE|
  TOOLWINDOW + 点击穿透），外观沿用皮肤（背景/文字色/DWM 圆角），位置跟随
  输入锚点、无锚点落主屏底部居中，焦点离开应用立即收起；5 处切换点接线 +
  纯函数定位单测 4 例。
- **划词翻译（Ctrl+Shift+T，微信/搜狗划词翻译同类）**：选中文本后按热键 →
  AI 翻译成中文（原文已是中文则译英文），面板回车覆盖选区。复用划词润色
  完整的"抓选区 + 面板 + 回车覆盖"链路：ai_panel 的 `mode_polish: bool`
  重构为 `PanelMode` 枚举（Write/Polish/Translate），翻译模式用独立系统
  提示 `SYSTEM_PROMPT_TRANSLATE`（与 AI 帮写模板无关）。新增
  `enable_translate_hotkey` 设置开关（默认开，通用页新增一行），热键门控
  位图扩到 3 位。依赖 AGNES_API_KEY（与 AI 帮写/划词润色相同）。
  调研结论：AI 帮写（Ctrl+Shift+W）/ 划词润色（Ctrl+Shift+R）在 wave 4
  已完整实现（ai_panel.rs 面板 + listener.rs 热键/门控轮询 + 设置开关），
  本轮验证完成态并补齐划词翻译；"边写边译"（输入时实时逐词翻译）评估后
  暂缓——与输入流冲突、复杂度高，划词翻译已覆盖主要使用场景。
- **中英混输自动空格（rime-ice en_spacer 同款，默认开可关）**：英文词上屏后
  再输入英文词，候选自动带前导空格（hello 上屏后 world → ` world`），
  中英混输不用手动敲空格。触发条件窄（上次上屏英文 + 本次候选纯英文词），
  误伤概率低。实现：`schemas/lua/en_spacer.lua`（挂了 `en_spacer` 开关，
  reset:1 默认开）+ filters 在 uniquifier 之前；设置页输入选项新增开关
  （engine_option 模式，与 Emoji 同款）。cn_en_spacer 评估后跳过：我们
  没有 rime-ice 的 cn_en 中英混输词典，无作用对象。新增集成测试
  （首次输入不加空格 / 英文后加空格 / 开关关不加）。
- **Unicode 输入（rime-ice unicode.lua 同款）**：输入 `U` + 十六进制码点 →
  对应字符（`U4f60` → 你、`U1F600` → 😀、`U03B1` → α），生僻字/emoji/
  特殊符号不再依赖词库收录。BMP 内码点附带按位遍历的变体候选（帮助从
  近似码点找字）。实现：`schemas/lua/unicode.lua` + recognizer/patterns
  `unicode: "^U[a-fA-F0-9]+"`（大写 U 前缀，不与辅码检字 uU 冲突；hex
  大小写都收，放宽 rime-ice 原版只收小写）。新增集成测试，实机验证通过。
- **符号配对（微信输入法同类，默认关）**：中文态、无组合时按 `(` `[` `{`
  `《` 自动补配对符并把光标居中（`()` `[]` `{}` `《》`）。纯 TSF 落盘
  （InsertTextAtSelection + SetSelection 光标居中），无引擎交互；放在
  Shift+可打印键分支之前（US 键盘上 `(` 是 Shift+9，否则会被截胡只插
  单个字符）。默认关避免与 IDE 自动补全/括号高亮冲突（微信默认同关）；
  设置页输入选项新增开关。新增配对表单测 + options 往返单测。
- **长候选缩写（weasel style/candidate_abbreviate_length 同款）**：单条候选
  超过 24 字符（皮肤 `abbreviate_length` 可调，0=关闭）时截断显示为
  `前缀…`，避免长词/长英文/日期时间戳/ID 撑爆候选行、把同排其它候选
  挤掉（配合既有 60% 屏宽封顶）。只影响显示——引擎按索引提交，上屏仍
  是完整文本。纯渲染层实现：候选窗布局时按字符数截断并实测宽度，D2D/
  DComp 两后端共用。
- **候选来源角标（P2 #14 部分落地，启发式）**：皮肤 `show_candidate_badge`
  开启时，候选按文本特征分类（EN=英文 / EMOJI / ◈=日期时间金额算式 /
  字 / 词）在文本右侧显示小角标。调研结论：librime 1.17 公开 C API 不
  暴露候选 type（RimeCandidate 仅 text/comment/reserved），且无云词库时
  "来源标识"的粗分类信息价值有限，因此**默认关闭**、按需开启。纯函数
  分类器 + 三路渲染共用，新增单元测试。
- **部件拆字辅码筛选（rime-ice search.lua，搜狗/微软候选内笔画/部件筛选同类）**：
  输入拼音出现候选后，输入辅码引导符（反引号 `）+ 部件码，按
  radical_pinyin 词典过滤候选（`nihao`ren` → 只留首字含 亻(ren) 部件的
  候选 你好/倪/伲/伱，拟/尼/妮 被过滤），生僻字选字不再靠翻页。
  实现：`schemas/lua/search.lua`（Mirtle 原版）+ `lua_filter@*search@radical_pinyin`
  挂 filters（uniquifier 之前，namespace=radical_pinyin 即 schema 反查方案）
  + `key_binder/search: "`"` + speller/alphabet 加 `（initials 不加，避免单独
  成码）。新增集成测试（nihao`ren 过滤 + 基线 passthrough），实机验证通过。
- **V 模式帮助（vhelp，rime-ice vhelp 浏览符号码同款思路）**：输入 `vhelp`
  列出本方案全部 V 模式触发码及说明（日期 rq/时间 sj/星期 xq/ISO dt/
  时间戳 ts/中文日期 rqzh/英文日期 rqen/金额大写 R/计算器 cC/农历 nl/N/
  辅码反查 uU/部件辅码 `），不用再翻文档记触发码。实现：
  `schemas/lua/v_help.lua`（lua_translator，注意 translator 用
  `(input, seg, env)` 三参签名，与 filter 的 `(input, env)` 不同）。
  新增集成测试（vhelp 列出触发码 + 不干扰正常输入）。
- **按应用自动英文（weasel app_options 同款）**：设置页新增「按应用自动
  英文」面板——按进程名（小写，如 `windowsterminal.exe`）配置进入该应用
  自动切英文直输、离开恢复进入前状态（终端/IDE 常用）。实现：
  options.json 新增 `app_options` 映射 + TSF 前台应用跟踪（复用
  is_secure_desktop 的前台窗口检测，提取进程名）+ 应用切换时按覆盖表
  set_option(ascii_mode)。纯决策函数 `decide_app_ascii` 全覆盖语义单测
  （应用没变不动作/有覆盖应用覆盖/离开恢复快照/识别失败宁可不触发）。
  设置页新增面板（增删行 + 保存），约 2 秒热生效。
- **英文自动大小写（rime-ice autocap_filter，成熟输入法更新日志方向）**：
  输入 `Hello` → 英文候选转首字母大写 `Hello`；`HELLo`/`HELLO` → 全大写
  `HELLO`；全小写 `hello` 不变；`Hel` 前缀联想也转 `Hello`/`Help`。实现：
  english.schema.yaml 的 speller/algebra 增加大小写派生规则（`\U`/`\L`，
  rime-ice melt_eng 同款），librime 构建 prism 时生成 Hello/HELLO/HELLo
  等变体编码使任意大小写输入都能命中词典；`lua_filter@*autocap_filter`
  挂 filters（corrector 之后、uniquifier 之前）按输入大小写转换候选文本。
  新增集成测试（Hello/HELLO/hello/Hel 前缀 + Nihao 不破坏既有行为），
  实机验证通过。
- **词汇别名（rime-ice 2025+「部分常用词自动展示翻译/别名/化学式/简称」）**：
  多个词条共享同一拼音编码，输入时一并出现（`aerfa` → 阿尔法 + alpha/α/A、
  `shui` → H2O/水分子、`beita` → beta/β）。词表
  `schemas/word_info.dict.yaml`（希腊字母全量 + 化学式 + 常见音译别名，
  约 120 条）+ `word_info.schema.yaml` 依赖方案（随 rime_ice 部署编译）。
  编码用**紧凑拼音**（aerfa 而非 a er fa）：table_translator 查询走 prism
  精确码命中（GetValue 单 key），与 script_translator 的音节图匹配不同
  （rime-ice cn_en 同款：`X光\tXguang`）。initial_quality 0.5 排在拼音
  候选之后，enable_completion false 仅精确码命中。新增集成测试，
  实机验证通过。
- **Emoji 候选（rime-ice 闲置资产，OpenCC 转换）**：输入中文词时附带 emoji
  候选（`weixiao` → 微笑 + 😊、`xihuan` → ❤️、`kaixin` → 😄），默认开启、
  开关在设置页输入选项（新增 engine_option_get/set 命令直连算法服务）。
  实现：`simplifier@emoji` 挂 filters + OpenCC 数据 `schemas/opencc/`
  （emoji.json 引用 text 词典 emoji.txt + others.txt，无需 .ocd2 编译）。
  简繁切换暂缓：s2t.json 需 4 个 .ocd2 二进制词典，librime 分发未携带。
  新增集成测试（默认开 → 😊 出现；同会话关闭 → 消失），实机验证通过。
- **辅码检字（部件拆字反查，P0 #2 收口）**：`uU` 前缀 + 部件码反查汉字
  （`uUheng` → 一、`uUbaishao` → 的），rime-ice 部件拆字方案（radical_pinyin
  词典 2.1MB/13 万词条 + affix_segmentor + reverse_lookup_filter）。部署器
  只编译主翻译器词典的坑已排：radical_pinyin 需同时具备 dict + schema 并
  列入 dependencies 才会被编译。新增集成测试（uUheng/uUbaishao 双断言），
  实机 pipe_drive 验证通过。
- **用户词库可视化管理（P1 #12）**：词库页新增「用户词库（本地学习记录）」
  区——列出各 userdb（名称/大小/备份数），支持**导出**（复制到
  `%APPDATA%\shurufa\userdb-backups\` 带时间戳）与**清空**（重置调频与
  自造词，删除前自动备份防误删）。不解析 leveldb 内部格式（非公开），
  以目录级快照交付备份/重置能力。
- **符号面板（P1 #11）**：设置页新增「符号」页——11 个分类标签
  （常用/箭头/数学/货币/单位/标点/表情/天气/音乐/棋牌/星座）+ 符号网格，
  点击即复制到剪贴板。数据取自 rime-ice symbols_v.yaml 常用子集；
  输入时 `/` 前缀符号码（/fh 商标、/1 数字）照常可用。
- **冷词丢弃/隐藏/降频（rime-ice cold_word_drop 模块）**：候选时 `Ctrl+D`
  强制删词（无视编码）、`Ctrl+J` 降频（词条移到第 4 候选位）、`Ctrl+X`
  隐藏当前输入码下的该词。processor 把词条写进用户目录
  `lua/cold_word_drop/*_words.lua`，filter 立即生效；数据文件可手工编辑
  （如 `{ "示例" }` 即永久丢弃）。新增集成测试（丢弃词不出现在候选）。
- **候选窗位置策略（Fcitx5/微软拼音同类）**：设置页「输入」新增位置策略——
  `跟随光标`（默认）/ `固定右下角` / `固定左下角`。固定模式忽略锚点、
  每次弹窗同一位置，且免去每键 GetActiveView/GetTextExt 的 COM 往返。
  选项存 options.json，TSF 每键热读约 2 秒生效；新增解析单测。
- **自定义短语可视化编辑器**：设置页新增「短语」页——编码/词条/权重三列
  表格增删改，保存写 `%APPDATA%\shurufa\rime\custom_phrase.txt`（格式
  与 rime-ice 官方一致：`词汇<Tab>编码<Tab>权重` + `#@/db_type tabledb`
  表头指令；权重 99 压过拼音候选置顶），「保存并部署」一键重建生效。
  新增引擎级测试（gs → 公司置顶），实机 pipe_drive 验证通过。
- **错音错字提示（rime-ice corrector.lua）**：输入读错的拼音时，候选旁
  comment 显示正确读音（`geiyu` → 「给予 (jǐ yǔ)」）；输入错别字时显示
  正确写法（纠错表覆盖馄饨/主角/角色/说服等常见错音与错字）。实现：
  `lua_filter@*corrector` 挂在 filters 首位 + `translator` 开
  `spelling_hints: 8` / `always_show_comments` / comment_format 用全角
  `［］` 包裹拼音供脚本提取匹配；未命中纠错表的候选 comment 自动清空，
  不留拼音噪音。新增集成测试（给予读音 + 无残留标记），实机验证通过。
- **候选窗刷新去重（内容指纹短路，weasel#1869 进一步）**：候选内容指纹
  （preedit/候选文本与副标/高亮序号/页码/中英全角模式/皮肤参数/DPI 的
  FNV-1a 散列）未变时，整帧跳过字体实测与 `InvalidateRect` 重绘——组合
  内容未变的按键（按住修饰键、重复键、锚点移动但内容相同）零窗口成本。
  几何未变仍跳过 MoveWindow/阴影同步（既有节流），三者皆变才全量刷新。
  新增指纹判定单测（相同内容同指纹、8 类可见变化各不同）。
- **引擎忙按键不排队（服务端 try-lock，weasel#1867 手段3 同类）**：算法
  服务多宿主并发时，某会话持锁处理长操作会让其他会话的按键排队阻塞
  （按键延迟）。新增 `Session::try_process_key` 非阻塞喂键：锁空闲等价
  普通路径；锁忙立即应答"未吃 + 空上下文"，客户端按键直通、下一键自然
  重试。正常单键持锁 <1ms，争用极少见，兜底保体验。新增冒烟断言
  （锁空闲返回 Some(eaten) 且上屏正确）。
- **V 模式快捷转写（rime-ice 官方 Lua 套件）**：沿用 librime-lua 通道启用四组
  转写能力——`rq/sj/xq/dt/ts` 日期/时间/星期/ISO 时间戳（date_translator.lua，
  日期候选 quality=100 压过拼音候选置顶）、`R`+数字 金额大写（number_translator.lua：
  R123 → 壹佰贰拾叁元整）、`cC`+算式 计算器（calc_translator.lua：cC1+1 → 2）、
  `nl` 今日农历与 `N`+YYYYMMDD 指定日农历（lunar.lua + lunar.db，含二十四节气
  与星期）。识别器模式（recognizer/patterns：number/calculator/gregorian_to_lunar）
  让大写/数字/符号经 recognizer 处理器直接入码，speller 字母表补全大写段。
  新增集成测试 5 项断言（日期/金额/计算器/指定农历/今日农历），实机 pipe_drive
  全链路验证通过。
- **候选窗深度自定义（皮肤参数化）**：皮肤 JSON 的 `metrics` 新增间距参数——
  `padding`（内边距）、`item_gap`（候选间距）、`label_gap`（序号与词间距）、
  `hl_pad`（高亮留白）、`row_h`（行高）、`preedit_h`（preedit 区高），均为
  基准 px、随 DPI 缩放、0=内置默认；配合既有 `font_scale`/`radius`/`opacity`/
  深色跟随，候选窗布局（show 宽度高度、GDI 绘制、命中测试、滚动条轨道）全链路
  读皮肤值。新增解析/钳制单测，v2 老皮肤文件零行为变化。
- **悬浮球菜单「重新部署」**：帮助菜单新增"重新部署方案（重建词典）"——宿主
  新增 `deploy` 子命令（rime_deployer 重建二进制词典，输出写到用户数据目录
  `%APPDATA%\shurufa\rime`，安装目录 schemas 只读也能跑），设置页新增
  `redeploy_dictionaries` 命令同步等待并带回编译结果；手动改 schema/词库后
  一键生效，无需重装。方案切换 toast、打开数据目录、系统输入法设置、启动/自愈
  服务等既有菜单项保持。
- **以词定字（librime-lua 打通，rime-ice 官方脚本）**：输入整词后按 `[` 上屏
  第一个字、`]` 上屏最后一个字（如 `zhongguo[` → 中、`zhongguo]` → 国）。
  这是 librime-lua 在引擎内的首次启用——经排查确认内置 rime.dll（1.17）
  已编译进 librime-lua，无需额外动态库；schema 侧以 `lua_processor@*select_character`
  挂在 processors 首位、按键绑定走 `key_binder` 的 librime 规范键名
  （`bracketleft`/`bracketright`）。新增集成测试覆盖首字/末字上屏与
  脚本搜索路径（user_data_dir/lua）。后续 V-mode、农历/日期等 lua 扩展
  均沿用此通道。

### 修复（实机使用体检发现）
- **cold_word_drop.filter 崩溃英文候选（实机回归）**：filter.lua 对无
  preedit 的候选（英文补全等）执行 `cand.preedit:gsub(...)` 抛 Lua 错误，
  导致候选流被丢弃、英文补全消失。修复①：nil 守卫回退到当前输入串；
  修复②：drop/hide/reduce 列表全空时直通不做重排；修复③：默认列表清空
  （原 rime-ice 演示内容含 NSFW 词，用户用 Ctrl+D/J 自行添加）。
- **Emoji 候选挤掉英文补全（集成测试回归）**：simplifier@emoji 为每个
  候选生成 shadow（inherit_comment: false 时 comment 为空），与带 ［拼音］
  注释的原候选并存 → uniquifier 无法去重 → 候选数翻倍 → 英文被挤出搜索
  窗口。修复：emoji 改 `inherit_comment: true`（shadow 继承原 comment，
  uniquifier 正常去重）+ english_mixing 测试搜索窗口 4→6 页（emoji 合法
  增加候选量）。
- **引擎启动全量重编译（librime#1077 同类）**：`RimeStartMaintenance(1)`
  每次启动全量比对/重建词典，慢启动。改为增量部署 `(0)`：只重建 mtime
  变化的 schema/词典；首装无 build 产物时 librime 仍会全量构建（等价
  首次行为）。安装器预构建 + host deploy 已覆盖 schema 变更场景。
- **每键候选窗锚点 COM 往返（weasel#1867 手段6 同类）**：TSF 每键
  `composition_anchor` 做 GetActiveView + GetTextExt 两次 COM 调用取
  锚点。组合对象未变时复用上次锚点（同一组合会话指针不变），跳过往返；
  固定位置模式下完全不取锚点。输入位置缓存字段在 Inner 上，跨键复用。
- **输入时候选框过大**：单行 9 个候选在长词/高 DPI 下横贯整个屏幕（此前实测
  候选窗可撑到近满屏宽），挡视线也看不清。现候选窗最大宽度钳制到屏幕宽度的
  60%，放不下的候选自动截断、靠翻页访问；窗口右侧钳制逻辑同步生效，保证
  候选窗不越出屏幕右缘。新增回归测试锁定该行为（真实窗口 + GDI 实测宽度）。
- **60% 屏宽钳制在缩放 >100% 下失效（候选窗仍横贯大半个屏幕）**：钳制上限用
  `GetSystemMetrics(SM_CXSCREEN)` 的**物理像素**（如 150% 缩放下 2560px）计算，
  而候选窗布局、文本测量与 MoveWindow 全程使用窗口 DPI 的**逻辑像素**（该屏
  1706px）——上限被放大 dpi/96 倍，150% 缩放下实际钳到 90% 屏宽，实测
  "xiu fu" 候选窗 w=1104 逻辑 px = 1656 物理 px（65% 屏宽）。现新增
  `logical_screen_dim`（物理 → 逻辑换算）并用于宽度钳制与右/下缘定位钳制，
  150% 缩放下候选窗最多 60% 屏宽（1023 逻辑 px）。回归测试同步按换算后的
  逻辑屏宽断言，新增换算函数单测（96/144/192 DPI 边界）。
- **安装器升级被运行中的进程锁死（升级卡在"写入 payload"步骤）**：实测
  `shurufa-algo.exe --once` 自检挂起时不退出，进程锁住 exe 导致安装器写入
  反复失败；且安装器杀掉 algo 后 host 的 supervise/自启动会在 1-2s 内把它
  重新拉起，再次撞上文件锁。两处修复：① `--once` 模式加 90s 超时 watchdog，
  挂起即强制退出（`[algo] --once 超时`），杜绝自检进程长期锁文件；② 安装器
  `stop_process` 从"单发 taskkill"改为"多轮 taskkill + tasklist 轮询确认 +
  WMI Terminate 兜底"，写文件重试时先再杀一轮宿主/算法进程并等待文件解锁
  （`wait_file_unlocked`），对抗进程被重新拉起的竞争。
- **打字莫名卡顿（每键同步磁盘 I/O）**：TSF 每键热路径上有两处同步磁盘
  访问——`debug_log` 每次按键写 2 次日志文件（实测 ~1.8ms，且多宿主进程
  并发 append 同一文件存在锁竞争，磁盘抖动时单次可飙到数十 ms），叠加
  `ui.show()` 每键重读皮肤文件 + JSON 解析（~1ms）。快速打字时每键 ~3ms
  阻塞，表现为"莫名其妙"的间歇卡顿。修复：① `debug_log` 改为**内存缓冲 +
  后台 500ms 节流落盘**（热路径零 I/O，缓冲上限 5000 行防日志风暴）；
  ② 皮肤加载改**带 mtime/长度校验的缓存**（文件未变直接复用，热切换皮肤
  仍生效）。新增缓存行为单测（首次加载/命中/文件改动重载）。
- **抗 CPU 降频（weasel#1250 同类）**：Windows 功耗管理把空闲的算法服务
  压到 0.5-1GHz，按键后频率爬升慢 → 间歇性"莫名其妙"卡顿。安装器（管理员）
  在安装时写 IFEO PerfOptions 高优先级（`CpuPriorityClass=3`，algo/host 每次
  启动自动 High，无需进程提权，不破坏 IPC 管道完整性），卸载时清除。
- **候选窗渲染热路径优化**：① GDI 字体缓存——show()/paint 每键创建 2-3 个
  HFONT（CreateFontW 系统调用）再删除，改为按字号缓存复用；② `MoveWindow`
  `bRepaint=false`——避免其立即触发一次 WM_PAINT 与下方 `InvalidateRect`
  重复全窗口重绘（每键两次绘制 → 一次）；③ **候选窗 UI 节流**（weasel#1869
  同类，实测长跑宿主下重复 ShowWindow/SetWindowPos 会随运行时间变慢，
  show 24→2.6ms）——几何未变不重复 MoveWindow、已显示不重复 ShowWindow、
  阴影壳只在几何变化时同步，每键省掉重复窗口系统调用。
- **超长输入串防护（weasel#649 同类）**：误触/粘贴造成组合 ≥64 码时，
  算法服务在喂下一键前自动清空组合（转纯字母直通，同微软输入法做法），
  防止 librime translator 在超大音节图上查找导致卡死。正常整句输入零影响。
- **延迟打点常驻 + 分析脚本**：按键→上屏延迟（LAT）默认写入日志（走内存
  缓冲零 I/O），新增 `scripts/analyze-latency.py` 输出 p50/p95/p99/max 与
  尖峰明细，卡顿排查直接出数据。
- **安装后引擎预热**：安装器启动 host 后经命名管道做一次 CreateSession +
  ToggleAscii 往返，把首键成本（会话/词典加载）移到安装收尾。
- **新增排查文档** `docs/输入卡顿排查.md`：卡顿分层定位、延迟分析用法、
  已知根因对照、Windows 11 系统级偶发延迟（休眠唤醒首输入 5-10s）排除法。

### 文档（2026-08-18）
- **新增 `docs/开发计划.md`**：以搜狗 2009-2026 更新日志时间线为依据的路线图
  （M7 候选条升级 → M8 跨设备/个性化 → M9 设置中心与 AI 整合 → M10 专业/无障碍），
  每里程碑映射目标版本、功能、搜狗依据与验收，附录含搜狗功能版本时间线速查。
- **新增 `docs/版本管理.md`**：SemVer 约定、0.x 阶段规则、version.json 单一事实源、
  版本节奏、分支/标签、产物与发布检查单（衔接 `docs/发布流程.md`）。
- **新增 `docs/文档管理.md`**：docs/ 目录分类、命名规范、文档生命周期（草案/生效/废弃）、
  SSOT 原则、交叉引用与变更记录规则。
- **README 同步**：状态徽章与当前开发版更新为 v0.7.1，文档导航登记三份新文档。

## [0.6.0] - 2026-08-15

### 修复（实机使用体检发现）
- **致命 bug：设置选「拼音」实际一直在跑双拼**。librime 新建会话的默认方案
  是 user.yaml 的 previously_selected_schema；创建会话时把 rime_ice 当作"默认、
  无需选择"跳过了 select_schema，一旦用过双拼，options.json 选拼音也始终走
  双拼引擎（凑词垃圾候选、英文混输全失效、输入体验错乱）。改为无条件
  select_schema，全拼回归真全拼。
- **双拼 schema 既有配置 bug**：主 translator 的整句/补全设置误落在
  full_pinyin 块（此前主 translator 只有 dictionary），双拼补全能力缺失。
- **输入方案热切换后新会话不生效**：options.json 变更已记录日志但会话仍用
  旧方案；现每个新会话严格按当前方案选择 schema。

### 新增
- **悬浮球（替代悬浮条）**：白色 44px 圆形 F logo，点击展开设置中心；
  右下角彩色中/En 徽标（中文=暖橙 / 英文=蓝）点击切换全局中英。剪贴板、
  语音、方案切换移入菜单与工具箱。小而美：微渐变白球 + 玻璃质感阴影。
- **英文混输（搜狗/微软拼音同款）**：内置 english 词库（1124 词，高频
  500000/常规 50000 权重）+ 全拼/双拼接入 english_translator（前缀联想、
  用户英文词学习）。实测 hello/help/world/zoom/email/app/hi/ok 英文首选；
  he/you/wo/man/ma 中文优先；nihao→你好、zhongguo→中国。
- **拼音纠错（近键级）**：enable_correction 启用 librime NearSearchCorrector，
  键盘相邻键错误自动纠正（如 nohao→你好）。转位类错误（zhogn→zhong）受
  librime 上游限制（官方构建禁用编辑距离纠错）暂不支持。

### 体验
- **候选窗悬停高亮**（GDI / D2D / DComp 三后端统一）：鼠标划过候选高亮，
  配色融合主题。
- **MRU 落盘移出热路径**：候选提频记录改为脏标记 + 后台 2 秒节流保存，
  高频打字不再每键同步写盘。
- **全拼首选质量**：修复双拼 bug 后，常用词句首选全部正确（你好/中国/我们/
  我要去北京/今天天气很好/我爱你…），整句与英文混输协同正常。
- **实机测试工具**：新增 pipe_drive（命名管道驱动，逐键/模拟喂入并打印候选）
  与 english_mixing 集成测试，回归有保障。

### 已知限制
- 拼音转位纠错（zhogn→zhong）不可用：librime 1.17 官方 Windows 构建把
  编辑距离纠错（EditDistanceCorrector）以 #if 0 禁用，仅剩 NearSearch。
- 双拼英文混输弱于全拼：双拼 2 键/音节切分把英文串切成不完整音节，命中大量
  中文凑词——双拼方案固有取舍，全拼（默认方案）已做到英文整词首选。
## [0.5.6] - 2026-08-14

### 新增（对比搜狗悬浮条参考图补齐）
- **悬浮条「中/En」状态指示**（搜狗条同款元素）：算法服务 ascii_mode 改为
  **全局语义**——任一会话切换（Shift / CapsLock / 悬浮条）后，所有应用在
  下一个按键自动跟上；悬浮条显示全局中英态，点击可切换（toast 确认）
- **AI 帮写 / 划词润色热键开关生效**（附录 K 接线兑现）：设置中心「通用」页
  的 enable_ai_hotkey / enable_polish_hotkey 此前只存不读；现由宿主每 2 秒
  轮询 options.json 门控，变化即反注册+重注册（默认全开，行为不变）

### 修复（用户反馈）
- **麦克风图标改为真正触发语音转写**：之前只打开设置页（名不副实）→ 经
  WM_APP 消息通知后台宿主呼出语音转写面板（与 Ctrl+Shift+S 同入口，非全局
  按键注入）；语音未开启时给出明确提示
- **移除重复的菜单入口**：悬浮条最右侧网格图标与 F logo 功能重复 → 删除
- **中/En 与 拼/双 切换改乐观更新**：点击立即翻转图标再等结果（失败回滚），
  不再整窗重渲染，切换干脆利落

## [0.5.5] - 2026-08-14

### 修复（悬浮条功能审计发现）
- **拖拽结束钳制不生效**：原生拖动循环会吞掉 JS mouseup，原先挂在 mouseup 上的
  "拖出屏幕自动拉回"永远不会触发（实测条可被拖到屏幕外）→ 钳制改挂 onMoved
  （最后一次移动的 onMoved 在拖动循环结束后落地），新增后端
  `clamp_window_to_work_area`（在位时 no-op，不与原生拖动抢位置）
- **skipTaskbar 配置未真正生效**：tao 在窗口创建（尚未显示）时调
  `ITaskbarList::DeleteTab` 大概率是 no-op，任务栏按钮仍会出现 → 窗口就绪后
  在 setup 里再次 `set_skip_taskbar(true)` 应用
- **悬浮条可被意外调大小**：无边框窗口 `resizable: true` 留有隐形调整大小边框，
  在条边缘附近拖动会误触 OS 尺寸调整（实测条被拖成整屏）→ 改 `resizable: false`

### 体验
- 悬浮条审计全项通过：F logo/网格开菜单、菜单项 hover 二级面板、工具箱/剪贴板/
  麦克风图标进对应页面、拼/双方案切换、Esc 与点击外部收起、拖拽移动、位置记忆
  与屏外恢复

## [0.5.4] - 2026-08-14

### 修复（悬浮条）
- **安装器完成页"启动控制中心"提权**：与 start_host 同源——提权安装器直接 spawn
  会让悬浮条以管理员窗口运行（跨提权层级无法正常交互）→ 抽出 `launch_as_user`
  （schtasks 一次性任务 + 交互用户受限令牌）统一给 start_host 与 run_fox 用
- **悬浮条位置记忆被隐藏窗口哨兵位污染**：窗口曾被隐藏时 onMoved 会把
  (-32000,-32000) 存进 localStorage，之后每次启动 restore 被钳到屏幕左上角死角
  → 前端调用前做 plausible 校验，脏值丢弃回退右下角
- 悬浮条误进任务栏（常驻置顶小条不需要任务栏图标）→ `skipTaskbar: true`

### 体验
- **Esc 层级收起**：菜单态按 Esc 收起为悬浮条；页面态按 Esc 返回菜单
- **拖拽结束自动钳制**：整条被拖出屏幕后松手自动拉回工作区，不会丢失

## [0.5.3] - 2026-08-14

### 修复
- **安装后输入法整体失效**（普通应用只出英文）：安装器以管理员直接 spawn 宿主，
  host→algo 链全部提权，算法服务以 High 完整性创建 IPC 管道，普通应用连接被
  完整性策略拒绝（err=5）→ 改为经 schtasks 一次性任务以普通用户令牌拉起；
  管道 SDDL 另加 Medium 完整性 ACE 双保险
- **Shift 按下即切中英文**：英文模式打大写字母时每次 Shift+字母 都会把模式切回
  中文组字 → 改为"按下挂起、松开/下一个键结算"；Shift+字母（大写/上档符号）直接
  上屏不进 rime，中文态打 "Hello 世界" 的 H 也立即上屏
- **Ctrl+. 切换中/英标点永不生效**：`OnTestKeyDown` 对带 Ctrl 的键一律放行，
  `handle_key` 里的切换分支永远收不到按键 → 补齐接管
- **候选"最近使用"提频是死代码**：MRU store/boost/record 已实现但从未接到请求
  路径 → 在算法服务应答前统一装饰（按提交前拼音记录、按当前拼音提频，高亮跟随）
- 防御双字：引擎返回上屏文本却声称未吃掉时，落盘即吃掉该键，防止应用重复处理

## [0.5.2] - 2026-08-13

### 修复
- 安装页进度条与步骤文字挨近（去掉右侧对齐）
- 悬浮条不能拖拽：控制中心缺 `core:window:allow-start-dragging` 权限 → 补 capability
- 悬浮条长度不匹配内容：启动时未按条尺寸调整窗口（appliedSizeKey 初始值问题）
  且 minWidth 300 挡住了条的目标宽度 → 启动强制 resize + minWidth 调至 120
- 安装后输入双字母（如 jjiinn）：旧版宿主进程残留 + 历次安装累积的孤儿 TSF DLL
  → 引擎安装后清理孤儿 shurufa_tsf-*.dll（只保留注册的那个）；旧宿主需重启清除

## [0.5.1] - 2026-08-13

### 修复
- 最终用户协议点不开（alert 被调试插件桩掉）→ 改页内莫兰迪弹层
- 安装器 exe 图标过丑 → 重绘莫兰迪陶土红 F 图标（7 尺寸 ICO）
- 安装进度条过短 → 加长至 420px
- 配置自启动失败"后台宿主不存在"：`register-host-startup.ps1`/`verify-install.ps1`
  点源 `Deploy-Shurufa.ps1` 时同名 `$InstallDir`/`$TargetDir` 参数被默认值覆盖
  → 改用独立变量 `$InstalledDir` 保存安装目录

## [0.5.0] - 2026-08-13

### 新增
- 品牌更名为 **FOX 输入法**，安装器与控制中心统一为莫兰迪色系（陶土红/雾蓝/豆沙绿/米白）
- 全新 **Tauri 自研安装器**（替代 NSIS）：欢迎/安装中/完成三页设计稿、真实安装引擎（payload 内嵌、rime 词典预构建、TSF 注册、AppContainer 权限、自启动、快捷方式、卸载器）、自动提权与 `/uninstall` 卸载模式
- **单实例**：一台机器只保留一个版本——安装器、控制中心、宿主/算法均以命名 Mutex 保证唯一；安装时自动检测并清理旧版目录
- 安装器引擎全程写 `install.log`，逐步自述命令与失败原因，便于排障

### 修复
- 安装器提权重启的空参命令导致**静默退出**（UAC 不弹出）
- `Deploy-Shurufa.ps1` 必选参数被点源时在非交互 PowerShell 报错，安装卡在"正在配置自启动"
- TSF DLL 被输入进程锁定时回退**唯一文件名**注册；孤儿算法进程未清理导致后续文件写入失败
- `Set-WinDefaultInputMethodOverride` 偶发永久挂起——默认输入法改直写注册表，外部命令加 60s 超时兜底

### 工程
- 控制中心品牌与主题改造（FOX + 莫兰迪），logo 由 S 改为 F
- 清理旧 NSIS 产物与脚本；`build-installer.ps1`/`set-version.ps1`/CI 全面改接 Tauri 安装器
- 版本单一事实源 `version.json` 覆盖安装器页面版本串；pics/ 设计素材不入库

## [0.4.1] - 2026-08-08

### 修复
- 候选词翻页越界、拼音中间光标插入错位、中英快速切换卡死遗留旧候选
- Android 面板在切换应用后残留旧候选、剪贴板同步延迟
- 安装前未关闭控制中心导致文件锁失败；后台服务停止后无法重启
- 桌面快捷方式工作目录错误；控制中心 IPC 与无障碍事件回调不可靠

### 工程
- Windows 版本统一为单一事实源 `version.json`，Android 追平至 0.4.1（versionCode 10）
- 引入共享部署模块 `installer/Deploy-Shurufa.ps1`，安装器与 `scripts/install.ps1` 走同一条代码路径
- 新增代码签名钩子（`build-installer.ps1 -Sign`）、CI 构建与 tag 自动 Release 流水线
- 补齐 GPL-3.0 `LICENSE` 全文；docs 文件名中文化以与产品界面一致

## [0.4.0] - 2026-08-03

### 新增
- Windows 控制中心（Tauri）：剪贴板历史、设备管理、皮肤、词库更新
- Android 微信输入法风格键盘布局（JSON 驱动渲染）、深色模式与按键反馈
- 跨端皮肤入口 `schemas/shurufa-skin.json`

### 修复
- 候选词滑动/展开、拼音光标定位、中文切换、桌面端 UI 响应式、Host 窗口闪现
- 安装器原位升级逻辑，TSF DLL 版本化避免系统锁定

## [0.3.4] - 2026-07-31

### 新增
- Android 主键盘 4 行布局对齐主流输入法，预览键盘与候选栏美化
- 安卓退格键上滑清空手势

### 修复
- 布局比例、长按连删、气泡按键反馈

## [0.3.0] - 2026-07-30

### 新增
- M4：Android↔Windows 跨设备剪贴板同步（局域网直连，文本与图片双向）
- Android 端历史面板与一键回填

### 修复
- 模拟器/真机下输入法启用、候选上屏与同步重连时序

## [0.2.0] - 2026-07-29

### 新增
- M2：Windows 剪贴板本地历史（监听入库、去重、搜索、留存策略）与全局热键呼出面板
- M3：Android 拼音键盘（librime 复用 + Rust JNI 桥 + 原生输入法服务）

### 修复
- 输入法无法切换/从键盘列表消失；引擎异步初始化与词典预编译

## [0.1.0] - 2026-07-29

### 新增
- M0/M1：librime FFI 桥接与 Windows TSF 文本服务最小可用实现，可注册并正常打字
