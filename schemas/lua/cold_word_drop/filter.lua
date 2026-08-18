-- 词条隐藏、降频
-- 在 engine/processors 增加 - lua_processor@*cold_word_drop.processor
-- 在 engine/filters 增加 - lua_filter@*cold_word_drop.filter
-- 在 key_binder 增加快捷键：
-- reduce_freq_cand: "Control+j"  # 匹配当前输入码后隐藏指定的候选字词 或候选词条放到第四候选位置
-- drop_cand: "Control+d"       # 强制删词, 无视输入的编码
-- get_record_filername() 函数中仅支持了 Windows、macOS、Linux

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
    -- 降频词表以 processor 实际写入的 reduce_freq_words.lua 为准；
    -- turn_down_words.lua 是旧版备用文件，若存在且为空（require 返回 true），
    -- 原先的 or 链会取到布尔值导致 filter 崩溃、候选流清空（2026-08-18 实测）。
    env.reduce_freq_words = (_sr and reduce_freq_words) or (_st and turn_down_words) or {}
end

function filter.func(input, env)
	local context = env.engine.context
	local preedit_str = context.input:gsub(" ", "")
	local drop_words = env.drop_words
	local hide_words = env.hide_words
	local word_reduce_idx = env.word_reduce_idx
	local reduce_freq_words = env.reduce_freq_words
	-- 无任何规则时直通（2026-08-16 修复）：drop/hide/reduce 全空时原样透传，
	-- 不做重排/截断——此前空列表也会走 word_reduce_idx 重排逻辑，把英文
	-- 补全候选挤出可见范围（english_mixing 回归）。
	if not drop_words or not hide_words or not reduce_freq_words then
		for cand in input:iter() do
			yield(cand)
		end
		return
	end
	local has_rules = next(drop_words) ~= nil or next(hide_words) ~= nil or next(reduce_freq_words) ~= nil
	if not has_rules then
		for cand in input:iter() do
			yield(cand)
		end
		return
	end
	local cands = {}
	for cand in input:iter() do
		local cand_text = cand.text:gsub(" ", "")
		-- 兼容无 preedit 的候选（英文补全等）：nil 时回退到当前输入串。
		-- 原代码 cand.preedit:gsub(...) 在 preedit 为 nil 时抛 Lua 错误，
		-- 导致 filter 崩溃、整条候选流被丢弃（英文候选消失，2026-08-16 实机复现）。
		local cand_preedit = cand.preedit
		local preedit_code = cand_preedit and cand_preedit:gsub(" ", "") or preedit_str

		local reduce_freq_list = reduce_freq_words[cand_text] or {}
		if word_reduce_idx > 1 then
			-- 前三个 候选项排除 要调整词频的词条, 要删的(实际假性删词, 彻底隐藏罢了) 和要隐藏的词条
			if reduce_freq_list and table.find_index(reduce_freq_list, preedit_code) then
				table.insert(cands, cand)
			elseif
				not (
					table.find_index(drop_words, cand_text)
					or (hide_words[cand_text] and table.find_index(hide_words[cand_text], preedit_code))

				)
			then
				yield(cand)
				word_reduce_idx = word_reduce_idx - 1
			end
		else
			if
				not (
					table.find_index(drop_words, cand_text)
					or (hide_words[cand_text] and table.find_index(hide_words[cand_text], preedit_code))
				)
			then
				table.insert(cands, cand)
			end
		end

		if #cands >= 180 then
			break
		end
	end

	for _, cand in ipairs(cands) do
		yield(cand)
	end
end

return filter

