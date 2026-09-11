// Dynamic resolution for Tauri 2 & 1 global IPC
function getTauriInvoke() {
  if (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.invoke === 'function') {
    return window.__TAURI__.core.invoke;
  }
  if (window.__TAURI__ && typeof window.__TAURI__.invoke === 'function') {
    return window.__TAURI__.invoke;
  }
  if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
    return window.__TAURI_INTERNALS__.invoke;
  }
  return async (cmd, args) => {
    console.warn(`[Tauri Mock] invoke called: ${cmd}`, args);
    return null;
  };
}

function getTauriListen() {
  if (window.__TAURI__ && window.__TAURI__.event && typeof window.__TAURI__.event.listen === 'function') {
    return window.__TAURI__.event.listen;
  }
  if (window.__TAURI__ && typeof window.__TAURI__.listen === 'function') {
    return window.__TAURI__.listen;
  }
  return () => () => {};
}

const invoke = (...args) => getTauriInvoke()(...args);
const listen = (...args) => getTauriListen()(...args);

let currentStatus = {
  is_installed: false,
  installed_version: "1.03.00",
  server_version: "1.03.00",
  runner_name: "GE-Proton11-6",
  has_gamemode: true,
  is_downloading: false,
  is_playing: false,
  is_authenticated: false,
  player_name: "",
  profile_img_url: "",
};

let currentConfig = null;

// Initialize on DOM load
document.addEventListener("DOMContentLoaded", async () => {
  setupCarousel();
  setupModals();
  setupButtons();
  await refreshStatus();
  setupTauriListeners();
});

// Refresh Launcher and Game Status from Rust Backend
async function refreshStatus() {
  try {
    const status = await invoke("get_launcher_status");
    if (status) {
      currentStatus = status;
      updateUI();
    }
  } catch (err) {
    console.error("Failed to get launcher status:", err);
  }
}

// Update UI elements based on state
function updateUI() {
  // Update Chips
  document.getElementById("version-text").textContent = `v${currentStatus.installed_version || "1.03.00"}`;
  document.getElementById("runner-text").textContent = currentStatus.runner_name || "GE-Proton";
  document.getElementById("gamemode-text").textContent = currentStatus.has_gamemode ? "GameMode ON" : "GameMode OFF";

  // Update Auth Profile & Header
  const btnHeaderAuth = document.getElementById("btn-header-auth");
  const chipHeaderProfile = document.getElementById("chip-header-profile");
  const headerUserName = document.getElementById("header-user-name");
  const headerUserAvatar = document.getElementById("header-user-avatar");

  if (currentStatus.is_authenticated) {
    if (btnHeaderAuth) btnHeaderAuth.style.display = "none";
    if (chipHeaderProfile) chipHeaderProfile.style.display = "flex";
    if (headerUserName) headerUserName.textContent = currentStatus.player_name || "Piloto";
    if (headerUserAvatar && currentStatus.profile_img_url) {
      headerUserAvatar.src = currentStatus.profile_img_url;
    }
  } else {
    if (btnHeaderAuth) btnHeaderAuth.style.display = "flex";
    if (chipHeaderProfile) chipHeaderProfile.style.display = "none";
  }

  const heroBtn = document.getElementById("btn-hero-action");
  const ctaSvg = document.getElementById("cta-svg");
  const ctaTitle = document.getElementById("cta-title");
  const ctaSubtitle = document.getElementById("cta-subtitle");
  const statusText = document.getElementById("status-text");
  const progressContainer = document.getElementById("progress-container");

  if (currentStatus.is_playing) {
    heroBtn.className = "hero-cta-btn btn-play";
    ctaSvg.innerHTML = '<path d="M8 5v14l11-7z"/>';
    ctaTitle.textContent = "JUGANDO...";
    ctaSubtitle.textContent = "Mongil: Star Dive en ejecución";
    statusText.textContent = "Juego en ejecución (GE-Proton activo)";
    progressContainer.style.display = "none";
  } else if (currentStatus.is_downloading) {
    heroBtn.className = "hero-cta-btn btn-cancel";
    ctaSvg.innerHTML = '<path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/>';
    ctaTitle.textContent = "CANCELAR DESCARGA";
    ctaSubtitle.textContent = "Pausar proceso actual";
    progressContainer.style.display = "flex";
  } else if (!currentStatus.is_installed) {
    heroBtn.className = "hero-cta-btn btn-download";
    ctaSvg.innerHTML = '<path d="M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"/>';
    ctaTitle.textContent = "DESCARGAR CLIENTE";
    ctaSubtitle.textContent = "838 MB • Descarga Directa CDN";
    statusText.textContent = "Cliente no instalado (Descarga: 838 MB)";
    progressContainer.style.display = "none";
  } else if (!currentStatus.is_authenticated) {
    heroBtn.className = "hero-cta-btn btn-play";
    ctaSvg.innerHTML = '<path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 3c1.66 0 3 1.34 3 3s-1.34 3-3 3-3-1.34-3-3 1.34-3 3-3zm0 14.2c-2.5 0-4.71-1.28-6-3.22.03-1.99 4-3.08 6-3.08 1.99 0 5.97 1.09 6 3.08-1.29 1.94-3.5 3.22-6 3.22z"/>';
    ctaTitle.textContent = "INICIAR SESIÓN";
    ctaSubtitle.textContent = "Netmarble SSO V5 requerido";
    statusText.textContent = "Inicia sesión con Google o Netmarble para jugar";
    progressContainer.style.display = "none";
  } else {
    heroBtn.className = "hero-cta-btn btn-play";
    ctaSvg.innerHTML = '<path d="M8 5v14l11-7z"/>';
    ctaTitle.textContent = "INICIAR JUEGO";
    ctaSubtitle.textContent = "GE-Proton • Sesión Netmarble Activa";
    statusText.textContent = `Sesión iniciada: ${currentStatus.player_name || "Piloto"} (Listo para jugar)`;
    progressContainer.style.display = "none";
  }
}

