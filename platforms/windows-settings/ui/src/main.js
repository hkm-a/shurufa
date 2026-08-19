import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowLeft,
  ArrowUpRight,
  BookOpenText,
  ChartColumn,
  ChevronUp,
  CircleDot,
  ClipboardList,
  Copy,
  createIcons,
  FolderOpen,
  History,
  Image,
  Info,
  Keyboard,
  Languages,
  LayoutDashboard,
  LayoutGrid,
  Lightbulb,
  Mic,
  MonitorSmartphone,
  Moon,
  Palette,
  Pin,
  Play,
  Power,
  RadioTower,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Sparkles,
  Sun,
  Trash2
} from "lucide";

const controlCenterIcons = {
  ArrowLeft,
  ArrowUpRight,
  BookOpenText,
  ChartColumn,
  ChevronUp,
  CircleDot,
  ClipboardList,
  Copy,
  FolderOpen,
  History,
  Image,
  Info,
  Keyboard,
  Languages,
  LayoutDashboard,
  LayoutGrid,
  Lightbulb,
  Mic,
  MonitorSmartphone,
  Moon,
  Palette,
  Pin,
  Play,
  Power,
  RadioTower,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Sparkles,
  Sun,
  Trash2
};

const pages = [
  { id: "workspace", label: "工作台", icon: "layout-dashboard" },
  { id: "general", label: "通用", icon: "settings-2" },
  { id: "input", label: "输入", icon: "keyboard" },
  { id: "stats", label: "统计", icon: "chart-column" },
  { id: "history", label: "历史", icon: "clipboard-list" },
  { id: "dictionary", label: "词库", icon: "book-open-text" },
  { id: "scheme", label: "方案", icon: "circle-dot" },
  { id: "phrases", label: "短语", icon: "list-plus" },
  { id: "symbols", label: "符号", icon: "smile-plus" },
  { id: "skin", label: "皮肤", icon: "palette" },
  { id: "sync", label: "跨设备", icon: "monitor-smartphone" },
  { id: "settings", label: "偏好", icon: "sliders-horizontal" }
];

// M9-1：导航分组（设置中心侧栏）；order 决定展示顺序
const NAV_GROUPS = [
  { label: "概览", pages: ["workspace"] },
  { label: "输入", pages: ["input", "scheme", "phrases", "symbols", "dictionary"] },
  { label: "效率", pages: ["history", "stats"] },
  { label: "外观", pages: ["skin"] },
  { label: "系统", pages: ["general", "sync", "settings"] }
];

// M9-1：全页搜索索引（页内面板关键词，静态声明即可覆盖全部设置）
const SETTINGS_SEARCH_INDEX = [
  { page: "workspace", label: "工作台", keywords: ["概览", "后台服务", "服务状态", "输入方案", "剪贴板历史", "热门词库", "直达"] },
  { page: "general", label: "通用", keywords: ["行为", "主题", "亮色", "暗色", "跟随系统", "悬浮球", "不透明度", "历史保留", "AI", "帮写", "润色", "翻译", "热键", "划词", "白名单"] },
  { page: "input", label: "输入", keywords: ["候选", "直达快捷", "快捷键", "中英切换", "大写锁定", "符号", "emoji", "引擎开关", "空格", "专业词", "医生", "律师", "代码", "场景词库"] },
  { page: "scheme", label: "方案", keywords: ["输入方案", "全拼", "双拼", "小鹤", "五笔", "仓颉"] },
  { page: "phrases", label: "短语", keywords: ["自定义词条", "词条", "短语", "编码"] },
  { page: "symbols", label: "符号", keywords: ["符号", "emoji", "表情", "搜索", "颜文字"] },
  { page: "history", label: "历史", keywords: ["剪贴板历史", "置顶", "批量删除", "批量置顶", "搜索"] },
  { page: "dictionary", label: "词库", keywords: ["词库", "更新", "回滚", "userdb", "词典", "云词库"] },
  { page: "stats", label: "统计", keywords: ["打字统计", "曲线", "字数", "日均"] },
  { page: "skin", label: "皮肤", keywords: ["外观", "颜色", "圆角", "字号", "JSON", "导入", "导出", "预设"] },
  { page: "sync", label: "跨设备", keywords: ["同步", "配对", "设备", "中继", "最近同步", "重试"] },
  { page: "settings", label: "偏好", keywords: ["默认输入法", "自启动", "开机启动", "高级", "会话"] }
];

// 主题："auto" 跟随系统（默认）；"light"|"dark" 由用户手选，记 localStorage
const THEME_KEY = "shurufa-settings-theme";
function currentTheme() {
  return localStorage.getItem(THEME_KEY) || "auto";
}
function applyTheme(theme) {
  const root = document.documentElement;
  if (theme === "light" || theme === "dark") {
    root.dataset.theme = theme;
  } else {
    delete root.dataset.theme;
  }
}
function toggleTheme() {
  const next = currentTheme() === "dark" ? "light" : currentTheme() === "light" ? "auto" : "dark";
  localStorage.setItem(THEME_KEY, next);
  applyTheme(next);
  // 顶部按钮的图标跟用户当前选的主题保持一致
  const btn = document.querySelector(".theme-toggle");
  if (btn) {
    const icon = next === "dark" ? "moon" : next === "light" ? "sun" : "monitor-smartphone";
    btn.innerHTML = `<i data-lucide="${icon}"></i>`;
    btn.title = `主题：${next === "auto" ? "跟随系统" : next === "light" ? "亮色" : "暗色"}`;
    createIcons({ icons: controlCenterIcons, nameAttr: "data-lucide", attrs: { class: "lucide" } });
  }
}
applyTheme(currentTheme());

let activePage = "workspace";
// M9-1：全页搜索状态与未保存修改追踪
let globalSearchQuery = "";
let searchOpen = false;
const dirtyPages = new Set();
function markDirty(page) { dirtyPages.add(page); }
function clearDirty(page) { dirtyPages.delete(page); }
let dashboard = {
  relay: "",
  service_status: "待启动",
  data_directory: ""
};
let historyEntries = [];
let historyQuery = "";
let notice = null;
// 输入法四项快捷键选项；null 表示尚未加载（ssr/失败场景一律禁用态）
let imeOptions = null;
let dictionaryInfo = { revision: "" };
let dictionaryHistoryList = [];
// 打字统计：null 表示尚未加载或读取失败（面板走兜底样式）
let typingStats = null;
// 皮肤编辑器：null=未加载；dirty=待保存；error=JSON 解析失败时给用户的提示
let skinState = { loaded: false, content: "", source: "none", user_path: "", dirty: false };
// 自定义短语（P1 #6）：编辑器行数据；null=未加载。
let phraseRows = null;
// 用户词库（P1 #12）：userdb 列表；null=未加载。
let userdbList = null;
// 按应用选项（weasel app_options）：进程名 → 自动英文直输；null=未加载。
let appOptions = null;
// 符号面板（P1 #11）：当前选中分类（null=全部；"recent"=最近使用）。
let activeSymbolCat = null;
// 符号面板搜索（Tier 11，搜狗/微信 emoji 面板搜索同款）：非空时显示
// 跨分类搜索结果（符号字符匹配 + 关键词索引 + 分类名匹配）。
let symbolQuery = "";
// emoji 肤色（Tier 8，搜狗 6.24.1「肤色多选及记忆」同款）：null=默认；
// 否则为修饰符码点（🏻🏼🏽🏾🏿），应用到手势类 emoji，本地持久化。
let emojiTone = loadEmojiTone();
// Emoji 引擎开关状态（直连算法服务读取）。
let engineOptionEmoji = true;
// 中英混输自动空格（librime switch en_spacer）；读取失败默认开。
let engineOptionEnSpacer = true;

