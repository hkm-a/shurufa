# M7-5 候选 Tab 多服务切换——方案评估

> 2026-08-21 ｜ 状态：待方案评估 → 本评估 ｜ 对应搜狗 16.x 候选条上方
> "拼音 / 英文 / 网址 / 翻译"多标签切换（PC 帮助未收录，源自版本 UI 与竞品观察）

## 1. 目标

候选窗/候选栏顶部提供一组服务标签（Tab），点击切换当前展示的候选来源：

| Tab | 候选源 | 现状 |
|---|---|---|
| 拼音 | librime 引擎候选（默认） | ✅ 已有 |
| 英文 | 本地英文词典/联想（可离线） | ❌ 无 |
| 网址 | V 模式已有（v+网址直达） | 🔶 部分（见 P4） |
| 翻译 | agnès API 翻译候选 | ❌ 无（AI 面板有划词翻译） |
| AI | 现有 AI 候选预测（🤖） | 🔶 已混排在候选行第二行 |

**本质**：把"混排在同一候选列表"的多来源候选，组织为"分组 + Tab 切换"，
让用户显式选择服务，而不是靠引擎/注入顺序。

## 2. 现状架构

### Windows（TSF）
- 候选数据：`ime_ipc::Context.candidates`（引擎 9 条）
- AI 候选：service 侧 `maybe_request_ai` → worker → `refresh_with_ai` 追加
  到候选窗（`AI_CANDIDATES` thread_local，show 时按 preedit 匹配合并，
  引擎保留前 6 + AI 至多 3，AI 恒在第二行）
- 点击：数字键（引擎索引）或 AI_COMMIT 钩子（AI 索引）→ Enter 落盘

### Android（IME）
- 候选数据：RimeBridge context（引擎候选）
- AI 候选：AiCandidateManager + Service debounce 注入（候选行尾部 🤖）
- 点击：onCandidateSelect 索引偏移处理

### 引擎边界（关键约束）
- **librime 无"按 translator/候选源过滤"API**：`context.candidates` 是
  schema 内所有 translator 合并后的结果，前端拿不到"这一条来自哪个
  translator"。
- 所以 **"Tab = 引擎侧候选源切换"不可行**（无法在 librime 内切换）。
- 可行的 Tab 源只能是**前端/服务侧注入的独立候选组**（同 AI 候选模式）：
  网址、翻译、英文联想——都由 TSF/Kotlin 层查询并注入，与引擎无关。

## 3. 候选源盘点（按价值排序）

1. **英文候选（最有价值）**：中文输入中偶尔要打英文词。搜狗 Tab 提供英文
   联想（如输入 "nihao" 时英文 Tab 给出 "nice/ni hao" 的英文候选）。
   - 本地实现：内置小词典（高频英文 1-2 万词）+ 前缀匹配；无网络。
   - 无词典时可用引擎 ascii 直输兜底。
2. **翻译候选**：输入中文拼音时翻译 Tab 给英文/日文候选（agnès 可做，
   类似 AI 候选但 prompt 不同）。成本 = 云端 + 延迟。
3. **网址候选**：已有 V 模式（`v` + 网址关键字 → 直达候选）。Tab 只是
   把网址候选从 V 模式提升为独立 Tab（输入 `v` 自动切 Tab 或 Tab 直达）。
4. **AI 候选**：已混排；可升级为独立 Tab（切到 AI Tab 时以 AI 候选为主
   展示，Rime 候选折叠）。

## 4. 方案设计

### 4.1 数据模型（两端统一）

```
enum CandidateSource { Rime, English, Url, Translate, Ai }
struct GroupedCandidates {
    active_tab: CandidateSource,
    tabs: Vec<(CandidateSource, Vec<Candidate>)>,  // 各 Tab 的候选
}
```

- 引擎候选固定进 Rime 组；注入候选（AI/英文/翻译/网址）各进自己的组。
- 默认 active = Rime（保持现行为）；有组合时 Tab 行显示（2+ 个组才有意义）。

### 4.2 UI（Windows 候选窗 / Android 候选栏）

- **Windows**：候选窗顶部加一条小号 Tab 行（高 ~22 逻辑 px，仅当
  active 组外还有候选组时显示）。命中测试优先 Tab 行 → 点击切组 → 重算
  布局（复用 compute_show_layout，Tab 行计入内容指纹）。
