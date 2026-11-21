# Reglas del Repositorio — Stardive Launcher

## Reglas de Diseño UI (Obligatorias)

Antes de modificar `ui/index.html`, `ui/style.css` o `ui/app.js`, lee y respeta estrictamente **`ui/DESIGN_SYSTEM.md`**.

Resumen rápido de restricciones no negociables:
1. **Estilo:** Material Design 3 Expressive (**Cosmic Pill** — paleta tonal oscura Índigo `#bac3ff` + Menta `#7ce0b0`).
2. **Formas:** Usa `9999px` (`--m3-shape-pill`) en todos los botones, chips, tabs, switches, tooltips y cápsulas; `28px` (`--m3-shape-xl`) en tarjetas y modales. No inventes radios intermedios.
3. **Rendimiento WebKitGTK:** Prohibido añadir `backdrop-filter: blur(...)`, `transition: all`, glows neón (`box-shadow: 0 0 ... <color>`) o gradientes saturados en botones/controles.
4. **Jerarquía:** Mantén agrupados los iconos secundarios del header en `.header-pill-dock` y conserva el dock inferior asimétrico (carrusel compacto a la izquierda, isla de acción principal a la derecha).
5. **Compatibilidad JS:** No renombres ni elimines IDs o clases usadas por `ui/app.js` (`.btn-play`, `.btn-download`, `.btn-cancel`, `#cta-title`, `#cta-subtitle`, etc.).
