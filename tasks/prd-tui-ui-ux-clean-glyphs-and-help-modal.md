# PRD: Rediseño UI/UX de Consola TUI, Saneamiento de Glifos (Anti-Signos de Interrogación) y Modal de Ayuda F1

## 1. Overview
La interfaz de terminal de LightCraft (`lightcraft-tui`) presenta actualmente problemas de legibilidad, saturación visual y fallos de codificación en entornos de consola:
1. **Glifos rotos en consolas de Windows:** Iconos, flechas, estrellas y marcas de verificación (`❯`, `✓`, `★`, `◄`, `►`) renderizan frecuentemente como signos de interrogación (`?`) o caracteres de reemplazo debido a limitaciones de codepage (CP437, CP850, CP1252) y fuentes sin cobertura Unicode extendida.
2. **Sobrecarga de texto en pantalla:** La cabecera superior y la barra inferior de estado se encuentran saturadas de texto explicativo denso, instrucciones de atajos apiñadas y cadenas largas que desmejoran la estética y dificultan la lectura rápida.
3. **Falta de documentación estructurada de comandos:** Los atajos de teclado se encuentran dispersos e incrustados directamente en las barras de estado en lugar de residir en un espacio dedicado y accesible por panel.

Esta iniciativa implementa un sistema centralizado de saneamiento de glifos con autodetección de codepage UTF-8 y fallback limpio a ASCII seguro, un rediseño de minimalismo visual extremo en cabecera y pie, y un diálogo emergente de Ayuda / Tutorial interactivo (`F1` / `?`) organizado en pestañas por panel.

---

## 2. Goals
- **Eliminación total de glifos corruptos (`?`):** Garantizar que ninguna consola ni terminal (especialmente Windows Command Prompt y PowerShell) muestre signos de interrogación por falta de soporte de fuentes o codepage.
- **Detección inteligente y Fallback ASCII:** Activar codepage UTF-8 en Windows durante la inicialización de la terminal y disponer de un modo fallback seguro a caracteres ASCII legibles (`->`, `[x]`, `*`, `v`, `^`), además de soporte para override explícito (`LC_ASCII=1` / `--ascii`).
- **Modal de Ayuda y Atajos interactivo (`F1` / `?`):** Centralizar la documentación de cada comando por panel en una ventana emergente estructurada con pestañas navegables (`[1] General`, `[2] Fotos`, `[3] Variantes`, `[4] Exportar`).
- **Minimalismo extremo y desaturación visual:** Limpiar la cabecera superior y la barra inferior, eliminando bloques extensos de texto y dejando únicamente indicadores esenciales de estado y contexto.
- **Preservación de la ergonomía y control de flujo:** Mantener intacta la lógica de navegación y operaciones de revelado, mejorando la experiencia de usuario (UI/UX).

---

## 3. Quality Gates

These commands must pass for every user story:
- `cargo check --all-targets` - Comprobación estricta de tipos y compilación sin advertencias.
- `cargo test` - Todas las suites de pruebas unitarias, de integración, adversariales y de layout deben pasar con 0 fallos.

---

## 4. User Stories

### US-001: Módulo centralizado de glifos y activación de codepage UTF-8 en Windows
**Description:** Como usuario de consola en Windows o terminales heterogéneas, quiero que la aplicación configure la codificación adecuada y sustituya automáticamente los glifos Unicode por caracteres ASCII seguros si la terminal no los soporta, para no ver signos de interrogación (`?`) en ningún panel o icono.

