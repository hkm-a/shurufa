-- 多时机表情推荐（M7-9，搜狗 15.9「输入 ？？？/okok 出表情」同类）
--
-- 在 engine/translators 增加：
--   - lua_translator@*emoji_timing
--
-- 功能：输入触发码（英文/拼音串）时，直接附加对应 emoji 候选——
--   okok → 👌、wanan（晚安）→ 🌙。
-- 与 OpenCC simplifier@emoji 的区别：那是"中文词候选附带 emoji"，
-- 这里覆盖"输入串本身即触发"（okok 不是中文词，OpenCC 管不到）。
-- 中文触发（如 aini/爱你）不再维护：`爱你 → 🤟` 已由 OpenCC emoji.txt
-- 覆盖，本模块再挂 `aini → ❤️` 会造成同词不同表情冲突。
-- 触发码不在 english/拼音词典里时不会造成候选冲突；有冲突时 emoji
-- 作为附加候选排在后面（uniquifier 按文本去重）。
--
-- 注意：lua_translator 的 func 签名是 (input, seg, env)（与 lua_filter
-- 的 (input, env) 不同）。"？？？" 触发需要拦截 Shift+/ 按键（TSF 层），
-- 本轮不做，见 docs/优化灵感库.md。

local emoji_timing = {}

-- 触发码 → emoji（保持小写；只保留 OpenCC 覆盖不到的非中文串）
local map = {
    okok = "👌",
    wanan = "🌙",
}

function emoji_timing.func(input, seg, env)
    local code = env.engine.context.input
    local emoji = map[code]
    if not emoji then
        return
    end
    -- 让 emoji 候选出现在普通候选之后（comment 标触发码，方便识别来源）
    local cand = Candidate("emoji_timing", seg.start, seg._end, emoji, "表情")
    cand.quality = 100
    yield(cand)
end

return emoji_timing