// 符号面板分类数据：文本符号取自 rime-ice symbols_v.yaml 常用子集；
// emoji 分类与颜文字为 Tier 8 新增（搜狗 6.24.1「emoji 面板优化：分类、
// 肤色多选及记忆、新增颜文字」同类）。点击复制到剪贴板。
const SYMBOL_CATEGORIES = [
  { id: "common", label: "常用", symbols: ["，", "。", "、", "；", "：", "？", "！", "…", "·", "—", "～", "『", "』", "「", "」", "《", "》", "（", "）", "【", "】", "￥", "＄", "％", "＃", "＆", "＊"] },
  { id: "arrow", label: "箭头", symbols: ["↑", "↓", "←", "→", "↕", "↔", "↖", "↗", "↙", "↘", "↩", "↪", "↺", "↻", "⇒", "⇐", "⇑", "⇓", "⇔", "➜", "➡", "➤", "⟶", "⟵"] },
  { id: "math", label: "数学", symbols: ["±", "÷", "×", "∈", "∏", "∑", "≠", "≤", "≥", "≡", "≈", "∞", "√", "∠", "⊥", "∥", "∪", "∩", "∈", "∉", "⊂", "⊃", "∧", "∨", "⊕", "⊗", "∴", "∵"] },
  { id: "currency", label: "货币", symbols: ["￥", "¥", "＄", "$", "￡", "£", "€", "₩", "₪", "₫", "₭", "₮", "₱", "₹", "₺", "₨", "﷼", "¢", "¤"] },
  { id: "unit", label: "单位", symbols: ["℃", "℉", "°", "‰", "‱", "％", "㎜", "㎝", "㎞", "㎡", "㎏", "㎎", "㎐", "㏄", "㏈", "㏒"] },
  { id: "punct", label: "标点", symbols: ["、", "。", "「", "」", "『", "』", "【", "】", "〈", "〉", "《", "》", "〖", "〗", "〔", "〕", "〘", "〙", "〜", "〰", "〃"] },
  { id: "face", label: "表情", symbols: ["😀", "😁", "😂", "🤣", "😊", "😇", "🙂", "😉", "😍", "😘", "😗", "😋", "😛", "😜", "🤪", "😝", "🤗", "🤭", "🤫", "🤔", "🤨", "😐", "😶", "😏", "😒", "🙄", "😌", "😔", "😪", "🤤", "😴", "😷", "🤒", "🤕", "🤢", "🤮", "🤧", "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "😎", "🤓", "🧐", "😕", "😟", "🙁", "😮", "😯", "😲", "😳", "🥺", "😢", "😭", "😱", "😖", "😣", "😤", "😡", "😠", "🤬", "😈", "👿", "💀", "💩", "🤡", "👻", "👽", "🤖", "🎃"] },
  { id: "hand", label: "手势", symbols: ["👍", "👎", "👏", "🙏", "💪", "✋", "🤙", "👌", "🤝", "🙌", "👐", "🤲", "👊", "✊", "🤛", "🤜", "☝", "✌", "🤞", "🤟", "🤘", "👈", "👉", "👆", "👇", "🫰", "🫶", "✍", "🖐", "👋"] },
  { id: "animal", label: "动物", symbols: ["🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵", "🐔", "🐧", "🐦", "🦆", "🦅", "🦉", "🐺", "🐗", "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞", "🐢", "🐍", "🦎", "🐙", "🦑", "🦐", "🦀", "🐠", "🐟", "🐬", "🐳", "🦈", "🦓", "🦍", "🐘", "🦒", "🐕", "🐈"] },
  { id: "life", label: "生活", symbols: ["🍎", "🍊", "🍋", "🍌", "🍉", "🍇", "🍓", "🍑", "🍍", "🥝", "🍅", "🥑", "🥕", "🌽", "🍞", "🥐", "🧀", "🍖", "🍔", "🍟", "🍕", "🌮", "🥗", "🍲", "☕", "🍵", "🧋", "🍺", "🍷", "🥂", "🍰", "🎂", "🍦", "🍿", "🏠", "🏡", "🏢", "🏥", "🏫", "🏪", "🏔", "🌋", "🗻", "🌊", "🏖", "⛺", "🚗", "🚕", "🚌", "🚲", "✈", "🚀", "🚢", "🚉", "🚦", "⛽", "⚽", "🏀", "🏈", "⚾", "🎾", "🏐", "🏓", "🏸", "⛳", "🏹", "🎣", "🎿", "🏊", "🚴", "🏋", "🎮", "🎲", "🎯", "🎨", "🎬", "🎤", "🎧", "🎹", "🎸", "🎺", "🥁", "🎻", "📱", "💻", "⌨", "🖥", "📷", "🎥", "📺", "📻", "⏰", "📅", "📌", "📎", "✂", "🔑", "🔒", "🔓", "💡", "🔋", "💰", "💎", "🎁", "🎈", "🎉", "🎊", "🏆", "🥇", "🥈", "🥉", "🛒", "🛍", "🧸"] },
  { id: "heart", label: "爱心", symbols: ["❤", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣", "💕", "💞", "💓", "💗", "💖", "💘", "💝", "💟", "💌", "💢", "💤", "💦", "✨", "⭐", "🌟", "💫", "🔥", "💥", "🌈", "☀", "☁", "⛅", "🌙", "☔", "❄", "⚡"] },
  { id: "kaomoji", label: "颜文字", symbols: ["(◕‿◕)", "(≧∇≦)ﾉ", "(´･ω･`)", "(￣▽￣)~*", "(∩´∀`)∩", "＼(^o^)／", "( ͡° ͜ʖ ͡°)", "¯\\_(ツ)_/¯", "(╯°□°)╯︵ ┻━┻", "┬─┬ ノ( ゜-゜ノ)", "(ノಠ益ಠ)ノ彡┻━┻", "(ﾉ>ω<)ﾉ", "(｡•̀ᴗ-)✧", "(づ｡◕‿‿◕｡)づ", "(◣_◢)", "(´▽`ʃ♡ƪ)", "(≧◡≦)", "(｡◕‿◕｡)", "(ง •̀_•́)ง", "ᕙ(⇀‸↼‶)ᕗ", "(๑•̀ㅂ•́)و✧", "( •̀ᴗ•́ )و", "✧(≖ ◡ ≖✿)", "(°ロ°)!" ] },
  { id: "weather", label: "天气", symbols: ["☀", "☁", "⛅", "⛈", "☂", "☔", "☃", "⛄", "⛇", "☼", "☾", "☽", "🌙", "⭐", "🌈"] },
  { id: "music", label: "音乐", symbols: ["♪", "♫", "♬", "♩", "♭", "♯", "♮", "𝄞", "𝄡", "𝄢"] },
  { id: "chess", label: "棋牌", symbols: ["♔", "♕", "♖", "♗", "♘", "♙", "♠", "♥", "♣", "♦", "♤", "♡", "♧", "♢", "🀄"] },
  { id: "zodiac", label: "星座", symbols: ["♈", "♉", "♊", "♋", "♌", "♍", "♎", "♏", "♐", "♑", "♒", "♓"] }
];
// emoji 肤色修饰符（U+1F3FB..U+1F3FF，搜狗「肤色多选」同款）；点击切换全局肤色。
const EMOJI_TONES = ["🏻", "🏼", "🏽", "🏾", "🏿"];
// 支持肤色变体的基础 emoji（单码点手势类）。ZWJ 组合（如 🧑💻）与多人组合
// 未列入——变体规则复杂（需按位置插入修饰符），当前实现只覆盖最高频的手势。
const TONE_CAPABLE = new Set([
  "👍", "👎", "👏", "🙏", "💪", "✋", "🤙", "👌", "🤝", "🙌", "👐", "🤲",
  "👊", "✊", "🤛", "🤜", "☝", "✌", "🤞", "🤟", "🤘", "👈", "👉", "👆",
  "👇", "🫰", "🫶", "✍", "🖐", "👋",
]);
// emoji 分类 id 集合：这些分类显示肤色选择条。
const EMOJI_CATS = new Set(["face", "hand", "animal", "life", "heart"]);

function isEmojiCat(id) {
  return EMOJI_CATS.has(id);
}

// 应用当前肤色：TONE_CAPABLE 的手势类 emoji 追加修饰符（如 👍 + 🏻 = 👍🏻）；
// 非手势类原样返回。
function emojiWithTone(s) {
  return emojiTone && TONE_CAPABLE.has(s) ? s + emojiTone : s;
}

// ---- 符号面板搜索（Tier 11，搜狗/微信 emoji 面板搜索同款）----
// 常用 emoji 关键词索引：中文名 / 拼音 / 英文名 → emoji。精选 ~120 条
// 最高频的 emoji（表情/手势/动物/食物/爱心/生活），让"输 weixiao 出 😊"
// 这类面板内查找成立。文本符号与颜文字按字符本身匹配（无名称元数据）。
const EMOJI_SEARCH_INDEX = [
  // 表情
  ["微笑", "weixiao", "smile", "😊"], ["大笑", "daxiao", "laugh", "😂"], ["笑哭", "xiaoku", "joy", "🤣"],
  ["开心", "kaixin", "happy", "😄"], ["喜欢", "xihuan", "love", "😍"], ["亲吻", "qinwen", "kiss", "😘"],
  ["调皮", "tiaopi", "playful", "😜"], ["酷", "ku", "cool", "😎"], ["思考", "sikao", "think", "🤔"],
  ["哭", "ku", "cry", "😭"], ["生气", "shengqi", "angry", "😡"], ["困", "kun", "sleepy", "😴"],
  ["生病", "shengbing", "sick", "🤒"], ["恶心", "exin", "vomit", "🤢"], ["震惊", "zhenjing", "shock", "😱"],
  ["晕", "yun", "dizzy", "😵"], ["魔鬼", "mogui", "devil", "😈"], ["骷髅", "kugu", "skull", "💀"],
  ["鬼", "gui", "ghost", "👻"], ["外星人", "waixingren", "alien", "👽"], ["机器人", "jiqiren", "robot", "🤖"],
  // 手势
  ["赞", "zan", "thumbsup", "👍"], ["踩", "cai", "thumbsdown", "👎"], ["鼓掌", "guzhang", "clap", "👏"],
  ["祈祷", "qidao", "pray", "🙏"], ["谢谢", "xiexie", "thanks", "🙏"], ["感恩", "ganen", "grateful", "🙏"], ["肌肉", "jirou", "muscle", "💪"], ["挥手", "huishou", "wave", "👋"],
  ["拳头", "quantou", "fist", "👊"], ["击掌", "jizhang", "highfive", "🙌"], ["比心", "bixin", "heart", "🫶"],
  ["抱拳", "baoquan", "folded", "🤝"], ["握手", "woshou", "handshake", "🤝"], ["耶", "ye", "victory", "✌"],
  // 动物
  ["狗", "gou", "dog", "🐶"], ["猫", "mao", "cat", "🐱"], ["老鼠", "laoshu", "mouse", "🐭"],
  ["兔子", "tuzi", "rabbit", "🐰"], ["狐狸", "huli", "fox", "🦊"], ["熊", "xiong", "bear", "🐻"],
  ["熊猫", "xiongmao", "panda", "🐼"], ["老虎", "laohu", "tiger", "🐯"], ["狮子", "shizi", "lion", "🦁"],
  ["牛", "niu", "cow", "🐮"], ["猪", "zhu", "pig", "🐷"], ["青蛙", "qingwa", "frog", "🐸"],
  ["猴子", "houzi", "monkey", "🐵"], ["鸡", "ji", "chicken", "🐔"], ["企鹅", "qie", "penguin", "🐧"],
  ["蝴蝶", "hudie", "butterfly", "🦋"], ["蜜蜂", "mifeng", "bee", "🐝"], ["乌龟", "wugui", "turtle", "🐢"],
  ["蛇", "she", "snake", "🐍"], ["章鱼", "zhangyu", "octopus", "🐙"], ["鱼", "yu", "fish", "🐟"],
  ["鲸鱼", "jingyu", "whale", "🐳"], ["鲨鱼", "shayu", "shark", "🦈"], ["大象", "daxiang", "elephant", "🐘"],
  // 食物
  ["苹果", "pingguo", "apple", "🍎"], ["橘子", "juzi", "orange", "🍊"], ["柠檬", "ningmeng", "lemon", "🍋"],
  ["香蕉", "xiangjiao", "banana", "🍌"], ["西瓜", "xigua", "watermelon", "🍉"], ["葡萄", "putao", "grape", "🍇"],
  ["草莓", "caomei", "strawberry", "🍓"], ["桃", "tao", "peach", "🍑"], ["菠萝", "boluo", "pineapple", "🍍"],
  ["番茄", "fanqie", "tomato", "🍅"], ["牛油果", "niuyouguo", "avocado", "🥑"], ["玉米", "yumi", "corn", "🌽"],
  ["面包", "mianbao", "bread", "🍞"], ["奶酪", "nailao", "cheese", "🧀"], ["汉堡", "hanbao", "burger", "🍔"],
  ["薯条", "shutiao", "fries", "🍟"], ["披萨", "pisa", "pizza", "🍕"], ["沙拉", "shala", "salad", "🥗"],
  ["火锅", "huoguo", "hotpot", "🍲"], ["咖啡", "kafei", "coffee", "☕"], ["茶", "cha", "tea", "🍵"],
  ["啤酒", "pijiu", "beer", "🍺"], ["蛋糕", "dangao", "cake", "🎂"], ["冰淇淋", "bingqilin", "icecream", "🍦"],
  // 爱心
  ["心", "xin", "heart", "❤"], ["红心", "hongxin", "redheart", "❤"], ["爱心", "aixin", "loveheart", "💕"],
  ["破碎", "posui", "broken", "💔"], ["火花", "huohua", "fire", "🔥"], ["星星", "xingxing", "star", "⭐"],
  ["彩虹", "caihong", "rainbow", "🌈"], ["闪电", "shandian", "bolt", "⚡"], ["雪", "xue", "snow", "❄"],
  ["太阳", "taiyang", "sun", "☀"], ["月亮", "yueliang", "moon", "🌙"], ["雨", "yu", "rain", "☔"],
  // 生活
  ["房子", "fangzi", "house", "🏠"], ["车", "che", "car", "🚗"], ["飞机", "feiji", "plane", "✈"],
  ["火箭", "huojian", "rocket", "🚀"], ["轮船", "lunchuan", "ship", "🚢"], ["足球", "zuqiu", "soccer", "⚽"],
  ["篮球", "lanqiu", "basketball", "🏀"], ["乒乓球", "pingpang", "tabletennis", "🏓"], ["游戏", "youxi", "game", "🎮"],
  ["骰子", "touzi", "dice", "🎲"], ["音乐", "yinyue", "music", "🎵"], ["吉他", "jita", "guitar", "🎸"],
  ["相机", "xiangji", "camera", "📷"], ["电话", "dianhua", "phone", "📱"], ["电脑", "diannao", "computer", "💻"],
  ["钱", "qian", "money", "💰"], ["钻石", "zuanshi", "diamond", "💎"], ["礼物", "liwu", "gift", "🎁"],
  ["气球", "qiqiu", "balloon", "🎈"], ["奖杯", "jiangbei", "trophy", "🏆"], ["金币", "jinbi", "medal", "🥇"],
  ["购物", "gouwu", "shopping", "🛒"], ["玩具熊", "wanjuxiong", "teddy", "🧸"], ["蜡烛", "lazhu", "candle", "🕯"],
];

/// 搜索当前词：返回 (符号, 分类名) 列表，去重。
/// 匹配规则：关键词索引（中文名/拼音/英文名，忽略大小写）+ 符号字符包含 +
/// 分类名包含。结果按「索引命中优先、其余按原顺序」排列，符号去重。
function searchSymbols(query) {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const hits = [];
  const seen = new Set();
  const push = (s, label) => {
    if (seen.has(s)) return;
    seen.add(s);
    hits.push({ symbol: s, label });
  };
  // 1) 关键词索引命中（最高优先）：weixiao/微笑 → 😊
  for (const entry of EMOJI_SEARCH_INDEX) {
    for (let i = 0; i < entry.length - 1; i++) {
      if (entry[i].toLowerCase().includes(q)) {
        push(emojiWithTone(entry[entry.length - 1]), "emoji");
        break;
      }
    }
  }
  // 2) 符号字符包含 + 分类名包含：全部分类扫描
  for (const cat of SYMBOL_CATEGORIES) {
    const labelHit = cat.label.toLowerCase().includes(q);
    for (const s of cat.symbols) {
      const charHit = s.toLowerCase().includes(q);
      if (charHit || labelHit) push(emojiWithTone(s), cat.label);
    }
  }
  return hits;
}

// ---- 本地持久化（最近使用 / 肤色记忆）----
const RECENT_KEY = "shurufa.symbolRecents";
const TONE_KEY = "shurufa.emojiTone";
const RECENT_CAP = 30;

function loadRecents() {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? arr.filter((s) => typeof s === "string") : [];
  } catch {
    return [];
  }
}

function saveRecent(symbol) {
  try {
    const recents = loadRecents().filter((s) => s !== symbol);
    recents.unshift(symbol);
    localStorage.setItem(RECENT_KEY, JSON.stringify(recents.slice(0, RECENT_CAP)));
  } catch {
    /* localStorage 不可用时静默降级（最近使用不持久化，功能其余部分照常） */
  }
}

function loadEmojiTone() {
  try {
    const t = localStorage.getItem(TONE_KEY);
    return EMOJI_TONES.includes(t) ? t : null;
  } catch {
    return null;
  }
}

function saveEmojiTone(tone) {
  try {
    if (tone) localStorage.setItem(TONE_KEY, tone);
    else localStorage.removeItem(TONE_KEY);
  } catch {
    /* 静默降级 */
  }
}
// 预设皮肤列表（schemas/skins-index.json）；banner 为一次性成功/失败提示
let skinPresets = [];
let skinPresetBanner = null;
// 跨设备同步活动流（M8-1：最近收发记录/来源标签/状态）；null=未加载
let syncActivity = null;
// M10：配对向导状态变量（null = 无进行中向导）。
let pairWizard = null;
// M8-2：已配对设备（peers.json）+ 重命名/移除交互状态
let peers = null;
let renamingFp = null;
let confirmRemoveFp = null;
// M8-4：应用/网站直达编辑文本（code<TAB>名称<TAB>app|url<TAB>目标）
let shortcutsText = "";
// M8-3：历史面板批量选择状态
let historySelectMode = false;
let historySelected = new Set();
let confirmBatchDelete = false;
// 通用页（通用 6 字段）；null=未加载/读取失败（表单全部禁用）
let generalSettings = null;
// 语音转写卡片（wave 4 新挂在通用页里；speechSettings 与 general 完全独立
// 存储 / 独立 Tauri 命令），null=未加载/读取失败
let speechSettings = null;
// 输入方案页（wave 4 新增）：null=未加载；list=后端 list_input_schemes 返回的 4 项
let schemeList = null;
let schemeCurrent = "pinyin";
let schemeBanner = null;

// ---------------------------------------------------------------------------
// 悬浮外壳：bar（悬浮球）/ menu（菜单面板）/ page（页面子视图）三态。
// 窗口尺寸由后端 set_window_size 控制；位置由 onMoved 记忆、启动时恢复。
// ---------------------------------------------------------------------------

let uiMode = "bar";
let appliedSizeKey = null;
let autostartInfo = null;
let defaultIme = null;

// 面板逻辑尺寸（不含透明窗口四周的 PANEL_PAD 留白，阴影绘制区）
const PANEL_PAD = 10;
const PAGE_SIZES = {
  workspace: { width: 520, height: 640 },
  general: { width: 480, height: 660 },
  input: { width: 480, height: 560 },
  stats: { width: 560, height: 660 },
  history: { width: 520, height: 640 },
  dictionary: { width: 500, height: 600 },
  scheme: { width: 480, height: 600 },
  phrases: { width: 560, height: 640 },
  symbols: { width: 560, height: 640 },
  skin: { width: 560, height: 700 },
  sync: { width: 480, height: 560 },
  settings: { width: 500, height: 660 }
};

function windowSizeFor(mode) {
  let panel;
  // bar 态 = 悬浮球（38×38 实心彩色球，小而美）；menu 宽 = 主菜单 320 +
  // 间距 4 + 二级面板 236，高含底部悬浮球 38+6
  if (mode === "bar") panel = { width: 38, height: 38 };
  else if (mode === "menu") panel = { width: 560, height: 560 };
  else panel = PAGE_SIZES[activePage] || { width: 520, height: 640 };
  return {
    width: Math.round(panel.width + PANEL_PAD * 2),
    height: Math.round(panel.height + PANEL_PAD * 2)
  };
}

function sizeKeyFor(mode) {
  return mode === "page" ? `page:${activePage}` : mode;
}

// bar → menu/page 展开前记住条的位置；收回 bar 时精确回到原位
// （展开时窗口向上生长可能被工作区钳制平移，不恢复会让条越用越漂）。
let barPosStash = null;

async function applyMode(mode) {
  const prev = uiMode;
  if (mode !== "bar" && prev === "bar") {
    try {
      const pos = await getCurrentWindow().outerPosition();
      barPosStash = { x: pos.x, y: pos.y };
    } catch (_error) {
      barPosStash = null;
    }
  }
  uiMode = mode;
  const key = sizeKeyFor(mode);
  if (key !== appliedSizeKey) {
    appliedSizeKey = key;
    try {
      // anchor_bottom：窗口底边不动向上生长——菜单/页面弹在悬浮球上方
      await invoke("set_window_size", { size: { ...windowSizeFor(mode), anchor_bottom: true } });
    } catch (error) {
      console.error("[shurufa] set_window_size", error);
    }
  }
  if (mode === "bar" && barPosStash) {
    try {
      await invoke("restore_window_position", { x: barPosStash.x, y: barPosStash.y });
    } catch (_error) { /* 位置恢复失败不影响使用 */ }
    barPosStash = null;
  }
  if (mode === "menu") void refreshMenuData();
  render();
}

// 菜单打开时并行拉取二级菜单数据；完成后仍在菜单态才重绘。
async function refreshMenuData() {
  await Promise.allSettled([
    refreshTypingStats().catch(() => { typingStats = null; }),
    refreshSchemes().catch(() => {}),
    refreshImeOptions().catch(() => { imeOptions = null; }),
    refreshAppOptions().catch(() => { appOptions = null; }),
    refreshHistory().catch(() => { historyEntries = []; }),
    invoke("list_skins").then((v) => { skinPresets = v; }).catch(() => {}),
    refreshImeMode() // 条上中/英指示随菜单刷新
  ]);
  if (uiMode === "menu") render();
}

// 悬浮球 F 字形：纯白粗体 F + 柔和投影。球体背景由 CSS data-mode 着色
// （中文=橙 / 英文=蓝），这里只画 F，不再套橙色圆底（小而美：单一元素）。
function ballF(size = 22) {
  return `
    <svg class="logo-f" width="${size}" height="${size}" viewBox="0 0 36 36" aria-hidden="true">
      <text x="18" y="26.6" text-anchor="middle" font-family="'Arial Black','Microsoft YaHei',sans-serif"
        font-size="27" font-weight="900" font-style="italic" fill="#ffffff"
        style="filter: drop-shadow(0 1.5px 2px rgba(20,10,0,.28))">F</text>
    </svg>`;
}

// ---------------------------------------------------------------------------
// 搜狗风自绘图标（pics/4.png 状态条蓝色系）。条上只放真实可用的功能：
// 方案切换（全局热生效）、剪贴板历史、语音设置、菜单——不放无法作用于
// 焦点应用的假开关（中英/标点是 per-TSF-会话状态，控制中心切不到）。
// ---------------------------------------------------------------------------

// 蓝色粗体字图标（「拼」「双」等，与搜狗状态条「中」同款视觉）
function glyphIcon(ch) {
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><text x="12" y="18.4" text-anchor="middle"
    font-family="'Microsoft YaHei','SimHei',sans-serif" font-size="17" font-weight="700" fill="currentColor">${ch}</text></svg>`;
}

const BAR_ICONS = {
  // 剪贴板（历史）
  clip: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true">
    <rect x="5" y="4.6" width="14" height="16" rx="1.6"/>
    <path d="M9 4.6a3 3 0 0 1 6 0"/>
    <path d="M8.6 10.4h6.8M8.6 14h6.8M8.6 17.6h4.2"/></svg>`,
  // 麦克风（语音）—— 实心头 + 描边支架
  mic: `<svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="9.4" y="3.4" width="5.2" height="9.4" rx="2.6" fill="currentColor"/>
    <path d="M6.4 11.4a5.6 5.6 0 0 0 11.2 0" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
    <path d="M12 17v3.2M8.8 20.4h6.4" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`,
  // 四宫格（工具箱/菜单）—— 对角实心
  grid: `<svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="4.2" y="4.2" width="6.8" height="6.8" rx="1.6" fill="currentColor"/>
    <rect x="13" y="4.2" width="6.8" height="6.8" rx="1.6" fill="none" stroke="currentColor" stroke-width="1.9"/>
    <rect x="4.2" y="13" width="6.8" height="6.8" rx="1.6" fill="none" stroke="currentColor" stroke-width="1.9"/>
    <rect x="13" y="13" width="6.8" height="6.8" rx="1.6" fill="currentColor"/></svg>`
};

// 菜单头像：白底方块内的橙色粗线人像（pics/5.png 左上角）
const AVATAR_ICON = `<svg viewBox="0 0 48 48" fill="none" stroke="#F45832" stroke-width="4.6" stroke-linecap="round" aria-hidden="true">
  <circle cx="24" cy="16.5" r="7.4"/>
  <path d="M9.5 41.5a14.5 14.5 0 0 1 29 0"/></svg>`;

// 工具箱 3×2 彩色扁平图标（pics/5.png：齿轮/T恤/Ω/柱状图/双气泡/四宫格+红点）
const TOOLBOX_ICONS = {
  gear: `<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="#FF8A00" d="M19.14 12.94c.04-.3.06-.61.06-.94s-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.49.49 0 0 0-.59-.22l-2.39.96a7.03 7.03 0 0 0-1.62-.94l-.36-2.54a.48.48 0 0 0-.48-.41h-3.84a.48.48 0 0 0-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 0 0-.59.22L2.73 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 0 0-.12-.61zM12 15.6A3.6 3.6 0 1 1 12 8.4a3.6 3.6 0 0 1 0 7.2z"/></svg>`,
  shirt: `<svg viewBox="0 0 24 24" aria-hidden="true">
    <path fill="#F5478C" d="M7.9 3.6c.7 1.2 2.3 2 4.1 2s3.4-.8 4.1-2L21 6.9l-2.3 3.4-1.7-1V20H7v-10.7l-1.7 1L3 6.9z"/>
    <text x="12" y="15.6" text-anchor="middle" font-family="'Arial Black',sans-serif" font-size="7.5"
      font-weight="900" font-style="italic" fill="#ffffff">S</text></svg>`,
  omega: `<svg viewBox="0 0 24 24" fill="none" stroke="#2F7BE0" stroke-width="2.7" stroke-linecap="round" aria-hidden="true">
    <path d="M5.4 19.6h4.4v-2.3a6.9 6.9 0 0 1-3.5-6C6.3 7.2 8.8 4.6 12 4.6s5.7 2.6 5.7 6.7a6.9 6.9 0 0 1-3.5 6v2.3h4.4"/></svg>`,
  chart: `<svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="3.6" y="11.4" width="4.6" height="8.6" rx="1" fill="#4A90E2"/>
    <rect x="9.7" y="7.4" width="4.6" height="12.6" rx="1" fill="#FFC30F"/>
    <rect x="15.8" y="3.6" width="4.6" height="16.4" rx="1" fill="#7FBA00"/></svg>`,
  translate: `<svg viewBox="0 0 24 24" aria-hidden="true">
    <path fill="#2F7BE0" d="M9.6 2.8a6.8 6.8 0 0 1 6.8 6.8 6.8 6.8 0 0 1-6.8 6.8c-.5 0-1-.05-1.5-.16L4.6 17.6l.9-3A6.8 6.8 0 0 1 9.6 2.8z"/>
    <text x="9.6" y="12.6" text-anchor="middle" font-family="'Arial Black',sans-serif" font-size="8" font-weight="900" fill="#ffffff">E</text>
    <path fill="#1953A8" d="M17.2 12.4a4.6 4.6 0 0 1 4.6 4.6 4.6 4.6 0 0 1-4.6 4.6 4.7 4.7 0 0 1-1.1-.13l-2.5.83.63-2.1a4.6 4.6 0 0 1 2.97-7.8z"/>
    <text x="17.2" y="19.4" text-anchor="middle" font-family="'Microsoft YaHei',sans-serif" font-size="5.4" font-weight="700" fill="#ffffff">中</text></svg>`,
  more: `<svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="3.4" y="3.4" width="8" height="8" rx="2" fill="#7CC142"/>
    <rect x="12.8" y="3.4" width="8" height="8" rx="2" fill="#7CC142"/>
    <rect x="3.4" y="12.8" width="8" height="8" rx="2" fill="#7CC142"/>
    <rect x="12.8" y="12.8" width="8" height="8" rx="2" fill="#7CC142"/>
    <circle cx="19.6" cy="4.6" r="3.3" fill="#FF4B2A"/></svg>`
};

// 主菜单 6 项：全部 hover 右弹二级（搜狗菜单交互，pics/6、7.png）
const MENU_ITEMS = [
  { id: "skins", label: "更换皮肤", key: "H", icon: "palette" },
  { id: "schemes", label: "输入方案", key: "F", icon: "circle-dot" },
  { id: "options", label: "输入选项", key: "E", icon: "keyboard" },
  { id: "history", label: "剪贴板历史", key: "K", icon: "clipboard-list" },
  { id: "search", label: "桌面搜索", key: "S", icon: "search" },
  { id: "ai", label: "AI 助手", key: "A", icon: "sparkles" },
  { id: "pages", label: "设置中心", key: "Y", icon: "layout-dashboard" },
  { id: "help", label: "帮助", key: "Z", icon: "info" }
];

// 工具箱 3×2（图标形状复刻 pics/5.png，入口=打开对应设置页；
// 与主菜单二级合计覆盖全部页面）
const TOOLBOX_ITEMS = [
  { page: "settings", label: "偏好设置", key: "P", icon: "gear" },
  { page: "skin", label: "皮肤盒子", key: "M", icon: "shirt" },
  { page: "dictionary", label: "词库更新", key: "X", icon: "omega" },
  { page: "stats", label: "输入统计", key: "B", icon: "chart" },
  { page: "sync", label: "跨设备", key: "N", icon: "translate" },
  { page: "general", label: "通用设置", key: "O", icon: "more" }
];

// 全局中/英状态（算法服务全局语义）：null=未知；true=英文直输；false=中文
let imeAscii = null;


async function refreshImeMode() {
  try {
    imeAscii = await invoke("ime_mode_status");
    updateBarModeButton();
  } catch (_e) { /* 服务未就绪时保持未知态 */ }
}

async function cycleImeMode() {
  const prev = imeAscii;
  // 乐观更新：先翻按钮再等结果（点击切换要干脆利落）
  imeAscii = !(imeAscii ?? false);
  updateBarModeButton();
  try {
    imeAscii = await invoke("ime_mode_toggle");
    updateBarModeButton();
    showToast(imeAscii ? "已切换：英文直输（Shift 可切回中文）" : "已切换：中文输入");
  } catch (error) {
    imeAscii = prev;
    updateBarModeButton();
    showToast(String(error), true);
  }
}

// 悬浮球：38px 品牌色实心球 + 白色 F。球体颜色即中英状态（橙=中文/蓝=英文），
// 点击展开设置中心（菜单态）。中英切换在设置中心里做——悬浮球保持单一、
// 干净（2026-08-15 用户改版：小而美）。
function ballTemplate() {
  const modeTitle = imeAscii === null
    ? "中英状态读取中…"
    : imeAscii ? "当前：英文直输 · 点击打开设置中心" : "当前：中文 · 点击打开设置中心";
  return `
    <div id="ball" class="floating-ball" data-mode="${imeAscii === true ? "en" : "cn"}" data-tauri-drag-region>
      <button class="ball-main" data-mode-toggle="menu" title="FOX 设置中心 · ${modeTitle}" aria-label="打开设置中心">${ballF(22)}</button>
    </div>`;
}

// 条上「中/En」/「拼/双」的乐观更新：切换点击要干脆利落，先翻按钮再等
// 结果（失败回滚 + 报错），不做整窗 render()。
function updateBarModeButton() {
  const ball = app.querySelector(".floating-ball");
  if (!ball) return;
  ball.dataset.mode = imeAscii === true ? "en" : "cn";
  const main = ball.querySelector(".ball-main");
  if (main) {
    main.title = "FOX 设置中心 · " + (imeAscii === null
      ? "中英状态读取中…"
      : imeAscii ? "当前：英文直输 · 点击打开设置中心" : "当前：中文 · 点击打开设置中心");
  }
}

function updateBarSchemeButton() {
  const btn = app.querySelector("[data-bar-scheme]");
  if (!btn) return;
  const glyph = schemeCurrent === "double_pinyin" ? "双" : "拼";
  btn.innerHTML = glyphIcon(glyph);
  btn.title = schemeCurrent === "double_pinyin"
    ? "当前：双拼（小鹤）· 点击切换到全拼"
    : "当前：全拼 · 点击切换到双拼（小鹤）";
}

// 菜单态外壳：菜单面板悬浮在条上方（窗口底边锚定，条保持原位可见），
// 二级面板从主菜单右侧弹出——与搜狗状态栏菜单的空间关系一致。
function menuShellTemplate() {
  return `
    <div class="floating-shell">
      <div class="menu-zone">
        ${menuTemplate()}
        <div id="submenu" class="floating-submenu" role="menu"></div>
      </div>
      ${ballTemplate()}
    </div>`;
}

function menuTemplate() {
  const today = typingStats ? formatCount(typingStats.today_chars) : "0";
  const list = MENU_ITEMS.map(
    (item) => `
      <button class="menu-item" data-submenu="${item.id}">
        <span class="menu-item-icon"><i data-lucide="${item.icon}"></i></span>
        <span class="menu-item-label">${item.label}</span>
        <span class="menu-item-key">(${item.key})</span>
        <span class="menu-item-arrow"></span>
      </button>`
  ).join("");
  const toolbox = TOOLBOX_ITEMS.map(
    (item) => `
      <button class="toolbox-item" data-page="${item.page}">
        <span class="toolbox-icon">${TOOLBOX_ICONS[item.icon]}</span>
        <span class="toolbox-label">${item.label}<em>(${item.key})</em></span>
      </button>`
  ).join("");
  return `
    <div id="menu" class="floating-menu">
      <header class="menu-header">
        <div class="menu-avatar">${AVATAR_ICON}</div>
        <div class="menu-heading">
          <div class="menu-brand">FOX输入法 <em>享受输入</em></div>
          <div class="menu-stats">今日共输入 <b>${today}</b> 字</div>
        </div>
        <div class="menu-header-actions">
          <button type="button" class="theme-toggle" aria-label="切换主题" title="主题：跟随系统"><i data-lucide="monitor-smartphone"></i></button>
          <button class="icon-action menu-collapse" data-mode-toggle="bar" title="收起菜单"><i data-lucide="chevron-up"></i></button>
        </div>
      </header>
      <nav class="menu-list">${list}</nav>
      <div class="menu-toolbox-title">FOX 工具箱</div>
      <div class="menu-toolbox">${toolbox}</div>
    </div>`;
}

// ---------------------------------------------------------------------------
// 二级菜单内容：全部绑定真实命令（apply_skin / set_input_scheme /
// save_ime_options / copy_history / 打开目录与系统设置）。
// ---------------------------------------------------------------------------

function submenuHtml(id) {
  if (id === "skins") {
    const rows = skinPresets.length
      ? skinPresets
          .map(
            (p) => `
        <button class="submenu-item" data-skin-apply="${escapeHtml(p.id)}" title="${escapeHtml(p.name_en)}">
          <span class="submenu-swatch" style="background:${escapeHtml(p.preview_hint)}"></span>
          <span class="submenu-label">${escapeHtml(p.name_zh)}</span>
        </button>`
          )
          .join("")
      : `<div class="submenu-empty">预设皮肤读取中…</div>`;
    return `${rows}
      <div class="submenu-divider"></div>
      <button class="submenu-item" data-page="skin"><span class="submenu-mark"></span><span class="submenu-label">皮肤编辑器…</span></button>`;
  }
  if (id === "schemes") {
    if (!schemeList) return `<div class="submenu-empty">方案读取中…</div>`;
    return schemeList
      .map((s) => {
        const active = schemeCurrent === s.id;
        const disabled = s.status === "unavailable";
        return `
        <button class="submenu-item${disabled ? " disabled" : ""}" data-scheme-set="${escapeHtml(s.id)}" ${disabled ? "disabled" : ""}>
          <span class="submenu-mark">${active ? "●" : ""}</span>
          <span class="submenu-label">${escapeHtml(s.name_zh)}</span>
          <span class="submenu-side">${escapeHtml(s.name_en)}</span>
        </button>`;
      })
      .join("");
  }
  if (id === "options") {
    if (!imeOptions) return `<div class="submenu-empty">选项读取中…</div>`;
    const items = [
      ["shift_switch_cn_en", "Shift 切换中英文"],
      ["shift_space_full_shape", "Shift+空格 全/半角"],
      ["ctrl_period_ascii_punct", "Ctrl+. 中/英标点"],
      ["capslock_to_english", "CapsLock 直输英文"]
    ];
    return items
      .map(
        ([key, label]) => `
      <button class="submenu-item" data-ime-toggle="${key}" title="改动对正在输入的应用约 2 秒内热生效">
        <span class="submenu-mark">${imeOptions[key] ? "✓" : ""}</span>
        <span class="submenu-label">${label}</span>
      </button>`
      )
      .join("");
  }
  if (id === "history") {
    const entries = historyEntries.slice(0, 8);
    const rows = entries.length
      ? entries
          .map((e) => {
            const text = String(e.text ?? "");
            const label = text.length > 18 ? `${text.slice(0, 18)}…` : text;
            return `
        <button class="submenu-item" data-copy-id="${e.id}" title="点击复制到剪贴板">
          <span class="submenu-label ellipsis">${escapeHtml(label)}</span>
          <span class="submenu-side">${escapeHtml(e.kind)}</span>
        </button>`;
          })
          .join("")
      : `<div class="submenu-empty">暂无剪贴板历史</div>`;
    return `${rows}
      <div class="submenu-divider"></div>
      <button class="submenu-item" data-page="history"><span class="submenu-label">管理历史…</span></button>`;
  }
  if (id === "pages") {
    return pages
      .map(
        (p) => `
      <button class="submenu-item" data-page="${p.id}">
        <span class="submenu-icon"><i data-lucide="${p.icon}"></i></span>
        <span class="submenu-label">${p.label}</span>
      </button>`
      )
      .join("");
  }
  if (id === "search") {
    return `
      <div class="desktop-search-box">
        <input id="desktop-search-input" placeholder="搜应用 / 文件 / 计算…" autocomplete="off" spellcheck="false" />
        <div id="desktop-search-results" class="desktop-search-results"><div class="submenu-empty">输入关键词；算式（如 1+2*3）直接出结果</div></div>
      </div>`;
  }
  if (id === "ai") {
    return `
      <button class="submenu-item" data-menu-act="ai-panel" title="唤起 AI 帮写面板（同 Ctrl+Shift+W）">
        <span class="submenu-label">AI 帮写</span><span class="submenu-side">Ctrl+Shift+W</span>
      </button>
      <button class="submenu-item" data-menu-act="ai-hint" title="选中文本后按热键">
        <span class="submenu-label">划词润色 / 划词翻译</span><span class="submenu-side">R / T</span>
      </button>
      <div class="submenu-divider"></div>
      <button class="submenu-item" data-page="general"><span class="submenu-label">AI 热键与开关设置…</span></button>`;
  }
  if (id === "help") {
    return `
      <button class="submenu-item" data-menu-act="redeploy"><span class="submenu-label">重新部署方案（重建词典）</span></button>
      <button class="submenu-item" data-menu-act="data-dir"><span class="submenu-label">打开数据目录</span></button>
      <button class="submenu-item" data-menu-act="system-ime"><span class="submenu-label">系统输入法设置</span></button>
      <div class="submenu-divider"></div>
      <button class="submenu-item" data-menu-act="restart-service"><span class="submenu-label">启动 / 自愈后台服务</span></button>`;
  }
  return "";
}

function navGroupsHtml() {
  return NAV_GROUPS.map((group) => `
    <div class="nav-group">
      <p class="nav-group-label">${group.label}</p>
      ${group.pages.map((id) => {
        const p = pages.find((x) => x.id === id);
        if (!p) return "";
        const active = id === activePage ? " active" : "";
        const dirty = dirtyPages.has(id) ? '<i class="nav-dirty-dot" title="有未保存修改"></i>' : "";
        return `<button class="nav-item${active}" data-page="${id}"><i data-lucide="${p.icon}"></i><span>${p.label}</span>${dirty}</button>`;
      }).join("")}
    </div>`).join("");
}

function searchMatches() {
  const q = globalSearchQuery.trim().toLowerCase();
  if (!q) return [];
  return SETTINGS_SEARCH_INDEX
    .filter((e) => e.label.toLowerCase().includes(q) || e.keywords.some((k) => k.toLowerCase().includes(q)))
    .slice(0, 12);
}

function updateSearchDropdown() {
  const wrap = document.querySelector("#global-search-wrap");
  if (!wrap) return;
  const drop = wrap.querySelector("#global-search-drop");
  if (!drop) return;
  const hits = searchMatches();
  if (!searchOpen || hits.length === 0) {
    drop.innerHTML = hits.length === 0 && globalSearchQuery.trim() && searchOpen
      ? `<div class="global-search-empty">无匹配设置项</div>`
      : "";
    return;
  }
  drop.innerHTML = hits.map((h) => {
    const meta = pages.find((p) => p.id === h.page);
    return `<button class="global-search-item" data-page="${h.page}">
      <i data-lucide="${meta ? meta.icon : "search"}"></i>
      <span>${escapeHtml(meta ? meta.label : h.page)}</span>
      <small>${escapeHtml(h.keywords[0] || "")}</small>
    </button>`;
  }).join("");
  createIcons({ icons: controlCenterIcons });
}

function pageShellTemplate() {
  const meta = pages.find((p) => p.id === activePage) || pages[0];
  return `
    <div id="page" class="floating-page page-with-nav">
      <nav class="page-nav" aria-label="设置分类">${navGroupsHtml()}</nav>
      <div class="page-main">
        <header class="page-topbar">
          <button class="page-back icon-action" data-mode-toggle="menu" title="返回菜单"><i data-lucide="arrow-left"></i></button>
          <span class="page-topbar-title">${meta.label}</span>
          <div class="global-search-wrap" id="global-search-wrap">
            <i data-lucide="search" class="global-search-icon"></i>
            <input id="global-search" type="search" placeholder="搜索设置…" value="${escapeAttr(globalSearchQuery)}" autocomplete="off" aria-label="搜索全部设置" />
            <div class="global-search-drop" id="global-search-drop"></div>
          </div>
          <span class="page-topbar-grow"></span>
          <button class="page-collapse icon-action" data-mode-toggle="bar" title="收起为悬浮球"><i data-lucide="chevron-up"></i></button>
        </header>
        <div class="page-content">${pageTemplate()}</div>
      </div>
    </div>`;
}

function bindShell() {
  // M9-1：全页搜索框键盘/焦点（下拉点击走 data-page 委托）
  const searchInput = app.querySelector("#global-search");
  if (searchInput) {
    searchInput.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        const first = app.querySelector(".global-search-item[data-page]");
        if (first) {
          globalSearchQuery = "";
          searchOpen = false;
          void navigateTo(first.dataset.page);
        }
      } else if (event.key === "Escape") {
        globalSearchQuery = "";
        searchOpen = false;
        render();
      }
    });
    searchInput.addEventListener("focusin", () => {
      searchOpen = true;
      updateSearchDropdown();
    });
    searchInput.addEventListener("focusout", () => {
      window.setTimeout(() => {
        searchOpen = false;
        updateSearchDropdown();
      }, 180);
    });
  }
  // 主题切换按钮（菜单面板头部；render 会重置 DOM，每次 render 后重新点亮）。
  // data-mode-toggle / data-page 按钮统一走 #app 点击委托，不在此重复绑定。
  const themeBtn = app.querySelector(".theme-toggle");
  if (themeBtn) {
    const t = currentTheme();
    const icon = t === "dark" ? "moon" : t === "light" ? "sun" : "monitor-smartphone";
    themeBtn.innerHTML = `<i data-lucide="${icon}"></i>`;
    themeBtn.title = `主题：${t === "auto" ? "跟随系统" : t === "light" ? "亮色" : "暗色"}`;
    createIcons({ icons: controlCenterIcons });
    themeBtn.onclick = () => toggleTheme();
  }
  bindBarDrag();
}

