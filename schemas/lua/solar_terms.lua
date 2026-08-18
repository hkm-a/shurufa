-- 节日/节气提醒（M10-6，搜狗 5.2 节气提示同类）
--
-- 在 engine/translators 增加：
--   - lua_translator@*solar_terms
--
-- 输入 jieqi（节气）→ 今日节气；输入 jieri（节日）→ 今日公历节日。
-- 节气日用 C 世纪常数近似公式（1901-2100 适用，误差 ±1 天内）：
--   D = INT(Y*0.2422 + C) - INT((Y-1)/4)，Y=年份后两位；
--   21 世纪闰年 1、2 月修正 -1。
-- 农历节日（春节/中秋）需农历换算，暂不含。

local TERMS = {
  "小寒", "大寒", "立春", "雨水", "惊蛰", "春分",
  "清明", "谷雨", "立夏", "小满", "芒种", "夏至",
  "小暑", "大暑", "立秋", "处暑", "白露", "秋分",
  "寒露", "霜降", "立冬", "小雪", "大雪", "冬至",
}

-- 世纪常数 C（1901-2100），与 TERMS 一一对应
local C = {
  6.11, 20.84, 4.63, 19.06, 6.30, 20.89,
  5.59, 21.32, 6.09, 21.51, 6.14, 21.87,
  7.54, 23.13, 7.65, 23.56, 7.87, 23.95,
  8.38, 24.10, 7.97, 24.17, 7.39, 22.96,
}

local function solar_term_day(year, idx)
  local yy = year % 100
  local d = math.floor(yy * 0.2422 + C[idx]) - math.floor((yy - 1) / 4)
  if math.floor(year / 100) == 21 and year % 4 == 0 and (idx == 1 or idx == 2) then
    d = d - 1
  end
  return d
end

local function today_solar_term()
  local t = os.date("*t")
  for i = 1, #TERMS do
    local month = math.ceil(i / 2)
    if month == t.month and solar_term_day(t.year, i) == t.day then
      return TERMS[i]
    end
  end
  return nil
end

-- 公历节日表（农历节日需农历换算，暂不含）
local FESTIVALS = {
  { 1, 1, "元旦" }, { 2, 14, "情人节" }, { 3, 8, "妇女节" },
  { 5, 1, "劳动节" }, { 6, 1, "儿童节" }, { 9, 10, "教师节" },
  { 10, 1, "国庆节" }, { 12, 25, "圣诞节" },
}

local function today_festival()
  local t = os.date("*t")
  for _, f in ipairs(FESTIVALS) do
    if f[1] == t.month and f[2] == t.day then
      return f[3]
    end
  end
  return nil
end

local solar_terms = {}

function solar_terms.func(input, seg, env)
  local code = env.engine.context.input
  if code == "jieqi" then
    local name = today_solar_term()
    local text = name and ("今日节气：" .. name) or "今日无节气"
    local cand = Candidate("solar", seg.start, seg._end, text, os.date("%Y-%m-%d"))
    cand.quality = 100
    yield(cand)
  elseif code == "jieri" then
    local name = today_festival()
    if name then
      local cand = Candidate("solar", seg.start, seg._end, "今日节日：" .. name, os.date("%Y-%m-%d"))
      cand.quality = 100
      yield(cand)
    end
  end
end

return solar_terms
