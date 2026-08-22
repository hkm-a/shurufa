# M-A3 验收报告（安卓无障碍与生僻字）

> **历史快照**：本报告记录该里程碑当时的验收情况，**不代表当前状态**。
> 当前质量信号以 CI 为准（见 README 徽章与 `scripts/` 下的四个门禁脚本）；
> 定位与理由见 [文档管理](文档管理.md) §9。

> 验收日期：2026-08-19 ｜ 目标版本：v1.5.0（versionCode 37）｜ 计划依据：[开发计划-Android](开发计划-Android.md) M-A3
> 对应搜狗安卓时间线：1.6 笔画输入 / 4.1 拆字输入 / 8.6 无障碍输入 / 11.4 声文互转 / 11.13 生僻字键盘与触觉输入

## 一、交付清单

| # | 功能 | 搜狗依据 | 落地方式 | 验证 |
|---|---|---|---|---|
| 1 | 触觉输入（振动层次 / 长按连续删除 / 末位强振） | 11.13.1 | HapticProfile 纯逻辑；长按删除启动 LONG_PRESS、每 tick CONTEXT_CLICK、删空组合末位强振 | JVM 单测 3 项 + 实机冒烟 |
| 2 | 生僻字拆字（牛牛牛→犇 等） | 11.13.1 / 4.1 | rime_ice 词库内联 8 条拆字词条（重叠拼音码） | 引擎测试 niuniuniu→犇、mamama→骉 |
| 3 | 笔画输入键盘 UI | 1.6 / 11.13.1 | stroke 方案接入 Android + 五笔画键（一丨丿丶乙→h/s/p/n/z）+ 数字直选行 + 底栏 | 引擎测试 h→一；布局单测 1 项 |
| 4 | 文字转语音（声文互转半边） | 11.4 | TtsSpeaker 系统 TTS；历史文本条目与候选长按菜单「🔊 朗读」 | JVM 编译 + 实机冒烟 |

## 二、重要实测结论

- **stroke 方案移植**：librime 自带五笔画方案（h/s/p/n/z），依赖 luna_pinyin；
  Android 打包需同时纳入 stroke.schema/dict + luna_pinyin.schema/dict 并加入
  schema_list，select_schema 才能生效（引擎测试 22.97s 全绿验证）。
- **修复 A1-3 遗留缺口**：A1-3 提交时「9 键拼音」面板入口实际未挂上（两个方案
  列表都缺 t9，面板仅 4 方案），本次连「笔画」一起补齐为 6 方案并重建 .so。
- **拆字走词条而非引擎拆字**：librime 无通用拆字规则，采用重叠拼音码词条
  （niuniuniu→犇），权重 1 不干扰常规拼音。

## 三、验证证据

| 检查 | 结果 |
|---|---|
| cargo test --workspace --locked | ✅ 全绿（含 stroke/拆字引擎集成） |
| cargo clippy --workspace --all-targets | ✅ 0 告警（仅依赖 nom future-incompat 提示） |
| cargo fmt --all --check | ✅ 0 diff |
| gradle :app:testDebugUnitTest | ✅ 全绿（触觉 3 + 笔画布局 1 + 既有全部） |
| gradle assembleDebug + adb 实机安装 | ✅ Redmi 23113RKC6C 冒烟通过、冷启动无崩溃 |
| set-version.ps1 -Check | ✅ 与 version.json 一致 |

## 四、阶段后置（不阻塞 M-A3）

- 生僻字键盘「部首拼音混合输入」的键盘内交互（当前笔画 + 拼音方案切换达成同一目标）
- 声文互转「实时语音转文字」已由 VoiceInput 提供；多语种识别与音色选择（11.4 的
  13 语种/8 音色）依赖系统 TTS 能力，留实机验证
- TalkBack 实机朗读/振动层次验证（需用户开启 TalkBack 操作）

## 五、验收结论

M-A3 四项（P0×1 + P1×2 + P2×1）全部落地并有测试/实机证据；版本由 1.4.0（code 36）
bump 至 **1.5.0（code 37）**。
