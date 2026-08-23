# 更新日志

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

## [Unreleased]

### 重构（2026-08-23，候选窗迁出宿主进程 S1/S2/S3）
- **S1 协议层**：`core/ime-ipc` 新增 `CandEvent` / `CandCommand` DTO 与帧
  编解码测试；`platforms/windows-ipc` 支持 `create_named` / `connect_named`
  命名管道，新增 `shurufa-cand` 事件管道 e2e 测试。
- **S2 ui 侧宿主**：`shurufa-ui` 新增 `cand_host`——多 client 候选窗池、
  无焦点顶层窗、皮肤复用 `windows-skin`、点击/滚轮回发 `CandCommand`；
  `--cand-selftest` 可本地/CI 自检（起服务→推 Show→断言窗口创建）。
- **S3 TSF 双路径灰度**：`options.json` 新增 `candidate_window`（默认
  `builtin`；`hosted` 启用独立进程候选窗）；`candidate_window.rs` 在 hosted
  时推送 `CandEvent`、连接失败自动回退内置；TSF 侧新增 `cand_client` 读取
  `CandCommand` 并 SendInput 合成选词/翻页键。
- **修复 LengthDelimitedCodec 前缀端序回归**：换库周切 tokio-util 后长度
  前缀默认大端，与既有小端线格式不兼容；显式 `.little_endian()` 并加回归
  测试。
- **候选窗 UI 冒烟测试**：新增 `scripts/test-cand-ui.py`（pywinauto），
  `shurufa-ui --cand-selftest` 保持候选窗可见约 2 秒供外部断言；候选文本
  写入窗口标题 + 新增 `cand_uia` UIA Provider，pywinauto 可做语义级断言；
  CI 在 Windows 构建后自动执行。
- **候选窗故障注入测试**：新增 `scripts/test-cand-faults.py`，覆盖多客户端
  并发候选窗、杀掉 `shurufa-ui` 后候选窗消失、重启后自动恢复；已接入 CI。
- **候选窗交互自动化**：新增 `scripts/test-cand-interact.py`，自动点击候选
  第 1 项并断言收到 `Select index=0`，再发滚轮上/下翻断言 `PagePrev/PageNext`；
  已接入 CI 与验收预检。
- **候选窗稳定性冒烟**：新增 `scripts/test-cand-stability.py`，反复多客户端
  Show + 杀进程重启恢复；已接入 CI 与验收预检。
- **管道断开清理自动化**：新增 `scripts/test-cand-disconnect.py`，客户端断开
  后旧候选窗隐藏、新客户端可恢复；修复 `peek_available` 返回 `Result` 以检测
  断管；已接入 CI 与验收预检。
- **候选窗全量一键跑**：新增 `scripts/run-cand-all.py`，一条命令依次执行
  Rust 单测、管道 e2e、UI 冒烟、交互、故障注入、稳定性、断开清理。
- **小白一键验收**：新增 `scripts/候选窗一键验收.bat`，自动检测 Python、
  自动构建 shurufa-ui、自动安装 pywinauto，然后跑验收预检；自动项通过后
  继续引导单屏手动验收（`scripts/test-cand-manual-guided.py`）。
- **S5 前置：全屏回退内置**：TSF 检测前台窗口覆盖虚拟屏 ≥95% 时，即使
  `candidate_window=hosted` 也走内置绘制，避免全屏应用下候选窗被遮挡/闪烁；
  新增 `is_fullscreen_rect` 纯函数测试。
- **S4 验收矩阵与预检脚本**：新增 `docs/候选窗迁出宿主进程-验收矩阵.md`
  与 `scripts/test-cand-acceptance.py`；有测试环境时先跑预检 + 自动项，
  再按矩阵逐项做手动验收，避免“有环境不知道测什么”。
- **便携候选窗验收工具**：新增 `shurufa-cand-tool` 单 exe（无 Python/无
  安装依赖），支持 `--selftest` / `--demo [秒]` / `--info`，适合拷到任意
  Windows 机器快速验证 hosted 候选窗。
- 方案文档状态更新为“S1/S2/S3 已实施，S4/S5 未开始”。

### 重构（2026-08-23，候选窗 S4/S5 与发布灰度管道）
- **候选窗迁出收尾**：默认 `hosted`（S5 翻转），删除 TSF 内置绘制路径与
  `candidate_window_d2d.rs`；hosted 支持右键菜单、固定位置、多行布局、
  完整皮肤、Tab 标识、AI 副标、异步 AI 刷新。
- **一键验收完善**：`test-cand-manual-auto.py` 改用 SendInput scancode，
  真实鼠标点击 hosted 首项，A/B/C/E 单屏项全自动通过；新增 S4 验收报告。
- **发布/灰度管道**：
  - `core/update` 灰度判定（版本比较 + 稳定哈希分桶）
  - `shurufa-ctl check-update / update-apply / update`（含下载进度）
  - `generate-update-manifest.ps1` 生成 update.json
  - Canary 自动构建 workflow，安装器写入 channel 配置
  - 计划任务自动更新脚本
  - `shurufa-ui` 常驻后台检查 + 系统托盘气球通知
  - 控制中心更新面板/横幅（含目标版本）

### 优化（2026-08-24，hosted 候选窗模式角标与 clippy 收尾）
- **hosted 候选窗右上角模式角标**：显示 `中` / `En` / `全` / `⇪`，
  角标右侧独立预留宽度，不遮挡候选；`ime_ipc::Context` 新增
  `caps_visual`（serde default 兼容旧帧），TSF 侧 `set_caps_visual`
  写入线程状态并在候选可见时立即重推一帧，Shift 长按视觉提示不再丢在
  hosted 路径外。
- **cand_host 单测**：新增 `模式角标_中英全角`，覆盖中文/英文/全角/
  长按大写四态与优先级。
- **clippy/fmt 收尾**：删除候选窗迁出后遗留的 `RIME_KEEP` 与
  `WM_AI_CANDIDATES_READY` 死常量；AI 缓存类型加别名；`shurufa-ctl`
  下载进度改用 `checked_div`；全工作区 `cargo clippy --all-targets
  -D warnings` 通过。

### 重构（2026-08-22，阶段 3 收尾：schemas 生成物出库）
- **三份构建期生成物出库**：`shurufa_t9.dict.yaml`（约 15 MB）、
  `jianpin_index.txt`（约 13.8 MB）、`rime_ice_nojianpin.schema.yaml` 不再提交
  Git，改为 `.gitignore` + 构建/部署前统一生成。
  - 新增跨平台 Python 入口 `scripts/regenerate-generated.py`，内部调用
    `gen-t9-dict.py` / `gen-jianpin-index.py` / `gen-nojianpin-schema.py`；
    `regenerate-generated.ps1` 改为薄封装。
  - 自动接入：`build-installer.ps1`、Android Gradle `syncSchemas`、
    `build-android.cmd`、`register-dev.cmd`、`update-all.ps1 -Schemas`。
  - CI 一致性校验改为对比 `schemas/generated-files.sha256`（小清单入库），
    不再对大文件做 `git diff`。
- **简拼上游穷尽验证补证据**：新增 `core/ime-bridge/tests/jianpin_switch.rs`
  测试，证明 librime 原生 `abbrev` 只对单音节生效、多音节简拼词（`lw` →
  另外/论文/礼物）不命中；因此 `windows-algo` 的前端 `jianpin_index.txt`
  仍是必要补丁。
- **记录 nojianpin 生成策略决策**：`*.custom.yaml` patch 覆盖层因 Rime
  `__include` 无法从列表中精准删除 `abbrev` 条目而不可行，继续保留完整副本
  生成，避免破坏引擎集成测试安全网。

### 重构（2026-08-22，阶段 4 第 1 批：core/ime-policy）
- **新增 `core/ime-policy` 零平台依赖策略层**：承接全局中/英状态机、
  超长组合防护、打字统计埋点、MRU 候选提频、简拼索引查询。
- **`windows-algo` 的 `mru.rs` / `jianpin.rs` 移入 `core/ime-policy`**：
  删除本地重复实现，`shurufa-algo` 改为复用 `ime_policy::{MruStore, JianpinIndex}`。
- **`core/ime-ipc` server 改用 `ime_policy`**：`GLOBAL_ASCII`、
  `is_overlong_composition`、`note_key` / `note_commit` 不再由传输层持有，
  为后续把 Windows 命名管道拆到 `platforms/windows-ipc` 铺路。
