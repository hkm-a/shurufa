-- 英文自动大小写（rime-ice autocap_filter.lua 同款，2026-08-17 引入）
--
-- 在 engine/filters 增加：
--   - lua_filter@*autocap_filter
--
-- 配合 english.schema.yaml 的 speller/algebra 大小写派生规则（\U/\L）：
-- 输入首字母大写（Hello）→ 英文候选词转为首字母大写；
-- 输入前 2 个及以上字母大写（HELLo）→ 英文候选词转为全大写（HELLO）；
-- 输入全小写（hello）→ 候选词保持原样。
--
-- 与 rime-ice 的差异说明：
-- - 规则保持一致（#302@abcdefg233 #305@Mirtle 的原始实现）；
-- - 不转换的边界：码长 1、码首字符为小写/标点、候选含非 ASCII/空格、
--   输入码完全匹配候选词、单词与编码不一致（如 PS - Photoshop）。

local function autocap_filter(input, env)
    local code = env.engine.context.input -- 输入码
    local codeLen = #code
    local codeAllUCase = false
    local codeUCase = false
    -- 不转换：
    if codeLen == 1 or       -- 码长为 1
        code:find("^[%l%p]") -- 输入码首位为小写字母或标点
    then                     -- 输入码不满足条件不判断候选项
        for cand in input:iter() do
            yield(cand)
        end
        return
    -- 输入码前 2 - n 位大写
    elseif code:find("^%u%u+.*") then
        codeAllUCase = true
    -- 输入码首位大写
    elseif code:find("^%u.*") then
        codeUCase = true
    end

    local pureCode = code:gsub("[%s%p]", "")     -- 删除标点和空格的输入码
    for cand in input:iter() do
        local text = cand.text                   -- 候选词
        local pureText = text:gsub("[%s%p]", "") -- 删除标点和空格的候选词
        -- 不转换：
        if
            text:find("[^%w%p%s]") or                 -- 候选词包含非字母和数字、非标点符号、非空格的字符
            text:find("%s") or                        -- 候选词中包含空格
            pureText:find("^" .. code) or             -- 输入码完全匹配候选词
            (cand.type ~= "completion" and            -- 单词与其对应的编码不一致
                pureCode:lower() ~= pureText:lower()) -- 例如 PS - Photoshop
        then
            yield(cand)
        -- 输入码前 2~10 位大写，候选词转换为全大写
        elseif codeAllUCase then
            text = text:upper()
            yield(Candidate(cand.type, 0, codeLen, text, cand.comment))
        -- 输入码首位大写，候选词转换为首位大写
        elseif codeUCase then
            text = text:gsub("^%a", string.upper)
            yield(Candidate(cand.type, 0, codeLen, text, cand.comment))
        else
            yield(cand)
        end
    end
end

return autocap_filter
