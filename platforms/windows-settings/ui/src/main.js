import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUpRight,
  BookOpenText,
  CircleDot,
  ClipboardList,
  Copy,
  createIcons,
  FolderOpen,
  Image,
  Info,
  Keyboard,
  Languages,
  LayoutDashboard,
  Lightbulb,
  MonitorSmartphone,
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
  Trash2
} from "lucide";

const controlCenterIcons = {
  ArrowUpRight,
  BookOpenText,
  CircleDot,
  ClipboardList,
  Copy,
  FolderOpen,
  Image,
  Info,
  Keyboard,
  Languages,
  LayoutDashboard,
  Lightbulb,
  MonitorSmartphone,
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
  Trash2
};

const pages = [
  { id: "workspace", label: "工作台", icon: "layout-dashboard" },
  { id: "input", label: "输入", icon: "keyboard" },
  { id: "history", label: "历史", icon: "clipboard-list" },
  { id: "dictionary", label: "词库", icon: "book-open-text" },
  { id: "sync", label: "跨设备", icon: "monitor-smartphone" },
  { id: "settings", label: "偏好", icon: "sliders-horizontal" }
];

let activePage = "workspace";
let dashboard = {
  relay: "",
  service_status: "待启动",
  data_directory: ""
};
let historyEntries = [];
let historyQuery = "";
let notice = null;

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
        <button class="metric-card metric-link" data-page="input"><div class="metric-icon teal"><i data-lucide="keyboard"></i></div><span>输入方案</span><strong>雾凇拼音</strong><p>查看输入与历史设置</p></button>
        <button class="metric-card metric-link" data-page="history"><div class="metric-icon blue"><i data-lucide="clipboard-list"></i></div><span>剪贴板历史</span><strong>Ctrl+Shift+V</strong><p>查看、复制和整理历史</p></button>
        <button class="metric-card metric-link" data-page="dictionary"><div class="metric-icon coral"><i data-lucide="book-open-text"></i></div><span>热门词库</span><strong>rime-ice</strong><p>检查并更新词库</p></button>
      </div>
    </section>`;
}

function inputPage() {
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">INPUT</p><h1>输入</h1></div>${statusPill()}</header>
      <article class="setting-panel">
        <div class="setting-row selected"><div class="row-icon"><i data-lucide="circle-dot"></i></div><div><h3>拼音输入</h3><p>雾凇拼音方案已部署</p></div><button class="outline-action" data-action="open-settings"><i data-lucide="arrow-up-right"></i>系统设置</button></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="sparkles"></i></div><div><h3>候选与历史</h3><p>使用 Ctrl+Shift+V 呼出剪贴板历史</p></div><button class="outline-action" data-page="history"><i data-lucide="clipboard-list"></i>管理历史</button></div>
      </article>
      <article class="hint-card"><i data-lucide="lightbulb"></i><p>后台服务负责剪贴板历史与跨设备同步。它会以隐藏窗口运行。</p></article>
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
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">DICTIONARY</p><h1>词库</h1></div></header>
      <article class="setting-panel dictionary-panel">
        <div class="setting-row"><div class="row-icon coral"><i data-lucide="book-open-text"></i></div><div><h3>热门云词库</h3><p>rime-ice · 常用词与流行表达</p></div><button class="outline-action" data-action="update-dictionary"><i data-lucide="refresh-cw"></i>更新词库</button></div>
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

function settingsPage() {
  return `
    <section class="page settings-page">
      <header class="page-header"><div><p class="eyebrow">PREFERENCES</p><h1>偏好</h1></div></header>
      <article class="setting-panel">
        <div class="setting-row"><div class="row-icon"><i data-lucide="settings-2"></i></div><div><h3>系统输入法</h3><p>管理语言、输入法和默认输入法</p></div><button class="outline-action" data-action="open-settings"><i data-lucide="arrow-up-right"></i>打开设置</button></div>
        <div class="divider"></div>
        <div class="setting-row"><div class="row-icon dim"><i data-lucide="folder-open"></i></div><div><h3>本地数据</h3><p class="path-value">${escapeHtml(dashboard.data_directory)}</p></div><button class="outline-action" data-action="open-data-directory"><i data-lucide="folder-open"></i>打开目录</button></div>
      </article>
    </section>`;
}

function pageTemplate() {
  switch (activePage) {
    case "input": return inputPage();
    case "history": return historyPage();
    case "dictionary": return dictionaryPage();
    case "sync": return syncPage();
    case "settings": return settingsPage();
    default: return workspacePage();
  }
}

function render() {
  app.innerHTML = `
    <aside class="sidebar">
      <div class="brand"><div class="brand-mark"><i data-lucide="languages"></i></div><div><strong>Shurufa</strong><span>拼音与剪贴板</span></div></div>
      <nav>${navTemplate()}</nav>
      <div class="sidebar-footer"><span class="footer-dot"></span><span>后台服务 ${dashboard.service_status}</span></div>
    </aside>
    <main class="content">${pageTemplate()}</main>
    <div id="toast" class="${notice ? `show${notice.error ? " error" : ""}` : ""}" aria-live="polite">${notice ? escapeHtml(notice.message) : ""}</div>`;
  createIcons({ icons: controlCenterIcons });
  app.querySelectorAll("button[data-action]").forEach((button) => {
    button.onclick = () => {
      button.disabled = true;
      void handleAction(button);
    };
  });
}

async function refreshDashboard() {
  dashboard = await invoke("dashboard_state");
}

async function refreshHistory() {
  historyEntries = await invoke("history_entries");
}

async function navigateTo(page) {
  activePage = page;
  if (page === "history") {
    try {
      await refreshHistory();
    } catch (error) {
      showToast(String(error), true);
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

refreshDashboard().catch((error) => showToast(String(error), true)).finally(render);