- 新增 13 项单元测试（全局中英、超长组合、MRU、简拼），全工作区
  fmt/clippy/ime-policy/ime-ipc/algo 验证通过。

### 重构（2026-08-22，阶段 4 第 2 批：ime-ipc 拆出 Windows 传输层）
- **新增 `platforms/windows-ipc`**：Windows 命名管道 `pipe`、算法服务接入
  `server`、管道 e2e 测试与示例全部从 `core/ime-ipc` 移出。
- **`core/ime-ipc` 变成纯跨平台协议 crate**：不再依赖 `ime-bridge` /
  `windows`，只保留 `Request` / `Response` / DTO / 帧编解码。
- **core/ 平台中立门禁清零**：`check-core-portable.ps1` 基线欠债从 1 降为 0。
- **CI 新增 `core-portable-check`**：Ubuntu 上用 Linux 原生 target 对
  `shurufa-options` / `ime-policy` / `ime-ipc` / `core-skin` /
  `clipboard-store` / `sync-core` 执行 `cargo check --locked`，
  真正把“core/ 必须非 Windows 可编译”变成机器门禁。

### 重构（2026-08-23，Android 历史面板迁 RecyclerView + Coil）
- **`ShurufaImeService` 历史面板迁 RecyclerView**：行视图滚动复用（此前
  100 条一次性 `addView` 进 ScrollView），行距改 ItemDecoration；适配器
  三种行类型（文本/文件/图片）样式与旧版逐行一致，跨次打开复用回收池。
- **图片缩略图改 Coil 按需解码**：IO 协程取 `ClipStore.imageData` 字节 →
  `load(ByteArray)` 按 THUMBNAIL_TARGET 采样解码，`memoryCacheKey` 用条目
  id 保跨滚动命中；行复用竞态以 tag 防错位。删除打开面板时一次性预解码
  100 张 Bitmap 常驻内存的 `PreparedHistory`。
- 新增 `ClipThumbLoader`（IO 取字节辅助）与历史作用域（onDestroy 取消）。
- 模拟器实测：seed 120 条文本 + 2 条图片历史，列表渲染/双向滚动/缩略图
  解码（红/蓝图块出现）全部正常，无崩溃。
- **候选窗迁出宿主进程方案定稿**（`docs/候选窗迁出宿主进程-方案.md`）：
  迁入 shurufa-ui + shurufa-cand 事件管道 + 五步灰度迁移；S1（协议层）
  可先行合入，实施排期为阶段 6。

### 验收（2026-08-23，Android 模拟器冒烟）
- 1.8.0/40 debug APK（含阶段4第4批四样依赖与 DataStore 版 KeyboardPrefs）
  在 emulator-5554 实测：安装/启动无崩溃；启用输入法后 `keyboard_prefs
  .preferences_pb` 落盘（DataStore 迁移路径在真设备执行）；键盘渲染正常，
  按键被 IME 截获，`ni`→你/呢/…、`nihao`→你好（含 emoji 候选）端到端通过。

### 修复与验收（2026-08-23，简拼开关实机验收）
- **简拼开关在隔离环境完成实测**（独立 APPDATA + rime_deployer 预编译 +
  `algo --once`）：开 → `lw` 出另外/论文/礼物（前端注入）；关 →
  `lw`/`bj`/`nh` 候选为空、`nihao`→你好 / `beijing`→北京 不回归；再开 →
  恢复。单字母 `j`→就 来自 `enable_word_completion`（rime_ice 上游特性），
  开关前后行为一致，UI 文案已按此校准。
- **修复变体方案编译反噬缺陷**：原变体沿用 `translator/dictionary:
  rime_ice`，librime 编译产物按词典名命名，编译变体会把 `rime_ice.prism.bin`
  覆盖成无 abbrev 版——正常方案的引擎简拼被静默关闭。生成器改为独立词典名
  `rime_ice_nojianpin`。
