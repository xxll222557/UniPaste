import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const app = document.querySelector("#app");

const NAV_ITEMS = [
  { id: "home", label: "主页", icon: "home" },
  { id: "pair", label: "配对", icon: "devices" },
  { id: "history", label: "历史", icon: "history" },
  { id: "settings", label: "设置", icon: "settings" },
  { id: "about", label: "关于", icon: "info" }
];

const state = {
  snapshot: null,
  loading: true,
  error: null,
  activeTab: "home",
  pairingCodeInput: "",
  pairingCodeError: null,
  deviceNameDraft: "",
  deviceNameError: null,
  syncPrefsError: null
};

const formatTime = (ms) => {
  if (!ms) return "-";
  return new Date(ms).toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit"
  });
};

const escapeHtml = (value) =>
  String(value ?? "").replace(/[&<>"']/g, (char) => {
    const map = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;"
    };
    return map[char] ?? char;
  });

const getPairingDigits = () =>
  Array.from({ length: 6 }, (_, index) => state.pairingCodeInput[index] ?? "");

const historyKindIcon = (kind) => {
  if (kind === "图片") return "image";
  if (kind === "HTML") return "html";
  if (kind === "文件") return "folder";
  return "text";
};

const networkStatusText = (status) => {
  if (status === "connected") return "已建立连接";
  if (status === "discovered") return "已发现设备";
  if (status === "ready") return "等待设备加入";
  if (status === "paused") return "同步已暂停";
  return "正在启动";
};

