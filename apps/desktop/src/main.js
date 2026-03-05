const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;

if (!invoke) {
  throw new Error("Tauri API is not available. This UI must run inside DropLocal Desktop.");
}

const refs = {
  serverState: document.getElementById("serverState"),
  deviceCount: document.getElementById("deviceCount"),
  serverPort: document.getElementById("serverPort"),
  uptime: document.getElementById("uptime"),
  primaryUrl: document.getElementById("primaryUrl"),
  copyUrlBtn: document.getElementById("copyUrlBtn"),
  openBrowserBtn: document.getElementById("openBrowserBtn"),
  qrCode: document.getElementById("qrCode"),
  toggleServerBtn: document.getElementById("toggleServerBtn"),
  refreshBtn: document.getElementById("refreshBtn"),
  runtimeMessage: document.getElementById("runtimeMessage"),
  settingsForm: document.getElementById("settingsForm"),
  portInput: document.getElementById("portInput"),
  storageDirInput: document.getElementById("storageDirInput"),
  showQrInput: document.getElementById("showQrInput"),
  autoCleanInput: document.getElementById("autoCleanInput"),
  notifyDeviceInput: document.getElementById("notifyDeviceInput")
};

const state = {
  runtime: null,
  pollTimer: null
};

function applyQrVisibility(showQr) {
  const qrSection = document.querySelector(".qrcode");
  if (!qrSection) {
    return;
  }
  qrSection.style.display = showQr ? "" : "none";
}

function setRuntimeMessage(message) {
  refs.runtimeMessage.textContent = message || "";
}

function formatUptime(seconds) {
  const safeSeconds = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  if (safeSeconds < 60) return `${safeSeconds}s`;
  if (safeSeconds < 3600) return `${Math.floor(safeSeconds / 60)}m`;
  if (safeSeconds < 86400) return `${Math.floor(safeSeconds / 3600)}h`;
  return `${Math.floor(safeSeconds / 86400)}d`;
}

async function renderQr(url) {
  refs.qrCode.innerHTML = "";
  if (!url) {
    refs.qrCode.textContent = "Server is stopped.";
    return;
  }

  try {
    const svg = await invoke("build_qr_svg", { payload: url });
    refs.qrCode.innerHTML = svg;
  } catch (error) {
    refs.qrCode.textContent = "QR unavailable";
    console.error(error);
  }
}

function renderRuntime(runtime) {
  state.runtime = runtime;
  const running = Boolean(runtime?.running);
  const deviceCount = running ? runtime.connectedDevices : 0;

  refs.serverState.textContent = running ? "Running" : "Stopped";
  refs.deviceCount.textContent = String(deviceCount);
  refs.serverPort.textContent = running ? String(runtime.port) : "-";
  refs.uptime.textContent = running ? formatUptime(runtime.uptimeSeconds) : "-";
  refs.primaryUrl.value = running ? runtime.primaryUrl : "";

  refs.toggleServerBtn.textContent = running ? "Stop Server" : "Start Server";
  refs.toggleServerBtn.classList.toggle("danger", running);
  refs.toggleServerBtn.classList.toggle("primary", !running);

  void renderQr(running ? runtime.primaryUrl : "");
}

async function refreshRuntime() {
  try {
    const runtime = await invoke("get_runtime_status");
    renderRuntime(runtime);
    setRuntimeMessage("");
  } catch (error) {
    setRuntimeMessage(`Runtime check failed: ${String(error)}`);
  }
}

async function toggleServer() {
  refs.toggleServerBtn.disabled = true;
  try {
    const running = Boolean(state.runtime?.running);
    const runtime = running ? await invoke("stop_server") : await invoke("start_server");
    renderRuntime(runtime);
    setRuntimeMessage(running ? "Server stopped." : "Server started.");
  } catch (error) {
    setRuntimeMessage(`Server action failed: ${String(error)}`);
  } finally {
    refs.toggleServerBtn.disabled = false;
  }
}

async function copyUrl() {
  try {
    await invoke("copy_share_url");
    setRuntimeMessage("Share URL copied to clipboard.");
  } catch (error) {
    setRuntimeMessage(`Copy failed: ${String(error)}`);
  }
}

async function openBrowser() {
  try {
    await invoke("open_share_url");
    setRuntimeMessage("Opened DropLocal in your default browser.");
  } catch (error) {
    setRuntimeMessage(`Open failed: ${String(error)}`);
  }
}

async function loadSettings() {
  try {
    const settings = await invoke("get_settings");
    refs.portInput.value = String(settings.port);
    refs.storageDirInput.value = settings.storageDir;
    refs.showQrInput.checked = Boolean(settings.showQrInTray);
    refs.autoCleanInput.checked = Boolean(settings.autoCleanOnQuit);
    refs.notifyDeviceInput.checked = Boolean(settings.notifyOnDeviceConnect);
    applyQrVisibility(refs.showQrInput.checked);
  } catch (error) {
    setRuntimeMessage(`Failed to load settings: ${String(error)}`);
  }
}

async function saveSettings(event) {
  event.preventDefault();

  const nextSettings = {
    port: Number.parseInt(refs.portInput.value, 10),
    storageDir: refs.storageDirInput.value.trim(),
    showQrInTray: refs.showQrInput.checked,
    autoCleanOnQuit: refs.autoCleanInput.checked,
    notifyOnDeviceConnect: refs.notifyDeviceInput.checked
  };

  refs.settingsForm.querySelector("button[type='submit']").disabled = true;
  try {
    await invoke("save_settings", { settings: nextSettings });
    const runtime = await invoke("restart_server_with_settings");
    renderRuntime(runtime);
    setRuntimeMessage("Settings saved and server restarted.");
  } catch (error) {
    setRuntimeMessage(`Save failed: ${String(error)}`);
  } finally {
    refs.settingsForm.querySelector("button[type='submit']").disabled = false;
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
  refs.toggleServerBtn.addEventListener("click", () => {
    void toggleServer();
  });
  refs.copyUrlBtn.addEventListener("click", () => {
    void copyUrl();
  });
  refs.openBrowserBtn.addEventListener("click", () => {
    void openBrowser();
  });
  refs.refreshBtn.addEventListener("click", () => {
    void refreshRuntime();
  });
  refs.settingsForm.addEventListener("submit", (event) => {
    void saveSettings(event);
  });
  refs.showQrInput.addEventListener("change", () => {
    applyQrVisibility(refs.showQrInput.checked);
  });

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
