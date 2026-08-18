-- Unicode 输入（rime-ice unicode.lua 同款，2026-08-18 引入）
--
-- 在 engine/translators 增加：
--   - lua_translator@*unicode
-- 并在 recognizer/patterns 增加：
--   unicode: "^U[a-fA-F0-9]+"   # 大小写 hex 都收（U4f60、U1F600）
--
-- 功能：输入 U + 十六进制码点 → 输出对应 Unicode 字符（生僻字/emoji/特殊
-- 符号），如 U4F60 → 你、U1F600 → 😀、U03B1 → α。BMP 内（<0x10000）的
-- 码点附带按位遍历的 16 个变体候选（帮助从近似码点找字）。
-- 触发前缀取 recognizer/patterns/unicode 的第 2 个字符（默认 U，大写
-- 不与辅码检字 uU 的小写 u 冲突）。
-- 来源：https://github.com/shewer/librime-lua-script/blob/main/lua/component/unicode.lua

local path = 'recognizer/patterns/unicode'
local function unicode(input, seg, env)
    if not seg:has_tag("unicode") or input == '' then return end
    -- 获取 recognizer/patterns/unicode 的第 2 个字符作为触发前缀
    -- config:get_string(path) 可能取得 nil 造成error
    if not env.unicode_keyword then
        local pattern = env.engine.schema.config:get_string(path) or "UU"
        env.unicode_keyword = pattern:sub(2,2)
    end

    local ucodestr = input:match(env.unicode_keyword .. "(%x+)")
    if ucodestr and #ucodestr > 1 then
        local code = tonumber(ucodestr, 16)
        if code > 0x10FFFF then
           yield(Candidate("unicode", seg.start, seg._end, "数值超限！", ""))
           return
        end
        local text = utf8.char(code)
        yield(Candidate("unicode", seg.start, seg._end, text, string.format("U%x", code)))
        if code < 0x10000 then
           for i = 0, 15 do
               local next_text = utf8.char(code * 16 + i)
                yield(Candidate("unicode", seg.start, seg._end, next_text, string.format("U%x~%x", code, i)))
           end
        end
    end
end

return unicode