// 悬浮球整体可拖（搜狗行为）：按下后位移超过阈值才开始拖窗口，
// 原地松开仍触发按钮点击。监听挂 window 级——球只有 38px 大，
// 挂在球上鼠标稍一移出就收不到 mousemove，拖动会时灵时不灵。
let barDragCtx = null;
window.addEventListener("mousemove", (event) => {
  if (!barDragCtx || barDragCtx.moved) return;
  if (Math.abs(event.screenX - barDragCtx.x) + Math.abs(event.screenY - barDragCtx.y) > 3) {
    barDragCtx.moved = true;
    try {
      void getCurrentWindow().startDragging();
    } catch (_error) { /* 非 Tauri 环境忽略 */ }
  }
});
window.addEventListener("mouseup", () => {
  if (!barDragCtx) return;
  const wasMoved = barDragCtx.moved;
  barDragCtx = null;
  // 拖拽结束：把条钳回工作区，防止整条被拖出屏幕丢失（无标题栏窗口没有
  // 系统级"部分可见"约束）。原地点击（未移动）不处理。
  if (wasMoved) void clampBarPosition();
});
// 拖拽结束后把窗口位置钳制回当前工作区（后端按 monitor work_area clamp，
// 触发 onMoved 落盘记忆，收敛无回环）。
async function clampBarPosition() {
  try {
    const win = getCurrentWindow();
    const pos = await win.outerPosition();
    await invoke("restore_window_position", { x: pos.x, y: pos.y });
  } catch (_e) { /* 忽略 */ }
}
// 拖动结束后的残留 moved 标记：任何新的按下（含菜单面板上）先清掉，
// 避免误吞下一次合法点击（原生拖动已吞掉自己的 click）。
window.addEventListener("mousedown", () => {
  if (barDragCtx && barDragCtx.moved) barDragCtx = null;
}, true);

function bindBarDrag() {
  const ball = app.querySelector("#ball");
  if (!ball) return;
  ball.addEventListener("mousedown", (event) => {
    if (event.button !== 0) return;
    // 阻止 SVG 文本选择与原生元素拖拽干扰窗口拖动
    event.preventDefault();
    barDragCtx = { x: event.screenX, y: event.screenY, moved: false };
  });
}

const app = document.querySelector("#app");

app.addEventListener("click", (event) => {
  // 拖动过的这次按下不当点击（原生拖动多数情况下已吞掉 click，这里兜底）
  if (barDragCtx && barDragCtx.moved) {
    barDragCtx = null;
    return;
  }
  const target = event.target;
  const el = target instanceof Element ? target : null;
  const button = el ? el.closest("button") : null;
  // 菜单态点击窗口透明区/面板外空白 → 收起菜单（等价"点外部关闭"）
  if (!button && uiMode === "menu" && el && !el.closest(".floating-menu, .floating-submenu, .floating-ball")) {
    void applyMode("bar");
    return;
  }
  if (!button || !app.contains(button) || button.disabled) return;
  if (button.dataset.modeToggle) {
    if (uiMode === "page" && button.dataset.modeToggle !== "page" && dirtyPages.has(activePage)
        && !window.confirm("当前页面有未保存的修改，收起将丢失。确定继续？")) {
      return;
    }
    void applyMode(button.dataset.modeToggle);
    return;
  }
  if (button.dataset.page) {
    void navigateTo(button.dataset.page);
    return;
  }
  if (button.dataset.barMode !== undefined) {
    void cycleImeMode();
    return;
  }
  if (button.dataset.barScheme !== undefined) {
    void cycleScheme();
    return;
  }
  if (button.dataset.skinApply) {
    void menuApplySkin(button.dataset.skinApply);
    return;
  }
  if (button.dataset.schemeSet) {
    void menuSetScheme(button.dataset.schemeSet);
    return;
  }
  if (button.dataset.imeToggle) {
    void menuToggleImeOption(button.dataset.imeToggle);
    return;
  }
  if (button.dataset.copyId) {
    void menuCopyHistory(Number(button.dataset.copyId));
    return;
  }
  if (button.dataset.searchHit) {
    void handleDesktopSearchHit(button.dataset.searchHit, button.dataset.target || "");
    return;
  }
  if (button.dataset.menuAct) {
    void menuHelpAction(button.dataset.menuAct);
  }
});

// Esc：菜单态收起为悬浮球；页面态返回菜单（搜狗式层级返回）。
// 仅在本窗口获得键盘焦点时生效；不干扰其它应用里的 Esc。
window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  if (uiMode === "page") {
    event.preventDefault();
    void applyMode("menu");
  } else if (uiMode === "menu") {
    event.preventDefault();
    void applyMode("bar");
  }
});

// ---------------------------------------------------------------------------
// 悬浮条 / 二级菜单动作：全部真实命令，成功后按搜狗习惯收起菜单 + toast。
// ---------------------------------------------------------------------------

// 条上「拼/双」：全拼 ⇄ 双拼（写 options.json，输入侧热生效）
async function cycleScheme() {
  const prev = schemeCurrent;
  const next = schemeCurrent === "double_pinyin" ? "pinyin" : "double_pinyin";
  // 乐观更新：先翻图标再等结果（点击切换要干脆利落）
  schemeCurrent = next;
  updateBarSchemeButton();
  try {
    await invoke("set_input_scheme", { scheme: next });
    updateBarSchemeButton();
    showToast(next === "double_pinyin" ? "已切换：双拼（小鹤）· 请输入双拼码" : "已切换：全拼");
  } catch (error) {
    schemeCurrent = prev;
    updateBarSchemeButton();
    showToast(String(error), true);
  }
}

async function menuApplySkin(id) {
  try {
    await invoke("apply_skin", { id });
    const meta = skinPresets.find((p) => p.id === id);
    await applyMode("bar");
    showToast(`已应用皮肤：${meta ? meta.name_zh : id}`);
  } catch (error) {
    showToast(String(error), true);
  }
}

async function menuSetScheme(id) {
  if (id === schemeCurrent) {
    await applyMode("bar");
    return;
  }
  try {
    await invoke("set_input_scheme", { scheme: id });
    schemeCurrent = id;
    const meta = (schemeList || []).find((s) => s.id === id);
    await applyMode("bar");
    showToast(id === "double_pinyin"
      ? "已切换到双拼（小鹤）· 请输入双拼码，如「我是说」= wouiuo"
      : `已切换到 ${meta ? meta.name_zh : id}`);
  } catch (error) {
    showToast(String(error), true);
  }
}

// 输入选项勾选：保存后留在菜单里刷新勾选态（便于连续调整多项）
async function menuToggleImeOption(key) {
  if (!imeOptions) return;
  const next = { ...imeOptions, [key]: !imeOptions[key] };
  try {
    await invoke("save_ime_options", { opts: next });
    imeOptions = next;
    render();
    showToast("已保存（输入侧约 2 秒内生效）");
  } catch (error) {
    showToast(String(error), true);
  }
}

async function menuCopyHistory(id) {
  try {
    await invoke("copy_history", { id });
    await applyMode("bar");
    showToast("已复制到剪贴板");
  } catch (error) {
    showToast(String(error), true);
  }
}

async function menuHelpAction(act) {
  // 麦克风：触发语音转写（命令返回实际结果消息）
  if (act === "speech") {
    try {
      const msg = await invoke("trigger_speech");
      await applyMode("bar");
      showToast(msg);
    } catch (error) {
      showToast(String(error), true);
    }
    return;
  }
  // M9-2：唤起 AI 帮写面板（host ai show → WM_AI_EXTERNAL_SHOW）
  if (act === "ai-panel") {
    try {
      const msg = await invoke("open_ai_panel");
      await applyMode("bar");
      showToast(msg);
    } catch (error) {
      showToast(String(error), true);
    }
    return;
  }
  if (act === "ai-hint") {
    showToast("划词润色：选中文本后按 Ctrl+Shift+R；划词翻译：Ctrl+Shift+T");
    return;
  }
  // 重新部署：同步等待宿主编译结果，成功/失败都带回具体消息
  if (act === "redeploy") {
    try {
      const msg = await invoke("redeploy_dictionaries");
      await applyMode("bar");
      showToast(msg);
    } catch (error) {
      showToast(String(error), true);
    }
    return;
  }
  const map = {
    "data-dir": ["open_data_directory", "已打开本地数据目录"],
    "system-ime": ["open_system_settings", "已打开 Windows 输入法设置"],
    "restart-service": ["start_service", "已发送后台服务启动请求"]
  };
  const entry = map[act];
  if (!entry) return;
  try {
    await invoke(entry[0]);
    await applyMode("bar");
    showToast(entry[1]);
  } catch (error) {
    showToast(String(error), true);
  }
}

// ---------------------------------------------------------------------------
// 二级菜单 hover 交互（pics/6、7.png）：悬停主项 ~140ms 展开右侧面板，
// 面板顶边与主项对齐、空间不足向上收；移到其他主项即切换，
// 移入头部/工具箱或移出菜单区则关闭。
// ---------------------------------------------------------------------------

function bindMenuHover() {
  const zone = app.querySelector(".menu-zone");
  if (!zone) return;
  const sub = zone.querySelector("#submenu");
  const menu = zone.querySelector("#menu");
  if (!sub || !menu) return;
  let timer = 0;
  const closeSubmenu = () => {
    sub.classList.remove("show");
    sub.innerHTML = "";
    menu.querySelectorAll(".menu-item.active").forEach((n) => n.classList.remove("active"));
  };
  const openSubmenu = (id, item) => {
    sub.innerHTML = submenuHtml(id);
    sub.classList.add("show");
    createIcons({ icons: controlCenterIcons });
    menu.querySelectorAll(".menu-item.active").forEach((n) => n.classList.remove("active"));
    item.classList.add("active");
    // 顶边对齐主项；底部越界时向上收（搜狗软键盘长列表同款行为）
    const zoneRect = zone.getBoundingClientRect();
    const itemRect = item.getBoundingClientRect();
    const wanted = itemRect.top - zoneRect.top;
    const top = Math.max(0, Math.min(wanted, zone.clientHeight - sub.offsetHeight));
    sub.style.top = `${Math.round(top)}px`;
  };
  menu.querySelectorAll(".menu-item[data-submenu]").forEach((item) => {
    item.addEventListener("mouseenter", () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => openSubmenu(item.dataset.submenu, item), 140);
    });
    // 点击主项立即展开（触屏/快速点击场景，与 hover 等价）
    item.addEventListener("click", () => {
      window.clearTimeout(timer);
      openSubmenu(item.dataset.submenu, item);
    });
  });
  menu.querySelectorAll(".toolbox-item, .menu-header").forEach((el) => {
    el.addEventListener("mouseenter", () => {
      window.clearTimeout(timer);
      closeSubmenu();
    });
  });
  sub.addEventListener("mouseenter", () => window.clearTimeout(timer));
  zone.addEventListener("mouseleave", () => {
    window.clearTimeout(timer);
    closeSubmenu();
  });
}

app.addEventListener("input", (event) => {
  if (event.target.id !== "history-search") return;
  historyQuery = event.target.value;
  render();
  const search = document.querySelector("#history-search");
  search?.focus();
  search?.setSelectionRange(historyQuery.length, historyQuery.length);
});

// 皮肤导入（M8-5）：webview 本地读文件文本，复用 save_skin 落盘后刷新编辑器
app.addEventListener("change", (event) => {
  if (event.target.id !== "skin-import-file") return;
  const file = event.target.files && event.target.files[0];
  if (!file) return;
  file.text()
    .then(async (text) => {
      await invoke("save_skin", { content: text });
      const payload = await invoke("skin_payload");
      skinState = { loaded: true, content: payload.content ?? "", source: payload.source, user_path: payload.user_path, dirty: false };
      clearDirty("skin");
      try { skinPresets = await invoke("list_skins"); } catch (_error) { skinPresets = []; }
      render();
      showToast("皮肤已导入并应用");
    })
    .catch((error) => showToast(String(error), true));
});

// 符号面板搜索（Tier 11）：实时过滤，重渲染后恢复焦点与光标位置。
app.addEventListener("input", (event) => {
  if (event.target.id !== "symbol-search") return;
  symbolQuery = event.target.value;
  render();
  const search = document.querySelector("#symbol-search");
  search?.focus();
  search?.setSelectionRange(symbolQuery.length, symbolQuery.length);
});

// M9-1：全页搜索（仅更新下拉，不整页重渲染以免丢焦点）
app.addEventListener("input", (event) => {
  if (event.target.id !== "global-search") return;
  globalSearchQuery = event.target.value;
  updateSearchDropdown();
});