// Setup Event Listeners for Tauri backend push events
function setupTauriListeners() {
  if (!listen) return;

  // Real-time download progress from Rust Downloader
  listen("download-progress", (event) => {
    const prog = event.payload;
    if (!prog) return;

    currentStatus.is_downloading = !prog.is_completed;
    const progressContainer = document.getElementById("progress-container");
    progressContainer.style.display = "flex";

    document.getElementById("progress-step-text").textContent = prog.status;
    const pct = (prog.progress_ratio * 100).toFixed(1);
    const speed = prog.speed_bytes_per_sec > 1048576 
      ? (prog.speed_bytes_per_sec / 1048576).toFixed(2) + " MB/s" 
      : (prog.speed_bytes_per_sec / 1024).toFixed(1) + " KB/s";
    const eta = prog.eta_seconds ? `${prog.eta_seconds}s` : "--";

    document.getElementById("progress-stats-text").textContent = `${pct}% • ${speed} • ETA: ${eta}`;
    document.getElementById("progress-bar-fill").style.width = `${pct}%`;

    if (prog.is_completed) {
      setTimeout(() => {
        refreshStatus();
      }, 1000);
    }
  });

  // Game state changes (terminated / running)
  listen("game-status", (event) => {
    const state = event.payload;
    currentStatus.is_playing = state.is_running;
    updateUI();
  });

  // Netmarble Authentication Status Change
  listen("auth-status-changed", async (event) => {
    const data = event.payload;
    if (data && data.success) {
      closeAuthModal();
      await refreshStatus();
    } else if (data && !data.success) {
      document.getElementById("auth-select-view").style.display = "block";
      document.getElementById("auth-waiting-view").style.display = "none";
      showAuthFeedback("Error al autenticar: " + (data.error || "Desconocido"), "error");
    }
  });
}

