-- V 模式帮助（vhelp，2026-08-17 引入）
--
-- 在 engine/translators 增加：
--   - lua_translator@*v_help
--
-- 功能：输入 vhelp（或 Vhelp）→ 列出本方案全部 V 模式/特殊触发码及说明，
-- 每个触发码一个候选（comment 是说明）。参考 rime-ice 的 vhelp 浏览
-- 符号输入支持列表的做法，改造成适配本方案的触发码帮助页。
-- 候选不可直接上屏（select 无意义），按 Esc 退出即可。
-- 与英文混输无冲突：vhelp 不在 english 词典里，拼音 v-h-e-l-p 也不成词。
--
-- 注意：lua_translator 的 func 签名是 (input, seg, env)，与 lua_filter
-- 的 (input, env) 不同（本脚本是 translator，必须用三参签名）。

local function v_help(input, seg, env)
    -- 仅当输入码精确等于 vhelp / Vhelp 时输出帮助页
    local code = env.engine.context.input
    if code ~= "vhelp" and code ~= "Vhelp" then
        return
    end
    -- 触发码表：编码 + 说明
    local triggers = {
        { "rq",     "日期：2026-08-17" },
        { "sj",     "时间：14:30" },
        { "xq",     "星期：星期一" },
        { "dt",     "ISO 时间戳：2026-08-17T14:30:00+08:00" },
        { "ts",     "秒级时间戳" },
        { "rqzh",   "中文日期：二〇二六年八月十七日" },
        { "rqen",   "英文日期：17 August 2026" },
        { "R<数字>", "金额大写：R123 → 壹佰贰拾叁元整" },
        { "cC<算式>", "计算器：cC1+1 → 2" },
        { "nl",     "今日农历" },
        { "N<日期>", "公历转农历：N20240210 → 甲辰年正月初一" },
        { "uU<部件>", "部件拆字反查：uUheng → 一" },
        { "`<辅码>", "部件辅码筛选：nihao`ren → 只留 亻 部首候选" },
    }
    for _, t in ipairs(triggers) do
        yield(Candidate("v_help", seg.start, seg._end, t[1], t[2]))
    end
end

return v_help
