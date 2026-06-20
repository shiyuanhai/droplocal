const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;

if (!invoke) {
  throw new Error("Tauri API is not available. This UI must run inside DropLocal Desktop.");
}

const I18N = JSON.parse(document.getElementById("i18n-data").textContent);
const SUPPORTED_LANGS = ["en", "zh", "ja", "ko", "es", "de", "fr"];

const refs = {
  langSelect: document.getElementById("langSelect"),
  settingsBtn: document.getElementById("settingsBtn"),
  statusPill: document.getElementById("statusPill"),
  statusWord: document.getElementById("statusWord"),
  statusMeta: document.getElementById("statusMeta"),
  statusLine: document.getElementById("statusLine"),
  primaryUrl: document.getElementById("primaryUrl"),
  copyUrlBtn: document.getElementById("copyUrlBtn"),
  openBrowserBtn: document.getElementById("openBrowserBtn"),
  copyInviteBtn: document.getElementById("copyInviteBtn"),
  copyDebugBtn: document.getElementById("copyDebugBtn"),
  dropClipboardBtn: document.getElementById("dropClipboardBtn"),
  doctorGrid: document.getElementById("doctorGrid"),
  doctorWarning: document.getElementById("doctorWarning"),
  qrCode: document.getElementById("qrCode"),
  toggleServerBtn: document.getElementById("toggleServerBtn"),
  toast: document.getElementById("toast"),
  settingsModal: document.getElementById("settingsModal"),
  settingsClose: document.getElementById("settingsClose"),
  settingsCancel: document.getElementById("settingsCancel"),
  settingsForm: document.getElementById("settingsForm"),
  portInput: document.getElementById("portInput"),
  storageDirInput: document.getElementById("storageDirInput"),
  pinInput: document.getElementById("pinInput"),
  networkInterfaceInput: document.getElementById("networkInterfaceInput"),
  expireInput: document.getElementById("expireInput"),
  showQrInput: document.getElementById("showQrInput"),
  autoCleanInput: document.getElementById("autoCleanInput"),
  notifyDropInput: document.getElementById("notifyDropInput"),
  notifyDeviceInput: document.getElementById("notifyDeviceInput"),
  autostartInput: document.getElementById("autostartInput"),
  dockIconRow: document.getElementById("dockIconRow"),
  dockIconInput: document.getElementById("dockIconInput")
};

// The Dock-icon toggle only means something on macOS.
const IS_MAC = navigator.userAgent.includes("Mac");

const state = {
  runtime: null,
  settings: null,
  pollTimer: null,
  toastTimer: 0,
  lang: "en"
};

function detectLang() {
  try {
    const stored = localStorage.getItem("droplocal-lang");
    if (stored && SUPPORTED_LANGS.includes(stored)) {
      return stored;
    }
  } catch (_error) {}
  const nav = (navigator.language || "en").toLowerCase();
  if (nav.startsWith("zh")) return "zh";
  if (nav.startsWith("ja")) return "ja";
  if (nav.startsWith("ko")) return "ko";
  if (nav.startsWith("es")) return "es";
  if (nav.startsWith("de")) return "de";
  if (nav.startsWith("fr")) return "fr";
  return "en";
}

function t(key, params) {
  const table = I18N[state.lang] || I18N.en;
  let value = table[key] || I18N.en[key] || key;
  if (params) {
    for (const [name, replacement] of Object.entries(params)) {
      value = value.replace(`{${name}}`, String(replacement));
    }
  }
  return value;
}

function applyLang(lang) {
  state.lang = lang;
  try {
    localStorage.setItem("droplocal-lang", lang);
  } catch (_error) {}
  document.documentElement.lang = lang === "zh" ? "zh-Hans" : lang;
  refs.langSelect.value = lang;

  for (const node of document.querySelectorAll("[data-i18n]")) {
    node.textContent = t(node.getAttribute("data-i18n"));
  }

  if (state.runtime) {
    renderRuntime(state.runtime);
  } else {
    refs.statusWord.textContent = t("status.starting");
  }
}

function showToast(message) {
  refs.toast.textContent = message;
  refs.toast.classList.add("show");
  window.clearTimeout(state.toastTimer);
  state.toastTimer = window.setTimeout(() => {
    refs.toast.classList.remove("show");
  }, 2400);
}

async function renderQr(url) {
  refs.qrCode.innerHTML = "";
  if (!url) {
    refs.qrCode.textContent = t("qr.stopped");
    return;
  }

  try {
    const svg = await invoke("build_qr_svg", { payload: url });
    refs.qrCode.innerHTML = svg;
  } catch (error) {
    refs.qrCode.textContent = t("qr.unavailable");
    console.error(error);
  }
}

