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

### 4.3 Windows 端（2026-08-20 已落地，与 Android 同语义）

TSF 层候选刷新后异步调 agnès → 注入候选窗尾部（🤖 副标），点击直接上屏。

- **新模块** `platforms/windows/src/ai_candidates.rs`：
  - `build_predict_prompt(preedit)` / `parse_candidates(raw)`：与 Android
    buildPrompt/parseCandidates 同语义的纯函数（逗号分隔、去引号、去重、上限 3）
  - `fetch_candidates(api_key, preedit)`：ureq 同步调 agnès（8s 超时，非流式）
  - `AiWorker`：每宿主进程一个常驻 worker（懒启动），`sync_channel(1)`
    缓冲 + 800ms 停顿窗口收最新 preedit（快打丢旧保新）→ 请求 → 结果经
    `PostMessageW(WM_AI_CANDIDATES_READY)` 回候选窗 UI 线程刷新布局
  - 缓存：同 preedit 结果 10s TTL（worker 内）
- **候选窗注入**（candidate_window.rs）：
  - show() 合并：引擎候选保留前 6 个 + AI 候选至多 3 个（合计 ≤9），
    AI 候选 comment 标 🤖（不影响引擎索引与翻页）
  - `LAST_CTX` 快照 + `refresh_with_ai`：AI 结果到达后按快照重建布局
    （复用 `compute_show_layout`，位置保持、仅重排宽高 + 重绘 + UIA 同步）
  - 点击 AI 候选：不发送引擎数字选词键（索引对不上），改走
    `AI_COMMIT` 钩子写 pending 槽 + 回发 VK_F13（标准键盘不存在该键），
    service 在 handle_key 入口识别后结束组合 + 插入文本落盘
- **开关与 key**：`ImeOptions.ai_candidates`（默认 false；设置中心「输入 →
  AI 候选预测」开关，data-general-field 通用保存）；key 从环境变量
  `AGNES_API_KEY` 读取（与 AI 帮写面板同源，永不落盘）
- **TSF 接线**（service.rs）：process_key 后 `maybe_request_ai`——开关开 +
  有 key + 中文态 + 有组合才投递；失败/无 key/超时静默降级

## 5. 验收

- 模拟器：输入暂停 → 候选行出现 🤖 AI 候选 → 点击上屏；无 key 静默
- 单测：提示词构造、节流逻辑、缓存（纯函数）

## 6. 风险

- API 延迟（1-3s）：候选晚到，用 debounce + 到期丢弃避免闪动
- API 消耗：默认关 + 节流 + 缓存
- 隐私：上文文本送云端——设置页明示，默认关