// Setup Interactive Carousel
function setupCarousel() {
  const slides = [
    {
      tag: "ACTUALIZACIÓN",
      title: "Cliente Nativo Linux v1.03.00",
      desc: "Bypass completo de Electron y CrashSight VEH con GE-Proton y DXVK Async.",
    },
    {
      tag: "EVENTO",
      title: "Recompensas de Pre-registro Activas",
      desc: "Reclama tus personajes estelares y armas exclusivas en el portal de cupones.",
    },
    {
      tag: "TIENDA",
      title: "Descuentos en la Web Store",
      desc: "Paquetes especiales y cristales con bonificación de lanzamiento.",
    },
  ];

  let activeIndex = 0;
  const tagEl = document.getElementById("carousel-tag");
  const titleEl = document.getElementById("carousel-title");
  const descEl = document.getElementById("carousel-desc");
  const dots = document.querySelectorAll(".carousel-dots .dot");

  function setSlide(idx) {
    activeIndex = idx;
    tagEl.textContent = slides[idx].tag;
    titleEl.textContent = slides[idx].title;
    descEl.textContent = slides[idx].desc;

    dots.forEach((d, i) => {
      d.classList.toggle("active", i === idx);
    });
  }

  dots.forEach((dot) => {
    dot.addEventListener("click", () => {
      const idx = parseInt(dot.getAttribute("data-index"), 10);
      setSlide(idx);
    });
  });

  // Auto rotate every 6 seconds
  setInterval(() => {
    setSlide((activeIndex + 1) % slides.length);
  }, 6000);
}

// Setup Button Actions
function setupButtons() {
  // Hero CTA Action Button
  document.getElementById("btn-hero-action").addEventListener("click", async () => {
    if (currentStatus.is_playing) return;

    if (currentStatus.is_downloading) {
      await invoke("cancel_download");
      currentStatus.is_downloading = false;
      updateUI();
      return;
    }

    if (!currentStatus.is_installed) {
      currentStatus.is_downloading = true;
      updateUI();
      await invoke("start_download");
    } else if (!currentStatus.is_authenticated) {
      openAuthModal();
      showAuthFeedback("Debes iniciar sesión con tu cuenta de Netmarble/Google para jugar.", "warning");
    } else {
      currentStatus.is_playing = true;
      updateUI();
      try {
        await invoke("launch_game");
      } catch (err) {
        alert("Error al iniciar juego: " + err);
        currentStatus.is_playing = false;
        updateUI();
      }
    }
  });

  // Repair Button
  document.getElementById("btn-repair").addEventListener("click", async () => {
    if (confirm("¿Deseas verificar y descargar nuevamente el cliente oficial?")) {
      currentStatus.is_downloading = true;
      updateUI();
      await invoke("start_download");
    }
  });

  // Open Game Folder Button
  document.getElementById("btn-open-folder").addEventListener("click", async () => {
    await invoke("open_folder", { target: "game" });
  });

  // Coupon Redeem Portal Button
  document.getElementById("btn-coupon").addEventListener("click", async () => {
    await invoke("open_external", { url: "https://couponweb.netmarble.com/coupon/monster2/" });
  });

  // Web Shop Button
  document.getElementById("btn-shop").addEventListener("click", async () => {
    await invoke("open_external", { url: "https://stardive-shop.netmarble.com/" });
  });

  // Discord Button
  document.getElementById("btn-discord").addEventListener("click", async () => {
    await invoke("open_external", { url: "https://discord.gg/stardive" });
  });

  // Fast FPS Quick Button
  document.getElementById("btn-fps").addEventListener("click", () => {
    document.getElementById("modal-quick-fps").style.display = "flex";
  });
  document.getElementById("close-quick-fps").addEventListener("click", () => {
    document.getElementById("modal-quick-fps").style.display = "none";
  });
  document.getElementById("btn-save-quick-fps").addEventListener("click", async () => {
    const selected = document.querySelector('input[name="quick-fps"]:checked').value;
    await invoke("apply_fps_tweak", { fpsLimit: parseInt(selected, 10), vsync: false });
    document.getElementById("modal-quick-fps").style.display = "none";
  });

  // Logs Modal
  document.getElementById("btn-logs").addEventListener("click", async () => {
    document.getElementById("modal-logs").style.display = "flex";
    await refreshLogs();
  });
  document.getElementById("close-logs").addEventListener("click", () => {
    document.getElementById("modal-logs").style.display = "none";
  });
  document.getElementById("btn-close-logs-footer").addEventListener("click", () => {
    document.getElementById("modal-logs").style.display = "none";
  });
  document.getElementById("btn-refresh-logs").addEventListener("click", refreshLogs);
  document.getElementById("btn-clear-logs").addEventListener("click", async () => {
    await invoke("clear_logs");
    document.getElementById("log-content").textContent = "";
  });

  // Netmarble SSO Auth Buttons
  const btnHeaderAuth = document.getElementById("btn-header-auth");
  if (btnHeaderAuth) {
    btnHeaderAuth.addEventListener("click", openAuthModal);
  }

  const chipHeaderProfile = document.getElementById("chip-header-profile");
  if (chipHeaderProfile) {
    chipHeaderProfile.addEventListener("click", openAccountModal);
  }

  document.getElementById("close-auth").addEventListener("click", closeAuthModal);
  document.getElementById("close-account").addEventListener("click", closeAccountModal);
  document.getElementById("btn-close-account").addEventListener("click", closeAccountModal);

  // Providers
  document.getElementById("btn-login-google").addEventListener("click", () => startAuth("google"));
  document.getElementById("btn-login-email").addEventListener("click", () => startAuth("email"));
  document.getElementById("btn-login-apple").addEventListener("click", () => startAuth("apple"));

  document.getElementById("btn-cancel-auth").addEventListener("click", () => {
    document.getElementById("auth-select-view").style.display = "block";
    document.getElementById("auth-waiting-view").style.display = "none";
  });

  // Manual fallback toggle
  document.getElementById("btn-toggle-manual").addEventListener("click", () => {
    const box = document.getElementById("manual-input-box");
    box.style.display = box.style.display === "none" ? "flex" : "none";
  });

  document.getElementById("btn-submit-manual").addEventListener("click", submitManualAuth);

  // Copy Auth URL
  document.getElementById("btn-copy-auth-url").addEventListener("click", () => {
    const input = document.getElementById("auth-fallback-url");
    input.select();
    navigator.clipboard.writeText(input.value);
    alert("URL copiada al portapapeles. Pégala en tu navegador.");
  });

  // Logout Button
  document.getElementById("btn-logout").addEventListener("click", async () => {
    if (confirm("¿Estás seguro de que deseas cerrar sesión de Netmarble?")) {
      await invoke("logout");
      closeAccountModal();
      await refreshStatus();
    }
  });
}