// M9-1：未保存修改追踪——直达编辑器 / 按应用选项 / 短语行编辑
app.addEventListener("input", (event) => {
  if (event.target.id === "shortcuts-editor") { markDirty("input"); return; }
  if (event.target.closest(".app-options-panel")) { markDirty("input"); return; }
  if (event.target.closest("[data-phrase-row]")) { markDirty("phrases"); return; }
});

// M9-3：桌面快捷搜索（悬浮条子菜单内嵌搜索框，200ms 防抖）
let desktopSearchTimer = 0;
app.addEventListener("input", (event) => {
  if (event.target.id !== "desktop-search-input") return;
  window.clearTimeout(desktopSearchTimer);
  desktopSearchTimer = window.setTimeout(() => void runDesktopSearch(), 200);
});
app.addEventListener("keydown", (event) => {
  if (event.target.id !== "desktop-search-input") return;
  if (event.key === "Enter") {
    event.preventDefault();
    window.clearTimeout(desktopSearchTimer);
    void runDesktopSearch();
  }
});

async function runDesktopSearch() {
  const input = document.querySelector("#desktop-search-input");
  const box = document.querySelector("#desktop-search-results");
  if (!input || !box) return;
  const query = input.value.trim();
  if (!query) {
    box.innerHTML = `<div class="submenu-empty">输入关键词；算式（如 1+2*3）直接出结果</div>`;
    return;
  }
  box.innerHTML = `<div class="submenu-empty">搜索中…</div>`;
  try {
    const result = await invoke("desktop_search", { query });
    const parts = [];
    if (result.calc) {
      parts.push(`<div class="desktop-search-hit desktop-search-calc"><span class="submenu-label">= ${escapeHtml(result.calc)}</span><button class="ghost-action" data-search-hit="calc" data-target="${escapeAttr(result.calc)}">复制</button></div>`);
    }
    if (result.apps && result.apps.length) {
      parts.push(`<p class="desktop-search-title">应用</p>`);
      parts.push(...result.apps.map((item) => `<button class="desktop-search-hit" data-search-hit="app" data-target="${escapeAttr(item.target)}" title="${escapeAttr(item.target)}"><span class="submenu-label ellipsis">${escapeHtml(item.name)}</span><span class="submenu-side">启动</span></button>`));
    }
    if (result.files && result.files.length) {
      parts.push(`<p class="desktop-search-title">文件</p>`);
      parts.push(...result.files.map((item) => `<button class="desktop-search-hit" data-search-hit="file" data-target="${escapeAttr(item.target)}" title="${escapeAttr(item.target)}"><span class="submenu-label ellipsis">${escapeHtml(item.name)}</span><span class="submenu-side">定位</span></button>`));
    }
    box.innerHTML = parts.length ? parts.join("") : `<div class="submenu-empty">无结果</div>`;
  } catch (error) {
    box.innerHTML = `<div class="submenu-empty">${escapeHtml(String(error))}</div>`;
  }
}

// M9-6：划词白名单保存（textarea change 即存，复用通用保存通道）
app.addEventListener("change", (event) => {
  if (event.target.id !== "general-selection-whitelist") return;
  if (!generalSettings) return;
  const list = (event.target.value || "").split("\n").map((item) => item.trim().toUpperCase()).filter(Boolean);
  const next = { ...generalSettings, selection_app_whitelist: [...new Set(list)].slice(0, 50) };
  invoke("save_general_settings", { s: next })
    .then(() => { generalSettings = next; showToast("已保存"); })
    .catch((error) => showToast(String(error), true));
});

async function handleDesktopSearchHit(kind, target) {
  try {
    const msg = await invoke("launch_desktop_target", { kind, target });
    if (kind !== "calc") await applyMode("bar");
    showToast(msg);
  } catch (error) {
    showToast(String(error), true);
  }
}

function statusPill() {
  const running = dashboard.service_status === "运行中";
  return `<span class="status-pill ${running ? "online" : "idle"}"><span></span>${dashboard.service_status}</span>`;
}

function workspacePage() {
  // 后台服务（输入引擎 + 剪贴板同步）应常驻自愈，不提供"停止"入口——
  // 停止会让 TSF 失去引擎（输入法失效），旧版还曾导致应用无响应。
  // 仅在异常停止时显示"启动"（自愈入口）。
  const serviceAction = dashboard.service_status === "运行中"
    ? `<span class="status-pill online"><span></span>后台服务运行中</span>`
    : `<button class="primary-action" data-action="start-service"><i data-lucide="play"></i>启动后台服务</button>`;
  // 当前输入方案（动态）：全拼 / 双拼（小鹤）；五笔/仓颉不可用则回落默认
  const schemeLabel = (() => {
    const meta = (schemeList || []).find((s) => s.id === schemeCurrent);
    if (meta && meta.status !== "unavailable") return meta.name_zh;
    return schemeCurrent === "double_pinyin" ? "双拼" : "拼音";
  })();
  const schemeSub = schemeCurrent === "double_pinyin"
    ? "小鹤双拼 · 输入双拼码（如 wouiuo=我是说）"
    : "雾凇拼音 · 全拼输入";
  return `
    <section class="page workspace-page">
      <header class="page-header">
        <div><p class="eyebrow">SHURUFA CONTROL CENTER</p><h1>工作台</h1></div>
        <div class="header-actions">${statusPill()}<button class="icon-action" data-action="refresh" title="刷新后台状态"><i data-lucide="refresh-cw"></i></button></div>
      </header>
      <div class="hero-card">
        <div class="hero-copy"><p class="eyebrow accent">输入与剪贴板</p><h2>管理输入与剪贴板同步</h2><p>雾凇拼音、云词库与跨设备剪贴板历史在此统一管理。</p></div>
        ${serviceAction}
      </div>
      <div class="metric-grid">
        <button class="metric-card metric-link" data-page="scheme"><div class="metric-icon teal"><i data-lucide="keyboard"></i></div><span>输入方案</span><strong>${escapeHtml(schemeLabel)}</strong><p>${escapeHtml(schemeSub)}</p></button>
        <button class="metric-card metric-link" data-page="history"><div class="metric-icon blue"><i data-lucide="clipboard-list"></i></div><span>剪贴板历史</span><strong>Ctrl+Shift+V</strong><p>查看、复制和整理历史</p></button>
        <button class="metric-card metric-link" data-page="dictionary"><div class="metric-icon coral"><i data-lucide="book-open-text"></i></div><span>热门词库</span><strong>rime-ice</strong><p>检查并更新词库</p></button>
      </div>
    </section>`;
}

function inputPage() {
  // M8-4：进入输入页即拉取直达清单（完成后若仍在本页则重绘）。
  void refreshShortcuts().catch(() => { shortcutsText = ""; });
  // 当前输入方案（与工作台一致）
  const schemeLabel = (() => {
    const meta = (schemeList || []).find((s) => s.id === schemeCurrent);
    if (meta && meta.status !== "unavailable") return meta.name_zh;
    return schemeCurrent === "double_pinyin" ? "双拼" : "拼音";
  })();
  const schemeDesc = schemeCurrent === "double_pinyin"
    ? "小鹤双拼已启用 · 每字两键（如 wouiuo=我是说）"
    : "雾凇拼音方案已部署 · 全拼输入";
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">INPUT</p><h1>输入</h1></div>${statusPill()}</header>
      <article class="setting-panel">
        <div class="setting-row selected"><div class="row-icon"><i data-lucide="circle-dot"></i></div><div><h3>${escapeHtml(schemeLabel)}输入</h3><p>${escapeHtml(schemeDesc)}</p></div><button class="outline-action" data-page="scheme"><i data-lucide="keyboard"></i>切换方案</button></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="panel-top"></i></div><div><h3>候选窗位置</h3><p>跟随输入光标，或固定屏幕角落（改动约 2 秒内生效）</p></div><div class="row-side"><select data-general-field="candidate_position" aria-label="候选窗位置">${[
          ["follow", "跟随光标"],
          ["bottom_right", "固定右下角"],
          ["bottom_left", "固定左下角"]
        ].map(([v, label]) => `<option value="${v}"${(generalSettings?.candidate_position || "follow") === v ? " selected" : ""}>${label}</option>`).join("")}</select></div></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="rows-3"></i></div><div><h3>候选面板</h3><p>单行候选条，或多行候选面板（按 ↓ 唤出，搜狗 16.3b 同款；多行布局随 M7 落地）</p></div><div class="row-side"><select data-general-field="candidate_panel_mode" aria-label="候选面板模式">${[
          ["single", "单行候选条"],
          ["multi", "多行候选面板"]
        ].map(([v, label]) => `<option value="${v}"${(generalSettings?.candidate_panel_mode || "single") === v ? " selected" : ""}>${label}</option>`).join("")}</select></div></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="sparkles"></i></div><div><h3>候选与历史</h3><p>使用 Ctrl+Shift+V 呼出剪贴板历史</p></div><button class="outline-action" data-page="history"><i data-lucide="clipboard-list"></i>管理历史</button></div>
      </article>
      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="rocket"></i></div><div><h3>直达快捷</h3><p>输入触发码出直达候选：🖥 打开应用 / 🌐 打开网址（搜狗 15.2 灵犀候选直达同类）</p></div></div>
        <textarea id="shortcuts-editor" rows="6" spellcheck="false" placeholder="每行一条：触发码  名称  app|url  目标&#10;weixin&#9;微信&#9;app&#9;C:/apps/wechat.exe&#10;baidu&#9;百度&#9;url&#9;https://www.baidu.com">${escapeHtml(shortcutsText)}</textarea>
        <p class="field-note">每行：触发码（小写字母数字） 名称  类型(app/url)  目标；保存即生效（引擎每次按键重新加载快捷表）</p>
        <button class="primary-action compact" data-action="save-shortcuts"><i data-lucide="save"></i>保存直达</button>
      </article>
      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="stethoscope"></i></div><div><h3>专业词场景（M10-1 / v1.2）</h3><p>按领域挂载场景词库：医生 / 律师 / 代码 / 生僻字；保存后重建词典生效（搜狗 16.2 场景词库同类）</p></div></div>
        <div class="field-action">
          <select id="scenario-select" aria-label="专业词场景">
            <option value="none" ${(generalSettings?.scenario_dict || "none") === "none" ? "selected" : ""}>无（默认）</option>
            <option value="doctor" ${(generalSettings?.scenario_dict || "") === "doctor" ? "selected" : ""}>医生</option>
            <option value="lawyer" ${(generalSettings?.scenario_dict || "") === "lawyer" ? "selected" : ""}>律师</option>
            <option value="code" ${(generalSettings?.scenario_dict || "") === "code" ? "selected" : ""}>代码</option>
            <option value="rare" ${(generalSettings?.scenario_dict || "") === "rare" ? "selected" : ""}>生僻字</option>
          </select>
          <button class="primary-action compact" data-action="save-scenario"><i data-lucide="save"></i>保存并重建词典</button>
        </div>
      </article>
      <article class="hint-card"><i data-lucide="lightbulb"></i><p>后台服务负责剪贴板历史与跨设备同步。它会以隐藏窗口运行。语音转写（Ctrl+Shift+S）当前为 dev-stub。</p></article>
    </section>`;
}

function historyPage() {
  const query = historyQuery.trim().toLocaleLowerCase();
  const entries = historyEntries.filter((entry) => `${entry.text} ${entry.source_app} ${entry.kind}`.toLocaleLowerCase().includes(query));
  const list = entries.length
    ? entries.map((entry) => {
        const selected = historySelected.has(entry.id);
        const selectBox = historySelectMode
          ? `<button class="history-check ${selected ? "checked" : ""}" data-action="history-select" data-id="${entry.id}" aria-label="选择条目"><i data-lucide="${selected ? "check-square" : "square"}"></i></button>`
          : "";
        const actions = historySelectMode
          ? ""
          : `<div class="history-actions">
              <button class="icon-action" data-action="copy-history" data-id="${entry.id}" title="复制到剪贴板"><i data-lucide="copy"></i></button>
              <button class="icon-action" data-action="toggle-pin-history" data-id="${entry.id}" data-pinned="${entry.pinned}" title="${entry.pinned ? "取消置顶" : "置顶"}"><i data-lucide="pin"></i></button>
              <button class="icon-action danger-action" data-action="delete-history" data-id="${entry.id}" title="删除历史条目"><i data-lucide="trash-2"></i></button>
            </div>`;
        return `
        <article class="history-entry${selected ? " selected" : ""}">
          ${selectBox}
          <div class="history-kind ${entry.pinned ? "pinned" : ""}"><i data-lucide="${entry.kind === "图片" ? "image" : entry.kind === "文件" ? "folder-open" : "copy"}"></i></div>
          <div class="history-copy"><div class="history-title">${escapeHtml(entry.text)}</div><p>${escapeHtml(entry.kind)}${entry.source_app ? ` · ${escapeHtml(entry.source_app)}` : ""}</p></div>
          ${actions}
        </article>`;
      }).join("")
    : `<div class="history-empty"><i data-lucide="clipboard-list"></i><p>${query ? "没有匹配的历史内容" : "还没有可管理的剪贴板历史"}</p></div>`;
  const batchBar = historySelectMode ? `<div class="history-batchbar">
    <button class="ghost-action" data-action="history-select-all">全选</button>
    <button class="ghost-action" data-action="history-batch-pin">置顶</button>
    <button class="ghost-action" data-action="history-batch-unpin">取消置顶</button>
    <button class="ghost-action ${confirmBatchDelete ? "danger-action" : ""}" data-action="history-batch-delete">${confirmBatchDelete ? "确认删除所选" : "删除所选"}</button>
    <span class="history-batchcount">已选 ${historySelected.size} 项</span>
  </div>` : "";
  return `
    <section class="page settings-page history-page">
      <header class="page-header"><div><p class="eyebrow">CLIPBOARD</p><h1>剪贴板历史</h1></div>${historySelectMode
        ? `<button class="outline-action" data-action="history-select-clear">清除选择</button><button class="outline-action" data-action="history-select-mode">完成</button>`
        : `<button class="outline-action" data-action="history-select-mode"><i data-lucide="check-square"></i>选择</button><button class="outline-action" data-action="clear-history"><i data-lucide="trash-2"></i>清空未置顶</button>`}</header>
      <div class="history-toolbar"><label class="history-search"><i data-lucide="search"></i><input id="history-search" value="${escapeHtml(historyQuery)}" placeholder="搜索历史内容或来源" /></label><button class="icon-action" data-action="refresh-history" title="刷新历史"><i data-lucide="refresh-cw"></i></button></div>
      ${batchBar}
      <section class="history-list">${list}</section>
    </section>`;
}

function dictionaryPage() {
  const revision = dictionaryInfo.revision || "读取中…";
  const history = Array.isArray(dictionaryHistoryList) ? dictionaryHistoryList : [];
  const options = history
    .map((rev) => `<option value="${escapeHtml(rev)}">${escapeHtml(rev)}</option>`)
    .join("");
  const historyBlock = history.length
    ? `<div class="setting-row"><div class="row-icon"><i data-lucide="history"></i></div><div><h3>回滚到指定版本</h3><p>最多保留最近 5 个本地快照；选好后点右侧按钮</p></div><div class="row-side"><select id="dict-rollback-target">${options}</select><button class="outline-action" data-action="rollback-dictionary-to"><i data-lucide="arrow-up-right"></i>回滚到所选</button></div></div>`
    : `<div class="setting-row"><div class="row-icon dim"><i data-lucide="history"></i></div><div><h3>回滚到指定版本</h3><p>暂无本地历史快照（更新一次后即会出现）</p></div><span class="row-state">空</span></div>`;
  // 用户词库管理（P1 #12）：列出 userdb + 导出/清空
  const userdbs = Array.isArray(userdbList) ? userdbList : [];
  const userdbRows = userdbs.length
    ? userdbs.map((u) => `
      <div class="setting-row">
        <div class="row-icon"><i data-lucide="database"></i></div>
        <div><h3>${escapeHtml(u.name)}</h3><p>${(u.size_bytes / 1024).toFixed(1)} KB · 本地学习记录${u.backups ? ` · ${u.backups} 份备份` : ""}</p></div>
        <div class="row-side">
          <button class="outline-action" data-action="export-userdb" data-name="${escapeHtml(u.name)}"><i data-lucide="download"></i>导出</button>
          <button class="outline-action danger-action" data-action="clear-userdb" data-name="${escapeHtml(u.name)}"><i data-lucide="trash-2"></i>清空</button>
        </div>
      </div>`).join("")
    : `<div class="setting-row"><div class="row-icon dim"><i data-lucide="database"></i></div><div><h3>用户词库</h3><p>读取中…</p></div></div>`;
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">DICTIONARY</p><h1>词库</h1></div></header>
      <article class="setting-panel dictionary-panel">
        <div class="setting-row"><div class="row-icon coral"><i data-lucide="book-open-text"></i></div><div><h3>热门云词库</h3><p>rime-ice · 常用词与流行表达</p><p class="field-note">当前词典版本：${escapeHtml(revision)}</p></div><div class="row-side"><button class="outline-action" data-action="update-dictionary"><i data-lucide="refresh-cw"></i>更新词库</button><button class="outline-action" data-action="rollback-dictionary"><i data-lucide="arrow-up-right"></i>回滚到上一版</button></div></div>
        <div class="divider"></div>
        ${historyBlock}
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon"><i data-lucide="shield-check"></i></div><div><h3>本地校验</h3><p>下载完成后校验内容，再替换本地词典</p></div><span class="row-state">已保护</span></div>
      </article>
      <article class="setting-panel dictionary-panel">
        <div class="setting-row"><div class="row-icon teal"><i data-lucide="database"></i></div><div><h3>用户词库（本地学习记录）</h3><p>导出可备份「用词习惯」；清空会重置该词库的调频与自造词</p></div></div>
        <div class="divider"></div>
        ${userdbRows}
      </article>
      <article class="hint-card"><i data-lucide="info"></i><p>更新完成后，重启输入法即可应用新词库。清空用户词库前会自动导出备份到数据目录 userdb-backups/。</p></article>
    </section>`;
}

function syncPage() {
  // M8-1：进入同步页即拉取活动流（完成后若仍在本页则重绘）。
  void refreshSyncActivity().catch(() => { syncActivity = null; });
  // M8-2：拉取已配对设备列表。
  void refreshPeers().catch(() => { peers = null; });
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">CROSS DEVICE</p><h1>跨设备</h1></div>${statusPill()}</header>
      <article class="setting-panel relay-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="radio-tower"></i></div><div><h3>自托管中继</h3><p>跨网段时使用；留空则关闭</p></div></div>
        <label class="field-label" for="relay">中继地址</label>
        <div class="field-action"><input id="relay" value="${escapeHtml(dashboard.relay)}" placeholder="relay.example.com:48633" /><button class="primary-action compact" data-action="save-relay">保存</button></div>
        <p class="field-note">保存后会在后台服务下次启动时生效。</p>
      </article>
      <div class="metric-grid sync-metrics"><article class="metric-card"><div class="metric-icon teal"><i data-lucide="copy"></i></div><span>文本</span><strong>双向同步</strong></article><article class="metric-card"><div class="metric-icon blue"><i data-lucide="image"></i></div><span>图片</span><strong>双向同步</strong></article></div>
      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="activity"></i></div><div><h3>最近同步</h3><p>跨设备收发记录（来源设备 / 状态 / 时间，最多 50 条）</p></div></div>
        ${syncActivityRows()}
      </article>
      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="smartphone"></i></div><div><h3>已配对设备</h3><p>设备状态 / 重命名 / 移除；发起配对请在终端执行 shurufa-host.exe pair &lt;对方IP&gt;</p></div></div>
        ${deviceRows()}
      </article>
    </section>`;
}

// M10：配对向导状态渲染（prompt → 确认码大号展示；done/failed → 结果）。
function pairWizardHtml() {
  if (!pairWizard || pairWizard.phase === 'idle') return '';
  if (pairWizard.phase === 'waiting') {
    return '<div class="setting-row"><div class="row-icon dim"><i data-lucide="loader-circle"></i></div><div><h3>正在连接对方…</h3><p>等待对方设备响应（约 30 秒内）</p></div></div>';
  }
  if (pairWizard.phase === 'prompt') {
    return '<div class="pair-wizard">' +
      '<p class="pair-code-label">对方设备「' + escapeHtml(pairWizard.peer_name || '未知') + '」的确认码</p>' +
      '<div class="pair-code">' + escapeHtml(pairWizard.code || '……') + '</div>' +
      '<p class="field-note">请核对对方屏幕上的确认码与本页一致后，点击「确认配对」；对方同时点击「是」即可完成配对。</p>' +
      '<div class="field-action">' +
      '<button class="primary-action compact" data-action="pair-confirm"><i data-lucide="check"></i>确认配对</button>' +
      '<button class="outline-action compact" data-action="pair-cancel">取消</button>' +
      '</div></div>';
  }
  const failed = pairWizard.phase === 'failed';
  return '<div class="setting-row"><div class="row-icon ' + (failed ? 'coral' : 'teal') + '"><i data-lucide="' + (failed ? 'x-circle' : 'check-circle-2') + '"></i></div>' +
    '<div><h3>' + (failed ? '配对失败' : '配对成功') + '</h3><p>' + escapeHtml(pairWizard.message || '') + '</p></div>' +
    '<button class="ghost-action" data-action="pair-dismiss">关闭</button></div>';
}

// M10：配对向导轮询（每 2s 拉状态，最多 40 次；离开同步页自动停）。
let pairPollTimer = 0;
let pairPollCount = 0;
async function pollPairState() {
  if (!pairWizard || activePage !== 'sync') return;
  if (pairPollCount >= 40) {
    stopPairPoll();
    if (pairWizard.phase !== 'done' && pairWizard.phase !== 'failed') {
      pairWizard = { phase: 'failed', message: '等待超时，请重试' };
      render();
    }
    return;
  }
  pairPollCount += 1;
  try {
    const state = await invoke('pair_ui_state');
    const prevPhase = pairWizard.phase;
    pairWizard = { ...pairWizard, ...state };
    if (state.phase === 'done' || state.phase === 'failed') {
      stopPairPoll();
      await refreshPeers().catch(() => {});
      render();
      showToast(state.message || (state.phase === 'done' ? '配对成功' : '配对失败'), state.phase === 'failed');
      return;
    }
    if (state.phase !== prevPhase) render();
  } catch (error) {
    showToast(String(error), true);
    stopPairPoll();
  }
}
function stopPairPoll() {
  if (pairPollTimer) {
    window.clearInterval(pairPollTimer);
    pairPollTimer = 0;
  }
}
function startPairPoll() {
  stopPairPoll();
  pairPollCount = 0;
  pairPollTimer = window.setInterval(() => void pollPairState(), 2000);
}
// M8-1：最近同步活动行（来源标签/方向/类型/状态/相对时间）。
// 相对时间（毫秒时间戳 → “N 秒/分钟/小时/天前”）。
function relTimeAgo(ms) {
  const s = Math.max(1, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.round(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.round(s / 3600)} 小时前`;
  return `${Math.round(s / 86400)} 天前`;
}