function renderRuntime(runtime) {
  state.runtime = runtime;
  const running = Boolean(runtime?.running);

  if (running) {
    const count = runtime.connectedDevices || 0;
    refs.statusPill.dataset.state = "running";
    refs.statusWord.textContent = t("status.running");
    // Friendly reassurance only — the techy port/uptime/IP details live in
    // Settings → Connection details now.
    refs.statusMeta.textContent = count > 0
      ? "· " + (count === 1 ? t("stat.deviceOne") : t("stat.deviceOther", { count }))
      : "";
    refs.statusLine.textContent = t("connect.scan");
    refs.primaryUrl.value = runtime.friendlyUrl || runtime.primaryUrl;
  } else {
    refs.statusPill.dataset.state = "stopped";
    refs.statusWord.textContent = t("status.stopped");
    refs.statusMeta.textContent = "";
    refs.statusLine.textContent = t("stat.offline");
    refs.primaryUrl.value = "";
  }

  refs.toggleServerBtn.textContent = running ? t("control.stop") : t("control.start");
  refs.toggleServerBtn.classList.toggle("danger", running);
  refs.toggleServerBtn.classList.toggle("primary", !running);

  refs.copyUrlBtn.disabled = !running;
  refs.openBrowserBtn.disabled = !running;
  refs.copyInviteBtn.disabled = !running;
  refs.copyDebugBtn.disabled = !running;
  refs.dropClipboardBtn.disabled = !running;
  renderDoctor(runtime);

  void renderQr(running ? runtime.primaryUrl : "");
}

function doctorRows(runtime) {
  const local = runtime.reachability?.ok ? t("doctor.ok") : t("doctor.failed");
  return [
    [t("doctor.interface"), runtime.selectedInterface || "-"],
    [t("doctor.primary"), runtime.primaryUrl || "-"],
    [t("doctor.friendly"), runtime.friendlyUrl || "-"],
    [t("doctor.local"), local],
    [t("doctor.pin"), runtime.pinEnabled ? t("doctor.on") : t("doctor.off")]
  ];
}

function renderDoctor(runtime) {
  refs.doctorGrid.textContent = "";
  if (!runtime?.running) {
    refs.doctorWarning.hidden = true;
    return;
  }

  doctorRows(runtime).forEach(([labelText, valueText]) => {
    const row = document.createElement("div");
    row.className = "doctor-row";
    const label = document.createElement("span");
    label.className = "doctor-label";
    label.textContent = labelText;
    const value = document.createElement("span");
    value.className = "doctor-value";
    value.textContent = valueText;
    row.append(label, value);
    refs.doctorGrid.appendChild(row);
  });

  const selected = (runtime.networkInterfaces || []).find((entry) => entry.selected);
  const warnings = [];
  if (runtime.preferredInterface && !runtime.preferredFound) {
    warnings.push(`Preferred interface "${runtime.preferredInterface}" was not found.`);
  }
  if (selected?.virtual) {
    warnings.push("Selected interface looks virtual or VPN-backed.");
  }
  refs.doctorWarning.hidden = warnings.length === 0;
  refs.doctorWarning.textContent = warnings[0] || "";
}

async function refreshRuntime() {
  try {
    const runtime = await invoke("get_runtime_status");
    renderRuntime(runtime);
  } catch (error) {
    showToast(t("msg.statusFailed", { error: String(error) }));
  }
}

async function toggleServer() {
  refs.toggleServerBtn.disabled = true;
  try {
    const running = Boolean(state.runtime?.running);
    const runtime = running ? await invoke("stop_server") : await invoke("start_server");
    renderRuntime(runtime);
    showToast(running ? t("msg.stopped") : t("msg.started"));
  } catch (error) {
    showToast(t("msg.actionFailed", { error: String(error) }));
  } finally {
    refs.toggleServerBtn.disabled = false;
  }
}

async function copyUrl() {
  try {
    await invoke("copy_share_url");
    showToast(t("msg.copied"));
  } catch (error) {
    showToast(t("msg.copyFailed", { error: String(error) }));
  }
}

async function openBrowser() {
  try {
    await invoke("open_share_url");
    showToast(t("msg.opened"));
  } catch (error) {
    showToast(t("msg.openFailed", { error: String(error) }));
  }
}

async function copyInvite() {
  try {
    await invoke("copy_invite_link");
    showToast(t("doctor.inviteCopied"));
  } catch (error) {
    showToast(t("msg.copyFailed", { error: String(error) }));
  }
}

async function copyDebug() {
  try {
    await invoke("copy_debug_info");
    showToast(t("doctor.debugCopied"));
  } catch (error) {
    showToast(t("msg.copyFailed", { error: String(error) }));
  }
}

async function dropClipboard() {
  refs.dropClipboardBtn.disabled = true;
  try {
    await invoke("drop_clipboard");
    showToast(t("msg.clipboardDropped"));
  } catch (error) {
    showToast(t("msg.clipboardDropFailed", { error: String(error) }));
  } finally {
    refs.dropClipboardBtn.disabled = !state.runtime?.running;
  }
}

/* ---------- settings modal ---------- */

