-- 应用/网站直达（M8-4，搜狗 15.2 灵犀候选直达同类）
--
-- 在 engine/translators 增加：
--   - lua_translator@*app_direct
--
-- 候选文本带前缀标记：🖥 <应用名> / 🌐 <网址名>；TSF 提交时识别标记
-- 并启动目标（应用或默认浏览器），不把标记文本落进文档。
-- 数据源：user_data_dir/lua/app_direct_shortcuts.lua（设置中心从
-- app-shortcuts.json 生成；每次调用重新 require，改完即生效无需部署）。

local function load_shortcuts()
    package.loaded["app_direct_shortcuts"] = nil
    local ok, mod = pcall(require, "app_direct_shortcuts")
    if not ok or type(mod) ~= "table" then
        return {}
    end
    return mod
end

local function app_direct(input, seg, env)
    local code = env.engine.context.input
    if code == "" then
        return
    end
    for _, s in ipairs(load_shortcuts()) do
        if s.code and s.code ~= "" and code == s.code then
            local prefix = (s.kind == "url") and "🌐 " or "🖥 "
            -- 高质量置顶：直达候选排在拼音候选之前（date_translator 同款做法）
            local cand = Candidate("app_direct", seg.start, seg._end, prefix .. s.label, s.target or "")
            cand.quality = 100
            yield(cand)
        end
    end
end

return app_direct