const icon = (name) => {
  const icons = {
    home: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 10.8 12 4l8 6.8V20a1 1 0 0 1-1 1h-4.5v-6h-5v6H5a1 1 0 0 1-1-1z"></path>
      </svg>
    `,
    devices: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v7A2.5 2.5 0 0 1 17.5 16h-4L15 18h2v2H7v-2h2l1.5-2h-4A2.5 2.5 0 0 1 4 13.5zm2 0v7h12v-7z"></path>
      </svg>
    `,
    history: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 5a7 7 0 1 1-6.32 10H3l3.5-3.5L10 15H7.74A5 5 0 1 0 12 7z"></path>
        <path d="M11 8h2v4.2l3 1.8-1 1.7L11 13.3z"></path>
      </svg>
    `,
    settings: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="m19.4 13 .1-1-.1-1 2-1.5-2-3.4-2.3.9a7.7 7.7 0 0 0-1.7-1L15 3h-6l-.4 3a7.7 7.7 0 0 0-1.7 1l-2.3-.9-2 3.4L4.6 11a9.4 9.4 0 0 0 0 2l-2 1.5 2 3.4 2.3-.9a7.7 7.7 0 0 0 1.7 1L9 21h6l.4-3a7.7 7.7 0 0 0 1.7-1l2.3.9 2-3.4zM12 15.5A3.5 3.5 0 1 1 12 8a3.5 3.5 0 0 1 0 7.5"></path>
      </svg>
    `,
    info: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 2.5A9.5 9.5 0 1 1 2.5 12 9.5 9.5 0 0 1 12 2.5m-1 6h2v2h-2zm0 4h2v5h-2z"></path>
      </svg>
    `,
    sync: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 5a7 7 0 0 1 6.5 4.5H16v2h5V6.5h-2v2A9 9 0 0 0 3.8 9.7l1.8.9A7 7 0 0 1 12 5"></path>
        <path d="M12 19a7 7 0 0 1-6.5-4.5H8v-2H3v5h2v-2a9 9 0 0 0 15.2-1.2l-1.8-.9A7 7 0 0 1 12 19"></path>
      </svg>
    `,
    shield: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 3 5 6v5c0 5 3 8.5 7 10 4-1.5 7-5 7-10V6zm-1 5h2v5h-2zm0 6h2v2h-2z"></path>
      </svg>
    `,
    network: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M5 7a2 2 0 1 1 2 2H6v2h4v2H8a2 2 0 1 1 0 2h2v2h4v-4h2a2 2 0 1 1 0-2h-2V9h4V7h-1a2 2 0 1 1 0-2 2 2 0 0 1-1 1.73V7h-4v4h-2V7z"></path>
      </svg>
    `,
    text: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M5 5h14v2H13v12h-2V7H5z"></path>
      </svg>
    `,
    html: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="m8 8-4 4 4 4 1.4-1.4L6.8 12l2.6-2.6zm8 0-1.4 1.4 2.6 2.6-2.6 2.6L16 16l4-4zm-2.7-3.9-3.2 15 2 .4 3.2-15z"></path>
      </svg>
    `,
    image: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M5 5h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2m0 2v10h14V7zm2 8 2.5-3 2 2.5 3-4 3.5 4.5zM9 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3"></path>
      </svg>
    `,
    folder: `
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 6.5A2.5 2.5 0 0 1 6.5 4H10l2 2h5.5A2.5 2.5 0 0 1 20 8.5v9A2.5 2.5 0 0 1 17.5 20h-11A2.5 2.5 0 0 1 4 17.5z"></path>
      </svg>
    `
  };
  return icons[name] ?? "";
};

async function refresh() {
  try {
    state.snapshot = await invoke("get_snapshot");
    if (!state.deviceNameDraft) {
      state.deviceNameDraft = state.snapshot.local_device.device_name;
    }
    state.error = null;
  } catch (error) {
    state.error = String(error);
  } finally {
    state.loading = false;
    render();
  }
}

async function requestPair(deviceId) {
  await invoke("request_pair", { deviceId });
  state.activeTab = "pair";
  await refresh();
}

async function approvePair(deviceId) {
  const shortCode = state.pairingCodeInput.trim();
  if (shortCode.length !== 6) {
    state.pairingCodeError = "请输入 6 位数字配对码";
    render();
    return;
  }
  try {
    await invoke("approve_pair", { deviceId, shortCode });
    state.pairingCodeInput = "";
    state.pairingCodeError = null;
    await refresh();
  } catch (error) {
    state.pairingCodeError = String(error);
    render();
  }
}

async function rejectPair(deviceId) {
  await invoke("reject_pair", { deviceId });
  state.pairingCodeInput = "";
  state.pairingCodeError = null;
  await refresh();
}

async function removeTrusted(deviceId) {
  await invoke("remove_trusted_device", { deviceId });
  await refresh();
}

async function setSyncEnabled(enabled) {
  await invoke("set_sync_enabled", { enabled });
  await refresh();
}

async function saveDeviceName() {
  const deviceName = state.deviceNameDraft.trim();
  if (!deviceName) {
    state.deviceNameError = "设备名称不能为空";
    render();
    return;
  }
  try {
    await invoke("update_device_name", { deviceName });
    state.deviceNameError = null;
    state.deviceNameDraft = deviceName;
    await refresh();
  } catch (error) {
    state.deviceNameError = String(error);
    render();
  }
}

async function clearHistory() {
  await invoke("clear_history");
  await refresh();
}

async function saveSyncPreferences(nextPrefs) {
  const snapshot = state.snapshot;
  if (!snapshot) return;
  try {
    await invoke("update_sync_preferences", {
      syncHtml: nextPrefs.syncHtml ?? snapshot.sync_html_enabled,
      syncImages: nextPrefs.syncImages ?? snapshot.sync_images_enabled,
      syncFiles: nextPrefs.syncFiles ?? snapshot.sync_files_enabled
    });
    state.syncPrefsError = null;
    await refresh();
  } catch (error) {
    state.syncPrefsError = String(error);
    render();
  }
}

function renderLoading() {
  app.innerHTML = `
    <main class="app-shell loading-shell">
      <section class="loading-card surface">
        <div class="brand-chip">
          <span class="brand-chip-mark">U</span>
          <span>UniPaste</span>
        </div>
        <h1>正在加载工作台</h1>
        <p>初始化设备状态、局域网发现与剪贴板服务。</p>
      </section>
    </main>
  `;
}

function renderNav() {
  return `
    <aside class="sidebar surface">
      <div class="sidebar-top">
        <div class="brand-lockup">
          <div class="brand-mark">U</div>
          <div>
            <p class="brand-title">UniPaste</p>
            <p class="brand-subtitle">Cross-device clipboard</p>
          </div>
        </div>
        <nav class="nav-list">
          ${NAV_ITEMS.map(
            (item) => `
              <button class="nav-item ${state.activeTab === item.id ? "active" : ""}" data-nav="${item.id}">
                <span class="nav-icon">${icon(item.icon)}</span>
                <span>${item.label}</span>
              </button>
            `
          ).join("")}
        </nav>
      </div>
      <div class="sidebar-foot">
        <div class="mini-status">
          <span class="mini-dot ${state.snapshot?.sync_enabled ? "online" : "paused"}"></span>
          <span>${state.snapshot?.sync_enabled ? "同步已开启" : "同步已暂停"}</span>
        </div>
      </div>
    </aside>
  `;
}

function renderHome(snapshot) {
  const onlineCount = snapshot.discovered_devices.length;
  const trustedCount = snapshot.trusted_devices.length;
  const pendingCount = snapshot.pending_pairs.length;
  const latestHistory = snapshot.history_entries.at(-1);

  return `
    <section class="content-stack">
      <header class="page-hero surface tinted">
        <div>
          <p class="eyebrow">主页</p>
          <h1>${escapeHtml(snapshot.local_device.device_name)}</h1>
          <p class="hero-copy">查看设备配对、局域网连接与剪贴板同步健康状态。</p>
        </div>
        <div class="hero-meta">
          <div class="hero-pill">
            <span class="hero-pill-icon">${icon("shield")}</span>
            <div>
              <strong>${escapeHtml(snapshot.local_device.fingerprint)}</strong>
              <span>本机指纹</span>
            </div>
          </div>
        </div>
      </header>

      <section class="stats-grid">
        <article class="metric surface">
          <div class="metric-icon">${icon("network")}</div>
          <p class="metric-label">已发现设备</p>
          <strong>${onlineCount}</strong>
        </article>
        <article class="metric surface">
          <div class="metric-icon">${icon("devices")}</div>
          <p class="metric-label">受信设备</p>
          <strong>${trustedCount}</strong>
        </article>
        <article class="metric surface">
          <div class="metric-icon">${icon("shield")}</div>
          <p class="metric-label">待确认配对</p>
          <strong>${pendingCount}</strong>
        </article>
        <article class="metric surface">
          <div class="metric-icon">${icon("sync")}</div>
          <p class="metric-label">同步状态</p>
          <strong>${networkStatusText(snapshot.network_status)}</strong>
        </article>
      </section>

      <section class="panel-grid">
        <article class="surface panel">
          <div class="section-head">
            <h2>近期设备</h2>
            <button class="text-button" data-nav-jump="pair">查看全部</button>
          </div>
          <div class="stack">
            ${
              snapshot.discovered_devices.slice(0, 3).map(
                (device) => `
                  <article class="list-tile">
                    <div>
                      <strong>${escapeHtml(device.device_name)}</strong>
                      <p>${escapeHtml(device.address)} · ${device.connected ? "已连接" : "待连接"}</p>
                    </div>
                    <span class="state-chip ${device.trusted ? "positive" : ""}">
                      ${device.trusted ? "已信任" : "未信任"}
                    </span>
                  </article>
                `
              ).join("") || '<p class="empty">还没有发现其他设备。</p>'
            }
          </div>
        </article>

        <article class="surface panel">
          <div class="section-head">
            <h2>最近动态</h2>
            <button class="text-button" data-nav-jump="history">打开历史</button>
          </div>
          ${
            latestHistory
              ? `
                <article class="highlight-log">
                  <time>${formatTime(latestHistory.timestamp_ms)}</time>
                  <p>${escapeHtml(latestHistory.device_name)} · ${escapeHtml(latestHistory.content_kind)} · ${escapeHtml(latestHistory.preview)}</p>
                </article>
              `
              : '<p class="empty">还没有同步记录。</p>'
          }
        </article>
      </section>
    </section>
  `;
}

function renderPair(snapshot) {
  const discovered = snapshot.discovered_devices
    .map((device) => {
      let action = `<button data-action="pair" data-id="${device.device_id}">发起配对</button>`;
      if (device.trusted) {
        action = `<button class="tonal-button" data-action="remove-trusted" data-id="${device.device_id}">移除信任</button>`;
      } else if (device.pending_direction === "outbound") {
        action = `<button class="tonal-button" disabled>等待对端确认</button>`;
      } else if (device.pending_direction === "inbound") {
        action = `<button class="tonal-button" disabled>等待本机确认</button>`;
      }

      return `
        <article class="device-card surface">
          <div class="device-card-top">
            <div class="device-avatar">${escapeHtml(device.device_name.slice(0, 1).toUpperCase())}</div>
            <div>
              <strong>${escapeHtml(device.device_name)}</strong>
              <p>${escapeHtml(device.address)} · QUIC ${device.quic_port}</p>
            </div>
          </div>
          <p class="muted">指纹 ${escapeHtml(device.fingerprint)}</p>
          <p class="muted">最后出现 ${formatTime(device.last_seen_ms)}</p>
          <div class="device-card-actions">${action}</div>
        </article>
      `;
    })
    .join("");

  const pending = snapshot.pending_pairs
    .map(
      (pair) => `
        <article class="pair-request surface">
          <div>
            <p class="eyebrow small">${pair.direction === "inbound" ? "入站请求" : "出站请求"}</p>
            <h3>${escapeHtml(pair.device_name)}</h3>
            <p class="muted">短码 ${escapeHtml(pair.short_code)} · 指纹 ${escapeHtml(pair.fingerprint)}</p>
            <p class="muted">将于 ${formatTime(pair.expires_at_ms)} 过期</p>
          </div>
              ${
            pair.direction === "inbound"
              ? `
                <div class="actions">
                  <button data-action="focus-pair-modal" data-id="${pair.device_id}">输入配对码</button>
                  <button class="tonal-button" data-action="reject" data-id="${pair.device_id}">拒绝</button>
                </div>
              `
              : '<span class="state-chip">等待对端核对短码</span>'
          }
        </article>
      `
    )
    .join("");

  const trusted = snapshot.trusted_devices
    .map(
      (device) => `
        <article class="trusted-row">
          <div>
            <strong>${escapeHtml(device.device_name)}</strong>
            <p class="muted">${escapeHtml(device.fingerprint)}</p>
          </div>
          <button class="text-button danger-text" data-action="remove-trusted" data-id="${device.device_id}">移除</button>
        </article>
      `
    )
    .join("");

  return `
    <section class="content-stack">
      <header class="page-header">
        <div>
          <p class="eyebrow">配对</p>
          <h1>设备发现与信任</h1>
        </div>
        <button class="tonal-button" data-action="refresh">刷新设备</button>
      </header>

      <section class="panel-grid">
        <article class="surface panel">
          <div class="section-head">
            <h2>可发现设备</h2>
          </div>
          <div class="device-grid">
            ${discovered || '<p class="empty">当前没有发现其他在线设备。</p>'}
          </div>
        </article>

        <article class="surface panel">
          <div class="section-head">
            <h2>待确认请求</h2>
          </div>
          <div class="stack">
            ${pending || '<p class="empty">没有待处理配对。</p>'}
          </div>
        </article>
      </section>

      <article class="surface panel">
        <div class="section-head">
          <h2>受信设备</h2>
        </div>
        <div class="stack">
          ${trusted || '<p class="empty">还没有受信设备。</p>'}
        </div>
      </article>
    </section>
  `;
}

function renderHistory(snapshot) {
  return `
    <section class="content-stack">
      <header class="page-header">
        <div>
          <p class="eyebrow">历史</p>
          <h1>同步活动</h1>
        </div>
      </header>
      <article class="surface panel">
        <div class="history-grid">
          ${
            snapshot.history_entries
              .slice()
              .reverse()
              .map(
                (entry) => `
                  <article class="history-card">
                    <div class="history-card-top">
                      <span class="history-kind-icon">${icon(historyKindIcon(entry.content_kind))}</span>
                      <span class="state-chip ${entry.direction === "received" ? "positive" : ""}">
                        ${entry.direction === "received" ? "已接收" : "已发送"}
                      </span>
                    </div>
                    <h3>${escapeHtml(entry.content_kind)}</h3>
                    <p>${escapeHtml(entry.preview)}</p>
                    <div class="history-meta">
                      <span>${escapeHtml(entry.device_name)}</span>
                      <time>${formatTime(entry.timestamp_ms)}</time>
                    </div>
                  </article>
                `
              )
              .join("") || '<p class="empty">还没有历史记录。</p>'
          }
        </div>
      </article>
    </section>
  `;
}

function renderSettings(snapshot) {
  return `
    <section class="content-stack">
      <header class="page-header">
        <div>
          <p class="eyebrow">设置</p>
          <h1>同步偏好</h1>
        </div>
      </header>
      <section class="panel-grid">
        <article class="surface panel">
          <div class="settings-stack">
            <div class="setting-row align-top">
              <div>
                <h2>设备名称</h2>
                <p>这个名称会出现在局域网发现、配对请求和同步历史里。</p>
              </div>
            </div>
            <div class="device-name-row">
              <input
                id="deviceNameInput"
                class="text-input"
                maxlength="32"
                value="${escapeHtml(state.deviceNameDraft || snapshot.local_device.device_name)}"
                placeholder="输入设备名称"
              />
              <button data-action="save-device-name">保存名称</button>
            </div>
            ${
              state.deviceNameError
                ? `<p class="pairing-error">${escapeHtml(state.deviceNameError)}</p>`
                : ""
            }
          </div>
        </article>

        <article class="surface panel">
          <div class="setting-row">
            <div>
              <h2>启用剪贴板同步</h2>
              <p>控制文本、HTML 和图片在受信设备之间自动传递。</p>
            </div>
            <label class="md-switch">
              <input id="syncToggle" type="checkbox" ${snapshot.sync_enabled ? "checked" : ""} />
              <span class="md-switch-track"><span class="md-switch-thumb"></span></span>
            </label>
          </div>
        </article>

        <article class="surface panel">
          <div class="settings-stack">
            <div class="setting-row">
              <div>
                <h2>内容同步范围</h2>
                <p>文本始终开启，其他内容可以按需控制，避免不必要的流量和剪贴板干扰。</p>
              </div>
            </div>
            <div class="setting-row">
              <div>
                <h3>文本</h3>
                <p>始终同步</p>
              </div>
              <span class="state-chip positive">固定开启</span>
            </div>
            <div class="setting-row">
              <div>
                <h3>HTML 富文本</h3>
                <p>保留富文本结构和纯文本后备内容。</p>
              </div>
              <label class="md-switch">
                <input id="htmlToggle" type="checkbox" ${snapshot.sync_html_enabled ? "checked" : ""} />
                <span class="md-switch-track"><span class="md-switch-thumb"></span></span>
              </label>
            </div>
            <div class="setting-row">
              <div>
                <h3>图片</h3>
                <p>通过压缩后的 PNG 进行传输，减小局域网开销。</p>
              </div>
              <label class="md-switch">
                <input id="imagesToggle" type="checkbox" ${snapshot.sync_images_enabled ? "checked" : ""} />
                <span class="md-switch-track"><span class="md-switch-thumb"></span></span>
              </label>
            </div>
            <div class="setting-row">
              <div>
                <h3>文件</h3>
                <p>复制文件后会把实际文件内容一起传输到对端临时目录。</p>
              </div>
              <label class="md-switch">
                <input id="filesToggle" type="checkbox" ${snapshot.sync_files_enabled ? "checked" : ""} />
                <span class="md-switch-track"><span class="md-switch-thumb"></span></span>
              </label>
            </div>
            ${
              state.syncPrefsError
                ? `<p class="pairing-error">${escapeHtml(state.syncPrefsError)}</p>`
                : ""
            }
          </div>
        </article>

        <article class="surface panel">
          <div class="info-list">
            <div class="info-row">
              <span>设备 ID</span>
              <strong>${escapeHtml(snapshot.local_device.device_id)}</strong>
            </div>
            <div class="info-row">
              <span>本机指纹</span>
              <strong>${escapeHtml(snapshot.local_device.fingerprint)}</strong>
            </div>
            <div class="info-row">
              <span>同步方式</span>
              <strong>mDNS + QUIC</strong>
            </div>
            <div class="info-row">
              <span>网络状态</span>
              <strong>${escapeHtml(networkStatusText(snapshot.network_status))}</strong>
            </div>
            <div class="info-row">
              <span>监听端口</span>
              <strong>${snapshot.quic_port || "-"}</strong>
            </div>
            <div class="info-row">
              <span>历史记录</span>
              <strong>${snapshot.history_entries.length} 条</strong>
            </div>
            <div class="info-row">
              <span>最近错误</span>
              <strong>${escapeHtml(snapshot.last_error ?? "无")}</strong>
            </div>
          </div>
          <div class="settings-actions">
            <button class="tonal-button" data-action="clear-history">清空历史</button>
          </div>
        </article>
      </section>
    </section>
  `;
}

function renderAbout(snapshot) {
  return `
    <section class="content-stack">
      <header class="page-header">
        <div>
          <p class="eyebrow">关于</p>
          <h1>UniPaste 工作台</h1>
        </div>
      </header>
      <section class="panel-grid">
        <article class="surface panel about-panel">
          <div class="about-hero">
            <div class="about-logo">U</div>
            <div>
              <h2>跨设备剪贴板工作台</h2>
              <p>用于在同一局域网下安全同步文本、HTML、图片和文件剪贴板内容。</p>
            </div>
          </div>
        </article>
        <article class="surface panel">
          <div class="info-list">
            <div class="info-row">
              <span>当前发现设备</span>
              <strong>${snapshot.discovered_devices.length}</strong>
            </div>
            <div class="info-row">
              <span>受信设备</span>
              <strong>${snapshot.trusted_devices.length}</strong>
            </div>
            <div class="info-row">
              <span>同步记录</span>
              <strong>${snapshot.logs.length}</strong>
            </div>
          </div>
        </article>
      </section>
    </section>
  `;
}

function renderContent(snapshot) {
  if (state.activeTab === "pair") return renderPair(snapshot);
  if (state.activeTab === "history") return renderHistory(snapshot);
  if (state.activeTab === "settings") return renderSettings(snapshot);
  if (state.activeTab === "about") return renderAbout(snapshot);
  return renderHome(snapshot);
}

function render() {
  if (state.loading && !state.snapshot) {
    renderLoading();
    return;
  }

  const snapshot = state.snapshot;
  if (!snapshot) {
    app.innerHTML = `
      <main class="app-shell loading-shell">
        <section class="loading-card surface error-card">
          <h1>无法读取应用状态</h1>
          <p>${escapeHtml(state.error ?? "未知错误")}</p>
        </section>
      </main>
    `;
    return;
  }

  const inboundPair = snapshot.pending_pairs.find((pair) => pair.direction === "inbound") ?? null;
  const pairingDigits = getPairingDigits();

  app.innerHTML = `
    <main class="app-shell">
      ${renderNav()}
      <section class="workspace">
        <header class="topbar">
          <div>
            <p class="topbar-kicker">Workspace</p>
            <h1>${escapeHtml(NAV_ITEMS.find((item) => item.id === state.activeTab)?.label ?? "主页")}</h1>
          </div>
          <div class="topbar-actions">
            <button class="tonal-button" data-action="refresh">立即刷新</button>
          </div>
        </header>
        ${state.error ? `<section class="inline-error surface"><p>${escapeHtml(state.error)}</p></section>` : ""}
        ${renderContent(snapshot)}
      </section>
    </main>
    ${
      inboundPair
        ? `
          <div class="modal-backdrop">
            <div class="modal surface">
              <p class="eyebrow">配对确认</p>
              <h2>${escapeHtml(inboundPair.device_name)}</h2>
              <p>请查看另一台设备上显示的 6 位配对码，并在这里输入完全一致的数字后再确认。</p>
              <div class="code-block">${escapeHtml(inboundPair.short_code)}</div>
              <p class="muted">远端指纹 ${escapeHtml(inboundPair.fingerprint)}</p>
              <div class="pairing-input-group">
                <span>输入 6 位配对码</span>
                <div class="pairing-code-grid">
                  ${pairingDigits
                    .map(
                      (digit, index) => `
                        <input
                          class="pairing-digit"
                          data-code-digit="${index}"
                          inputmode="numeric"
                          maxlength="1"
                          value="${escapeHtml(digit)}"
                        />
                      `
                    )
                    .join("")}
                </div>
              </div>
              ${
                state.pairingCodeError
                  ? `<p class="pairing-error">${escapeHtml(state.pairingCodeError)}</p>`
                  : ""
              }
              <div class="modal-actions">
                <button data-action="approve" data-id="${inboundPair.device_id}" ${
                  state.pairingCodeInput.length === 6 ? "" : "disabled"
                }>确认配对</button>
                <button class="tonal-button" data-action="reject" data-id="${inboundPair.device_id}">拒绝</button>
              </div>
            </div>
          </div>
        `
        : ""
    }
  `;

  document.querySelectorAll("[data-nav]").forEach((element) => {
    element.addEventListener("click", () => {
      state.activeTab = element.dataset.nav;
      state.pairingCodeError = null;
      state.pairingCodeInput = "";
      state.deviceNameError = null;
      state.syncPrefsError = null;
      render();
    });
  });

  document.querySelectorAll("[data-nav-jump]").forEach((element) => {
    element.addEventListener("click", () => {
      state.activeTab = element.dataset.navJump;
      state.pairingCodeError = null;
      state.pairingCodeInput = "";
      state.deviceNameError = null;
      state.syncPrefsError = null;
      render();
    });
  });

  document.querySelector("#deviceNameInput")?.addEventListener("input", (event) => {
    state.deviceNameDraft = event.target.value;
    state.deviceNameError = null;
  });

  document.querySelectorAll("[data-code-digit]").forEach((element) => {
    element.addEventListener("input", (event) => {
      const index = Number(event.target.dataset.codeDigit);
      const value = event.target.value.replace(/\D/g, "").slice(-1);
      const digits = getPairingDigits();
      digits[index] = value;
      state.pairingCodeInput = digits.join("").slice(0, 6);
      state.pairingCodeError = null;
      event.target.value = value;
      if (value && index < 5) {
        document.querySelector(`[data-code-digit="${index + 1}"]`)?.focus();
      }
      render();
      if (value && index < 5) {
        document.querySelector(`[data-code-digit="${index + 1}"]`)?.focus();
      }
    });

    element.addEventListener("keydown", (event) => {
      const index = Number(event.target.dataset.codeDigit);
      if (event.key === "Backspace" && !event.target.value && index > 0) {
        const digits = getPairingDigits();
        digits[index - 1] = "";
        state.pairingCodeInput = digits.join("");
        state.pairingCodeError = null;
        render();
        document.querySelector(`[data-code-digit="${index - 1}"]`)?.focus();
      }
      if (event.key === "ArrowLeft" && index > 0) {
        event.preventDefault();
        document.querySelector(`[data-code-digit="${index - 1}"]`)?.focus();
      }
      if (event.key === "ArrowRight" && index < 5) {
        event.preventDefault();
        document.querySelector(`[data-code-digit="${index + 1}"]`)?.focus();
      }
    });
  });

  document.querySelector("#syncToggle")?.addEventListener("change", (event) => {
    setSyncEnabled(event.target.checked);
  });

  document.querySelectorAll("[data-action='pair']").forEach((element) => {
    element.addEventListener("click", () => requestPair(element.dataset.id));
  });

  document.querySelectorAll("[data-action='approve']").forEach((element) => {
    element.addEventListener("click", () => approvePair(element.dataset.id));
  });

  document.querySelectorAll("[data-action='focus-pair-modal']").forEach((element) => {
    element.addEventListener("click", () => {
      state.pairingCodeError = null;
      render();
      document.querySelector('[data-code-digit="0"]')?.focus();
    });
  });

  document.querySelectorAll("[data-action='reject']").forEach((element) => {
    element.addEventListener("click", () => rejectPair(element.dataset.id));
  });

  document.querySelectorAll("[data-action='remove-trusted']").forEach((element) => {
    element.addEventListener("click", () => removeTrusted(element.dataset.id));
  });

  document.querySelectorAll("[data-action='refresh']").forEach((element) => {
    element.addEventListener("click", refresh);
  });

  document.querySelectorAll("[data-action='save-device-name']").forEach((element) => {
    element.addEventListener("click", saveDeviceName);
  });

  document.querySelectorAll("[data-action='clear-history']").forEach((element) => {
    element.addEventListener("click", clearHistory);
  });

  document.querySelector("#htmlToggle")?.addEventListener("change", (event) => {
    saveSyncPreferences({ syncHtml: event.target.checked });
  });

  document.querySelector("#imagesToggle")?.addEventListener("change", (event) => {
    saveSyncPreferences({ syncImages: event.target.checked });
  });

  document.querySelector("#filesToggle")?.addEventListener("change", (event) => {
    saveSyncPreferences({ syncFiles: event.target.checked });
  });
}

refresh();
setInterval(refresh, 2500);
