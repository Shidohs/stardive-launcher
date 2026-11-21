# Stardive Launcher — Design System (Material 3 Expressive · Cosmic Pill)

Estas reglas son obligatorias para cualquier cambio en `ui/index.html`, `ui/style.css` o `ui/app.js`. El objetivo es mantener una interfaz coherente en **Material Design 3 Expressive (estilo píldoras)** con rendimiento fluido a 60 FPS en WebKitGTK / Wayland.

## 1. Paleta Tonal Oscura (Tokens en `:root`)

Usa siempre las variables CSS definidas en `ui/style.css`; no introduzcas colores hexadecimales sueltos para botones o superficies.

| Rol | Variable | Valor | Uso |
| :--- | :--- | :--- | :--- |
| Fondo base | `--md-sys-color-surface` | `#0e1118` | Base del viewport |
| Superficie baja | `--md-sys-color-surface-container-low` | `rgba(18, 22, 33, 0.86)` | Contenedores secundarios (carrusel) |
| Superficie principal | `--md-sys-color-surface-container` | `rgba(23, 28, 42, 0.92)` | Islas principales (Action Pod, Header Dock) |
| Superficie elevada | `--md-sys-color-surface-high` / `highest` | `#22283c` / `#2c344d` | Hover, controles secundarios, switches apagados |
| Acento primario (Instalar / Activo) | `--md-sys-color-primary` / `--md-sys-color-on-primary` | `#bac3ff` / `#17225c` | Botón Descargar/Instalar, tabs/radios/switches activos |
| Contenedor primario | `--md-sys-color-primary-container` / `--md-sys-color-on-primary-container` | `#2d3a78` / `#e0e3ff` | Botón de cuenta, tab activa, badges |
| Acción principal (Jugar) | `--md-sys-color-play` / `--md-sys-color-on-play` | `#7ce0b0` / `#042919` | Botón Iniciar Juego (contraste AAA) |
| Peligro / Cancelar | `--md-sys-color-red` / `--md-sys-color-on-red` | `#ffb4ab` / `#690005` | Botón Cancelar y errores |
| Texto | `--md-sys-color-on-surface` / `variant` | `#e6e9f4` / `#a6aec6` | Títulos y descripciones |

## 2. Sistema de Formas (Escala de Píldoras)

No mezcles radios arbitrarios (`4px`, `8px`, `18px`, `24px`). Respeta estrictamente la escala:

* **Píldora completa (`--m3-shape-pill: 9999px`):** Todos los botones (`hero-cta-btn`, `m3-btn-*`, `m3-icon-btn`, `m3-mini-btn`, `close-btn`, `browse-btn`, `auth-provider-btn`), cápsula del header (`.header-pill-dock`), chips, badges, tabs, radio pills, switches, barra de progreso y tooltips.
* **Contenedores grandes (`--m3-shape-xl: 28px`):** `.carousel-card`, `.action-pod`, `.m3-dialog`, `.auth-dialog`.
* **Contenedores medianos (`--m3-shape-md: 14px` / `--m3-shape-lg: 20px`):** Inputs (`.m3-input`), filas interactivas (`.switch-row`), `.log-viewport` y `.account-card`.

## 3. Jerarquía y Layout

* **Header derecho:** Mantén la píldora de cuenta/perfil separada de la cápsula agrupada `.header-pill-dock` (no pongas 6–7 botones circulares sueltos compitiendo entre sí).
* **Dock inferior asimétrico:** `.carousel-card` (~`356px`, superficie `container-low`) siempre debe ser más compacto y discreto que `.action-pod` (`400px`, superficie `container`) para que el foco visual sea el CTA principal y respire el keyart de fondo.
* **Botón Hero CTA (`.hero-cta-btn`):** Color tonal plano sin borde blanco, sin gradiente y sin resplandor neón. Texto e ícono heredan `currentColor` con alto contraste oscuro sobre tono pastel.
* **Modales:** Tabs segmentadas en píldora (sin subrayado inferior plano) y botón de cierre circular.

## 4. Prohibiciones Estrictas (Rendimiento WebKitGTK y Consistencia)

* **NO usar `backdrop-filter: blur(...)`:** Causa lag severo al mover el cursor en WebKitGTK / Wayland sobre GPUs integradas antiguas. Usa las superficies tonales con opacidad `0.86–0.92`.
* **NO usar glows neón (`box-shadow: 0 0 30px <color>`):** Usa únicamente `--m3-elevation-1/2/3` con sombras negras neutras.
* **NO usar gradientes saturados en botones ni switches:** Usa colores planos de la paleta tonal.
* **NO usar `transition: all`:** Anima únicamente propiedades específicas (`background-color`, `border-color`, `color`, `transform`, `opacity`).
* **NO duplicar metadatos técnicos** innecesariamente entre el header, el carrusel y el botón principal.