// ------------------------------------------------------------
// AUTHENTICATION CONTROLLER HELPERS
// ------------------------------------------------------------

function openAuthModal() {
  document.getElementById("modal-auth").style.display = "flex";
  document.getElementById("auth-select-view").style.display = "block";
  document.getElementById("auth-waiting-view").style.display = "none";
  document.getElementById("auth-feedback").style.display = "none";
}

function closeAuthModal() {
  document.getElementById("modal-auth").style.display = "none";
}

function openAccountModal() {
  document.getElementById("account-name").textContent = currentStatus.player_name || "Piloto";
  document.getElementById("account-id").textContent = currentStatus.player_id
    ? `ID: ${currentStatus.player_id}`
    : "Sesión activa";
  if (currentStatus.profile_img_url) {
    document.getElementById("account-avatar").src = currentStatus.profile_img_url;
  }
  document.getElementById("modal-account").style.display = "flex";
}

function closeAccountModal() {
  document.getElementById("modal-account").style.display = "none";
}

function showAuthFeedback(msg, type = "info") {
  const fb = document.getElementById("auth-feedback");
  fb.textContent = msg;
  fb.className = `auth-feedback feedback-${type}`;
  fb.style.display = "block";
}

async function startAuth(channel) {
  try {
    document.getElementById("auth-select-view").style.display = "none";
    document.getElementById("auth-waiting-view").style.display = "block";

    const authUrl = await invoke("start_auth_flow", { channel });
    if (authUrl) {
      document.getElementById("auth-fallback-url").value = authUrl;
      document.getElementById("auth-url-box").style.display = "flex";
    }
  } catch (err) {
    document.getElementById("auth-select-view").style.display = "block";
    document.getElementById("auth-waiting-view").style.display = "none";
    showAuthFeedback("Error al iniciar SSO: " + err, "error");
  }
}

