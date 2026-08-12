import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUpRight,
  BookOpenText,
  ChartColumn,
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
  Lightbulb,
  MonitorSmartphone,
  Moon,
  Palette,
  Pin,
  Play,
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
  ArrowUpRight,
  BookOpenText,
  ChartColumn,
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
  Lightbulb,
  MonitorSmartphone,
  Moon,
  Palette,
  Pin,
  Play,
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
  { id: "skin", label: "皮肤", icon: "palette" },
  { id: "sync", label: "跨设备", icon: "monitor-smartphone" },
  { id: "settings", label: "偏好", icon: "sliders-horizontal" }
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
// 预设皮肤列表（schemas/skins-index.json）；banner 为一次性成功/失败提示
let skinPresets = [];
let skinPresetBanner = null;
// 通用页（通用 6 字段）；null=未加载/读取失败（表单全部禁用）
let generalSettings = null;
// 语音转写卡片（wave 4 新挂在通用页里；speechSettings 与 general 完全独立
// 存储 / 独立 Tauri 命令），null=未加载/读取失败
let speechSettings = null;
// 输入方案页（wave 4 新增）：null=未加载；list=后端 list_input_schemes 返回的 4 项
let schemeList = null;
let schemeCurrent = "pinyin";
let schemeBanner = null;

const app = document.querySelector("#app");

app.addEventListener("click", (event) => {
  const target = event.target;
  const button = target instanceof Element ? target.closest("button") : null;
  if (!button || !app.contains(button) || button.disabled) return;
  if (button.dataset.page) {
    void navigateTo(button.dataset.page);
  }
});

app.addEventListener("input", (event) => {
  if (event.target.id !== "history-search") return;
  historyQuery = event.target.value;
  render();
  const search = document.querySelector("#history-search");
  search?.focus();
  search?.setSelectionRange(historyQuery.length, historyQuery.length);
});

function navTemplate() {
  return pages
    .map(
      (page) => `
        <button class="nav-item ${page.id === activePage ? "active" : ""}" data-page="${page.id}">
          <i data-lucide="${page.icon}"></i>
          <span>${page.label}</span>
        </button>`
    )
    .join("");
}

function statusPill() {
  const running = dashboard.service_status === "运行中";
  return `<span class="status-pill ${running ? "online" : "idle"}"><span></span>${dashboard.service_status}</span>`;
}

function workspacePage() {
  const serviceAction = dashboard.service_status === "运行中"
    ? `<button class="outline-action" data-action="stop-service"><i data-lucide="square"></i>停止后台服务</button>`
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
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="sparkles"></i></div><div><h3>候选与历史</h3><p>使用 Ctrl+Shift+V 呼出剪贴板历史</p></div><button class="outline-action" data-page="history"><i data-lucide="clipboard-list"></i>管理历史</button></div>
      </article>
      <article class="hint-card"><i data-lucide="lightbulb"></i><p>后台服务负责剪贴板历史与跨设备同步。它会以隐藏窗口运行。语音转写（Ctrl+Shift+S）当前为 dev-stub。</p></article>
    </section>`;
}

function historyPage() {
  const query = historyQuery.trim().toLocaleLowerCase();
  const entries = historyEntries.filter((entry) => `${entry.text} ${entry.source_app} ${entry.kind}`.toLocaleLowerCase().includes(query));
  const list = entries.length
    ? entries.map((entry) => `
        <article class="history-entry">
          <div class="history-kind ${entry.pinned ? "pinned" : ""}"><i data-lucide="${entry.kind === "图片" ? "image" : entry.kind === "文件" ? "folder-open" : "copy"}"></i></div>
          <div class="history-copy"><div class="history-title">${escapeHtml(entry.text)}</div><p>${escapeHtml(entry.kind)}${entry.source_app ? ` · ${escapeHtml(entry.source_app)}` : ""}</p></div>
          <div class="history-actions">
            <button class="icon-action" data-action="copy-history" data-id="${entry.id}" title="复制到剪贴板"><i data-lucide="copy"></i></button>
            <button class="icon-action" data-action="toggle-pin-history" data-id="${entry.id}" data-pinned="${entry.pinned}" title="${entry.pinned ? "取消置顶" : "置顶"}"><i data-lucide="pin"></i></button>
            <button class="icon-action danger-action" data-action="delete-history" data-id="${entry.id}" title="删除历史条目"><i data-lucide="trash-2"></i></button>
          </div>
        </article>`).join("")
    : `<div class="history-empty"><i data-lucide="clipboard-list"></i><p>${query ? "没有匹配的历史内容" : "还没有可管理的剪贴板历史"}</p></div>`;
  return `
    <section class="page settings-page history-page">
      <header class="page-header"><div><p class="eyebrow">CLIPBOARD</p><h1>剪贴板历史</h1></div><button class="outline-action" data-action="clear-history"><i data-lucide="trash-2"></i>清空未置顶</button></header>
      <div class="history-toolbar"><label class="history-search"><i data-lucide="search"></i><input id="history-search" value="${escapeHtml(historyQuery)}" placeholder="搜索历史内容或来源" /></label><button class="icon-action" data-action="refresh-history" title="刷新历史"><i data-lucide="refresh-cw"></i></button></div>
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
      <article class="hint-card"><i data-lucide="info"></i><p>更新完成后，重启输入法即可应用新词库。</p></article>
    </section>`;
}

function syncPage() {
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
    </section>`;
}