function syncActivityRows() {
  const list = syncActivity?.entries || [];
  if (list.length === 0) {
    return `<div class="setting-row"><div class="row-icon dim"><i data-lucide="activity"></i></div><div><h3>暂无同步记录</h3><p>跨设备收发后这里会显示最近 50 条（来源设备 / 状态 / 时间）</p></div></div>`;
  }
  const iconFor = { text: "type", image: "image", file: "file-text" };
  const dirFor = (d) => (d === "in" ? "收到" : "发出");
  return list.map((e) => {
    const peer = e.peer ? `来自 ${escapeHtml(e.peer)}` : (e.direction === "in" ? "来自对端" : "本机发出");
    const statusCls = e.status === "failed" ? "pill pill-error" : "pill pill-ok";
    const statusText = e.status === "failed" ? `失败${e.detail ? " · " + escapeHtml(e.detail) : ""}` : "成功";
    // M8-1b：仅带重试载荷的失败条目提供一键重发
    const retryBtn = e.status === "failed" && e.retry_id
      ? `<button class="ghost-action" data-action="retry-sync-activity" data-id="${e.id}">重试</button>`
      : "";
    return `<div class="setting-row">
      <div class="row-icon"><i data-lucide="${iconFor[e.kind] || "activity"}"></i></div>
      <div><h3>${dirFor(e.direction)}${e.kind === "image" ? " 图片" : e.kind === "file" ? " 文件" : " 文本"}<span class="pill ${statusCls}">${statusText}</span></h3><p>${escapeHtml(String(e.preview).slice(0, 60))} · ${peer} · ${relTimeAgo(e.ts_ms)}</p></div>
      ${retryBtn}
    </div>`;
  }).join("");
}

// M8-2：已配对设备行（名称/指纹/最近在线/地址 + 重命名/移除）。
function deviceRows() {
  const list = peers || [];
  if (list.length === 0) {
    return `<div class="setting-row"><div class="row-icon dim"><i data-lucide="smartphone"></i></div><div><h3>暂无已配对设备</h3><p>在终端运行 <code>shurufa-host.exe pair &lt;对方IP&gt;</code> 发起配对（对方屏幕确认码一致后输入 y）</p></div></div>`;
  }
  return list.map((p) => {
    const fp = String(p.fingerprint || "");
    const fpShort = fp.slice(0, 8);
    const seen = p.last_seen_ms ? `最近在线 ${relTimeAgo(p.last_seen_ms)}` : "尚未连通过";
    const addr = p.last_addr ? ` · ${escapeHtml(p.last_addr)}` : "";
    if (renamingFp === fp) {
      return `<div class="setting-row">
        <div class="row-icon"><i data-lucide="pencil-line"></i></div>
        <div class="device-edit"><input data-peer-rename-input value="${escapeHtml(p.name)}" maxlength="40" aria-label="设备名称" /><p>${fpShort} · ${seen}</p></div>
        <button class="outline-action" data-action="peer-rename" data-fp="${fp}">保存</button>
        <button class="ghost-action" data-action="peer-rename-cancel">取消</button>
      </div>`;
    }
    const removing = confirmRemoveFp === fp;
    return `<div class="setting-row">
      <div class="row-icon"><i data-lucide="smartphone"></i></div>
      <div><h3>${escapeHtml(p.name)}</h3><p>${fpShort} · ${seen}${addr}</p></div>
      <button class="ghost-action" data-action="peer-rename-start" data-fp="${fp}">重命名</button>
      <button class="${removing ? "danger-action" : "ghost-action"}" data-action="peer-remove" data-fp="${fp}">${removing ? "确认移除" : "移除"}</button>
    </div>`;
  }).join("");
}

function imeOptionsPanel() {
  if (!imeOptions) {
    return `<article class="setting-panel"><div class="setting-row"><div class="row-icon"><i data-lucide="keyboard"></i></div><div><h3>输入选项</h3><p>读取中…</p></div></div></article>`;
  }
  const items = [
    ["shift_switch_cn_en", "Shift 切换中英文", "按下 Shift 即在中文/英文直输之间切换"],
    ["shift_space_full_shape", "Shift+空格 切换全角/半角", "无组合时切换空格与字母的全/半角"],
    ["ctrl_period_ascii_punct", "Ctrl+. 切换中文/英文标点", "收尾当前组合后切换标点全/半角"],
    ["capslock_to_english", "CapsLock 直接输入英文", "按下 CapsLock 即切到英文直输（再按 Shift 回中文）"],
    ["symbol_pairing", "符号配对（微信输入法同类）", "中文态输入 ( [ { 《 自动补配对符并光标居中；默认关，避免与 IDE 自动补全冲突"]
  ];
  // 引擎开关（librime switch，非 shurufa 选项）：Emoji + 中英混输空格。
  const engineRows = [
    {
      key: "emoji",
      icon: "smile",
      title: "Emoji 候选",
      desc: "输入中文词时附带 emoji（微笑 → 😊）",
      checked: engineOptionEmoji,
    },
    {
      key: "en_spacer",
      icon: "space",
      title: "中英混输自动空格",
      desc: "英文词上屏后再输入英文词自动加空格（hello 后 world → hello world）",
      checked: engineOptionEnSpacer,
    },
  ]
    .map(
      (row) => `<div class="setting-row">
      <div class="row-icon"><i data-lucide="${row.icon}"></i></div>
      <label class="setting-toggle"><div><h3>${row.title}</h3><p>${row.desc}</p></div></label>
      <label class="switch"><input type="checkbox" data-engine-option="${row.key}" ${row.checked ? "checked" : ""} /><span></span></label>
    </div>`
    )
    .join(`<div class="divider"></div>`);
  const rows = items
    .map(([key, title, desc]) => {
      const checked = imeOptions[key] ? "checked" : "";
      return `<div class="setting-row"><div class="row-icon"><i data-lucide="circle-dot"></i></div><label class="setting-toggle"><div><h3>${title}</h3><p>${desc}</p></div></label><label class="switch"><input type="checkbox" data-ime-option="${key}" ${checked} /><span></span></label></div>`;
    })
    .join(`<div class="divider"></div>`);
  return `<article class="setting-panel ime-options-panel"><div class="panel-heading"><div class="row-icon blue"><i data-lucide="keyboard"></i></div><div><h3>输入选项</h3><p>全部对正在输入的应用热生效，延迟约 2 秒</p></div></div>${rows}<div class="divider"></div>${engineRows}</article>`;
}

// 按应用选项（weasel app_options）：进程名 → 自动英文直输 / vim 模式。
// 自动英文：进入匹配的应用自动切英文（终端/IDE 常用），离开恢复。
// vim 模式（weasel vim_mode 同款）：该应用下按 vim 回 normal 模式键
// （Esc / Ctrl+C / Ctrl+[）自动切英文，让 vim/emacs 拿到按键。
function appOptionsPanel() {
  const list = appOptions ?? [];
  const rows = list.map(
    (item, i) => `<div class="setting-row app-option-row" data-index="${i}">
      <div class="row-icon"><i data-lucide="app-window"></i></div>
      <div class="app-option-fields">
        <input class="app-option-name" value="${escapeHtml(item.app)}" placeholder="app.exe（进程名，小写）" aria-label="进程名" />
        <label class="app-option-toggle"><span>自动英文直输</span><label class="switch"><input type="checkbox" class="app-option-ascii" ${item.ascii_mode ? "checked" : ""} /><span></span></label></label>
        <label class="app-option-toggle"><span>vim 模式</span><label class="switch"><input type="checkbox" class="app-option-vim" ${item.vim_mode ? "checked" : ""} /><span></span></label></label>
      </div>
      <button class="icon-action" data-action="remove-app-option" data-index="${i}" aria-label="删除"><i data-lucide="trash-2"></i></button>
    </div>`
  ).join("");
  const empty = list.length === 0 ? `<div class="setting-row"><div class="row-icon dim"><i data-lucide="info"></i></div><div><h3>还没有按应用设置</h3><p>添加后，进入该应用自动切英文直输（终端 / IDE 常用）或启用 vim 模式（vim / emacs 回 normal 模式自动切英文），离开恢复</p></div></div>` : "";
  return `<article class="setting-panel app-options-panel">
    <div class="panel-heading"><div class="row-icon violet"><i data-lucide="app-window"></i></div><div><h3>按应用输入法行为</h3><p>进程名匹配时生效：自动英文直输 / vim 模式（weasel app_options 同款），离开恢复</p></div></div>
    ${rows || empty}
    <div class="divider"></div>
    <div class="panel-actions"><button class="outline-action" data-action="add-app-option"><i data-lucide="plus"></i>添加应用</button><button class="primary-action compact" data-action="save-app-options"><i data-lucide="save"></i>保存</button></div>
  </article>`;
}

function settingsPage() {
  const autostartOn = autostartInfo ? autostartInfo.enabled : false;
  const imeRow = defaultIme
    ? `<div class="setting-row">
        <div class="row-icon blue"><i data-lucide="keyboard"></i></div>
        <div><h3>系统默认输入法</h3><p>${escapeHtml(defaultIme.is_default ? "当前默认就是 FOX 拼音" : defaultIme.tip ? `当前默认：${defaultIme.tip}` : "未设置默认输入法（安装器可自动设置）")}</p></div>
        <div class="row-side">
          ${defaultIme.is_default
            ? `<button class="outline-action" data-action="clear-default-ime"><i data-lucide="arrow-up-right"></i>清除默认</button>`
            : `<button class="primary-action compact" data-action="set-default-ime"><i data-lucide="arrow-up-right"></i>设为默认</button>`}
        </div>
      </div>
      <div class="divider"></div>`
    : "";
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">PREFERENCES</p><h1>偏好</h1></div></header>
      ${imeOptionsPanel()}
      ${appOptionsPanel()}
      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon teal"><i data-lucide="layout-grid"></i></div><div><h3>悬浮条</h3><p>控制中心以悬浮条常驻桌面，点 logo 或 ⊞ 展开菜单</p></div></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="play"></i></div>
          <label class="setting-toggle"><div><h3>开机自启常驻</h3><p>登录时自动显示悬浮球（HKCU Run · FOXSettings）</p></div></label>
          <label class="switch"><input type="checkbox" data-settings-field="autostart" ${autostartOn ? "checked" : ""} ${autostartInfo ? "" : "disabled"} /><span></span></label>
        </div>
      </article>
      <article class="setting-panel">
        ${imeRow}
        <div class="setting-row"><div class="row-icon"><i data-lucide="settings-2"></i></div><div><h3>系统输入法</h3><p>管理语言、输入法和默认输入法</p></div><button class="outline-action" data-action="open-settings"><i data-lucide="arrow-up-right"></i>打开设置</button></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="folder-open"></i></div><div><h3>本地数据</h3><p class="path-value">${escapeHtml(dashboard.data_directory)}</p></div><button class="outline-action" data-action="open-data-directory"><i data-lucide="folder-open"></i>打开目录</button></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="power"></i></div><div><h3>退出控制中心</h3><p>完全退出进程；开机自启开启时下次登录重新出现</p></div><button class="outline-action" data-action="exit-app"><i data-lucide="power"></i>退出</button></div>
      </article>
    </section>`;
}

// ---------------------------------------------------------------------------
// 通用页：自启 / 日志级别 / 历史条数 / 划词润色 + AI 帮写热键开关
// 皮肤目录走 SSOT（候选窗皮肤文件），此字段保留给后续版本——只读展示。
// ---------------------------------------------------------------------------

function generalPage() {
  if (!generalSettings) {
    return `
      <section class="page settings-page">
        <header class="page-header"><div><p class="eyebrow">GENERAL</p><h1>通用</h1></div></header>
        <article class="setting-panel"><div class="setting-row"><div class="row-icon dim"><i data-lucide="settings-2"></i></div><div><h3>通用设置</h3><p>读取中或暂不可用…</p></div></div></article>
      </section>`;
  }
  const g = generalSettings;
  const logOptions = ["info", "debug", "trace"]
    .map((lv) => `<option value="${lv}" ${g.log_level === lv ? "selected" : ""}>${lv === "info" ? "信息" : lv === "debug" ? "调试" : "跟踪"}</option>`)
    .join("");
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">GENERAL</p><h1>通用</h1></div></header>

      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="play"></i></div><div><h3>启动</h3><p>登录时自动启动后台服务</p></div></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="circle-dot"></i></div>
          <label class="setting-toggle"><div><h3>登录自启</h3><p>勾选后写入 HKCU Run 键（shurufa-host supervise）</p></div></label>
          <label class="switch"><input type="checkbox" data-general-field="autostart" ${g.autostart ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="info"></i></div>
          <div><h3>日志级别</h3><p>跟踪级别最详细，日志文件增长更快</p></div>
          <div class="row-side"><select data-general-field="log_level">${logOptions}</select></div>
        </div>
      </article>

      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon coral"><i data-lucide="palette"></i></div><div><h3>皮肤</h3><p>皮肤目录由 SSOT 决定，此字段保留给后续版本</p></div></div>
        <div class="setting-row">
          <div class="row-icon dim"><i data-lucide="folder-open"></i></div>
          <div>
            <h3>皮肤目录覆盖</h3>
            <input type="text" data-general-field="skin_dir_override" value="${escapeHtml(g.skin_dir_override ?? "")}" placeholder="（未设置）" disabled />
            <p class="field-note">当前版本由 SSOT 文件（%APPDATA%\\shurufa\\shurufa-skin.json）决定</p>
          </div>
        </div>
      </article>

      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon teal"><i data-lucide="history"></i></div><div><h3>历史</h3><p>剪贴板历史上限（条）</p></div></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="clipboard-list"></i></div>
          <div>
            <h3>最大条数 <output id="general-history-max-label">${g.history_max_entries}</output></h3>
            <input type="range" min="50" max="2000" step="10" value="${g.history_max_entries}" data-general-field="history_max_entries" />
            <p class="field-note">范围 50 - 2000，超出将被钳到边界</p>
          </div>
        </div>
      </article>

      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="blend"></i></div><div><h3>外观</h3><p>悬浮球与控制中心窗口透明度（搜狗 16.1 状态栏不透明度同类）</p></div></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="circle-dot"></i></div>
          <div>
            <h3>悬浮球不透明度 <output id="general-opacity-label">${g.ball_opacity ?? 100}</output>%</h3>
            <input type="range" min="30" max="100" step="5" value="${g.ball_opacity ?? 100}" data-general-field="ball_opacity" data-range-label="#general-opacity-label" data-range-suffix="%" />
            <p class="field-note">范围 30% - 100%，改动即时生效</p>
          </div>
        </div>
      </article>

      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="keyboard"></i></div><div><h3>快捷键</h3><p>面板唤起热键（取消勾选即不再注册，wave 4 起生效）</p></div></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="sparkles"></i></div>
          <label class="setting-toggle"><div><h3>Ctrl+Shift+R 划词润色</h3><p>选中文本后调 AI 润色</p></div></label>
          <label class="switch"><input type="checkbox" data-general-field="enable_polish_hotkey" ${g.enable_polish_hotkey ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="sparkles"></i></div>
          <label class="setting-toggle"><div><h3>Ctrl+Shift+W AI 帮写</h3><p>打开 AI 帮写面板</p></div></label>
          <label class="switch"><input type="checkbox" data-general-field="enable_ai_hotkey" ${g.enable_ai_hotkey ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="languages"></i></div>
          <label class="setting-toggle"><div><h3>Ctrl+Shift+T 划词翻译</h3><p>选中文本后调 AI 翻译成中文（原文中文则译英文），回车覆盖选区</p></div></label>
          <label class="switch"><input type="checkbox" data-general-field="enable_translate_hotkey" ${g.enable_translate_hotkey ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="list-filter"></i></div>
          <div class="setting-toggle" style="flex:1"><div><h3>划词应用白名单（M9-6）</h3><p>每行一个 exe 文件名，如 WINWORD.EXE / chrome.exe；留空 = 所有应用均可划词</p></div>
          <textarea id="general-selection-whitelist" rows="3" spellcheck="false" placeholder="WINWORD.EXE&#10;chrome.exe">${escapeTextarea((g.selection_app_whitelist || []).join("\n"))}</textarea></div>
        </div>
      </article>

      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon coral"><i data-lucide="mic-2"></i></div><div><h3>语音转写 <span class="pill pill-dev">dev-stub</span></h3><p>Ctrl+Shift+S · 当前为 stub（固定文字"你好，世界。"）；真实引擎 wave 6 接入</p></div></div>
        ${!speechSettings ? `<div class="setting-row"><div class="row-icon dim"><i data-lucide="mic-2"></i></div><div><h3>语音设置读取中…</h3><p>请稍候或检查后台服务</p></div></div>` : `
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="power"></i></div>
          <label class="setting-toggle"><div><h3>启用语音转写</h3><p>options.json speech.enabled — 关则热键不注册</p></div></label>
          <label class="switch"><input type="checkbox" data-speech-field="enabled" ${speechSettings.enabled ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="keyboard"></i></div>
          <label class="setting-toggle"><div><h3>Ctrl+Shift+S 唤起</h3><p>options.json speech.hotkey_enabled</p></div></label>
          <label class="switch"><input type="checkbox" data-speech-field="hotkey_enabled" ${speechSettings.hotkey_enabled ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="wand-2"></i></div>
          <label class="setting-toggle"><div><h3>书面语化（agnes-2.5-flash）</h3><p>把口语转写润色为书面语；失败回退到原文</p></div></label>
          <label class="switch"><input type="checkbox" data-speech-field="written_style_polish" ${speechSettings.written_style_polish ? "checked" : ""} /><span></span></label>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="row-icon"><i data-lucide="timer"></i></div>
          <div>
            <h3>单次会话最长 <output id="speech-max-label">${speechSettings.max_session_secs}</output> 秒</h3>
            <input type="range" min="30" max="600" step="30" value="${speechSettings.max_session_secs}" data-speech-field="max_session_secs" />
            <p class="field-note">到点即自动收尾并提交当前累计文本</p>
          </div>
        </div>`}
      </article>
    </section>`;
}

// ---------------------------------------------------------------------------
// 统计页：3 张合计卡 + 7 天柱状图 + 30 天折线图（纯手写 SVG，不引任何图表库）。
// 数据来自后端 typing_stats 命令：last7/last30 均为 (YYYY-MM-DD, 字数) 升序定长序列。
// ---------------------------------------------------------------------------

// 千分位格式化：12,345 / 1.2万（中文习惯大额缩写）
function formatCount(value) {
  const n = Number(value) || 0;
  return n.toLocaleString("zh-CN");
}

// 取 MM-DD 短标签（后端给 YYYY-MM-DD；非法输入原样透出）
function shortDateLabel(date) {
  return typeof date === "string" && date.length >= 10 ? date.slice(5) : String(date ?? "");
}

// 7 天柱状图：柱高与当日字数成正比，今日高亮（accent），其余降透明度。
function statsBarChartSvg(days, today) {
  if (!Array.isArray(days) || days.length === 0) return "";
  const width = 640;
  const height = 180;
  const padX = 10;
  const padTop = 14;
  const padBottom = 26;
  const plotH = height - padTop - padBottom;
  const max = Math.max(1, ...days.map(([, chars]) => Number(chars) || 0));
  const slot = (width - padX * 2) / days.length;
  const barW = Math.min(44, slot * 0.58);
  const bars = days
    .map(([date, chars], index) => {
      const value = Number(chars) || 0;
      const h = value === 0 ? 2 : Math.max(4, (value / max) * plotH);
      const x = padX + slot * index + (slot - barW) / 2;
      const y = padTop + plotH - h;
      const isToday = date === today;
      return `<rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${barW.toFixed(1)}" height="${h.toFixed(1)}" rx="4"
        class="stats-bar${isToday ? " today" : ""}"><title>${shortDateLabel(date)} · ${formatCount(value)} 字</title></rect>`;
    })
    .join("");
  const labels = days
    .map(([date], index) => {
      const x = padX + slot * index + slot / 2;
      const isToday = date === today;
      return `<text x="${x.toFixed(1)}" y="${height - 8}" text-anchor="middle"
        class="stats-axis-label${isToday ? " today" : ""}">${shortDateLabel(date)}</text>`;
    })
    .join("");
  return `<svg viewBox="0 0 ${width} ${height}" class="stats-chart" role="img" aria-label="近 7 天打字柱状图">${bars}${labels}</svg>`;
}

// 30 天折线：单调面积+折线，末点（今日）画高亮圆点；全为 0 时折线贴底仍可见。
function statsLineChartSvg(days, today) {
  if (!Array.isArray(days) || days.length === 0) return "";
  const width = 640;
  const height = 160;
  const padX = 12;
  const padTop = 12;
  const padBottom = 24;
  const plotW = width - padX * 2;
  const plotH = height - padTop - padBottom;
  const max = Math.max(1, ...days.map(([, chars]) => Number(chars) || 0));
  const step = days.length > 1 ? plotW / (days.length - 1) : 0;
  const points = days.map(([date, chars], index) => {
    const x = padX + step * index;
    const y = padTop + plotH - ((Number(chars) || 0) / max) * plotH;
    return { x, y, date, value: Number(chars) || 0 };
  });
  const polyline = points.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(" ");
  const area = `M ${points[0].x.toFixed(1)},${(padTop + plotH).toFixed(1)} L ${points
    .map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`)
    .join(" L ")} L ${points[points.length - 1].x.toFixed(1)},${(padTop + plotH).toFixed(1)} Z`;
  const todayPoint = points.find((p) => p.date === today) || points[points.length - 1];
  const xLabels = [points[0], points[Math.floor(points.length / 2)], points[points.length - 1]]
    .filter(Boolean)
    .map(
      (p) =>
        `<text x="${p.x.toFixed(1)}" y="${height - 6}" text-anchor="middle" class="stats-axis-label">${shortDateLabel(p.date)}</text>`
    )
    .join("");
  return `<svg viewBox="0 0 ${width} ${height}" class="stats-chart" role="img" aria-label="近 30 天打字曲线">
      <path d="${area}" class="stats-line-area"></path>
      <polyline points="${polyline}" class="stats-line"></polyline>
      <circle cx="${todayPoint.x.toFixed(1)}" cy="${todayPoint.y.toFixed(1)}" r="4" class="stats-line-today"><title>今日 ${formatCount(todayPoint.value)} 字</title></circle>
      ${xLabels}
    </svg>`;
}

function statsPage() {
  if (!typingStats) {
    return `
      <section class="page settings-page">
        <header class="page-header"><div><p class="eyebrow">TYPING STATS</p><h1>统计</h1></div><button class="icon-action" data-action="refresh-stats" title="刷新统计"><i data-lucide="refresh-cw"></i></button></header>
        <article class="setting-panel"><div class="setting-row"><div class="row-icon dim"><i data-lucide="chart-column"></i></div><div><h3>打字统计</h3><p>读取中或暂不可用…</p></div></div></article>
      </section>`;
  }
  const cards = [
    { icon: "keyboard", tone: "teal", label: "今日打字", value: `${formatCount(typingStats.today_chars)} 字` },
    { icon: "circle-dot", tone: "blue", label: "今日按键", value: `${formatCount(typingStats.today_keys)} 次` },
    { icon: "history", tone: "coral", label: "累计打字", value: `${formatCount(typingStats.total_chars)} 字` }
  ];
  const cardsHtml = cards
    .map(
      (card) => `<article class="metric-card stats-card"><div class="metric-icon ${card.tone}"><i data-lucide="${card.icon}"></i></div><span>${card.label}</span><strong>${card.value}</strong></article>`
    )
    .join("");
  const today = typingStats.today || "";
  const weekSvg = statsBarChartSvg(typingStats.last7, today);
  const monthSvg = statsLineChartSvg(typingStats.last30, today);
  return `
    <section class="page settings-page stats-page">
      <header class="page-header"><div><p class="eyebrow">TYPING STATS</p><h1>统计</h1></div><button class="icon-action" data-action="refresh-stats" title="刷新统计"><i data-lucide="refresh-cw"></i></button></header>
      <div class="metric-grid stats-grid">${cardsHtml}</div>
      <article class="setting-panel stats-chart-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="chart-column"></i></div><div><h3>近 7 天打字</h3><p>单位：字 · 当日高亮</p></div></div>
        ${weekSvg}
      </article>
      <article class="setting-panel stats-chart-panel">
        <div class="panel-heading"><div class="row-icon teal"><i data-lucide="arrow-up-right"></i></div><div><h3>近 30 天打字曲线</h3><p>单位：字 / 天</p></div></div>
        ${monthSvg}
      </article>
    </section>`;
}

function pageTemplate() {
  switch (activePage) {
    case "general": return generalPage();
    case "input": return inputPage();
    case "scheme": return schemePage();
    case "stats": return statsPage();
    case "history": return historyPage();
    case "dictionary": return dictionaryPage();
    case "phrases": return phrasesPage();
    case "symbols": return symbolsPage();
    case "skin": return skinPage();
    case "sync": return syncPage();
    case "settings": return settingsPage();
    default: return workspacePage();
  }
}

// ---------------------------------------------------------------------------
// 方案页（wave 4 新增）：4 个输入方案单选，选中即写 options.json。
// 绿色 banner 在 schemeBanner 内维护；预先回显当前方案（get_general_settings
// 返回的 input_scheme 已包含）。preview 状态的方案给出"需重启输入法"提示。
// ---------------------------------------------------------------------------

function schemePage() {
  if (!schemeList) {
    return `
      <section class="page settings-page">
        <header class="page-header"><div><p class="eyebrow">INPUT SCHEME</p><h1>方案</h1></div></header>
        <article class="setting-panel"><div class="setting-row"><div class="row-icon dim"><i data-lucide="circle-dot"></i></div><div><h3>输入方案</h3><p>读取中或暂不可用…</p></div></div></article>
      </section>`;
  }
  const bannerHtml = schemeBanner
    ? `<div class="skin-preset-banner ${schemeBanner.error ? "error" : "ok"}"><i data-lucide="${schemeBanner.error ? "info" : "sparkles"}"></i>${escapeHtml(schemeBanner.message)}</div>`
    : "";
  const rows = schemeList
    .map((scheme) => {
      const checked = schemeCurrent === scheme.id ? "checked" : "";
      const disabled = scheme.status === "unavailable" ? "disabled" : "";
      const tone = scheme.status === "stable" ? "teal" : scheme.status === "unavailable" ? "dim" : "coral";
      return `
        <div class="setting-row">
          <div class="row-icon ${tone}"><i data-lucide="circle-dot"></i></div>
          <label class="setting-toggle" style="flex:1">
            <div>
              <h3>${escapeHtml(scheme.name_zh)} <span style="opacity:0.55;font-weight:400;font-size:12px;margin-left:6px">${escapeHtml(scheme.name_en)}</span></h3>
              <p>${escapeHtml(scheme.subtitle)}</p>
            </div>
          </label>
          <label class="switch">
            <input type="radio" name="scheme" value="${escapeHtml(scheme.id)}" data-scheme-id="${escapeHtml(scheme.id)}" ${checked} ${disabled} />
            <span></span>
          </label>
        </div>`;
    })
    .join(`<div class="divider"></div>`);
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">INPUT SCHEME</p><h1>方案</h1></div></header>
      ${bannerHtml}
      <article class="setting-panel">
        <div class="panel-heading"><div class="row-icon blue"><i data-lucide="keyboard"></i></div><div><h3>输入方案</h3><p>默认全拼；切换到双拼后请用双拼码输入（两模式互不干扰）</p></div></div>
        ${rows}
      </article>
      <article class="hint-card"><i data-lucide="lightbulb"></i><p>全拼：输入完整拼音（nihao → 你好）。双拼（小鹤）：每字两键，如「我是说」= wouiuo、「你好」= nihc。切换后对新输入生效；五笔/仓颉码表待接入。</p></article>
    </section>`;
}

