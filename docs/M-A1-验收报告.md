# M-A1 验收报告（安卓键盘形态与输入效率）

> **历史快照**：本报告记录该里程碑当时的验收情况，**不代表当前状态**。
> 当前质量信号以 CI 为准（见 README 徽章与 `scripts/` 下的四个门禁脚本）；
> 定位与理由见 [文档管理](文档管理.md) §9。

> 验收日期：2026-08-19 ｜ 目标版本：v1.3.0（versionCode 35）｜ 计划依据：[开发计划-Android](开发计划-Android.md) M-A1
> 对应搜狗安卓时间线：1.40 九宫格 / 3.7 快捷设置 / 5.1 键盘调节 / 5.4 按键反馈 / 7.6 单手 / 11.49 拼音14键

## 一、交付清单

| # | 功能 | 搜狗依据 | 落地方式 | 验证 |
|---|---|---|---|---|
| 1 | 键盘快捷设置（高度/按键音/振动/单手入口） | 3.7 快捷设置 | 功能行 ⚙️ 弹层，SharedPreferences 持久化 | JVM 单测 + 实机冒烟 |
| 2 | 键盘高度/宽度调节 | 5.1 键盘调节 | 高度滑块 40%–120%，缩放自然高度与可用余量 | 高度即时生效、分屏不破版 |
| 3 | 按键音与振动反馈 | 11.37 / 5.4 / 8.36 | performHapticFeedback + AudioManager 音效，可开关 | 开关生效、长按仅首触反馈 |
| 4 | 单手模式（左/右/收起） | 7.6 单手键盘 | 键区收窄 70% + 重力吸附 | 三态切换正确 |
| 5 | 9 键 T9 拼音（引擎 + 键盘 UI + 方案切换） | 1.40 / 8.13 大九键 | shurufa_t9 词库（54.2 万条）+ schema + 3×3 键盘页 + rimejni 方案即时生效 | 引擎测试 7487832→输入法 / 64426→你好；实机安装冒烟 |

## 二、重要实测结论（修正）

- **librime 不接受扁平 schema 格式**：invalid schema definition；改用 schema: 包装 +
  完整 processors（ascii_composer/recognizer/key_binder/speller/punctuator/selector/
  navigator/express_editor）后生效（2026-08-19，c626afc）。
- **Android 方案切换此前只持久化、未切引擎**（双拼/五笔/仓颉/T9 均不生效，文案标
  「预览 · 需重启」）；rimejni 在 nativeInit 与切换时对会话执行 librime select_schema
  修复（a193374），方案切换即时生效。
- **T9 词库采用整词数字串单码索引**：词条 key 为整词 T9 编码（如 输入法→7487832），
  无逐音节分词需求，规避 librime 对数字音节切分的限制。

## 三、验证证据

| 检查 | 结果 |
|---|---|
| cargo test --workspace --locked | ✅ 全绿（含 ime-bridge 全部引擎集成：双拼/场景词/生僻字/T9） |
| cargo clippy --workspace --all-targets | ✅ 0 告警（仅依赖 nom future-incompat 提示） |
| cargo fmt --all --check | ✅ 0 diff |
| gradle :app:testDebugUnitTest | ✅ 全绿（含 KeyboardPrefsTest 3 项、九键布局 1 项） |
| gradle assembleDebug + adb 实机安装 | ✅ Redmi 23113RKC6C 冒烟通过、冷启动无崩溃 |
| set-version.ps1 -Check | ✅ 与 version.json 一致 |

## 四、阶段后置（P2，不阻塞 M-A1）

- 拼音 14 键（11.49 / 11.50，皮肤适配与多布局）
- 滑行输入（6.2.6 / 6.5，手势轨迹识别）
- 悬浮键盘（8.18，IME 内浮动小窗）
- 九键实机手感微调（键高/误触边界）与 T9 词库质量评估

## 五、验收结论

M-A1 五项 P0/P1 全部落地并有测试/实机证据；版本由 1.2.0（code 34）bump 至
**1.3.0（code 35）**；Unreleased 中 Windows 读屏阶段二（ITextProvider + 运行时探针）
随本次发布一并落版。