function imeOptionsPanel() {
  if (!imeOptions) {
    return `<article class="setting-panel"><div class="setting-row"><div class="row-icon"><i data-lucide="keyboard"></i></div><div><h3>输入选项</h3><p>读取中…</p></div></div></article>`;
  }
  const items = [
    ["shift_switch_cn_en", "Shift 切换中英文", "按下 Shift 即在中文/英文直输之间切换"],
    ["shift_space_full_shape", "Shift+空格 切换全角/半角", "无组合时切换空格与字母的全/半角"],
    ["ctrl_period_ascii_punct", "Ctrl+. 切换中文/英文标点", "收尾当前组合后切换标点全/半角"],
    ["capslock_to_english", "CapsLock 直接输入英文", "按下 CapsLock 即切到英文直输（再按 Shift 回中文）"]
  ];
  const rows = items
    .map(([key, title, desc]) => {
      const checked = imeOptions[key] ? "checked" : "";
      return `<div class="setting-row"><div class="row-icon"><i data-lucide="circle-dot"></i></div><label class="setting-toggle"><div><h3>${title}</h3><p>${desc}</p></div></label><label class="switch"><input type="checkbox" data-ime-option="${key}" ${checked} /><span></span></label></div>`;
    })
    .join(`<div class="divider"></div>`);
  return `<article class="setting-panel ime-options-panel"><div class="panel-heading"><div class="row-icon blue"><i data-lucide="keyboard"></i></div><div><h3>输入选项</h3><p>全部对正在输入的应用热生效，延迟约 2 秒</p></div></div>${rows}</article>`;
}

function settingsPage() {
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">PREFERENCES</p><h1>偏好</h1></div></header>
      ${imeOptionsPanel()}
      <article class="setting-panel">
        <div class="setting-row"><div class="row-icon"><i data-lucide="settings-2"></i></div><div><h3>系统输入法</h3><p>管理语言、输入法和默认输入法</p></div><button class="outline-action" data-action="open-settings"><i data-lucide="arrow-up-right"></i>打开设置</button></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="folder-open"></i></div><div><h3>本地数据</h3><p class="path-value">${escapeHtml(dashboard.data_directory)}</p></div><button class="outline-action" data-action="open-data-directory"><i data-lucide="folder-open"></i>打开目录</button></div>
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
  app.innerHTML = `
    <aside class="sidebar">
      <div class="brand"><div class="brand-mark"><i data-lucide="languages"></i></div><div><strong>Shurufa</strong><span>拼音与剪贴板</span></div></div>
      <nav>${navTemplate()}</nav>
      <div class="sidebar-footer"><span class="footer-dot"></span><span>后台服务 ${dashboard.service_status}</span></div>
    </aside>
    <main class="content">${pageTemplate()}</main>
    <div id="toast" class="${notice ? `show${notice.error ? " error" : ""}` : ""}" aria-live="polite">${notice ? escapeHtml(notice.message) : ""}</div>
    <button type="button" class="theme-toggle" aria-label="切换主题" title="主题：跟随系统"><i data-lucide="monitor-smartphone"></i></button>`;
  createIcons({ icons: controlCenterIcons });
  const themeBtn = app.querySelector(".theme-toggle");
  if (themeBtn) {
    // 把按钮图标同步成当前主题（render 会重置 DOM，这里要每次 render 后重新点亮）
    const t = currentTheme();
    const icon = t === "dark" ? "moon" : t === "light" ? "sun" : "monitor-smartphone";
    themeBtn.innerHTML = `<i data-lucide="${icon}"></i>`;
    themeBtn.title = `主题：${t === "auto" ? "跟随系统" : t === "light" ? "亮色" : "暗色"}`;
    createIcons({ icons: controlCenterIcons });
    themeBtn.onclick = () => toggleTheme();
  }
  app.querySelectorAll("button[data-action]").forEach((button) => {
    button.onclick = () => {
      button.disabled = true;
      void handleAction(button);
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
        const label = document.querySelector("#general-history-max-label");
        if (label) label.textContent = input.value;
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
}

async function refreshGeneralSettings() {
  generalSettings = await invoke("get_general_settings");
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

async function navigateTo(page) {
  activePage = page;
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
  } else if (page === "skin") {
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
  } else if (page === "dictionary") {
    try {
      await refreshDictionaryInfo();
    } catch (_error) {
      // 失败时 dictionaryPage 内已做兜底
    }
  }
  render();
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
      refresh: [undefined, undefined, "后台状态已刷新"]
    };
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
      render();
      showToast("已删除自定义皮肤");
      return;
    }
    if (action === "reload-skin") {
      const payload = await invoke("skin_payload");
      skinState = { loaded: true, content: payload.content ?? "", source: payload.source, user_path: payload.user_path, dirty: false };
      render();
      showToast("已重新加载");
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
    const [command, args, success, delay] = actions[action];
    if (command) await invoke(command, args);
    if (delay) await new Promise((resolve) => window.setTimeout(resolve, delay));
    await refreshDashboard();
    if (activePage === "history") await refreshHistory();
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

refreshDashboard()
  .catch((error) => showToast(String(error), true))
  .then(() => refreshSchemes().catch(() => {})) // 工作台首页方案卡需要当前方案
  .finally(render);