async function refreshSchemes() {
  schemeList = await invoke("list_input_schemes");
  // 一并取当前方案（与通用页共用同一 options.json 读路径）
  try {
    const g = await invoke("get_general_settings");
    schemeCurrent = g.input_scheme || "pinyin";
  } catch (_error) {
    schemeCurrent = "pinyin";
  }
}

// 自定义短语编辑器（P1 #6）：编码/词条/权重三列表格。
// 加载（read_custom_phrases）→ 编辑 → 保存（save_custom_phrases）→
// 重建（redeploy_dictionaries）四步分离，避免误触触发重编译。
async function refreshPhrases() {
  try {
    phraseRows = await invoke("read_custom_phrases");
  } catch (_error) {
    phraseRows = [];
  }
  if (uiMode === "page" && activePage === "phrases") render();
}

function phrasesPage() {
  const rows = phraseRows === null
    ? []
    : phraseRows;
  const rowHtml = rows.length
    ? rows.map((p, i) => `
      <div class="phrase-row" data-phrase-row="${i}">
        <input class="phrase-code" data-phrase-field="code" value="${escapeAttr(p.code)}" placeholder="编码（如 gs）" spellcheck="false">
        <input class="phrase-text" data-phrase-field="text" value="${escapeAttr(p.text)}" placeholder="词条（如 公司）">
        <input class="phrase-weight" data-phrase-field="weight" type="number" min="1" max="999" value="${p.weight ?? 100}" title="权重越大越靠前">
        <button class="icon-action" data-action="phrase-remove" data-index="${i}" title="删除该条"><i data-lucide="trash-2"></i></button>
      </div>`).join("")
    : `<p class="field-note phrase-empty">还没有自定义短语。添加后保存并部署即可在输入时置顶命中。</p>`;
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">PHRASES</p><h1>自定义短语</h1></div><span class="status-pill info">${rows.length} 条</span></header>
      <p class="skin-note">固定短语置顶于普通拼音候选之前。编码用拼音简写，如 <code>gs</code> → 公司、<code>wz</code> → 位置。格式：<code>编码 &lt;Tab&gt; 词条 &lt;Tab&gt; 权重</code>，保存在 <code>%APPDATA%\\shurufa\\rime\\custom_phrase.txt</code>。</p>
      <article class="setting-panel">
        <div class="phrase-grid-header"><span>编码</span><span>词条</span><span>权重</span><span></span></div>
        <div id="phrase-rows">${rowHtml}</div>
        <div class="field-action">
          <button class="outline-action" data-action="phrase-add"><i data-lucide="plus"></i>添加条目</button>
          <button class="primary-action" data-action="phrase-save"><i data-lucide="save"></i>保存</button>
          <button class="outline-action" data-action="phrase-deploy"><i data-lucide="refresh-cw"></i>保存并部署</button>
        </div>
      </article>
    </section>`;
}

function escapeAttr(value) {
  return String(value ?? "").replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// 符号面板（P1 #11 + Tier 8/11 增强）：分类标签 + 符号网格，点击复制到剪贴板。
// Tier 8（搜狗 6.24.1 方向）：emoji 分类（表情/手势/动物/生活/爱心）、颜文字、
// 肤色选择条（应用到手势类 emoji，本地记忆）与最近使用页签。
// Tier 11（搜狗/微信 emoji 面板搜索同款）：顶部搜索框跨分类实时过滤——
// 关键词索引（中文名/拼音/英文名 → emoji）+ 符号字符匹配 + 分类名匹配。
// 文本符号数据来自 rime-ice symbols_v.yaml 常用子集（见 SYMBOL_CATEGORIES）。
function symbolsPage() {
  const cats = SYMBOL_CATEGORIES;
  const recents = loadRecents();
  const q = (symbolQuery || "").trim();
  // 搜索态：跨分类平铺结果，隐藏分类页签与肤色条
  if (q) {
    const hits = searchSymbols(q);
    const grid = hits
      .map(
        (h) => `<button class="symbol-cell" data-symbol-copy="${escapeAttr(h.symbol)}" title="点击复制 ${escapeAttr(h.symbol)}（${escapeAttr(h.label)}）">${escapeHtml(h.symbol)}</button>`
      )
      .join("");
    return `
      <section class="page settings-page">
        <header class="page-header"><div><p class="eyebrow">SYMBOLS</p><h1>符号</h1></div><span class="status-pill info">${hits.length} 个</span></header>
        <p class="skin-note">点击符号复制到剪贴板，再到目标位置粘贴。输入时也可用 <code>/</code> 前缀直接打出（如 /fh 商标符号、/1 数字符号）。</p>
        <article class="setting-panel">
          <input id="symbol-search" class="symbol-search" type="search" value="${escapeAttr(symbolQuery)}" placeholder="搜索：微笑 / weixiao / 😊 / 分类名…" aria-label="搜索符号" autofocus />
          <div class="symbol-scroll"><div class="symbol-grid">${grid || `<p class="symbol-empty">没有匹配「${escapeHtml(q)}」的符号——试试 微笑 / weixiao / 箭头 / 心</p>`}</div></div>
        </article>
        <article class="hint-card"><i data-lucide="info"></i><p>搜索支持中文名（微笑）、拼音（weixiao）、英文（smile）与符号本身；命中后点击即复制。</p></article>
      </section>`;
  }
  const current = activeSymbolCat || cats[0].id;
  const cat = cats.find((c) => c.id === current) || cats[0];
  const recentTab = recents.length
    ? `<button class="symbol-tab${current === "recent" ? " active" : ""}" data-symbol-cat="recent">最近</button>`
    : "";
  const tabs =
    recentTab +
    cats
      .map((c) => `<button class="symbol-tab${c.id === current ? " active" : ""}" data-symbol-cat="${c.id}">${escapeHtml(c.label)}</button>`)
      .join("");
  // 最近使用页签：点击过的符号（含已应用肤色的 emoji）排在最前，去重保留 30 个
  const shown = current === "recent" ? recents : cat.symbols.map(emojiWithTone);
  const grid = shown
    .map((s) => `<button class="symbol-cell" data-symbol-copy="${escapeAttr(s)}" title="点击复制 ${escapeAttr(s)}">${escapeHtml(s)}</button>`)
    .join("");
  // 肤色选择条（搜狗「肤色多选及记忆」同款）：仅在 emoji 分类显示；
  // 默认 + 5 档肤色，选中即应用到手势类 emoji 并持久化。
  const toneStrip = isEmojiCat(cat.id)
    ? `<div class="symbol-tone-strip" title="肤色记忆（搜狗同款）：选中的肤色应用到手势类 emoji，点击后记住">
        <span class="symbol-tone-label">肤色</span>
        <button class="symbol-tone${!emojiTone ? " active" : ""}" data-symbol-tone="" title="默认肤色">✋</button>
        ${EMOJI_TONES.map(
          (t) => `<button class="symbol-tone${emojiTone === t ? " active" : ""}" data-symbol-tone="${t}" title="肤色 ${t}">✋${t}</button>`
        ).join("")}
      </div>`
    : "";
  const count = current === "recent" ? recents.length : cat.symbols.length;
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">SYMBOLS</p><h1>符号</h1></div><span class="status-pill info">${count} 个</span></header>
      <p class="skin-note">点击符号复制到剪贴板，再到目标位置粘贴。输入时也可用 <code>/</code> 前缀直接打出（如 /fh 商标符号、/1 数字符号）。</p>
      <article class="setting-panel">
        <input id="symbol-search" class="symbol-search" type="search" value="${escapeAttr(symbolQuery)}" placeholder="搜索：微笑 / weixiao / 😊 / 分类名…" aria-label="搜索符号" />
        <div class="symbol-tabs">${tabs}</div>
        ${toneStrip}
        <div class="symbol-scroll"><div class="symbol-grid">${grid || `<p class="symbol-empty">还没有最近使用的符号——点几个试试</p>`}</div></div>
      </article>
      <article class="hint-card"><i data-lucide="info"></i><p>文本符号取自 rime-ice 官方 symbols_v.yaml；emoji 分类 + 颜文字 + 肤色记忆 + 搜索为面板增强。最近使用的符号自动排在「最近」页签，本地保存。</p></article>
    </section>`;
}

function skinPage() {
  const status = skinState.source === "user" ? "自定义" : skinState.source === "builtin" ? "内置默认" : "未找到皮肤文件";
  const statusClass = skinState.dirty ? "warn" : skinState.source === "user" ? "info" : "";
  const skin = parseSkinJson(skinState.content);
  const formHtml = skin ? skinFormHtml(skin) : "";
  const previewHtml = skinPreviewHtml(skin);
  const presetsHtml = skinPresets.length
    ? `<article class="setting-panel skin-presets-panel">
        <p class="skin-presets-title">预设皮肤</p>
        <div class="skin-presets-strip">${skinPresets
          .map(
            (p) => `<button class="skin-preset-card" data-action="apply-skin" data-skin-id="${escapeHtml(p.id)}" title="${escapeHtml(p.name_en)}">
              <span class="skin-preset-swatch" style="background:${escapeHtml(p.preview_hint)}"></span>
              <span class="skin-preset-name">${escapeHtml(p.name_zh)}</span>
            </button>`
          )
          .join("")}</div>
      </article>`
    : "";
  const bannerHtml = skinPresetBanner
    ? `<div class="skin-preset-banner ${skinPresetBanner.error ? "error" : "ok"}"><i data-lucide="${skinPresetBanner.error ? "info" : "sparkles"}"></i>${escapeHtml(skinPresetBanner.message)}</div>`
    : "";
  return `
    <section class="page skin-page">
      <header class="page-header"><div><p class="eyebrow">APPEARANCE</p><h1>皮肤</h1></div><div class="header-actions"><span class="status-pill ${statusClass}">${status}${skinState.dirty ? " · 未保存" : ""}</span></div></header>
      <p class="skin-note">编辑候选窗颜色、圆角、字号与阴影。左侧表单实时改动右侧预览与下方 JSON；保存后立即生效（Windows 候选窗 / 历史面板 / AI 帮写面板 / Android 键盘都会读取）。键盘、面板等其余字段保持原值。</p>
      ${bannerHtml}
      ${presetsHtml}
      ${skin ? "" : `<div class="skin-invalid-banner" id="skin-invalid-banner"><i data-lucide="info"></i>JSON 无效，修复 JSON 才能继续编辑（表单已禁用，改动不会被覆盖）</div>`}
      <div class="skin-grid${skin ? "" : " skin-form-disabled"}" id="skin-grid">
        <div class="skin-form" id="skin-form">${formHtml}</div>
        <div class="skin-preview-col">${previewHtml}</div>
      </div>
      <article class="setting-panel skin-json-panel">
        <p class="skin-path">${skinState.user_path ? `<i data-lucide="folder-open"></i><code>${escapeHtml(skinState.user_path)}</code>` : ""}</p>
        <textarea id="skin-editor" spellcheck="false" placeholder='{"version":2, ...}'>${escapeTextarea(skinState.content)}</textarea>
        <div class="field-action">
          <button class="primary-action" data-action="save-skin"><i data-lucide="sparkles"></i>保存并应用</button>
          <button class="outline-action" data-action="reset-skin"><i data-lucide="trash-2"></i>删除自定义（回退内置）</button>
          <button class="outline-action" data-action="reload-skin"><i data-lucide="refresh-cw"></i>重新加载</button>
        </div>
        <div class="field-action skin-file-actions">
          <input id="skin-export-name" maxlength="40" placeholder="导出文件名（如：护眼绿）" aria-label="导出文件名" />
          <button class="outline-action" data-action="export-skin"><i data-lucide="download"></i>导出为文件</button>
          <label class="outline-action" for="skin-import-file"><i data-lucide="upload"></i>导入文件</label>
          <input id="skin-import-file" type="file" accept=".json,application/json" hidden />
        </div>
      </article>
    </section>`;
}

// ---------------------------------------------------------------------------
// 皮肤表单 / 预览（纯 JS，mock 不依赖 TSF；亮暗双模独立渲染）
// ---------------------------------------------------------------------------

// 与 platforms/windows/src/skin.rs 的 CandidateColors::light()/dark() 保持一致。
const SKIN_DEFAULTS = {
  light: {
    background: "#F5F5F1",
    highlight_background: "#C6D2B8",
    text: "#1C1812",
    preedit: "#9C8F85",
    label: "#7A9A24"
  },
  dark: {
    background: "#282422",
    highlight_background: "#323E2C",
    text: "#F4F2F1",
    preedit: "#9A938F",
    label: "#A0CC4C"
  },
  metrics: { radius: 8, font_scale: 1.0, opacity: 1.0 },
  shadow: { enabled: false, radius: 18, alpha: 64 }
};
const SKIN_COLOR_FIELDS = [
  ["background", "背景"],
  ["highlight_background", "高亮背景"],
  ["text", "正文文字"],
  ["preedit", "预编辑（拼音）"],
  ["label", "序号标签"]
];

function isHexColor(value) {
  return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value);
}

// 读用户 JSON：只挑 candidate(5 色)+metrics+shadow；其他字段一律不碰。
// 解析失败 / 结构不符一律返回 null（由调用方禁用表单，绝不允许覆盖原文）。
function parseSkinJson(text) {
  if (!text || !text.trim()) return null;
  let json;
  try {
    json = JSON.parse(text);
  } catch (_error) {
    return null;
  }
  if (!json || typeof json !== "object" || Array.isArray(json)) return null;
  const readVariant = (key) => {
    const defaults = SKIN_DEFAULTS[key];
    const candidate = json[key] && typeof json[key].candidate === "object" ? json[key].candidate : {};
    const metricsSrc = json[key] && typeof json[key].metrics === "object" ? json[key].metrics : {};
    const metrics = {};
    for (const name of ["radius", "font_scale", "opacity"]) {
      const value = Number(metricsSrc[name]);
      metrics[name] = Number.isFinite(value) ? value : SKIN_DEFAULTS.metrics[name];
    }
    const colors = {};
    for (const [name] of SKIN_COLOR_FIELDS) {
      const value = candidate[name];
      colors[name] = isHexColor(value) ? value.toUpperCase() : defaults[name];
    }
    return { colors, metrics };
  };
  const shadowSrc = json.shadow && typeof json.shadow === "object" ? json.shadow : {};
  const shadow = {
    enabled: typeof shadowSrc.enabled === "boolean" ? shadowSrc.enabled : SKIN_DEFAULTS.shadow.enabled,
    radius: Number.isFinite(Number(shadowSrc.radius)) ? Number(shadowSrc.radius) : SKIN_DEFAULTS.shadow.radius,
    alpha: Number.isFinite(Number(shadowSrc.alpha)) ? Math.round(Number(shadowSrc.alpha)) : SKIN_DEFAULTS.shadow.alpha
  };
  return { light: readVariant("light"), dark: readVariant("dark"), shadow };
}