function openSettings() {
  // Refresh the diagnostics block with the latest runtime each time it opens.
  renderDoctor(state.runtime);
  refs.settingsModal.hidden = false;
  refs.portInput.focus();
}

function closeSettings() {
  refs.settingsModal.hidden = true;
}

function isEditableTarget(target) {
  if (!target || !(target instanceof HTMLElement)) {
    return false;
  }
  return Boolean(target.closest("input, textarea, select, [contenteditable='true']"));
}

async function loadSettings() {
  try {
    const settings = await invoke("get_settings");
    // Keep the whole object so saving round-trips fields the form doesn't
    // surface (showQrInTray, menuBarHintSeen, future additions).
    state.settings = settings;
    refs.portInput.value = String(settings.port);
    refs.storageDirInput.value = settings.storageDir;
    refs.pinInput.value = settings.pin || "";
    refs.networkInterfaceInput.value = settings.networkInterface || "";
    refs.expireInput.value = String(settings.expireMinutes || 0);
    // Preserved verbatim — the dashboard always shows the QR now, so this
    // setting is no longer surfaced as a checkbox but must round-trip.
    refs.showQrInput.value = settings.showQrInTray ? "1" : "";
    refs.autoCleanInput.checked = Boolean(settings.autoCleanOnQuit);
    refs.notifyDropInput.checked = Boolean(settings.notifyOnNewDrop);
    refs.notifyDeviceInput.checked = Boolean(settings.notifyOnDeviceConnect);
    refs.autostartInput.checked = Boolean(settings.launchAtLogin);
    refs.dockIconInput.checked = Boolean(settings.showDockIcon);
  } catch (error) {
    showToast(t("msg.settingsLoadFailed", { error: String(error) }));
  }
}

async function saveSettings(event) {
  event.preventDefault();

  const nextSettings = {
    ...(state.settings || {}),
    port: Number.parseInt(refs.portInput.value, 10),
    storageDir: refs.storageDirInput.value.trim(),
    pin: refs.pinInput.value.trim(),
    networkInterface: refs.networkInterfaceInput.value.trim(),
    expireMinutes: Number.parseInt(refs.expireInput.value, 10) || 0,
    showQrInTray: refs.showQrInput.value === "1",
    autoCleanOnQuit: refs.autoCleanInput.checked,
    notifyOnNewDrop: refs.notifyDropInput.checked,
    notifyOnDeviceConnect: refs.notifyDeviceInput.checked,
    launchAtLogin: refs.autostartInput.checked,
    showDockIcon: refs.dockIconInput.checked
  };

  const submit = refs.settingsForm.querySelector("button[type='submit']");
  submit.disabled = true;
  try {
    await invoke("save_settings", { settings: nextSettings });
    state.settings = nextSettings;
    const runtime = await invoke("restart_server_with_settings");
    renderRuntime(runtime);
    closeSettings();
    showToast(t("msg.saved"));
  } catch (error) {
    showToast(t("msg.saveFailed", { error: String(error) }));
  } finally {
    submit.disabled = false;
  }
}

function startRuntimePolling() {
  if (state.pollTimer) {
    window.clearInterval(state.pollTimer);
  }
  state.pollTimer = window.setInterval(() => {
    void refreshRuntime();
  }, 4000);
}

async function initialize() {
  applyLang(detectLang());
  refs.dockIconRow.hidden = !IS_MAC;

  refs.langSelect.addEventListener("change", () => applyLang(refs.langSelect.value));
  refs.toggleServerBtn.addEventListener("click", () => void toggleServer());
  refs.copyUrlBtn.addEventListener("click", () => void copyUrl());
  refs.openBrowserBtn.addEventListener("click", () => void openBrowser());
  refs.copyInviteBtn.addEventListener("click", () => void copyInvite());
  refs.copyDebugBtn.addEventListener("click", () => void copyDebug());
  refs.dropClipboardBtn.addEventListener("click", () => void dropClipboard());

  refs.settingsBtn.addEventListener("click", openSettings);
  refs.settingsClose.addEventListener("click", closeSettings);
  refs.settingsCancel.addEventListener("click", closeSettings);
  refs.settingsModal.addEventListener("click", (event) => {
    if (event.target === refs.settingsModal) {
      closeSettings();
    }
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !refs.settingsModal.hidden) {
      closeSettings();
      return;
    }
    if (
      event.key.toLowerCase() === "v" &&
      event.shiftKey &&
      (event.metaKey || event.ctrlKey) &&
      refs.settingsModal.hidden &&
      !isEditableTarget(event.target)
    ) {
      event.preventDefault();
      void dropClipboard();
    }
  });
  refs.settingsForm.addEventListener("submit", (event) => void saveSettings(event));

  if (listen) {
    await listen("droplocal://runtime-updated", (event) => {
      if (event?.payload) {
        renderRuntime(event.payload);
      }
    });
  }

  await Promise.all([loadSettings(), refreshRuntime()]);
  startRuntimePolling();
}

void initialize();
