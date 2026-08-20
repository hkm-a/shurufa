# AI 候选预测（搜狗 13.0+ AI 化主线）——方案设计

> 制定：2026-08-20 ｜ 状态：生效
> 复用：agnès（apihub.agnes-ai.com/v1/chat/completions，agnes-2.5-flash）、
> AiPrompt（提示词构造）、DynamicCandidates（候选注入模式，P4-2）

## 1. 目标

输入拼音出现候选后，候选行注入 AI 预测候选（🤖 标记）：基于当前拼音与上文
预测最可能的下一词/补全，类似搜狗 AI 候选、讯飞随声译的"AI 建议"。

## 2. 与现有能力的边界

| 能力 | 现状 | AI 候选补充 |
| --- | --- | --- |
| 候选行 | 引擎候选 + 动态快捷码（P4-2）+ emoji 混排（P1-3） | 追加 AI 预测候选（头部/尾部） |
| AI 帮写/润色/翻译 | AiPanel 面板式（Ctrl+Shift+W/R/T） | 候选行**轻量直出**，不切面板 |
| 引擎候选 | librime（词频/上下文调频） | AI 补充引擎未覆盖的长尾表达 |

## 3. 交互设计

- 输入拼音暂停约 **800ms** 且引擎候选已出现 → 触发 AI 预测
- AI 候选带 **🤖 前缀**标记，排在候选行末尾（不抢引擎候选位）
- 点击 AI 候选直接上屏；无需可忽略（自动消失）
- 失败/超时/无 key → 静默（不打扰输入）

## 4. 实现方案（Android 端，Kotlin 层，与 P4-2 同模式）

### 4.1 AiCandidateManager（新文件）

- `predict(preedit, context): List<String>?`：同步调 agnès，返回 1-3 个候选
  - prompt：`上文 + 拼音 preedit → 预测 1-3 个最可能的词（仅输出词，逗号分隔）`
  - 节流：输入暂停 800ms（Handler debounce），请求期间不重复触发
  - 缓存：同 preedit 结果 10s 内复用
  - 开关 + API key 校验（复用 readAiApiKey）
- 候选注入：updateCandidates 时尾部追加 AI 候选（`🤖 词` 显示、点击提交词本体）

### 4.2 设置

- 设置面板「AI 候选」开关（默认关，API 消耗需用户主动开启）
- 依赖 API key（现有 AI 面板设置）

### 4.3 Windows 端（后续）

- TSF 层候选刷新后异步调 agnès → 注入候选窗（同 app_direct 的 lua 标记模式）
- 本轮先做 Android，Windows 复用同一 agnès 接口

## 5. 验收

- 模拟器：输入暂停 → 候选行出现 🤖 AI 候选 → 点击上屏；无 key 静默
- 单测：提示词构造、节流逻辑、缓存（纯函数）

## 6. 风险

- API 延迟（1-3s）：候选晚到，用 debounce + 到期丢弃避免闪动
- API 消耗：默认关 + 节流 + 缓存
- 隐私：上文文本送云端——设置页明示，默认关