// rgba(hex, alpha) → "rgba(r,g,b,a)"；预览/表单色板共用。
function hexWithAlpha(hex, alpha) {
  const value = hex.replace("#", "");
  const r = parseInt(value.slice(0, 2), 16);
  const g = parseInt(value.slice(2, 4), 16);
  const b = parseInt(value.slice(4, 6), 16);
  return `rgba(${r},${g},${b},${alpha})`;
}

function numberAttr(value, digits = 2) {
  const rounded = Number(value);
  return String(Number.isFinite(rounded) ? Number(rounded.toFixed(digits)) : 0);
}

function skinColorRowHtml(theme, name, label, value) {
  return `
    <div class="skin-color-row" data-skin-field="${theme}.${name}">
      <label class="skin-color-chip" style="background:${value}"><input type="color" data-skin-color="${theme}.${name}" value="${value}" /></label>
      <span class="skin-color-label">${label}</span>
      <span class="skin-color-hex">${value}</span>
    </div>`;
}

function skinMetricsHtml(theme, metrics) {
  return `
    <div class="skin-slider-row" data-skin-field="${theme}.radius">
      <label>圆角半径 <output>${numberAttr(metrics.radius, 0)}px</output></label>
      <input type="range" min="0" max="64" step="1" value="${numberAttr(metrics.radius, 0)}" data-skin-range="${theme}.metrics.radius" />
    </div>
    <div class="skin-slider-row" data-skin-field="${theme}.font_scale">
      <label>字号倍率 <output>${numberAttr(metrics.font_scale)}×</output></label>
      <input type="range" min="0.5" max="2" step="0.05" value="${numberAttr(metrics.font_scale)}" data-skin-range="${theme}.metrics.font_scale" />
    </div>
    <div class="skin-slider-row" data-skin-field="${theme}.opacity">
      <label>整体透明度 <output>${numberAttr(metrics.opacity)}</output></label>
      <input type="range" min="0.2" max="1" step="0.05" value="${numberAttr(metrics.opacity)}" data-skin-range="${theme}.metrics.opacity" />
    </div>`;
}

function skinFormHtml(skin) {
  const variantSection = (theme, title) => {
    const { colors, metrics } = skin[theme];
    const rows = SKIN_COLOR_FIELDS.map(([name, label]) => skinColorRowHtml(theme, name, label, colors[name])).join("");
    return `<fieldset class="skin-fieldset"><legend>${title}</legend>${rows}${skinMetricsHtml(theme, metrics)}</fieldset>`;
  };
  const shadow = skin.shadow;
  return `
    ${variantSection("light", "亮色")}
    ${variantSection("dark", "暗色")}
    <fieldset class="skin-fieldset">
      <legend>阴影</legend>
      <div class="skin-toggle-row" data-skin-field="shadow.enabled">
        <label>启用阴影</label>
        <label class="switch"><input type="checkbox" data-skin-check="shadow.enabled" ${shadow.enabled ? "checked" : ""} /><span></span></label>
      </div>
      <div class="skin-slider-row" data-skin-field="shadow.radius">
        <label>模糊半径 <output>${numberAttr(shadow.radius, 0)}px</output></label>
        <input type="range" min="0" max="64" step="1" value="${numberAttr(shadow.radius, 0)}" data-skin-range="shadow.radius" />
      </div>
      <div class="skin-slider-row" data-skin-field="shadow.alpha">
        <label>阴影不透明度 <output>${numberAttr(shadow.alpha, 0)}</output></label>
        <input type="range" min="0" max="255" step="1" value="${numberAttr(shadow.alpha, 0)}" data-skin-range="shadow.alpha" />
      </div>
    </fieldset>`;
}

// 实盘预览 mock：仿真候选窗 layout（预编辑区 + 5 个候选 + 高亮），
// 亮暗两栏独立渲染，与设置中心当前主题完全解耦。
function skinMockCandidateHtml(theme, colors, metrics, shadow, candidateIndex) {
  const scale = metrics.font_scale;
  const candidates = [
    ["1", "书法", "shū fǎ"],
    ["2", "输入法", "shū rù fǎ"],
    ["3", "舒服", "shū fu"],
    ["4", "叔父", "shū fù"],
    ["5", "抒发", "shū fā"]
  ];
  const items = candidates
    .map(([num, word, pinyin], index) => {
      const active = index === candidateIndex;
      return `<span class="mock-candidate${active ? " active" : ""}" style="${active ? `background:${hexWithAlpha(colors.highlight_background, metrics.opacity)};` : ""}border-radius:${Math.max(0, metrics.radius - 2)}px;padding:${Math.round(3 * scale)}px ${Math.round(8 * scale)}px;"><i style="color:${colors.label};font-size:${(11 * scale).toFixed(1)}px;">${num}</i><b style="color:${colors.text};font-size:${(15 * scale).toFixed(1)}px;">${word}</b><small style="color:${colors.preedit};font-size:${(10.5 * scale).toFixed(1)}px;">${pinyin}</small></span>`;
    })
    .join("");
  const shadowCss = shadow.enabled ? `box-shadow:0 6px ${shadow.radius}px rgba(0,0,0,${(shadow.alpha / 255).toFixed(3)});` : "box-shadow:0 2px 8px rgba(0,0,0,0.12);";
  return `<div class="mock-candidate-bar" data-theme-variant="${theme}" style="background:${hexWithAlpha(colors.background, metrics.opacity)};border-radius:${metrics.radius}px;${shadowCss}">
      <div class="mock-preedit" style="color:${colors.preedit};font-size:${(12.5 * scale).toFixed(1)}px;">ni hao <span class="mock-caret" style="background:${colors.preedit}"></span></div>
      <div class="mock-candidates">${items}</div>
    </div>`;
}

function skinPreviewHtml(skin) {
  const renderVariant = (theme, title, colors, metrics, shadowCssVars) => {
    const mockA = skinMockCandidateHtml(theme, colors, metrics, skin.shadow, 1);
    const mockB = skinMockCandidateHtml(theme, colors, metrics, { ...skin.shadow, enabled: false }, 2);
    return `<div class="skin-preview-variant" data-variant="${theme}" style="${shadowCssVars}">
        <p class="skin-preview-title">${title}</p>
        ${mockA}
        <p class="skin-preview-sub">无阴影对照</p>
        ${mockB}
      </div>`;
  };
  if (!skin) {
    return `<div class="skin-preview-variant"><p class="skin-preview-title">实时预览</p><p class="skin-preview-empty">修复 JSON 后恢复预览</p></div>`;
  }
  return `
    ${renderVariant("light", "亮色候选窗", skin.light.colors, skin.light.metrics)}
    ${renderVariant("dark", "暗色候选窗", skin.dark.colors, skin.dark.metrics)}`;
}

// 表单改动 → 内存 JSON → 100ms debounce 写回 textarea（textarea 仍是 SSOT，
// 保存按钮直接读 textarea；表单绝不主动 render 以免打断输入）。
const skinFormSync = { timer: 0, lastSignature: "" };

function applySkinField(skin, path, value) {
  const [a, b, c] = path.split(".");
  if (a === "shadow") {
    skin.shadow[b] = value;
    return true;
  }
  if ((a === "light" || a === "dark") && b === "metrics") {
    skin[a].metrics[c] = value;
    return true;
  }
  if ((a === "light" || a === "dark") && Object.prototype.hasOwnProperty.call(skin[a].colors, b)) {
    skin[a].colors[b] = String(value).toUpperCase();
    return true;
  }
  return false;
}

// 把表单状态写回 JSON 文本：只更新 candidate+metrics+shadow，其余字段原样保留。
function writeSkinJson(content, skin) {
  let json;
  try {
    json = JSON.parse(content);
  } catch (_error) {
    return null;
  }
  if (!json || typeof json !== "object" || Array.isArray(json)) return null;
  for (const theme of ["light", "dark"]) {
    const variant = skin[theme];
    json[theme] = json[theme] && typeof json[theme] === "object" ? json[theme] : {};
    json[theme].candidate = { ...variant.colors };
    json[theme].metrics = {
      radius: Math.round(variant.metrics.radius),
      font_scale: Number(Number(variant.metrics.font_scale).toFixed(2)),
      opacity: Number(Number(variant.metrics.opacity).toFixed(2))
    };
  }
  json.shadow = {
    enabled: !!skin.shadow.enabled,
    radius: Math.round(skin.shadow.radius),
    alpha: Math.max(0, Math.min(255, Math.round(skin.shadow.alpha)))
  };
  if (json.version !== 1 && json.version !== 2) json.version = 2;
  return JSON.stringify(json, null, 2);
}

function skinSignature(skin) {
  return JSON.stringify(skin);
}

// 表单 input/change（颜色、滑杆、开关都要走进来；event delegation 挂在 #skin-grid）。
function onSkinFormInput(event) {
  const target = event.target;
  if (!(target instanceof HTMLInputElement)) return;
  const path = target.dataset.skinColor || target.dataset.skinRange || target.dataset.skinCheck;
  if (!path) return;
  const skin = parseSkinJson(skinState.content);
  if (!skin) return;
  let value;
  if (target.dataset.skinCheck !== undefined) {
    value = target.checked;
  } else if (target.dataset.skinRange !== undefined) {
    value = Number(target.value);
    if (!Number.isFinite(value)) return;
  } else {
    value = target.value;
  }
  if (!applySkinField(skin, path, value)) return;
  // 1) 同步旁边的显示（color 的 hex 文本 / range 的 output），不做整页 render
  syncSkinFieldChrome(target, path, value);
  // 2) debounce 写回 textarea + 刷新预览（100ms 内最多一次）
  const signature = skinSignature(skin);
  if (signature === skinFormSync.lastSignature) return;
  skinFormSync.lastSignature = signature;
  window.clearTimeout(skinFormSync.timer);
  skinFormSync.timer = window.setTimeout(() => {
    const editor = document.querySelector("#skin-editor");
    const latest = parseSkinJson(skinState.content);
    if (!latest) return;
    const nextContent = writeSkinJson(skinState.content, skin);
    if (nextContent === null) return;
    skinState.content = nextContent;
    skinState.dirty = true;
    markDirty("skin");
    const pill = document.querySelector(".skin-page .status-pill");
    if (pill) {
      pill.textContent = `${skinState.source === "user" ? "自定义" : skinState.source === "builtin" ? "内置默认" : "未找到皮肤文件"} · 未保存`;
      pill.className = "status-pill warn";
    }
    if (editor) editor.value = nextContent;
    refreshSkinPreview(skin);
  }, 100);
}

function syncSkinFieldChrome(target, path, value) {
  const row = target.closest("[data-skin-field]");
  if (!row) return;
  if (target.dataset.skinColor !== undefined) {
    const hex = String(value).toUpperCase();
    const chip = row.querySelector(".skin-color-chip");
    const label = row.querySelector(".skin-color-hex");
    if (chip) chip.style.background = hex;
    if (label) label.textContent = hex;
  } else if (target.dataset.skinRange !== undefined) {
    const output = row.querySelector("output");
    if (!output) return;
    if (path.endsWith("radius") || path.endsWith("alpha")) {
      output.textContent = `${Math.round(Number(value))}${path.endsWith("radius") ? "px" : ""}`;
    } else if (path.endsWith("font_scale")) {
      output.textContent = `${Number(Number(value).toFixed(2))}×`;
    } else {
      output.textContent = Number(Number(value).toFixed(2));
    }
  }
}

// 仅替换预览列 innerHTML（不动表单焦点 / textarea）。
function refreshSkinPreview(skin) {
  const col = document.querySelector(".skin-preview-col");
  if (!col) return;
  col.innerHTML = skinPreviewHtml(skin);
}

// 绑定 / 解绑表单（render 后由 bindSkinForm 调一次；JSON 无效时表单整块禁用 + 黄条提示）。
function bindSkinForm() {
  const grid = document.querySelector("#skin-grid");
  if (!grid) return;
  const editor = document.querySelector("#skin-editor");
  const syncDisabled = () => {
    const skin = parseSkinJson(editor ? editor.value : skinState.content);
    grid.classList.toggle("skin-form-disabled", !skin);
    const banner = document.querySelector("#skin-invalid-banner");
    if (banner) banner.style.display = skin ? "none" : "";
    if (!editor) return;
    if (skin) {
      // 用户在 textarea 里手改 JSON：只要整份 JSON 有效就刷新预览，
      // 但不改写表单值（避免打断正在打字的手）——首次加载/reset 已在 render 时完成 JSON→表单。
      refreshSkinPreview(skin);
    }
  };
  if (editor) editor.addEventListener("input", syncDisabled);
  grid.removeEventListener("input", onSkinFormInput);
  grid.addEventListener("input", onSkinFormInput);
}

function render() {
  const shell = uiMode === "bar"
    ? ballTemplate()
    : uiMode === "menu"
      ? menuShellTemplate()
      : pageShellTemplate();
  app.innerHTML = `${shell}
    <div id="toast" class="${notice ? `show${notice.error ? " error" : ""}` : ""}" aria-live="polite">${notice ? escapeHtml(notice.message) : ""}</div>`;
  createIcons({ icons: controlCenterIcons });
  bindShell();
  if (uiMode === "menu") bindMenuHover();
  app.querySelectorAll("button[data-action]").forEach((button) => {
    button.onclick = () => {
      button.disabled = true;
      void handleAction(button);
    };
  });
  // 引擎开关（emoji 等 librime switch）：change 即写算法服务
  app.querySelectorAll("input[data-engine-option]").forEach((input) => {
    input.onchange = () => {
      const key = input.dataset.engineOption;
      if (!key) return;
      invoke("engine_option_set", { name: key, value: input.checked })
        .then(() => {
          if (key === "emoji") engineOptionEmoji = input.checked;
          if (key === "en_spacer") engineOptionEnSpacer = input.checked;
          showToast(input.checked ? "已开启" : "已关闭（正在输入的应用约 2 秒生效）");
        })
        .catch((error) => {
          input.checked = !input.checked;
          showToast(String(error), true);
        });
    };
  });
  // 四项输入选项：change 即存，不做提交按钮
  app.querySelectorAll("input[data-ime-option]").forEach((input) => {
    input.onchange = () => {
      const key = input.dataset.imeOption;
      if (!key || !imeOptions) return;
      const next = { ...imeOptions, [key]: input.checked };
      invoke("save_ime_options", { opts: next })
        .then(() => {
          imeOptions = next;
          showToast("已保存");
        })
        .catch((error) => {
          input.checked = !input.checked;
          showToast(String(error), true);
        });
    };
  });
  // 通用页：change 即存。range 先在 label 上实时回显，存储仍是 change 时一次提交。
  app.querySelectorAll("[data-general-field]").forEach((input) => {
    const key = input.dataset.generalField;
    if (!key) return;
    // range 实时更新旁边的 output 文本（不打扰正在拖动的手）
    if (input.type === "range") {
      input.addEventListener("input", () => {
        const label = input.dataset.rangeLabel
          ? document.querySelector(input.dataset.rangeLabel)
          : document.querySelector("#general-history-max-label");
        if (label) label.textContent = input.value + (input.dataset.rangeSuffix || "");
      });
    }
    input.onchange = () => {
      if (!generalSettings || input.disabled) return;
      let value;
      if (input.type === "checkbox") {
        value = input.checked;
      } else if (input.type === "range") {
        value = Number(input.value);
        if (!Number.isFinite(value)) return;
      } else if (input.tagName === "SELECT") {
        value = String(input.value);
      } else if (input.type === "text") {
        value = input.value === "" ? null : String(input.value);
      } else {
        return;
      }
      const next = { ...generalSettings, [key]: value };
      // autostart 需要先改注册表，再回写 options.json；后端 set_autostart 一次做完。
      if (key === "autostart") {
        invoke("set_autostart", { enabled: next.autostart })
          .then(() => {
            generalSettings = next;
            showToast("已保存");
          })
          .catch((error) => {
            input.checked = !input.checked;
            showToast(String(error), true);
          });
        return;
      }
      invoke("save_general_settings", { s: next })
        .then(() => {
          generalSettings = next;
          // M7-7：不透明度改动即时应用（options.json 持久化，重启不丢）。
          if (key === "ball_opacity") {
            void getCurrentWindow().setOpacity((next.ball_opacity ?? 100) / 100);
          }
          showToast("已保存");
        })
        .catch((error) => {
          // 失败时回滚 UI
          if (input.type === "checkbox") input.checked = !input.checked;
          showToast(String(error), true);
        });
    };
  });
  // 语音转写：change 即存（同 general 模型，独立 Tauri 命令）
  app.querySelectorAll("[data-speech-field]").forEach((input) => {
    const key = input.dataset.speechField;
    if (!key) return;
    if (input.type === "range") {
      input.addEventListener("input", () => {
        const label = document.querySelector("#speech-max-label");
        if (label) label.textContent = input.value;
      });
    }
    input.onchange = () => {
      if (!speechSettings || input.disabled) return;
      let value;
      if (input.type === "checkbox") {
        value = input.checked;
      } else if (input.type === "range") {
        value = Number(input.value);
        if (!Number.isFinite(value)) return;
      } else {
        return;
      }
      const next = { ...speechSettings, [key]: value };
      invoke("save_speech_settings", { s: next })
        .then(() => {
          speechSettings = next;
          showToast("已保存");
        })
        .catch((error) => {
          if (input.type === "checkbox") input.checked = !input.checked;
          showToast(String(error), true);
        });
    };
  });
  // 偏好页：悬浮条自启开关（change 即存；注册表写入在后端）
  app.querySelectorAll("input[data-settings-field]").forEach((input) => {
    input.onchange = () => {
      const key = input.dataset.settingsField;
      if (key === "autostart") {
        invoke("settings_autostart_set", { enabled: input.checked })
          .then(() => {
            autostartInfo = { ...(autostartInfo || { enabled: false, installed: false }), enabled: input.checked };
            showToast("已保存");
          })
          .catch((error) => {
            input.checked = !input.checked;
            showToast(String(error), true);
          });
      }
    };
  });
  // 方案页：radio 点击 → 立即写入 options.json → 绿 banner（失败红色 + 回滚选中态）
  app.querySelectorAll("input[data-scheme-id]").forEach((input) => {
    input.onchange = () => {
      const id = input.dataset.schemeId;
      if (!id) return;
      if (input.disabled) return; // unavailable 方案（五笔/仓颉数据待接入）
      const previous = schemeCurrent;
      invoke("set_input_scheme", { scheme: id })
        .then(() => {
          schemeCurrent = id;
          const meta = (schemeList || []).find((s) => s.id === id);
          const label = meta ? meta.name_zh : id;
          // 双拼是独立模式（井水不犯河水）：提示用户此时输入双拼码，全拼
          // 键序不会被识别为拼音（librime 双拼 speller 按 2 键切分）。
          const hint = id === "double_pinyin"
            ? "已切换到双拼（小鹤）。请在键盘上输入双拼码，如「我是说」= wouiuo；此模式不识别全拼键序。"
            : `已切换到 ${label}`;
          schemeBanner = { message: hint, error: false };
          render();
          window.setTimeout(() => {
            if (schemeBanner && schemeBanner.message.startsWith("已切换到")) {
              schemeBanner = null;
              if (activePage === "scheme") render();
            }
          }, 6000);
        })
        .catch((error) => {
          schemeBanner = { message: String(error), error: true };
          schemeCurrent = previous;
          render();
        });
    };
  });
  // 皮肤编辑器：只更新脏标记（不重渲染，否则丢焦点 / 重写光标）。
  const skinEditor = app.querySelector("#skin-editor");
  if (skinEditor) {
    skinEditor.addEventListener("input", () => {
      skinState.dirty = skinEditor.value !== skinState.content;
      if (skinState.dirty) markDirty("skin"); else clearDirty("skin");
      // 更新头部 status-pill（仅一次，不深 render）
      const pill = app.querySelector(".skin-page .status-pill");
      if (pill) {
        pill.textContent = `${skinState.source === "user" ? "自定义" : skinState.source === "builtin" ? "内置默认" : "未找到皮肤文件"}${skinState.dirty ? " · 未保存" : ""}`;
      }
    });
    bindSkinForm();
  }
}

async function refreshDashboard() {
  dashboard = await invoke("dashboard_state");
}

async function refreshHistory() {
  historyEntries = await invoke("history_entries");
}

async function refreshImeOptions() {
  imeOptions = await invoke("ime_options");
  // Emoji / 中英混输空格是引擎开关，从算法服务单独读取
  try {
    engineOptionEmoji = await invoke("engine_option_get", { name: "emoji" });
  } catch (_error) {
    engineOptionEmoji = true;
  }
  try {
    engineOptionEnSpacer = await invoke("engine_option_get", { name: "en_spacer" });
  } catch (_error) {
    engineOptionEnSpacer = true;
  }
}

async function refreshAppOptions() {
  appOptions = await invoke("app_options");
}

async function refreshGeneralSettings() {
  generalSettings = await invoke("get_general_settings");
  // M7-7：悬浮球/控制中心窗口不透明度（搜狗 16.1 状态栏不透明度同类）。
  void getCurrentWindow().setOpacity((generalSettings.ball_opacity ?? 100) / 100);
}

// M8-1：拉取跨设备同步活动流；仍在同步页时重绘（新事件到达即刷新）。
async function refreshSyncActivity() {
  syncActivity = await invoke("sync_activity");
  if (activePage === "sync") render();
}

// M8-2：拉取已配对设备列表（peers.json）；仍在同步页时重绘。
async function refreshPeers() {
  peers = await invoke("list_peers");
  if (activePage === "sync") render();
}