- **修复 import_tables 不传递缺陷**：词典壳只 `import: rime_ice` 得到空表
  （table.bin 仅 5KB、全拼无候选）；改为平铺镜像 rime_ice 的叶子词表
  （cn_dicts/* + shurufa_ext + rime_ice 壳自身的大写字母/数字注音），
  编译产物 29MB 与主方案同量级。
- **`algo --once` 忠实化**：与 serve 路径一致地按 options 选方案（含简拼
  开关）并应用前端简拼注入门控，冒烟结果可代表实机行为；注入逻辑抽为
  `apply_jianpin_injection` 共用。
- `rime_ice_nojianpin.dict.yaml` 纳入出库清单与 sha256 门禁；安装包已用
  修复后的生成器重建。

### 变更（2026-08-23，架构审视后续项收尾）
- **简拼开关全链路接入（M10 收尾）**：
  - `schemas/default.yaml` schema_list 注入 `rime_ice_nojianpin`（构建期
    生成，maintenance/deployer 随 schema_list 预编译）；
  - `options.jianpin_enabled`（默认开）新字段；
  - algo：`schema_id_for` 感知开关（拼音关简拼 → 无 abbrev 变体，未知值
    回退同样处理），`input_scheme_differs` 同时比较开关，前端简拼词注入
    按开关门控（否则切到变体后多音节简拼词仍会出现，开关形同虚设）；
  - 设置页「方案」页新增简拼开关（`set_jianpin_enabled` 命令 + checkbox，
    algo 2 秒 watcher 热切换，无需重建词典）；新增 algo 单测。
- **删除 core/sync 裸 JSON 兼容回退**（架构审视 §7.1 点名）：`read_msg`
  只接受长度前缀帧；原路径对 TLS 流单字节 `read_exact` 扫描大括号配平，
  最坏循环 1600 万次。`FrameFormat`/`*_with_format` 一并删除，配对流程
  与 duplex 不再透传格式。新增「裸 JSON 流被拒绝」测试。
- **iroh 迁移评估结论：不迁**（阶段 4 后履约评估，结论与触发条件记录于
  架构审视报告 §7.1）：收益边际被高估、迁移需三端同发 + 全量重新配对、
  现网无 NAT 打洞痛点驱动。
- TSF `input_scheme_differs` 与 algo 同步扩展（含 jianpin_enabled）。
- 验证：workspace clippy/fmt 干净；algo/options/settings/ime-bridge 测试
  通过（含 jianpin_switch 集成测试）；TSF 两个交互测试因本机宿主服务
  占用剪贴板环境性失败（与改动无关）。

### 工程与治理（2026-08-22，阶段 5：文档止血收尾）
- **CHANGELOG↔tag 对账门禁**：check-docs.ps1 新增「每个 git tag vX.Y.Z 必须有
  `## [X.Y.Z]` 条目」规则；上线即抓到 v0.4.2 已打 tag 但 CHANGELOG 漏记，
  按 tag 说明补记条目（标注追记）。
- **安装包产物名门禁**：md 中不得再出现旧产物名 `Shurufa-Setup`（CHANGELOG
  历史叙述豁免）；`Windows安装指南` / `发布流程` / `开发环境` / `版本管理`
  四份文档旧名全部改为 `FOX-Setup-*`。
- **`发布流程.md` 失败回退段重写**：删除对旧 NSIS 时代的过时描述，改为与
  `installer/shurufa.nsi` 实际行为一致的回退说明。
- 其余阶段 5 项（架构说明四处承诺、README 版本统一、gradle.properties 语义、
  验收报告降级为历史快照）已在阶段 0/1 完成，本次在架构报告补记执行状态。

### 重构（2026-08-22，阶段 4 第 6 批：shurufa-host 按故障域拆三二进制）
- **`platforms/windows-host` 改为 lib + 三个 bin**：
  - `shurufa-clipd`（数据路径）：剪贴板监听入库、同步 daemon、supervisor、
    自启注册，无 UI；
  - `shurufa-ui`（面板集合）：历史/AI/语音面板、全部热键与热键门控轮询、
    AI 面板预热，独立消息循环（窗口类 `ShurufaUiHost`），崩溃不影响数据路径；
  - `shurufa-ctl`（CLI）：历史库查询管理、写回剪贴板、配对、词库维护。
- **supervisor 新增 shurufa-ui 看护**：独立故障域重启（退避），停机令牌
  一并结束 ui；检测到独立运行的 ui 时跳过拉起。
- **跨进程协作**：`ctl copy` 写回经 clipd 监听窗口（release 也允许按类名
  FindWindow 跨进程发现）；设置页麦克风按钮改投 `ShurufaUiHost`；
  `ai show` 仍按 `ShurufaAiPanel` 类名跨进程唤起。
- **设置页与脚本/安装器同步改引用**：`windows-settings` 按子命令路由
  clipd/ctl；`Deploy-Shurufa.ps1`、`update-all.ps1`、`install.ps1`、
  `start-host.cmd`、`unregister-dev.cmd`、NSIS 脚本全部改为新进程名，
  IFEO 高优先级覆盖 algo/clipd/ui。
- 修复上一批遗留的 `SkinPaletteTest.kt` 类提前闭合导致的 JVM 测试编译失败。
- `cargo clippy/test`、core-portable、Android 单测全部通过。

### 重构（2026-08-22，阶段 4 第 5 批：安装器回 NSIS）
- **删除自研 Tauri 安装器（`platforms/windows-installer`，约 1,300 行 Rust +
  850 行 HTML/JS）**，回归 NSIS（weasel 同款技术栈）。
- **新增 `installer/shurufa.nsi`**：完整移植 engine.rs 的十步安装逻辑——
  停旧进程（多轮 taskkill + WMI Terminate 兜底）、清理旧安装目录、写 payload、
  TSF DLL 被占用回退唯一文件名（6 次重试 + GetTickCount 后缀）、rime 词典
  预构建、icacls AppContainer 授权、regsvr32 注册、孤儿 TSF DLL 清理、
  自启动、快捷方式、卸载注册表、IFEO 高优先级、schtasks 降权启动宿主、
  终态验证；卸载段对称还原（含 IFEO 三键清除）。
- **`build-installer.ps1` 改用 makensis**（本机需装 NSIS 3.x），产物仍为
  `dist\FOX-Setup-<版本>.exe` + sha256；`set-version.ps1` 移除 FOX 安装器
  派生点（版本号改由 `-DFOX_VERSION` 传入）。
- README / `docs/版本管理.md` 同步更新。

### 重构（2026-08-22，阶段 4 第 4 批：Android 引入四样）
- **新增依赖**：RecyclerView 1.3.2、Coil 2.7.0、kotlinx-coroutines-android
  1.8.1、DataStore Preferences 1.1.1（阶段 4 第 6 项；Compose 迁移可缓）。
- **`KeyboardPrefs` 持久化迁到 DataStore**：首次读取自动从旧
  SharedPreferences 迁移（`migrated_from_sp` 标记），save 保持 fire-and-forget
  语义（自有 IO scope）；`kb_*` 键仅该文件使用，无其他读写方。
- RecyclerView/Coil 的面板接入属后续迭代（候选/历史/表情列表回收化）。

### 重构（2026-08-22，阶段 4 第 3 批：core/skin 与 windows-skin）
- **新增 `core/skin`**：纯数据模型/解析（v1/v2、颜色、间距、滚动条），
  零平台依赖，可直接被 Windows/Android/测试复用。
- **新增 `platforms/windows-skin`**：Windows 专属的皮肤文件装载/mtime 缓存、
  DWM 圆角/沉浸式深色/阴影壳；通过 `SkinExt` trait 扩展 `core_skin::Skin`。
- **删除 `panel.rs:43` 的 `#[path]` 源码注入**：`windows-host` 的
  `panel.rs` / `ai_panel.rs` / `speech.rs` 改为直接依赖 `windows-skin`；
  `platforms/windows/src/skin.rs` 退化为 re-export 兼容层。
- **core-skin 纳入 CI 平台中立检查**，`check-core-portable.ps1` 保持基线 0。
- **Android `SkinPalette` 补齐 v2 全段解析**：读取 `candidate` 的
  `background` / `highlight_background` / `label` / `preedit` 与顶层
  `metrics` / `shadow`，JVM 单测覆盖。
- 新增 13 项核心解析/滚动条测试、2 项 Windows 缓存测试与 1 项 Android v2
  解析测试。

### 工程与治理（2026-08-21）
- **CI 恢复为可信信号**：此前 main 分支连续 14 次运行全红，README 却声称
  「clippy 与 fmt 零告警、约 240 项测试全绿」。实测四项声明全部不成立
  （clippy 6 处告警、fmt 41 处不合规、1 个测试失败、CI 全红）。本次修复：
  - 新增 `rust-toolchain.toml` 钉死 Rust 1.97.0，避免浮动 stable 的新 lint
    在与改动无关的时间点让 `-D warnings` 转红；
  - 修掉全部 clippy 命中（3 处 `too_many_arguments`、1 处 `if_same_then_else`、
    1 处未用变量、1 处死方法），并对全工作区应用 `cargo fmt`；
  - `.cargo/config.toml` 含作者本机绝对 NDK 路径却已入库，任何其他环境的交叉
    编译必然失败——改为 `.cargo/config.toml.example` 并 gitignore 实际文件；
  - CI 的 Windows 构建顺序此前与 `build-installer.ps1` 不一致，安装器 `build.rs`
    会因 payload 未生成而 panic——改为直接复用打包脚本，消除两套顺序；
  - CI 的 Android 作业下载 `rime-…-Android-arm64-v8a.7z`，但 librime 上游从未
    发布 Android 产物（该 URL 恒 404，curl 无 `-f` 时把错误页写进 .7z）。改为
    只跑 Gradle lint 与 JVM 单元测试——CI 只承诺能真正验证的事。
- **新增三条机器门禁**（`scripts/`）：
  - `check-core-portable.ps1`：`core/` 下的 crate 不得无条件依赖平台专属 crate。
    这是防止 `core/` 退化成文件夹的长期机制；当前记录 1 项基线欠债
    （`ime-ipc` 混装了 IPC 传输与 IME 策略），基线只允许缩短。
  - `check-docs.ps1`：文档内部链接可达、写死的「当前版本」等于 `version.json`、
    CHANGELOG 有当前版本条目。
  - `set-version.ps1 -Check` 扩展：新增「文本声明点」表，把 README 的状态徽章
    与「当前版本」段纳入校验。此前 `-Check` 只覆盖 4 个 JSON 白名单，导致
    `version.json`=1.8.0 / 徽章=0.8.0 / 正文=1.7.0 三方矛盾长期无人察觉。

### 重构（2026-08-21，换库周第 1 批：libloading + chrono + hex + uuid）
- **Windows 运行期 DLL 加载改用 `libloading`**：`ime-bridge` 不再手写
  `LoadLibraryW`/`GetProcAddress`/`transmute`；`self_module_dir` 保留（TSF
  场景仍需要按当前 DLL 所在目录解析 rime.dll），加载后有意保持库常驻，
  避免 `Library` drop 导致已取出的 `RimeApi` 指针失效。
- **打字统计日期算法改用 `chrono`**：`core/options` 删掉手写 Hinnant
  civil-from-days 及互逆/倒退实现，`today_utc`/`last_days_of` 直接用
  `NaiveDate`，测试改为验证 `pred_opt`/`succ_opt` 语义。
- **手写 hex 编码改用 `hex::encode`**：`fingerprint_hex`、文件 sha256、
  `sha256_of_file` 三处统一走成熟库。
- **自研 UUID v4 改用 `uuid::Uuid::new_v4()`**：删除基于时间戳+自增+熵的
  手写 `new_msg_id`/`rand_part`，消息 id 仍保持 32 位小写无连字符格式。

### 重构（2026-08-21，换库周第 2 批：JSON 原子读写合一）
- **`core/options` 五处 JSON 读/写模板合并为通用助手**：新增
  `load_json_from<T>` 与 `write_json_atomically<T>`，options/stats/
  favorites/sync_activity/app_shortcuts 的 load_from/save_to 全部改为调用
  同一实现；保留各自的 pretty/compact 与既有原子替换语义。
- **原子写改用 `tempfile::NamedTempFile::persist`**：`write_json_atomically`
  不再手写 `.json.tmp` 文件名，改为同目录临时文件 + 写入 + `sync_all` +
  `persist`，避免临时文件命名冲突并保留崩溃安全。

### 重构（2026-08-21，换库周第 3 批：clap CLI）
- **`shurufa-host` 手写 CLI 解析改用 `clap 4 derive`**：删除按 `args[0]`
  手工 match 和 `parse_arg`，全部子命令（run/supervise/status/list/search/
  clip-* / ai / chat / pair* / dict* / retention / 调试命令）改为声明式
  `Parser + Subcommand`，自动生成帮助、参数校验与 `--help`。

### 重构（2026-08-21，换库周第 4 批：tracing 日志）
- **`shurufa-host` 手写文件日志改用 `tracing + tracing-subscriber`**：`log_line`
  保留为兼容入口，内部改为 `tracing::info!`；subscriber 用 `Mutex<File>` 写
  `%TEMP%\shurufa-host.log`（`SHURUFA_LOG_PATH` 可覆盖），删除手写时间戳与
  每次 `OpenOptions`/`write_all`。`run`/`supervise` 启动时初始化日志。

### 重构（2026-08-21，换库周第 5 批：官方 Tauri 插件 single-instance）
- **`shurufa-settings` 手写 `Global\\FOXControlCenter` Mutex 改用
  `tauri-plugin-single-instance`**：删除 `is_single_instance()` 与启动早退，
  由插件保证单实例；重复启动时聚焦已有主窗口。

### 重构（2026-08-21，换库周第 6 批：arboard 剪贴板）
- **`shurufa-settings` 手写 GlobalAlloc/CF_UNICODETEXT 剪贴板写入改用
  `arboard`**：`write_clipboard_text_impl` 删除 Win32 内存句柄样板，
  直接 `Clipboard::set_text`。

### 重构（2026-08-21，换库周第 7 批：官方 Tauri 插件 window-state）
- **`shurufa-settings` 窗口位置持久化改用 `tauri-plugin-window-state`**：
  删除 localStorage 里的 `shurufa-window-pos` 读写；首次运行仍用
  `place_window_bottom_right` 落右下角（一次性标志位），之后由插件自动
  保存/恢复位置。

### 重构（2026-08-21，换库周第 8 批：官方 Tauri 插件 autostart）
- **`shurufa-settings` 悬浮条自启改用 `tauri-plugin-autostart`**：删除手写
  HKCU Run `FOXSettings` 键读写，`settings_autostart_info` / `set` 改用
  `AutoLaunchManager`；仍保留“已部署目录才允许自动开启”的 installed 判断。

### 重构（2026-08-21，换库周第 9 批：multipart boundary 改用 uuid）
- **`windows-host` ASR multipart boundary 改用 `uuid::Uuid::new_v4()`**：
  删除手写时间戳 boundary，减少随机源手写代码。

### 重构（2026-08-21，换库周第 10 批：bindgen 生成 librime FFI）
- **`ime-bridge` 手抄 librime C API 声明改为 bindgen 生成**：新增
  `build.rs` 从 `third_party/librime/dist/include/rime_api.h` 生成
  `rime_bindings.rs`，删除约 360 行手写结构体/函数指针表。
- 构建现在需要 libclang；`docs/开发环境.md` 已补充 LLVM/libclang 依赖。
- 兼容处理：保留 `Bool` 别名；生成的函数指针对空值用 `expect` 明确断言。

### 重构（2026-08-21，换库周第 11 批：ASR 改 reqwest multipart）
- **`windows-host` ASR 手写 multipart 改用 `reqwest`**：删除手写
  `build_multipart`/`random_boundary`，`transcribe` 改用
  `reqwest::blocking::multipart::Form` 构造请求并发送。

### 重构（2026-08-21，换库周第 12 批：ime-ipc 帧协议用 LengthDelimitedCodec）
- **`core/ime-ipc` 手写 `[u32 LE][JSON]` 帧编解码改用
  `tokio_util::codec::LengthDelimitedCodec`**：`encode_request` /
  `encode_response` / `decode_frame` 不再手写长度前缀与边界判断，
  统一走成熟 codec 并保留 `MAX_FRAME_BYTES` 限制。

### 重构（2026-08-21，换库周第 13 批：皮肤颜色解析用 csscolorparser）
- **`platforms/windows` 手写 #RRGGBB/#AARRGGBB 解析改用
  `csscolorparser`**：`parse_colorref` 同时支持 CSS 颜色名与 hex，
  减少手写进制转换；测试改用真正非法的 `#xyz` 验证回退。

### 重构（2026-08-21，换库周第 14 批：IPC 熔断退避用 backoff）
- **`shurufa-tsf` IPC 客户端手写冷却常量改用 `backoff::ExponentialBackoff`**：
  删除 `CIRCUIT_BREAKER_COOLDOWN_MS` 等常量，`note_failure` 推进
  `next_backoff()`，`note_success` 调用 `reset()`；冷却仍 2s 起步、上限 4s。

### 重构（2026-08-21，换库周第 15 批：AI SSE 改用 eventsource-client）
- **`windows-host` AI 帮写流式 SSE 改用 `eventsource-client`**：`call_agnes_stream`
  不再用 ureq 手读 `read_until`/手写 `data:` 行解析，改用
  `eventsource_client::Client` + `HyperTransport` 消费 SSE 流；
  `parse_sse_line`/`SseEvent` 保留为 `#[cfg(test)]` 历史单测。

### 重构（2026-08-21，换库周第 16 批：WAV 头改用 hound）
- **`windows-host` PCM→WAV 组装改用 `hound`**：删除手写 44 字节 RIFF 头
  `build_wav_header`，`pcm_to_wav` 用 `hound::WavWriter` 写头与采样。

### 重构（2026-08-21，换库周第 17 批：Android AI SSE 改用 OkHttp-SSE）
- **Android `ShurufaImeService` 流式 AI 改用 OkHttp SSE**：新增
  `okhttp` / `okhttp-sse` 依赖，`callAgnesChat` 不再手写
  `HttpURLConnection` 逐行读 `data:`；改用 `EventSourceListener`
  消费增量，取消路径改用 `EventSource.cancel()`。

### 重构（2026-08-21，换库周第 18 批：waveIn 改用 cpal）
- **`windows-host` 麦克风采集改用 `cpal`**：删除 `waveIn*` 全家桶与
  `WAVEHDR` 缓冲管理，`AudioCapture` 改为 cpal `build_input_stream`
  回调累积 PCM；优先选择 16k 单声道 I16/F32 配置。

### 重构（2026-08-21，阶段 3 第 1 步：本地词条移出 rime_ice.dict.yaml）
- **新增 `schemas/shurufa_ext.dict.yaml`**：把 `rime_ice.dict.yaml` 内联的
  专业词/生僻字/拆字等本地词条移出，主词典改为 `import_tables` 挂载
  `shurufa_ext`；Android 构建同步拷贝该文件。
- **`gen-rare-dict.py` / `gen-t9-dict.py` 去除硬编码仓库路径**：生僻字输出到
  `shurufa_ext.dict.yaml`，T9 生成器支持命令行传入仓库根目录。
- **重新校准 `rime-ice-2026.06.30.json` sha256/size**：按仓内实际词典文件更新
  base/ext/8105 三项（此前 3/4 与本地不符），并补充 `upstream` 标记。
- **新增 `scripts/regenerate-generated.ps1`**：一键重新生成 T9 词典、
  简拼索引、无简拼 schema 三份构建期产物，为后续“生成物出库”铺路。
- **CI 增加生成物一致性校验**：`consistency` 作业重跑生成脚本并用
  `git diff --exit-code` 检查三份产物是否与脚本输出一致。
- **记录 rime-ice 上游 commit SHA**：`rime-ice-2026.06.30.json` 新增
  `upstream_commit = 6810e8916d160498620a16fef2135956fecbd485`。
- **`gen-t9-dict.py` 纳入 `shurufa_ext.dict.yaml`**：生僻字/拆字等本地扩展
  词条进入 T9 词典，避免本地词条移出主词典后 T9 丢失这些字。

### 删除（2026-08-21，换库周同期纯删除）
- **移除 DirectComposition 第三套候选窗渲染后端**：`candidate_window_dcomp.rs`
  约 1060 行整体删除，候选窗渲染收敛为「D2D + GDI 兜底」两条路径；同步移除
  `candidate_window.rs` 中的 DComp 选路/初始化/重绘/尺寸通知/皮肤刷新分支，
  以及 Cargo 的 `Win32_Graphics_DirectComposition` feature。
- **移除自研虚拟键盘（软键盘）**：`onscreen_kbd.rs` 861 行整体删除，同时移除
  `listener.rs` 里的 Ctrl+Shift+K 热键注册与 WM_HOTKEY 分支。Windows 自带
  `osk.exe`/`TabTip.exe`，不再维护第三套 GDI 面板。
- **移除 `solar_terms.lua` 二十四节气近似公式**：该脚本自陈误差 ±1 天，与
  `lunar.lua + lunar.db`（GB/T 33661-2017 标准）重复；删除后从两个 schema
  移除 `lua_translator@*solar_terms`，原集成测试改为验证 `nl` 农历链路。
- **移除 vendored `@tauri-apps/api`（约 9,116 行）**：安装器 UI 已启用
  `withGlobalTauri`，`tauri-api/` 整目录删除；`mcp-plugin.js` 改为直接使用
  `window.__TAURI__` 全局 API，三个 HTML 的 import map 一并移除。
- **根目录 `sougou.txt` 移入 `tools/research/`**：一次性逆向研究数据归位，
  `docs/开发计划.md` 引用同步更新。
- **移除 `emoji_timing.lua` 的中文触发项 `aini → ❤️`**：`爱你 → 🤟` 已由
  OpenCC `emoji.txt` 覆盖，同一输入不应出现两种表情；保留 `okok`/`wanan`
  等 OpenCC 覆盖不到的非中文串，集成测试同步收紧。

### 修复（2026-08-21）
- **辅码检字（uU 部件反查）多部件码完全失效**：`recognizer/patterns/radical_lookup`
  为 `^uU[a-z]+$` 不含撇号，而 `radical_pinyin.schema.yaml` 的 algebra 自 c12a576
  起改为保留 `'` 作部件分隔符（词典编码即 `bai'shao`）。两者不匹配的后果是
  **两种写法都打不出**：带撇号的 `uUbai'shao` 匹配不到 pattern、拿不到 tag，整串
  落回普通拼音（实测出「白芍/白沙」）；去掉撇号的 `uUbaishao` 又与词典编码对不上。
  修复：pattern 改为 `^uU[a-z']+$`。单部件码（`uUheng` → 一）本就正常。
  集成测试同步补上「无撇号必须不命中」的回归护栏。
- **英文候选词表含 120 个重复条目且 `suggest` 不去重**：输入 `wor` 返回四个
  一模一样的 `work`，把 `world` 挤出候选槽。修复：词表去重（689 → 569 条），
  并在 `suggest` 内用 HashSet 在过滤阶段去重（不能用 `Vec::dedup`——按长度排序
  后同长的重复词之间可能夹着别的词）。新增两条测试锁定该行为。
- **`set-version.ps1` 用 `-Encoding ascii` 写 `gradle.properties`**，已把该文件的
  中文注释永久毁成问号，且每次 bump 重复破坏（同文件内就有正确的
  `Write-Utf8NoBom`）。修复并恢复被毁的注释。
- **候选窗 Tab 标签用系统 DPI 而非窗口 DPI**（`draw_tab_label` 内部调
  `GetDpiForSystem()`），多显示器不同缩放时 Tab 内边距会算错。改为由调用方
  传入窗口 DPI。
- **候选窗 show 路径每次多两次无用的 `GetSystemMetrics` 系统调用**：
  `compute_show_layout` 的 `_screen_h` 参数从未使用，两处调用点却仍在计算它。
- **架构审视 S1：IPC 读超时后不重置连接，导致应答永久错位一格**：协议无请求
  ID，超时后同一管道上会残留服务端迟到的一帧应答，下一次请求读到的就是上一键
  的结果。修复：`roundtrip` 读失败/超时即丢弃连接，下次请求自动重连。
- **架构审视 S2：卸载时用 `split_whitespace().last()` 解析注册表安装路径**：
  默认目录 `C:\Program Files` 含空格时只取到 `Files`，卸载静默空转却报「卸载
  完成」。修复：以 `REG_SZ` 类型列为界取右侧整段路径。
- **架构审视 S3：候选窗隐藏后 LAST_CTX 未清空，空格/数字键误提交上一个简拼词**：
  隐藏时同步清空 LAST_CTX 与 AI 候选快照，提交拦截不再命中上一帧。
- **架构审视 S4：引擎不可用时兜底路径把 Backspace/Enter/方向键/标点全落成空格**：
  `fallback_commit` 现在只直出字母/数字，其余按键返回交给宿主进程处理。
- **架构审视 S5：出站广播仅 64 槽，大文件传输必因 `Lagged` 丢块中止**：广播容量
  提升到 1024+128，足以容纳 64MB 文件 v3 的全部 Chunk（64KB×1024），并保留
  文本/图片/控制消息余量。根治方案（每连接独立通道/流控）仍见路线图阶段 2。
- **架构审视 S6：读帧被 Ping 抢占后丢弃半帧，大帧传输必断连**：`duplex` 原先在
  同一个 `tokio::select!` 里 poll 读帧和 Ping tick，Ping 到点会取消正在进行的
  `read_msg_with_format`，已消费的半帧数据从 TLS 流中丢失。现拆成独立读任务 +
  主循环写侧：读任务只读帧，需要回写的应答经 `mpsc` 交回写侧，Ping 不再打断读帧。
- **架构审视 S7：无简拼方案的自定义短语配置过期**：重新运行
  `gen-nojianpin-schema.ps1`，`rime_ice_nojianpin.schema.yaml` 的
  `custom_phrase` 与源方案对齐（tabledb + `词条<TAB>编码`），并同步补齐近期
  源方案的 solar_terms/纠错/辅码变更。
- **架构审视 S8：长按剪贴板历史条目用 AlertDialog 必崩 IME 进程**：IME Service
  无窗口 token，改为与候选长按菜单同款的 PopupWindow。

### 文档（2026-08-21）
- **修正 `架构说明.md` 的四项失实承诺**：Compose 键盘（实为自绘 Canvas View）、
  WiX 安装器（实为自研 Tauri 外壳）、uniffi 绑定（实为手写 JNI）、以及最关键的
  一条——「weasel/trime 为 GPL-3，直接复用需 GPL 兼容开源」。**本仓自身即
  GPL-3.0**，该许可障碍从不存在，而这条错误前提正是约 6000 行候选窗框架重写的
  起点。新增 §8「架构约束（CI 强制）」与 §8.2「决策记录」规范。
- **修正 `版本管理.md` 与实现相反的描述**：该文档把 `gradle.properties` 列为
  「由 set-version.ps1 自动同步的派生点」，而实现是主动拦截该文件里的版本行
  （Android 由 `build.gradle.kts` 构建期直读 `version.json`）。照文档操作会直接
  锁死整条 CI。改为区分「结构化派生点 / 文本声明点 / 构建期直读」三类。
- **`文档管理.md` 新增 §8 机器门禁与 §9 验收报告定位**：确立「写不进
  `check-docs.ps1` 的文档承诺就不要写进文档」原则；15 份 `M*-验收报告.md`
  统一标注为历史快照（保留可追溯，不代表当前状态）。
- **README 修正**：状态徽章 v0.8.0 与正文 v1.7.0 → v1.8.0；产物名
  `Shurufa-Setup-*.exe` → 实际的 `FOX-Setup-*.exe`；删除「构建机需要 NSIS」
  （仓内已无 `.nsi`，安装器是自研 Tauri 外壳）；删除指向未入库文件
  `.claude/verification-report.md` 的死链；工程现状改为陈述 CI 门禁构成。
- 修复两处坏链接：`docs/M10-验收报告.md` 中多写一层 `docs/` 前缀；
  `docs/开发计划-Android.md` 中用全角 `）` 闭合 markdown 链接。
- 新增 [架构审视与选型替换报告](docs/架构审视与选型替换报告.md)：全仓 10 个子系统
  测绘、99 条细节缺陷、约 12000 行可替换自研代码的分档清单与改造路线图。

### 新增（2026-08-21）
- **候选服务 Tab（M7-5 P1+P2，Windows）**：候选窗顶部 Tab 行「拼音 | 英文」
  （仅英文候选非空时显示）；内置 ~500 高频英文词表前缀联想（≥2 位 ASCII
  字母触发，最长 5 条）；Rime 无候选时自动切英文组；点击 Tab 即时切换
  （三渲染后端 GDI/D2D/DComp 均实现）；英文候选点击经提交钩子落盘。
  方案见 docs/M7-5-候选Tab多服务切换-方案评估.md。
- **前端简拼索引（搜狗同款，2026-08-21）**：librime 原生不支持多音节简拼词
  （简拼音节不参与词条匹配，单字简拼/完整拼音正常）——由 scripts/
  gen-jianpin-index.py 从词库（cn_dicts base/ext/others）自动生成简拼映射
  （27 万编码/60 万词条，模拟 rime-ice abbrev：zh/ch/sh 双字母、其余首字母），
  algo 启动加载 jianpin_index.txt；引擎候选为空且输入为纯辅音串（2-8 位）时
  注入简拼词候选（lw→另外/论文/礼物、wsh→晚上/完善、wm→我们、bm→部门）。
  **选中提交**：简拼词不是 librime 候选，数字键/空格/点击选中时经编辑会话
  直接落盘（不走引擎数字选词，修"选中上屏拼音"双字问题）；候选窗点击走
  AI 同款提交钩子。真机 chrome 验证：lw+1 → 上屏「另外」。

### 修复（2026-08-21）
- **拼音简拼垃圾候选（lwyg 出“了/可/刻/克/乐”）：librime 1.17 在 `enable_correction: true` 且 speller 未显式配置 correction 规则时，启用 NearSearchCorrector（键盘相邻键纠错）兜底，把无匹配的简拼串（lw/wg/yw/wm）按编辑距离映射到邻近拼音（lw→le/ke、wg→e+g），产出与输入无关的候选；同时 w 声母简拼（我/万/无）被 e 声母（饿/呃/恶）挤掉。修复：speller 显式追加 rime-ice 上游的 spelling_correction/key_correction 规则（带 `/correction` 标记），并关闭 `enable_correction`——规则纠错（zho→中、dagn→大）保留，键盘相邻乱纠消失；w 简拼恢复（w→我/哇/无/外/为/问/王）。
  已部署 FOX 部署根（schemas 同步 + rime_deployer 全量重建 + algo 重启），引擎与真机 chrome 验证：lw/lwyg/wg/yw/wm 候选干净为空，完整拼音（laiwanyugeng→来玩鱼羹）、单字简拼（l→了/来、w→我/哇）正常。
  - 说明：librime 简拼音节不参与多音节词条匹配，纯简拼词（如 lwyg）需完整拼音/混合输入（laiwanyugeng），词库自定义短语另行支持。
- **AI 候选跳过拼音简拼**：纯辅音串（如 lwyg，无 aeiouv）视为简拼缩写，
  跳过 AI 预测——用户输入简拼要的是词库候选，不劳 AI（should_skip_ai，
  单测覆盖 lwyg/wyg 跳过、nihao/wo/lv/he 正常）。
- **中英文混合输入（严查 shift 后修复）**：
  - **Enter 被无条件吞掉**：AI 候选提交的 return true 在 pending 判断之外，
    导致所有回车键（无组合换行 / 有组合引擎选词）都被输入法吃掉——改为
    仅 pending_ai 非空（AI 候选点击回发）时消费 Enter，正常回车放行。
  - **Shift 切英文丢拼音**：end_pending_composition 此前 set_composition_text("")
    清空组合再 EndComposition——中文组合（如 "nihao"）按 Shift 切英文时拼音被
    丢弃；改直接 EndComposition（TSF 语义：组合文本保留在文档），Shift 提交
    拼音原文落盘（主流输入法一致）。
  - **AI 提交清 Shift 挂起**：AI 候选提交直接 return（跳过 Shift 挂起结算），
    若此前 Shift 按下未结算会残留导致后续按键误切换——提交时主动清除挂起。

### 新增（2026-08-20）
- **AI 候选预测**：输入暂停约 800ms 后基于拼音与上文调 agnès 预测候选
  （🤖 标记，排在候选行尾部，点击直接上屏）；设置 → AI 智能 → 开关（默认关，
  需 API Key，开启后输入内容送云端）；无 Key/失败静默。方案见 docs/AI候选预测方案.md。
  - Android：AiCandidateManager + Service debounce 注入（2026-08-20 首版）。
  - **Windows TSF 对照落地**：ai_candidates.rs（prompt/parse/fetch + AiWorker
    worker 线程，sync_channel(1) 800ms 停顿收最新 preedit → agnès 8s 超时 →
    PostMessage 回候选窗）；候选窗 show 合并引擎前 6 + AI 至多 3（🤖 副标，
    单行模式 AI 恒在第二行、Rime 放不下不波及），AI 结果到达按 LAST_CTX
    快照重建布局（compute_show_layout 复用）；点击 AI 候选走 AI_COMMIT 钩子
    写 pending + 回发 Enter（chrome 只路由文本键；仅 pending 非空时消费）经
    TSF 编辑会话落盘（不走引擎数字选词）；修复 D2D/DComp SetDpi 放大布局
    导致第二行被裁（改 96 1:1，本机端到端实测 2 行候选 + 提交上屏）；开关
    ImeOptions.ai_candidates（设置中心「输入 → AI 候选预测」），key 读环境
    变量 AGNES_API_KEY（与 AI 帮写面板同源，永不落盘）。

## [1.8.0] - 2026-08-20

### 新增（借鉴搜狗输入法，2026-08-20）
- **展开候选编号（UI-2）**：展开候选列表带小号灰色序号（1-9 直选提示），
  参照搜狗编号候选模式；模拟器实测「1 来 2 那 3 里 4 老 5 啦」。
- **滑动上屏（P1）**：字母/数字键上滑输入副字符（如 q 键上滑输入 1），与长按互斥；
  模拟器实测上滑 q → 字段精确上屏「1」。
- **候选栏滑动翻页（P1）**：候选行左滑/右滑翻页（dispatchTouchEvent 覆写，
  候选词可点击不拦截手势）；模拟器实测「来那里老啦 ⇄ 了年女两力」双向翻页正常。
- **按键气泡预览（P0）**：按下字母/数字/笔画键时，键上方自绘圆角气泡显示
  放大主字符与右上角副字符（如 q 键的 1），松手隐藏；顶部行无空间时气泡
  自动翻转到键下方（方向自适应，参照搜狗 KeyboardPopupView /
  DirectionalKeyboardPopupView 设计）。KeyPopupView 自绘 Canvas，可随
  深色皮肤切换；模拟器实测普通行/顶部行两种方向均正常。
- **模式切换竖栏（P2）**：候选区右侧常驻 拼/九/笔/双 竖栏，一键切换
  拼音全键盘 / T9 九键 / 笔画 / 双拼，当前方案实心高亮（参照搜狗候选栏
  右侧功能竖栏）；模拟器实测三态切换（QWERTY ⇄ 九键 ⇄ 笔画）即时生效。
- **设置分组标题统一（P2）**：设置面板按组分类（布局与显示 / 按键反馈 /
  单手模式 / 工具栏），小号灰色加粗分组标题，参照搜狗 SogouCategory 模式；
  模拟器实测分组显示正常（中英双语）。
- **自研 Preference 控件库（P3）**：新增 SettingControls.kt 可复用控件集
  （分组标题/开关行/单选组/链接行/说明副文本/分隔线，参照搜狗
  com.sogou.lib.preference 38 类控件库），设置面板全部改用控件库构建；
  候选字大小行顺带显示当前值（如 140%）。后续新增设置页直接复用，样式统一。
- **拆字方案（P4-3）**：注册 radical_pinyin 独立方案（部件反查，bai'shao → 部件字候选），
  修复 Android 增量部署不编译附加词典的问题（nativeDeploySchema + 解包时清 build 缓存），
  符号页新增撇号键作部件码分隔符。
- **输入风格预设（P4-5）**：候选数 经典 5 / 高效 9 单选（设置 → 布局与显示 → 输入风格），
  引擎 page_size 9 + 主行截断控制显示条数，展开列表始终全量。
- **符号页中文标点行（P4-6）**：数字符号页底部新增常用全角标点行
  （，。、；：？！……——《》），一键输入；设置 → 布局与显示 →
  「符号页中文标点行」开关可显隐（引擎 punctuation 映射已有，补键盘入口）。

### 真机验收（Redmi K70，2026-08-20）
- **P1-3 表情混排回归**：wo → 候选「我😊 握 窝 卧 起」，点击候选上屏正常。
- **P2-1 模式竖栏**：拼/九/笔/双 一键切换（QWERTY ⇄ 九键 ⇄ 笔画）即时生效，
  竖栏右侧常驻渲染正常；T9/笔画键盘布局切换正确。
- **P2-2 设置分组 + P3 控件库**：设置面板按组渲染（布局与显示 / 按键反馈 /
  单手模式 / 工具栏），候选字大小行显示当前值（100%），开关/单选组/链接行正常。

### 修复（真机验收，2026-08-19）
- **候选上屏失效（真机发现，P0）**：RimeBridge.nativeSelectCandidate / nativeChangePage
  在 Kotlin 声明为 external，但 rimejni 从未实现（git 历史亦无记录）；点击候选触发
  UnsatisfiedLinkError 使进程重启，候选永远无法上屏。补齐两个 JNI 实现
  （select_candidate_on_current_page + commit / change_page），并抽取
  session_context_string 供 nativeContext / nativeChangePage 共用。真机实测：
  拼音 nihao → 点击候选上屏「你好」、候选栏展开正常。

### 修复（模拟器联调，2026-08-19）
- **面板互斥**：设置/短语/快捷插入/表情/计算器/方案等开关面板时只隐藏各自的
  旧面板，计算器开着时开设置会出现双面板叠加；新增统一 hideAllPanels()，
  所有面板开关与 onStartInput/onFinishInput/图片预览共用，互斥生效。
- **工具栏显隐失效（M-A5 回归）**：ToolbarPrefs.resolve 把被用户隐藏的项当
  「缺失的新项」补回，隐藏开关永远无效（UI 会复活被隐藏的入口）；新增
  toolbar_hidden 持久化，显隐与排序分离，隐藏项不复活、新版本新增默认项
  仍补在末尾；ToolbarPrefsTest 增至 7 项（含隐藏不复活 / 新项补齐用例）。
- **rimejni 部署自死锁**：nativeInit 持 SESSION 锁期间调用 apply_input_scheme
  （内部重入 SESSION 锁），引擎就绪后方案切换全部阻塞；改为先 drop(session)
  再应用持久化方案，模拟器实测引擎就绪 ~40s、engineReady=true。
- **方案切换 ANR 防护**：引擎部署（约 40s）期间 handleDebugCommand 方案命令
  已加 engineReady 门控，避免主线程 select_schema 卡死弹 ANR。
- 新增 Debug APK 自动化验收工具（DebugUiActivity 前台驱动 / 面板、方案、
  键序命令 + 脚本），模拟器全流程回归：T9 九键打字、笔画键盘、短语/表情/
  计算器/设置面板互斥、工具栏 9→8→9 显隐、计算器 7+3=10 上屏均通过。

## [1.7.0] - 2026-08-19

### 新增（安卓 M-A5：跨端生态与设置——工具栏自定义 / 设置极简 / 专业词 / 链接直达，2026-08-19）
- **工具栏自定义**（搜狗 20.10/20.11，P0）：功能行 9 个入口可显隐 + 排序，
  ⚙️ 设置面板「工具栏」小节（⇧⇩ 排序 + 显隐 Switch），逗号分隔持久化、
  顺序保真；ToolbarPrefs 纯逻辑 + JVM 单测 5 项。
- **设置极简**（搜狗 20.3.1，P1）：MainActivity 新增「键盘设置与小秘籍」卡片
  （引导 ⚙️ 入口 + 六方案/长按语音/长按删除等技巧）。
- **专业模式**（搜狗 20.2，P1）：设置面板说明行——医生/律师/代码/生僻字场景
  词库已随包内联启用（M10 机制，Android 同词库引擎测试已覆盖）。
- **链接直达**（搜狗 20.9，P2）：剪贴板关键内容提取的网址 chip 长按直接用
  浏览器打开（ACTION_VIEW + NEW_TASK）。
- 词库每日更新（20.12，P2）：沿用既有 CloudDictionaryUpdater 云词库更新链路，
  增量拉取留实机验证。
- 验证：ToolbarPrefsTest 5 项；JVM 全量单测通过；APK 构建成功（实机安装待
  adb 重连）。

### 修复（UIA Provider 引用计数，2026-08-19）
- 全量回归暴露的 0xc0000374 堆损坏：上次"泄漏一个引用"只覆盖一次 from_raw
  包装，第二次调用 provider() 后对象即被释放；改为静态持 1 引用 + 每次调用
  前 vtable AddRef（8a4166e），全量 40 组回归全绿。

## [1.6.0] - 2026-08-19

### 新增（安卓 M-A4：AI 助手深化——帮写风格/场景、快查、书面语化、翻译，2026-08-19）
- **AI 面板模式**（搜狗 11.48/11.49/20.2/8.21）：新增模式 chips——帮写 / 快查 /
  翻译 / 书面语化；帮写模式再展开风格（默认/幽默/文学/极简/古风/搞怪）与场景
  （通用/朋友圈/种草笔记/爆款标题/聊天润色/长文）chips，选择注入 system 提示。
- **AI 快查**（11.49）：诗词/汇率/公式/天气等查询直接给简洁答案，可粘贴。
- **书面语化**（20.2 语音转写升级）：口语→规范书面语润色模式（系统语音识别
  结果可直接贴入二次润色）。
- **翻译**（8.21 / 20.2）：中英/多语互译模式，只输出译文。
- 提示词构造抽为 AiPrompt.kt 纯函数（模式/风格/场景三段式），JVM 单测 4 项；
  callAgnesChat 增 system 参数；APK 装实机冒烟无崩溃。真实 API 调用留用户验证。

## [1.5.0] - 2026-08-19

### 新增（安卓 M-A3：无障碍与生僻字——触觉输入 / 笔画键盘 / 拆字 / 文字转语音，2026-08-19）
- **触觉输入**（搜狗 11.13.1）：长按连续删除振动分层——启动 LONG_PRESS、每 tick
  CONTEXT_CLICK、删空组合末位强振（LONG_PRESS）；HapticProfile.kt 纯逻辑 + 单测 3 项。
- **笔画键盘**（搜狗 11.13.1 生僻字键盘 / 1.6 笔画输入）：方案面板新增「笔画」，
  stroke 方案接入 Android（schema_list + gradle 资产 + rimejni 列表）；键盘渲染
  五笔画键（一丨丿丶乙 → h/s/p/n/z）+ 数字直选行 + 底栏；引擎测试：h → 一。
- **生僻字拆字**（搜狗 11.13.1「牛牛牛→犇」/ PC 4.1）：rime_ice 词库内联 8 条
  拆字词条（犇 niuniuniu / 骉 mamama / 焱 huohuohuo / 磊 shishishi / 鑫 jinjinjin /
  淼 shuishuishui / 垚 tututu / 森 mumumu）；引擎测试 niuniuniu→犇、mamama→骉。
- **文字转语音**（搜狗 11.4 声文互转半边）：TtsSpeaker 系统 TTS 懒初始化；
  剪贴板历史文本条目与候选长按菜单新增「🔊 朗读」，onDestroy 释放。
- **修复 A1-3 缺口**：方案面板此前漏挂「9 键拼音」入口（两个方案列表都缺 t9），
  本次连同「笔画」一并补上（面板共 6 方案）。
- 验证：引擎测试 +2（stroke_and_chai）；JVM 单测 +6（触觉 3 + 笔画布局 1 + 既有）；
  APK 装实机冒烟无崩溃。

## [1.4.0] - 2026-08-19

### 新增（安卓 M-A2-4：键盘计算器，2026-08-19）
- **计算器面板**：功能行新增 🧮 入口（搜狗安卓 9.5 计算器 / 11.28 长度上限与
  % 运算）：表达式 + 实时预览 + 5×4 键位（C/⌫/%/÷/×/−/+/./=），点 = 直接上屏
  结果并清空；支持四则、取模、小数、一元负号、运算优先级（×÷% 先于 +−）。
- **纯逻辑**：Calculator.kt 状态机 + 两遍优先级解析，长度上限 24 字符、
  连续运算符/重复小数点拒绝；JVM 单测 5 项（优先级、取模/除法、负号/小数、
  除零与非法、长度上限）。

### 新增（安卓 M-A2-5：剪贴板增强——关键内容提取 + 100 条容量，2026-08-19）
- **关键内容提取**（搜狗 11.46）：剪贴板历史面板顶部新增 🔗网址/📞手机号/✉邮箱
  提取 chips（最新一条文本自动提取，点击一键上屏）；ClipboardInsights.kt 纯正则，
  JVM 单测 4 项。
- **容量 100 条**（搜狗 9.5）：历史列表与搜索 limit 30 → 100（Rust 存储层留存
  上限 2000 条，UI 展示扩容到位）。

### 新增（安卓 M-A2-2：表情面板——分类 / 搜索 / 最近，2026-08-19）
- **表情面板**：功能行新增 😀 入口（搜狗安卓 8.0 表情面板 / 4.8 表情搜索）：
  6 大类（表情/手势/动物/生活/爱心/颜文字，约 250 个 emoji + 24 个颜文字），
  数据与 PC 设置中心符号面板同源（scripts/gen-emoji-panel.py 从 main.js 提取）。
- **搜索**：119 条关键词索引（中文/拼音/英文，如 微笑/weixiao/smile → 😊），
  命中优先、符号字符/分类名兜底、去重保序。
- **最近使用**：点击发送即记录（最多 30 条、重复上移），「最近」分类直达；
  发送走 InputConnection.commitText。
- 测试：EmojiPanelTest 5 项（分类齐全、中英搜索命中、去重、最近上限/上移、
  编解码往返）；JVM 全量测试通过；APK 装实机冒烟无崩溃。

### 新增（安卓 M-A2-3：时间/日期/邮箱后缀快捷输入，2026-08-19）
- **快捷输入面板**：功能行新增 ⏱ 入口（搜狗安卓 7.4「输入即得」）：时间
  （HH:mm / HH:mm:ss）、日期（yyyy-MM-dd / yyyy年M月d日 / 星期X）、邮箱后缀
  （@qq.com 等 10 个常用域）三组，点击即上屏。
- 纯逻辑 QuickInsert 基于 Calendar 生成，JVM 单测 3 项（固定 2026-08-19 周三
  20:30:45 断言格式与星期）；JVM 全量测试通过；APK 装实机冒烟无崩溃。
- 说明：搜狗「输入 时间/日期 即输即得」的候选栏联想需要引擎词条，本切片以
  面板一键上屏替代，候选联想列 M-A2 后续评估。

### 新增（安卓 M-A2-1：常用语 / 快捷短语面板，2026-08-19）
- **常用语面板**：功能行新增 💬 入口（搜狗安卓 8.0 快捷短语 / 11.46 常用语）：
  20 条默认短语分 4 类（常用回复/问候/祝福/表情文字）+ 分类筛选 chips + 搜索；
  点击上屏、长按删除、底部新增行保存（归入当前选中分类，默认「自定义」）。
- **持久化**：行格式（category	text）存 SharedPreferences，顺序保真、脏数据
  容错；默认短语在用户首次增删前保持可恢复。
- 测试：QuickPhrasesTest 4 项（默认分类、编码解析往返、脏数据容错、分类+搜索
  过滤）；JVM 全量 54 测试通过；APK 装实机冒烟无崩溃。
- 后续切片：「输入前 3 字快速发送」（11.46）需候选栏引擎集成，待排。

## [1.3.0] - 2026-08-19

### 新增（安卓 M-A1-3：九键 T9 键盘 UI + 方案即时生效，2026-08-19）
- **九键 T9 键盘页**：shurufa_t9 方案下键盘渲染 3×3 数字键（1-9，键面带
  abc…wxyz 字母提示）+ 底栏 符/中英/空格/删除/换行；数字键整键送引擎
  （T9 词库吃 2-9），引擎拒绝的 1 直接上屏数字。
- **方案即时生效（修复既有缺口）**：rimejni 在 nativeInit 与方案切换时对
  会话执行 librime select_schema（此前 Android 端只持久化偏好、引擎仍跑
  雾凇拼音，双拼/五笔/仓颉/T9 均不生效）；options 新增 t9 方案 id 与
  schema_id_of 映射；方案面板文案去掉「预览 · 需重启」。
- **方案面板**：新增「9 键拼音」入口；切换成功即重建键盘（T9 ⇄ QWERTY）。
- 测试：KeyboardLayoutSpecTest +1（九键页 3×3 数字 + 底栏五种功能键）；
  options validate/schema 映射 +3；JVM 全量单测通过；rimejni 交叉编译
  arm64 .so 并随 APK 装实机（体积 ~60MB），冷启动无崩溃。

### 新增（安卓 M-A1-2：9 键 T9 拼音引擎，2026-08-19）
- **T9 词库**：scripts/gen-t9-dict.py 从雾凇拼音基础词库（base+others）生成
  shurufa_t9.dict.yaml（542,559 词条，整词 T9 数字串作单码索引；
  2abc 3def 4ghi 5jkl 6mno 7pqrs 8tuv 9wxyz，ü/v 并入 u 组）。
- **9 键方案**：schemas/shurufa_t9.schema.yaml（digit alphabet + table_translator，
  无整句学习/补全，preedit 前缀「九键:」）；已加入 PC/安卓 default.yaml schema_list
  与安卓 gradle 资产同步清单。
- **引擎集成测试**：t9_dict.rs 验证 7487832→输入法、64426→你好 可打（翻页查找）；
  修复了扁平 schema 格式不被 librime 接受的问题（改用 schema: 包装 + 完整 processors）。
- 验证：ime-bridge 全量测试通过（含双拼/场景词/生僻字回归）；APK 重建并装实机
  （含 T9 词库资产，体积 +~15MB）。

### 新增（安卓 M-A1-1：键盘快捷设置——高度 / 按键音 / 振动 / 单手模式，2026-08-19）
- **键盘设置面板**：功能行新增 ⚙️ 入口（搜狗安卓 3.7 快捷设置 / 5.1 键盘调节 /
  5.4 按键反馈），面板内提供键盘高度滑块（40%–120%，松手即重建生效并持久化）、
  按键音开关、按键振动开关、单手模式三态（关闭 / 左手 / 右手，键区收窄至 70%
  并按方向吸附）；全部偏好经 SharedPreferences 记忆，重启不丢。
- **按键反馈**：WetypeKeyboardView 按下键触发 performHapticFeedback(KEYBOARD_TAP)
  ＋ AudioManager.playSoundEffect(CLICK)（分别尊重系统触觉/音效设置，且受偏好门控）；
  长按连删/滑行清空等手势仅首触反馈一次，不随重复触发轰炸。
- **键盘高度**：高度百分比缩放自然高度与可用余量两个输入，分屏/受限输入区下
  不破版；候选行与功能行高度不受影响。
- 测试：新增 KeyboardPrefsTest 3 项（高度夹取 40–120、单手模式解析回退、默认值
  保持既有行为）；Android JVM 全量单测通过；APK 已装实机（Redmi 23113RKC6C），
  冷启动无崩溃。安卓路线图见 [开发计划-Android](docs/开发计划-Android.md)（M-A1..M-A5）。

### 新增（读屏无障碍阶段二：候选窗 ITextProvider，2026-08-19）
- **Windows 候选窗 ITextProvider / ITextRangeProvider**：uia_provider.rs 新增
  CandidateTextProvider 与 CandidateRange（#[implement] 完整 COM vtable）：
  DocumentRange / GetVisibleRanges 覆盖整条候选行文本（"1.你，2.你好…"），
  GetText 支持 maxlength 截断、GetSelection 返回空数组、SupportedTextSelection=None；
  GetPatternProvider(UIA_TextPatternId) 现在返回 ITextProvider。只读"全文范围"
  口径：逐候选偏移/选区/滚动等按 UIA 规范返回 E_NOTIMPL / 0，与 M11 验收报告一致。
- **修复 Provider 生命周期缺陷（实测发现）**：原实现每次 WM_GETOBJECT 都
  from_raw 重新包装静态指针，而 from_raw 不增加引用计数——首个请求返回后
  对象即被释放，读屏器拿到悬垂指针导致堆损坏（0xc0000374）。改为初始化时
  泄漏一个引用计数由静态永久持有（进程生命周期单例），与 MSDN 推荐模式一致。
- **端到端 UIA 运行时探针**：新增单测创建真实窗口 + WM_GETOBJECT 注册，
  经 CUIAutomation 客户端走读屏器同款路径验证：ElementFromHandle →
  CurrentName == 候选行文本 → 客户端 TextPattern（IUIAutomationTextPattern）
  → DocumentRange → GetText 全文一致。
- 测试：TSF 57（+6：DocumentRange 全文、GetText 截断、VisibleRanges 单范围、
  Selection 空数组、TextPattern 可达、运行时探针）；workspace 全绿；clippy 0
  告警（顺带清理 shurufa-host 5 项既有告警：asr mut、audio_capture 死代码/
  vec_box/fn 转型）。

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

## [0.4.2] - 2026-08-06

> 追记（2026-08-22）：tag 已存在但 CHANGELOG 漏记，本次按 tag 说明补全，
> 使 CHANGELOG↔tag 对账门禁成立。

### 修复
- 候选词滑动/展开、拼音光标定位、中文切换、桌面端 UI 响应式、Host 窗口
  闪现、安卓剪贴板初始捕获延迟

### 变更
- 协议扩展 `cursor_pos` / `page_no` / `page_size` / `is_last_page`
- Windows 候选窗恢复点击选词与滚轮翻页；Android 九宫格展开 + 横滑翻页
- 拼音光标 Collapse + ShiftStart 精准定位
- `VK_SHIFT` / `VK_CAPITAL` 不被输入法接管
- 控制中心响应式布局（≤760px 单列适配）
- Host `#![windows_subsystem = windows]` 消除终端窗口闪现
- 补全 gradle wrapper 支持本地构建

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