async function submitManualAuth() {
  const input = document.getElementById("manual-token-input").value.trim();
  if (!input) {
    showAuthFeedback("Por favor ingresa un token válido.", "warning");
    return;
  }
  try {
    const status = await invoke("manual_auth", { input });
    if (status) {
      closeAuthModal();
      await refreshStatus();
    }
  } catch (err) {
    showAuthFeedback("Error de autenticación: " + err, "error");
  }
}

// Refresh execution logs
async function refreshLogs() {
  try {
    const logs = await invoke("get_logs");
    document.getElementById("log-content").textContent = logs || "Sin registros recientes.";
  } catch (err) {
    document.getElementById("log-content").textContent = "Error al leer logs: " + err;
  }
}

// Setup Settings Modal & Tabs
function setupModals() {
  const modal = document.getElementById("modal-settings");
  const openBtn = document.getElementById("btn-settings");
  const closeBtn = document.getElementById("close-settings");
  const cancelBtn = document.getElementById("btn-cancel-settings");
  const saveBtn = document.getElementById("btn-save-settings");

  openBtn.addEventListener("click", async () => {
    modal.style.display = "flex";
    await loadSettingsForm();
  });

  closeBtn.addEventListener("click", () => (modal.style.display = "none"));
  cancelBtn.addEventListener("click", () => (modal.style.display = "none"));

  // Tab switching
  const tabBtns = document.querySelectorAll(".dialog-tabs .tab-btn");
  const tabContents = document.querySelectorAll(".dialog-body .tab-content");

  tabBtns.forEach((btn) => {
    btn.addEventListener("click", () => {
      tabBtns.forEach((b) => b.classList.remove("active"));
      tabContents.forEach((c) => c.classList.remove("active"));
      btn.classList.add("active");
      const targetId = btn.getAttribute("data-tab");
      document.getElementById(targetId).classList.add("active");
    });
  });

  // Save Settings
  saveBtn.addEventListener("click", async () => {
    if (!currentConfig) return;

    currentConfig.proton_path = document.getElementById("cfg-proton").value;
    currentConfig.game_dir = document.getElementById("cfg-game-dir").value;
    currentConfig.prefix_dir = document.getElementById("cfg-prefix-dir").value;
    currentConfig.launch_args = document.getElementById("cfg-args").value;
    currentConfig.use_gamemode = document.getElementById("cfg-gamemode").checked;
    currentConfig.use_mangohud = document.getElementById("cfg-mangohud").checked;
    currentConfig.use_gamescope = document.getElementById("cfg-gamescope").checked;
    currentConfig.gamescope_args = document.getElementById("cfg-gamescope-args").value;
    currentConfig.dxvk_async = document.getElementById("cfg-dxvk").checked;
    currentConfig.wine_esync = document.getElementById("cfg-esync").checked;
    currentConfig.wine_fsync = document.getElementById("cfg-fsync").checked;
    currentConfig.nvapi = document.getElementById("cfg-nvapi").checked;
    currentConfig.proton_use_wined3d = document.getElementById("cfg-wined3d").checked;

    // Parse custom env variables
    const envStr = document.getElementById("cfg-custom-env").value.trim();
    currentConfig.custom_env = {};
    if (envStr) {
      envStr.split(/\s+/).forEach((pair) => {
        const idx = pair.indexOf("=");
        if (idx > 0) {
          const k = pair.substring(0, idx);
          let v = pair.substring(idx + 1);
          if (v.toLowerCase() === "false") v = "0";
          if (v.toLowerCase() === "true") v = "1";
          currentConfig.custom_env[k] = v;
        }
      });
    }

    const fpsRadio = document.querySelector('input[name="fps"]:checked');
    if (fpsRadio) {
      currentConfig.fps_limit = parseInt(fpsRadio.value, 10);
    }
    currentConfig.vsync = document.getElementById("cfg-vsync").checked;

    await invoke("save_config", { config: currentConfig });
    modal.style.display = "none";
    await refreshStatus();
  });

  // Gamescope toggle input visibility
  document.getElementById("cfg-gamescope").addEventListener("change", (e) => {
    document.getElementById("gamescope-args-group").style.display = e.target.checked ? "block" : "none";
  });

  // Clear Shader cache
  document.getElementById("btn-clear-shaders").addEventListener("click", async () => {
    const count = await invoke("clear_shader_cache");
    document.getElementById("dialog-feedback").textContent = `Se eliminaron ${count} archivos de caché de shaders.`;
    setTimeout(() => {
      document.getElementById("dialog-feedback").textContent = "";
    }, 4000);
  });

  // Rewrite Manifest
  document.getElementById("btn-rewrite-manifest").addEventListener("click", async () => {
    await invoke("rewrite_manifest");
    document.getElementById("dialog-feedback").textContent = "game_manifest.json regenerado.";
    setTimeout(() => {
      document.getElementById("dialog-feedback").textContent = "";
    }, 4000);
    await refreshStatus();
  });

  // Apply FPS tweak inside tab
  document.getElementById("btn-apply-fps").addEventListener("click", async () => {
    const fpsRadio = document.querySelector('input[name="fps"]:checked');
    const fps = fpsRadio ? parseInt(fpsRadio.value, 10) : 144;
    const vsync = document.getElementById("cfg-vsync").checked;
    await invoke("apply_fps_tweak", { fpsLimit: fps, vsync });
    document.getElementById("dialog-feedback").textContent = "Configuración de FPS inyectada.";
    setTimeout(() => {
      document.getElementById("dialog-feedback").textContent = "";
    }, 4000);
  });
}