**Acceptance Criteria:**
- [ ] Crear el módulo `src/tui/glyphs.rs` con una estructura/enum `GlyphSet` que provea símbolos unificados para toda la interfaz (ej. `step_arrow`, `check_mark`, `favorite_star`, `triangle_left`, `triangle_right`, `bullet`, `separator`).
- [ ] Implementar dos modos en `GlyphSet`: `Unicode` (glifos modernos elegantes como `❯`, `✓`, `★`) y `Ascii` (caracteres seguros universales como `>`, `[x]`, `*`, `<`).
- [ ] En la inicialización de la terminal (`src/tui/terminal.rs`), invocar configuración de consola UTF-8 en plataformas Windows (`SetConsoleOutputCP(65001)` o equivalente vía winapi/crossterm).
- [ ] Implementar detección automática de entorno en `GlyphSet::detect()`: si se detecta consola legacy de Windows sin UTF-8 garantizado, o si se define la variable de entorno `LC_ASCII=1` o el argumento CLI `--ascii`, activar modo `Ascii`.
- [ ] Auditar y reemplazar los caracteres Unicode fijos en `src/app.rs`, `src/tui/mode_a.rs` y `src/tui/mode_b.rs` para utilizar exclusivamente `GlyphSet`.

### US-002: Modal emergente de Ayuda y Tutorial por pestañas (`F1` / `?`)
**Description:** Como fotógrafo en la TUI, quiero presionar `F1` o `?` para abrir una ventana emergente organizada por pestañas que explique detalladamente los atajos y propósitos de cada panel, para consultar la guía sin saturar la pantalla principal de trabajo.

**Acceptance Criteria:**
- [ ] Agregar el estado `show_help_modal: bool` y `help_active_tab: usize` al controlador principal en `src/app.rs`.
- [ ] Interceptar las teclas `F1` y `?` para abrir y cerrar el modal de ayuda de manera alternada; `Esc` y `q` también deben cerrar el modal si está abierto.
- [ ] Renderizar el modal centrado sobre la pantalla usando `Clear` y un `Block` con bordes nítidos y título dinámico según el modo de glifos activo.
- [ ] Disponer de 4 pestañas navegables con teclas numéricas `1..4` o `Tab` / flechas:
  - `[1] General`: Navegación entre paneles, cambio de modo, pantalla completa y salida.
  - `[2] Biblioteca`: Movimiento entre fotos RAW, selección múltiple, recarga y filtrado.
  - `[3] Variantes/Visor`: Selección de recetas, favoritos, comparación A/B, zoom y loupe.
  - `[4] Exportar/Asistente`: Pasos del wizard, selección de proporciones, formatos y renderizado.
- [ ] Mientras el modal esté abierto, bloquear cualquier evento que modifique el estado de las fotos o paneles de fondo.

### US-003: Rediseño minimalista extremo de Cabecera y Barra de Estado
**Description:** Como usuario de la consola, quiero que la cabecera superior y la barra de estado inferior sean visualmente limpias y minimalistas, para concentrarme en las fotografías sin distracciones ni párrafos apiñados.

**Acceptance Criteria:**
- [ ] Rediseñar `render_header` en `src/app.rs`:
  - Eliminar los párrafos explicativos extensos del asistente y modo estándar.
  - Presentar una única línea elegante con: Nombre de la aplicación, badge del modo activo (`[Mode A]` / `[Mode B]`), indicador discreto del paso del Asistente y badge de estado de conexión MCP.
- [ ] Rediseñar `render_status` en `src/app.rs`:
  - Reducir la barra inferior a una sola línea compacta y con alto contraste.
  - Sección izquierda: Contador de foto activa (`Foto X/Y`) y nombre del archivo seleccionado.
  - Sección central: Mensaje de estado dinámico o badge de selección (`Sel: N`).
  - Sección derecha: Indicador compacto de comandos globales: `[F1 Ayuda]  [Tab Panel]  [Q Salir]`.
- [ ] Eliminar los strings concatenados largos de atajos (`[↑/↓] Moverse | [Space] Sel | [Enter] ...`) de la barra inferior, trasladando su contenido al modal de ayuda de `F1`.

### US-004: Pulido de densidad visual y bordes en los Paneles Principales (Modos A y B)
**Description:** Como fotógrafo, quiero que los tres paneles principales tengan títulos concisos y espaciado proporcional en sus bordes, para tener una jerarquía visual armónica en resoluciones tanto pequeñas como amplias.

