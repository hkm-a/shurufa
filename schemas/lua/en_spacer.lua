-- 英文词条上屏自动添加空格（rime-ice en_spacer.lua 同款，2026-08-18 引入）
--
-- 在 engine/filters 增加（uniquifier 之前）：
--   - lua_filter@*en_spacer
-- 并在 switches 增加（可关）：
--   - name: en_spacer
--     reset: 1
--     states: [ 空格, 无 ]
--
-- 功能：上次上屏的是英文词（如 hello），本次候选也是纯英文词（world）时，
-- 候选自动带前导空格（" world"）——中英混输时英文单词之间自动加空格，
-- 不用手动敲空格。触发条件窄（上次上屏英文 + 本次候选纯英文词），
-- 误伤概率低；开关 en_spacer 可关。
--
-- 来源：https://github.com/iDvel/rime-ice/blob/main/lua/en_spacer.lua

local F = {}

function F.func(input, env)
    -- 开关关闭时原样透传
    if not env.engine.context:get_option('en_spacer') then
        for cand in input:iter() do
            yield(cand)
        end
        return
    end
    local latest_text = env.engine.context.commit_history:latest_text()
    for cand in input:iter() do
        local c = cand
        if c.text:match('^[%a\']+[%a\']*$') and latest_text and #latest_text > 0 and
            latest_text:find('^ ?[%a\']+[%a\']*$') then
            c = c:to_shadow_candidate('en_spacer', c.text:gsub('(%a+\'?%a*)', ' %1'), c.comment)
        end
        yield(c)
    end
end

return F