- **Android**：候选栏左侧/顶部 Tab chips（同模式切换竖栏风格），点击切换。

### 4.3 点击提交

- Rime 组：数字键（现状）。
- 注入组（英文/翻译/AI/网址）：**不走数字键**（引擎索引对不上），统一走
  `AI_COMMIT` 式钩子（Windows：pending + Enter；Android：直接上屏）。
- 切到注入组时候选编号重新从 1 开始（仅前端语义），点击映射到该组索引。

### 4.4 与现有 AI 候选的关系

- 过渡：AI 候选保持混排（现状），Tab 行先提供 **英文** 与 **翻译** 两组。
- 后续：AI 候选收敛为独立 Tab（避免混排占 Rime 槽位）。

## 5. 分阶段落地

| 阶段 | 内容 | 工作量 |
|---|---|---|
| P1 | 候选分组模型 + Windows Tab 行 UI（Rime + AI 两组先行） | 2-3 天 |
| P2 | 英文候选（内置词典 + 前缀联想）两端 | 2-3 天 |
| P3 | 翻译候选（agnès，复用 AI 候选的 worker/缓存基建） | 2-3 天 |
| P4 | 网址 Tab（复用 V 模式直达库）+ Android Tab UI | 2 天 |

合计约 1 周+（与附录 D 预估一致）。

## 6. 风险与决策点

1. **英文词典规模**：内置 1 万词 vs 20 万词（体积/精度权衡）——建议先
   1 万高频 + 用户自增。
2. **翻译延迟**：云端 1-3s——必须 debounce + 缓存（复用 AI 候选基建）；
   无 key/失败时 Tab 置灰。
3. **Tab 行占用空间**：Windows 候选窗增高 22px——仅在有第二组候选时显示，
   避免干扰纯中文输入。
4. **是否保留 AI 混排**：决策点——保留（现状）还是收编为 Tab（干净但
   多一次点击）。倾向：保留混排 + Tab 行提供英文/翻译，AI 维持现状。
5. **librime 边界**：不试图在引擎内切候选源（不可行）；全部走前端注入。

## 7. 结论

- **可行**，但必须接受"Tab 源 = 前端注入候选组"的架构（引擎无法切源）。
- 第一增量（P1）价值有限（AI 已有混排），**P2 英文候选**才是 Tab 的
  真正价值点——建议直接以 P1+P2 合并为一个里程碑实施。
- 当前先落地**方案评估**，具体实施待用户确认优先级（英文词典规模、是否
  收编 AI Tab）。

---

## 8. 实施进展（2026-08-21）

### 已落地：Windows P1+P2（候选分组模型 + Tab 行 UI + 英文候选）

- **english_candidates.rs**：内置 ~500 高频英文词表 + 前缀联想纯函数
  （≥2 位 ASCII 字母 → 长度升序取前 5；6 项单测）。
- **Tab 行 UI**（candidate_window.rs）：
  - TabKind { Rime, English } + ACTIVE_TAB thread_local；
  - 窗口顶部 Tab 行（BASE_TAB_HEIGHT 22 逻辑 px），仅当英文候选非空时
    显示（拼音 | 英文，激活态高亮）；
  - 三渲染后端（GDI/D2D/DComp）均实现 Tab 行绘制 + preedit/候选行偏移；
  - 内容指纹隐含 tab（候选内容不同 → 自动重算）。
- **切换与提交**：
  - 点击 Tab 行 → tab_switch 按 LAST_CTX 快照重建布局（无需按键）；
  - Rime 无候选且英文候选有值 → 自动激活英文组；
  - 英文候选点击 → AI_COMMIT 钩子（pending + Enter，与 AI 候选同路径）。
- **本机端到端验证**：
  - 输入 wor → Tab 行出现（窗口高 +33px）；
  - 点击英文 Tab → 候选 Tab 切换：English → 候选变英文（word/work/world/worry）；
  - Rime 无候选时自动切英文（wor 场景截图 win-en-wor.png：Tab 行 + 英文候选）。

### 未落地（后续）

- 翻译 Tab（agnès，复用 AI 候选基建）
- 网址 Tab（复用 V 模式直达库）
- Android Tab UI（P4）
- 英文词表扩充（1 万 → 更多）+ 用户自增