// M8-4：拉取直达清单 → 编辑文本（每行 code/名称/类型/目标）。
async function refreshShortcuts() {
  const list = await invoke("list_shortcuts");
  shortcutsText = (list.entries || []).map((s) => `${s.code}\t${s.label}\t${s.kind}\t${s.target}`).join("\n");
  if (activePage === "input") render();
}

async function refreshSpeechSettings() {
  speechSettings = await invoke("get_speech_settings");
}

async function refreshDictionaryInfo() {
  try {
    dictionaryInfo = await invoke("dictionary_info");
  } catch (error) {
    dictionaryInfo = { revision: "读取失败" };
  }
  try {
    dictionaryHistoryList = await invoke("dictionary_history");
  } catch (_error) {
    dictionaryHistoryList = [];
  }
}

async function refreshTypingStats() {
  typingStats = await invoke("typing_stats");
}

// 用户词库（P1 #12）：拉取 userdb 列表。
async function refreshUserdbs() {
  userdbList = await invoke("list_userdbs");
}

async function navigateTo(page) {
  if (page === activePage) return;
  if (dirtyPages.has(activePage)
      && !window.confirm("当前页面有未保存的修改，离开将丢失。确定继续？")) {
    render();
    return;
  }
  globalSearchQuery = "";
  searchOpen = false;
  activePage = page;
  if (page !== 'sync') stopPairPoll();
  if (page === "history") {
    try {
      await refreshHistory();
    } catch (error) {
      showToast(String(error), true);
    }
  } else if (page === "stats") {
    try {
      await refreshTypingStats();
    } catch (error) {
      typingStats = null;
      showToast(String(error), true);
    }
  } else if (page === "settings") {
    try {
      await refreshImeOptions();
    } catch (error) {
      imeOptions = null;
      showToast(String(error), true);
    }
    try {
      autostartInfo = await invoke("settings_autostart_info");
    } catch (_error) {
      autostartInfo = null;
    }
    try {
      defaultIme = await invoke("default_ime_status");
    } catch (_error) {
      defaultIme = null;
    }
  } else if (page === "general") {
    try {
      await refreshGeneralSettings();
      await refreshSpeechSettings();
    } catch (error) {
      generalSettings = null;
      speechSettings = null;
      showToast(String(error), true);
    }
  } else if (page === "scheme") {
    try {
      await refreshSchemes();
    } catch (error) {
      schemeList = null;
      showToast(String(error), true);
    }
  } else if (page === "phrases") {
    try {
      await refreshPhrases();
    } catch (error) {
      phraseRows = null;
      showToast(String(error), true);
    }
  } else if (page === "dictionary") {
    try {
      await refreshDictionaryInfo();
      await refreshUserdbs();
    } catch (error) {
      showToast(String(error), true);
    }
  } else if (page === "skin") {
    clearDirty("skin");
    try {
      const payload = await invoke("skin_payload");
      skinState = { loaded: true, content: payload.content ?? "", source: payload.source, user_path: payload.user_path, dirty: false };
    } catch (error) {
      skinState = { loaded: false, content: "", source: "none", user_path: "", dirty: false };
      showToast(String(error), true);
    }
    try {
      skinPresets = await invoke("list_skins");
    } catch (_error) {
      skinPresets = [];
    }
  }
  await applyMode("page");
}

async function handleAction(button) {
  const action = button.dataset.action;
  const id = Number(button.dataset.id);
  button.disabled = true;
  try {
    const actions = {
      "start-service": ["start_service", undefined, "已发送后台服务启动请求", 900],
      "stop-service": ["stop_service", undefined, "已发送后台服务停止请求", 900],
      "update-dictionary": ["update_dictionary", undefined, "词库更新已在后台启动"],
      "open-settings": ["open_system_settings", undefined, "已打开 Windows 输入法设置"],
      "open-data-directory": ["open_data_directory", undefined, "已打开本地数据目录"],
      "save-relay": ["save_relay", { relay: document.querySelector("#relay")?.value ?? "" }, "中继设置已保存"],
      "copy-history": ["copy_history", { id }, "已复制到剪贴板"],
      "toggle-pin-history": ["set_history_pinned", { id, pinned: button.dataset.pinned !== "true" }, button.dataset.pinned === "true" ? "已取消置顶" : "已置顶"],
      "delete-history": ["delete_history", { id }, "已删除历史条目"],
      "clear-history": ["clear_unpinned_history", undefined, "已清空未置顶历史"],
      "refresh-history": [undefined, undefined, "历史已刷新"],
      "retry-sync-activity": ["retry_sync_activity", { id }, "重试已提交，host 数秒内执行", 3200],
      refresh: [undefined, undefined, "后台状态已刷新"]
    };
    if (action === 'pair-start') {
      const ip = (document.querySelector('#pair-ip')?.value || '').trim();
      if (!ip) { showToast('请输入对方设备 IP', true); return; }
      try {
        await invoke('pair_ui_start', { ip });
        pairWizard = { phase: 'waiting' };
        render();
        startPairPoll();
        showToast('配对已发起，等待对方响应…');
      } catch (error) { showToast(String(error), true); }
      return;
    }
    if (action === 'pair-confirm') {
      try {
        const msg = await invoke('pair_ui_confirm', { yes: true });
        showToast(msg);
      } catch (error) { showToast(String(error), true); }
      return;
    }
    if (action === 'pair-cancel') {
      try { await invoke('pair_ui_confirm', { yes: false }); } catch (_error) { /* 忽略 */ }
      stopPairPoll();
      pairWizard = null;
      render();
      showToast('已取消配对');
      return;
    }
    if (action === 'pair-dismiss') {
      pairWizard = null;
      render();
      return;
    }
    if (action === "add-app-option") {
      appOptions = [...(appOptions ?? []), { app: "", ascii_mode: true }];
      markDirty("input");
      render();
      return;
    }
    if (action === "remove-app-option") {
      const idx = Number(button.dataset.index);
      appOptions = (appOptions ?? []).filter((_v, i) => i !== idx);
      markDirty("input");
      render();
      return;
    }
    if (action === "save-app-options") {
      try {
        const rows = [...document.querySelectorAll(".app-options-panel .app-option-row")];
        const items = rows
          .map((row) => ({
            app: row.querySelector(".app-option-name")?.value ?? "",
            ascii_mode: row.querySelector(".app-option-ascii")?.checked ?? false,
            vim_mode: row.querySelector(".app-option-vim")?.checked ?? false
          }))
          .filter((item) => item.app.trim() !== "");
        await invoke("save_app_options", { items });
        appOptions = items;
        clearDirty("input");
        render();
        showToast("按应用设置已保存（约 2 秒内生效）");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "set-default-ime" || action === "clear-default-ime") {
      try {
        await invoke(action === "set-default-ime" ? "set_default_ime" : "clear_default_ime");
        defaultIme = await invoke("default_ime_status");
        render();
        showToast(action === "set-default-ime" ? "已设为默认输入法（新应用生效）" : "已清除默认输入法");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "exit-app") {
      try {
        await invoke("exit_app");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "refresh-stats") {
      try {
        await refreshTypingStats();
        showToast("统计已刷新");
      } catch (error) {
        typingStats = null;
        showToast(String(error), true);
      }
      render();
      return;
    }
    if (action === "apply-skin") {
      const skinId = String(button.dataset.skinId || "");
      if (!skinId) return;
      try {
        await invoke("apply_skin", { id: skinId });
        const payload = await invoke("skin_payload");
        skinState = { loaded: true, content: payload.content ?? "", source: payload.source, user_path: payload.user_path, dirty: false };
        clearDirty("skin");
        try {
          skinPresets = await invoke("list_skins");
        } catch (_error) {
          skinPresets = [];
        }
        const meta = skinPresets.find((p) => p.id === skinId);
        skinPresetBanner = {
          message: `已应用 ${meta ? meta.name_zh : skinId}（SSOT 文件已更新，候选窗下次启动生效）`,
          error: false
        };
        render();
        window.setTimeout(() => {
          skinPresetBanner = null;
          if (activePage === "skin") render();
        }, 4200);
      } catch (error) {
        skinPresetBanner = { message: String(error), error: true };
        render();
        window.setTimeout(() => {
          skinPresetBanner = null;
          if (activePage === "skin") render();
        }, 4200);
      }
      return;
    }
    if (action === "save-skin") {
      const editor = document.querySelector("#skin-editor");
      // 若表单 debounce 尚在等待，先等它落盘到 textarea（textarea 是 SSOT）
      if (skinFormSync.timer) {
        await new Promise((resolve) => window.setTimeout(resolve, 130));
      }
      const content = editor ? editor.value : "";
      await invoke("save_skin", { content });
      skinState = { ...skinState, content, source: "user", dirty: false };
      clearDirty("skin");
      render();
      showToast("皮肤已保存，候选窗与面板即时应用");
      return;
    }
    if (action === "reset-skin") {
      if (!window.confirm("删除自定义皮肤，回退到内置默认外观？")) {
        render();
        return;
      }
      await invoke("reset_skin");
      const payload = await invoke("skin_payload");
      skinState = { loaded: true, content: payload.content ?? "", source: payload.source, user_path: payload.user_path, dirty: false };
      clearDirty("skin");
      render();
      showToast("已删除自定义皮肤");
      return;
    }
    if (action === "reload-skin") {
      const payload = await invoke("skin_payload");
      skinState = { loaded: true, content: payload.content ?? "", source: payload.source, user_path: payload.user_path, dirty: false };
      clearDirty("skin");
      render();
      showToast("已重新加载");
      return;
    }
    if (action === "export-skin") {
      const name = (document.querySelector("#skin-export-name")?.value || "").trim() || "custom";
      const editor = document.querySelector("#skin-editor");
      const json = editor ? editor.value : "";
      if (!json.trim()) { showToast("编辑器为空，无法导出", true); return; }
      try {
        const path = await invoke("export_skin", { name, json });
        showToast(`已导出：${path}`);
      } catch (error) { showToast(String(error), true); }
      return;
    }
    if (action === "rollback-dictionary") {
      const last = dictionaryInfo.revision || "上一版";
      if (!window.confirm(`将回滚到 ${last}，继续？`)) {
        render();
        return;
      }
      try {
        const result = await invoke("rollback_dictionary");
        await refreshDictionaryInfo().catch(() => {});
        render();
        showToast(result || "已回滚");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "rollback-dictionary-to") {
      const select = document.querySelector("#dict-rollback-target");
      const target = select ? select.value : "";
      if (!target) {
        showToast("请先选择要回滚到的历史版本", true);
        render();
        return;
      }
      if (!window.confirm(`将回滚到 ${target}，继续？`)) {
        render();
        return;
      }
      try {
        const result = await invoke("rollback_dictionary_to", { revision: target });
        await refreshDictionaryInfo().catch(() => {});
        render();
        showToast(result || "已回滚");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "phrase-add") {
      if (!Array.isArray(phraseRows)) phraseRows = [];
      phraseRows.push({ id: 0, code: "", text: "", weight: 100 });
      markDirty("phrases");
      render();
      return;
    }
    if (action === "phrase-remove") {
      const index = Number(button.dataset.index);
      if (!Array.isArray(phraseRows) || Number.isNaN(index) || index < 0 || index >= phraseRows.length) return;
      phraseRows.splice(index, 1);
      markDirty("phrases");
      render();
      return;
    }
    if (action === "phrase-save" || action === "phrase-deploy") {
      // 从 DOM 收集当前行数据（编辑后的值以 DOM 为准）
      const rows = [];
      document.querySelectorAll("[data-phrase-row]").forEach((rowEl) => {
        const code = rowEl.querySelector("[data-phrase-field=code]")?.value ?? "";
        const text = rowEl.querySelector("[data-phrase-field=text]")?.value ?? "";
        const weightRaw = rowEl.querySelector("[data-phrase-field=weight]")?.value ?? "";
        const weight = weightRaw === "" ? undefined : Number(weightRaw);
        if (code.trim() && text.trim()) rows.push({ id: 0, code: code.trim(), text: text.trim(), weight });
      });
      try {
        const saved = await invoke("save_custom_phrases", { phrases: rows });
        phraseRows = rows;
        clearDirty("phrases");
        render();
        if (action === "phrase-deploy") {
          showToast(`${saved}，正在重建词典…`);
          try {
            const result = await invoke("redeploy_dictionaries");
            showToast(result);
          } catch (deployError) {
            showToast(String(deployError), true);
          }
        } else {
          showToast(`${saved}（保存后需「保存并部署」或重启输入法生效）`);
        }
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "export-userdb") {
      const name = String(button.dataset.name || "");
      if (!name) return;
      try {
        const result = await invoke("export_userdb", { name });
        await refreshUserdbs().catch(() => {});
        render();
        showToast(result);
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "clear-userdb") {
      const name = String(button.dataset.name || "");
      if (!name) return;
      if (!window.confirm(`清空用户词库「${name}」？会重置该词库的调频与自造词（先自动备份）。`)) {
        render();
        return;
      }
      try {
        const result = await invoke("clear_userdb", { name });
        await refreshUserdbs().catch(() => {});
        render();
        showToast(result);
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "symbol-copy") {
      const symbol = String(button.dataset.symbolCopy || "");
      if (!symbol) return;
      try {
        await navigator.clipboard.writeText(symbol);
        saveRecent(symbol); // 记忆最近使用（搜狗「记忆功能」同款）
        showToast(`已复制「${symbol}」`);
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "symbol-cat") {
      activeSymbolCat = String(button.dataset.symbolCat || "common");
      render();
      return;
    }
    if (action === "symbol-tone") {
      const tone = String(button.dataset.symbolTone || "");
      emojiTone = tone && EMOJI_TONES.includes(tone) ? tone : null;
      saveEmojiTone(emojiTone);
      render();
      return;
    }
    if (action === 'save-scenario') {
      const name = document.querySelector('#scenario-select')?.value || 'none';
      try {
        const msg = await invoke('save_scenario_dict', { name });
        if (generalSettings) generalSettings.scenario_dict = name;
        showToast(msg);
      } catch (error) { showToast(String(error), true); }
      return;
    }
    // M8-4：保存直达清单（解析 textarea 行格式）
    if (action === "save-shortcuts") {
      const text = document.querySelector("#shortcuts-editor")?.value ?? "";
      const entries = [];
      for (const line of text.split("\n")) {
        const parts = line.split("\t").map((p) => p.trim());
        if (parts.length < 4 || !parts[0]) continue;
        const [code, label, kind, target] = parts;
        if (kind !== "app" && kind !== "url") {
          showToast(`类型需为 app 或 url：${code}`, true);
          return;
        }
        entries.push({ id: 0, code, label, kind, target });
      }
      try {
        const saved = await invoke("save_shortcuts", { shortcuts: { next_id: 0, entries } });
        shortcutsText = (saved.entries || []).map((s) => `${s.code}\t${s.label}\t${s.kind}\t${s.target}`).join("\n");
        clearDirty("input");
        render();
        showToast(`已保存 ${saved.entries.length} 条直达`);
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    // M8-2：设备管理（重命名/移除，两步确认避免误删）
    if (action === "peer-rename-start") {
      renamingFp = String(button.dataset.fp || "");
      render();
      return;
    }
    if (action === "peer-rename-cancel") {
      renamingFp = null;
      render();
      return;
    }
    if (action === "peer-rename") {
      const fp = String(button.dataset.fp || "");
      const input = document.querySelector("[data-peer-rename-input]");
      const name = input ? input.value.trim() : "";
      if (!name) {
        showToast("设备名称不能为空", true);
        return;
      }
      try {
        await invoke("rename_peer", { fingerprint: fp, name });
        renamingFp = null;
        await refreshPeers();
        showToast("已重命名设备");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "peer-remove") {
      const fp = String(button.dataset.fp || "");
      if (confirmRemoveFp !== fp) {
        confirmRemoveFp = fp;
        render();
        return;
      }
      confirmRemoveFp = null;
      try {
        await invoke("remove_peer", { fingerprint: fp });
        await refreshPeers();
        showToast("已移除设备");
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    // M8-3：历史面板批量选择/置顶/删除
    if (action === "history-select-mode") {
      historySelectMode = !historySelectMode;
      historySelected.clear();
      confirmBatchDelete = false;
      render();
      return;
    }
    if (action === "history-select") {
      const id = Number(button.dataset.id);
      if (historySelected.has(id)) historySelected.delete(id);
      else historySelected.add(id);
      render();
      return;
    }
    if (action === "history-select-all") {
      const q = historyQuery.trim().toLocaleLowerCase();
      historyEntries.forEach((e) => {
        if (`${e.text} ${e.source_app} ${e.kind}`.toLocaleLowerCase().includes(q)) historySelected.add(e.id);
      });
      render();
      return;
    }
    if (action === "history-select-clear") {
      historySelected.clear();
      confirmBatchDelete = false;
      render();
      return;
    }
    if (action === "history-batch-pin" || action === "history-batch-unpin") {
      const ids = [...historySelected];
      if (!ids.length) {
        showToast("请先选择条目", true);
        return;
      }
      const pinned = action === "history-batch-pin";
      try {
        await invoke("batch_set_pinned", { ids, pinned });
        await refreshHistory();
        render();
        showToast(pinned ? `已置顶 ${ids.length} 条` : `已取消置顶 ${ids.length} 条`);
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    if (action === "history-batch-delete") {
      const ids = [...historySelected];
      if (!ids.length) {
        showToast("请先选择条目", true);
        return;
      }
      if (!confirmBatchDelete) {
        confirmBatchDelete = true;
        render();
        return;
      }
      confirmBatchDelete = false;
      try {
        const n = await invoke("batch_delete_history", { ids });
        historySelected.clear();
        historySelectMode = false;
        await refreshHistory();
        render();
        showToast(`已删除 ${n} 条`);
      } catch (error) {
        showToast(String(error), true);
      }
      return;
    }
    const [command, args, success, delay] = actions[action];
    if (command) await invoke(command, args);
    if (delay) await new Promise((resolve) => window.setTimeout(resolve, delay));
    await refreshDashboard();
    if (activePage === "history") await refreshHistory();
    if (activePage === "sync") await refreshSyncActivity();
    render();
    showToast(success);
  } catch (error) {
    showToast(String(error), true);
    button.disabled = false;
  }
}

function showToast(message, error = false) {
  const currentNotice = { message: String(message), error };
  notice = currentNotice;
  render();
  window.setTimeout(() => {
    if (notice === currentNotice) {
      notice = null;
      render();
    }
  }, 4200);
}

function escapeHtml(value) {
  return String(value).replace(/[&<>'"]/g, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[character]);
}

// 文本域用：JSON 里的引号/单引号不需要转义，< & > 需要。
function escapeTextarea(value) {
  return String(value).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);
}

// 启动：恢复上次悬浮条位置（没有则放主屏右下角）；注册窗口事件；
// 已部署且未配置时默认开启悬浮条自启。失败全部静默，不阻塞 UI。
async function bootShell() {
  try {
    // 启动即按悬浮条内容尺寸调整窗口（此前 appliedSizeKey 初始为 "bar"，
    // 导致窗口停在 tauri.conf 的初始尺寸，与条内容长度不匹配）
    await applyMode("bar");
  } catch (_error) { /* 忽略 */ }
  try {
    const saved = localStorage.getItem("shurufa-window-pos");
    if (saved) {
      const parsed = JSON.parse(saved);
      // 位置校验：Windows 隐藏窗口会把窗口移到 (-32000,-32000) 哨兵位，
      // onMoved 会把它存进 localStorage → 下次启动 restore 被钳到屏幕左上角
      // 死角（2026-08-14 实机复现）。NaN/超范围值同样丢弃，回退右下角。
      const plausible = Number.isFinite(parsed.x) && Number.isFinite(parsed.y)
        && parsed.x > -10000 && parsed.y > -10000
        && parsed.x < 100000 && parsed.y < 100000;
      if (plausible) {
        await invoke("restore_window_position", { x: parsed.x, y: parsed.y });
      } else {
        await invoke("place_window_bottom_right");
      }
    } else {
      await invoke("place_window_bottom_right");
    }
  } catch (_error) {
    try {
      await invoke("place_window_bottom_right");
    } catch (_ignored) { /* 忽略 */ }
  }
  try {
    const win = getCurrentWindow();
    win.onMoved(async () => {
      try {
        const pos = await win.outerPosition();
        localStorage.setItem("shurufa-window-pos", JSON.stringify({ x: pos.x, y: pos.y }));
      } catch (_e) { /* 忽略 */ }
      // 拖拽中窗口被拖出工作区时拉回：原生拖动循环会吞掉 JS mouseup，
      // 拖拽结束的钳制必须挂在 onMoved 上（最后一次移动的 onMoved 会在
      // 循环结束后落地）。在位时后端 no-op，不影响正常拖动。
      try {
        await invoke("clamp_window_to_work_area");
      } catch (_e) { /* 忽略 */ }
    });
    // 菜单面板失焦自动收回悬浮条；页面子视图不收回（避免打断正在进行的操作）
    win.onFocusChanged(({ payload: focused }) => {
      if (!focused && uiMode === "menu") void applyMode("bar");
    });
  } catch (_e) { /* 非 Tauri 环境忽略 */ }
  try {
    const info = await invoke("settings_autostart_info");
    autostartInfo = info;
    if (info.installed && !info.enabled) {
      await invoke("settings_autostart_set", { enabled: true });
      autostartInfo = { ...info, enabled: true };
    }
  } catch (_e) { /* 忽略 */ }
  // 条上「中/En」指示：启动时读一次全局中英态
  void refreshImeMode();
}

refreshDashboard()
  .catch((error) => showToast(String(error), true))
  .then(() => refreshSchemes().catch(() => {})) // 工作台首页方案卡需要当前方案
  .then(bootShell)
  .finally(render);