// Load current configuration into settings inputs
async function loadSettingsForm() {
  try {
    const res = await invoke("get_config_and_runners");
    currentConfig = res.config;
    const runners = res.runners || [];

    document.getElementById("cfg-proton").value = currentConfig.proton_path || "";
    document.getElementById("cfg-game-dir").value = currentConfig.game_dir || "";
    document.getElementById("cfg-prefix-dir").value = currentConfig.prefix_dir || "";
    document.getElementById("cfg-args").value = currentConfig.launch_args || "NMENV=nmp";

    document.getElementById("cfg-gamemode").checked = !!currentConfig.use_gamemode;
    document.getElementById("cfg-mangohud").checked = !!currentConfig.use_mangohud;
    document.getElementById("cfg-gamescope").checked = !!currentConfig.use_gamescope;
    document.getElementById("gamescope-args-group").style.display = currentConfig.use_gamescope ? "block" : "none";
    document.getElementById("cfg-gamescope-args").value = currentConfig.gamescope_args || "-W 1920 -H 1080 -f";

    document.getElementById("cfg-dxvk").checked = !!currentConfig.dxvk_async;
    document.getElementById("cfg-esync").checked = !!currentConfig.wine_esync;
    document.getElementById("cfg-fsync").checked = !!currentConfig.wine_fsync;
    document.getElementById("cfg-nvapi").checked = !!currentConfig.nvapi;
    document.getElementById("cfg-wined3d").checked = !!currentConfig.proton_use_wined3d;
    document.getElementById("cfg-vsync").checked = !!currentConfig.vsync;

    // Load custom env vars as string
    if (currentConfig.custom_env && typeof currentConfig.custom_env === "object") {
      const envPairs = Object.entries(currentConfig.custom_env)
        .map(([k, v]) => `${k}=${v}`)
        .join(" ");
      document.getElementById("cfg-custom-env").value = envPairs;
    } else {
      document.getElementById("cfg-custom-env").value = "";
    }

    // Set FPS radio
    const fpsVal = currentConfig.fps_limit !== undefined ? currentConfig.fps_limit.toString() : "144";
    const radio = document.querySelector(`input[name="fps"][value="${fpsVal}"]`);
    if (radio) radio.checked = true;

    // Populate auto-detected Proton runners
    const chipsContainer = document.getElementById("runner-chips-list");
    chipsContainer.innerHTML = "";
    runners.forEach((r) => {
      const btn = document.createElement("button");
      btn.className = "runner-chip-btn";
      const parts = r.split("/");
      btn.textContent = `Usar ${parts[parts.length - 1]}`;
      btn.addEventListener("click", () => {
        document.getElementById("cfg-proton").value = r;
      });
      chipsContainer.appendChild(btn);
    });
  } catch (err) {
    console.error("Failed to load settings:", err);
  }
}
