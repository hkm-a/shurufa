-- 冷词丢弃/隐藏/降频 filter（扁平模块名，避免点号组件名问题）
local filter = {}
function filter.init(env)
    local engine = env.engine
    local config = engine.schema.config
    local _sd, drop_words = pcall(require, "cold_word_drop/drop_words")
    local _sh, hide_words = pcall(require, "cold_word_drop/hide_words")
    local _st, turn_down_words = pcall(require, "cold_word_drop/turn_down_words")
    local _sr, reduce_freq_words = pcall(require, "cold_word_drop/reduce_freq_words")
    env.word_reduce_idx = config:get_int("cold_word_reduce/idx") or 4
    env.drop_words = _sd and drop_words or {}
    env.hide_words = _sh and hide_words or {}
    env.reduce_freq_words = (_st and turn_down_words) or (_sr and reduce_freq_words) or {}
end
function filter.func(input, env)
    local drop_words = env.drop_words
    local hide_words = env.hide_words
    local reduce_freq_words = env.reduce_freq_words
    if not drop_words or not hide_words or not reduce_freq_words then
        for cand in input:iter() do yield(cand) end
        return
    end
    if next(drop_words) == nil and next(hide_words) == nil and next(reduce_freq_words) == nil then
        for cand in input:iter() do yield(cand) end
        return
    end
    local context = env.engine.context
    local preedit_str = context.input:gsub(" ", "")
    local word_reduce_idx = env.word_reduce_idx
    local cands = {}
    for cand in input:iter() do
        local cand_text = cand.text:gsub(" ", "")
        local cand_preedit = cand.preedit
        local preedit_code = cand_preedit and cand_preedit:gsub(" ", "") or preedit_str
        local reduce_freq_list = reduce_freq_words[cand_text] or {}
        if word_reduce_idx > 1 then
            if reduce_freq_list and table.find_index(reduce_freq_list, preedit_code) then
                table.insert(cands, cand)
            elseif not (table.find_index(drop_words, cand_text) or (hide_words[cand_text] and table.find_index(hide_words[cand_text], preedit_code))) then
                yield(cand)
                word_reduce_idx = word_reduce_idx - 1
            end
        else
            if not (table.find_index(drop_words, cand_text) or (hide_words[cand_text] and table.find_index(hide_words[cand_text], preedit_code))) then
                table.insert(cands, cand)
            end
        end
        if #cands >= 180 then break end
    end
    for _, cand in ipairs(cands) do
        yield(cand)
    end
end
return filter