**Acceptance Criteria:**
- [ ] Estandarizar los títulos de los paneles a etiquetas concisas y elegantes: `[1] Biblioteca`, `[2] Visor`, `[3] Inspector`, usando el glifo activo de foco.
- [ ] El panel activo debe destacar mediante estilo de borde (doble línea o color cian brillante) mientras los inactivos usan bordes oscuros sutiles.
- [ ] Comprobar que en terminales compactas (80x24) ningún texto de cabecera o pie sufra saltos de línea antiestéticos o truncamientos accidentales.
- [ ] Validar que todos los tests de layout y renderizado con `TestBackend` (`tier1_tui_layout_tests.rs`) sigan validando correctamente las dimensiones y la ausencia de desbordamientos.

---

## 5. Functional Requirements
- **FR-1:** La aplicación debe inicializar la terminal configurando UTF-8 en Windows (`SetConsoleOutputCP(65001)`).
- **FR-2:** Si la variable `LC_ASCII=1` está presente en el entorno o se pasa el flag `--ascii`, la TUI debe forzar el modo de glifos ASCII en todos los paneles.
- **FR-3:** Al presionar `F1` o `?`, se debe abrir el modal de ayuda centrado con dimensiones del 75% del ancho y 70% del alto de la terminal.
- **FR-4:** El modal de ayuda debe responder a `1`, `2`, `3`, `4`, `Tab`, `Left`, `Right` para alternar pestañas, y `Esc`, `F1`, `q` para cerrarse.
- **FR-5:** La cabecera superior estándar no debe exceder 2 líneas de altura, independientemente de la resolución de la terminal.
- **FR-6:** La barra de estado inferior debe mantener una altura fija de 1 línea, sin desbordamiento de texto hacia líneas invisibles.
- **FR-7:** Ningún carácter renderizado en pantalla debe generar el símbolo de interrogación `?` por codepage no coincidente.

---

## 6. Non-Goals (Out of Scope)
- No se modificará el pipeline de procesamiento de revelado RAW ni los algoritmos matemáticos de color y recetas de `src/engine/`.
- No se añadirá configuración de fuentes personalizadas dentro de la terminal (la elección tipográfica depende de la configuración del emulador del usuario).
- No se creará una interfaz gráfica basada en ventanas nativas (GUI externa); LightCraft se mantiene 100% como una TUI nativa de consola en Rust con Ratatui.

---

## 7. Technical Considerations
- **Crate Crossterm & Ratatui:** Aprovechar `crossterm::event::KeyCode::F(1)` para la captura limpia de la tecla F1.
- **Limpieza de pantalla y superposición:** Utilizar `ratatui::widgets::Clear` antes de renderizar el modal de ayuda para evitar que el contenido previo de los paneles se transparente detrás del diálogo.
- **Consistencia de tests existentes:** Varias pruebas en `tests/tier1_interactive_flow_tests.rs` inspeccionan texto del banner o de la barra de estado; estas pruebas deben actualizarse para verificar los nuevos textos concisos y la existencia del modal de ayuda.

---

## 8. Success Metrics
- Cero reportes de caracteres `?` en terminales estándar de Windows (cmd.exe, PowerShell 5.1/7+, Windows Terminal).
- Reducción de más del 50% de la densidad de caracteres de texto plano en la cabecera y el footer durante la navegación normal.
- Disponibilidad inmediata de la guía de comandos completa accesible en menos de 1 segundo al presionar `F1`.
- 100% de pase en los Quality Gates (`cargo check --all-targets` y `cargo test`).

---

## 9. Open Questions
- ¿Deseamos persistir la preferencia de modo ASCII en el archivo de configuración de presets o recetas en una fase posterior? (Actualmente cubierto con `LC_ASCII` y autodetección).
