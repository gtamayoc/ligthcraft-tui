# PRD: Navegación Estricta de Paneles, Ciclo de Color de Renderizado, Multi-Selección de Variantes y Selector Rápido de Formatos de Exportación

## Overview
El módulo de inspección individual (Mode A) de LightCraft TUI presenta áreas de mejora en el aislamiento de navegación entre paneles, retroalimentación visual del ciclo de vida de renderizado y capacidades de exportación por lote:
1. **Fuga de eventos de teclado en Panel 1**: Estando en el Panel 1 (`Library`), las flechas horizontales (`Left`/`Right`, `h`/`l`) continúan interactuando con el selector de variantes del Panel 2 (`PreviewCanvas`), alterando la variante enfocada a pesar de que el foco activo está en la biblioteca de fotos.
2. **Falta de ciclo cromático de renderizado en Panel 2**: No existe un código de colores claro en los bordes de los slots de variantes para identificar de inmediato si un slot está enfocado/en espera (Naranja), procesando render (Rojo), o finalizado con éxito (Verde).
3. **Ausencia de multi-selección de variantes**: En el Panel 2 sólo se puede operar sobre una variante a la vez. Se requiere una selección múltiple estilo checklist (similar al Panel 1 con `Space`), resaltando las variantes seleccionadas en tono blanco con indicador visual `[X]`.
4. **Exportación masiva de variantes con selección de formatos**: Al presionar `E`, se deben exportar todas las variantes seleccionadas permitiendo al usuario elegir rápidamente entre TIFF, JPEG o ambos formatos mediante un selector ágil e interactivo en la barra inferior de estado sin sobrecargar la interfaz.

---

## Goals
- **Aislamiento estricto de atajos de teclado**: Asegurar que las teclas de navegación horizontal de variantes solo operen cuando el foco resida explícitamente en el Panel 2 (`PaneId::PreviewCanvas`).
- **Feedback visual del ciclo de renderizado**: Proporcionar identificación visual inmediata del estado de render mediante bordes de color dinámicos (Naranja = Selección/Enfoque, Rojo = Render en proceso, Verde = Render finalizado y persistente).
- **Multi-selección ágil de variantes**: Permitir seleccionar una o múltiples variantes del grid mediante la tecla `Space` (con indicador `[X]` y color blanco), reasignando favoritos a `f`/`F`.
- **Selector de formatos sin fricción**: Presentar una confirmación rápida en la barra de estado al pulsar `E` para elegir el tipo de formato (`[1] Solo TIFF`, `[2] Solo JPEG`, `[3] Ambos`, `[Esc] Cancelar`) y procesar todas las variantes seleccionadas.

---

## Quality Gates

These commands must pass for every user story:
- `cargo check` - Verificación de tipos y compilación limpia
- `cargo test` - Suite de pruebas unitarias y de integración sin fallos

---

## User Stories

### US-001: Aislamiento estricto de eventos de teclado entre Panel 1 y Panel 2
**Description:** Como usuario que navega por el Panel 1 (Biblioteca), quiero que únicamente las flechas Arriba/Abajo naveguen la lista de archivos, evitando que las flechas Izquierda/Derecha alteren las variantes del Panel 2 mientras estoy en el Panel 1.

**Acceptance Criteria:**
- [ ] En `src/app.rs` (`handle_mode_a_key`), condicionar `KeyCode::Left`, `KeyCode::Right`, `Char('h')` y `Char('l')` para que solo se ejecuten cuando `self.focused_pane == PaneId::PreviewCanvas`.
- [ ] Verificar que en el Panel 1 (`PaneId::Library`) solo respondan `Up`/`Down`, `k`/`j`, `Space` y teclas globales (`Tab`, `q`, `m`, `F5`, `F10`).
- [ ] Añadir prueba unitaria en `tests/` que verifique que el envío de eventos de teclas izquierda/derecha con `focused_pane == PaneId::Library` no modifica `mode_a.focused_variant`.

### US-002: Ciclo de color dinámico en los bordes de slots de variantes (Naranja / Rojo / Verde)
**Description:** Como fotógrafo que previsualiza variantes, quiero que los bordes de cada recuadro cambien de color según su fase de renderizado (Naranja en espera/selección, Rojo procesando, Verde completado), manteniéndose en verde hasta que inicie una nueva imagen.

**Acceptance Criteria:**
- [ ] Actualizar la lógica de estilo de borde en `render_variant_matrix` en `src/tui/mode_a.rs`:
  - Slot enfocado/esperando render: borde en color Naranja (`Color::Rgb(255, 165, 0)` o `Color::Yellow`).
  - Slot en renderizado activo (`VariantRenderStatus::Rendering`): borde en color Rojo (`Color::Red`).
  - Slot completado (`VariantRenderStatus::Ready`): borde en color Verde (`Color::Green`).
- [ ] El estado Verde debe mantenerse visible en el slot completado hasta que se seleccione otra imagen o se fuerce un nuevo ciclo de renderizado.
- [ ] En caso de fallo (`VariantRenderStatus::Failed`), el borde o indicador debe reflejar error (Rojo o Magenta con mensaje descriptivo).

### US-003: Multi-selección de variantes con tecla Space y resaltado Blanco
**Description:** Como usuario que analiza variantes, quiero seleccionar múltiples variantes en el grid usando `Space` para que se marquen con `[X]` y resalten con borde blanco, mientras que la acción de favoritos se traslada a la tecla `F`.

**Acceptance Criteria:**
- [ ] Agregar propiedad `is_selected: bool` a `VariantSlot` (o un `HashSet<usize>` en `ModeAState`) para rastrear qué variantes están seleccionadas.
- [ ] Reasignar la tecla `Space` cuando `focused_pane == PaneId::PreviewCanvas` para alternar la selección múltiple de la variante bajo el cursor.
- [ ] Reasignar la marcación de favoritos (`toggle_favorite`) a la tecla `f` / `F`.
- [ ] Renderizar las variantes seleccionadas con título/badge conteniendo `[X]` y con estilo de borde Blanco (`Color::White` o `Color::Rgb(240, 240, 240)`), prevaleciendo sobre el color verde o naranja de foco para identificar claramente las variantes que integrarán el lote de exportación.
- [ ] Actualizar el footer de ayuda en `src/tui/layout.rs` para reflejar `[Space] Seleccionar variante` y `[F] Favorito`.

### US-004: Selector ágil de formatos en barra de estado y exportación por lote al presionar E
**Description:** Como fotógrafo que desea exportar variantes seleccionadas, quiero presionar `E` y ver en la barra de estado un selector directo de formato (TIFF, JPEG o ambos) para procesar todas las variantes seleccionadas sin ventanas complejas.

**Acceptance Criteria:**
- [ ] Al pulsar `E` en `PaneId::PreviewCanvas`, si no hay variantes multi-seleccionadas, tomar automáticamente la variante enfocada actual.
- [ ] Activar un estado de diálogo modal rápido en la barra de estado (`AppController.export_format_prompt = true` o similar).
- [ ] Renderizar en la barra de estado el selector interactivo: `Exportar [N] variante(s): [1] Solo TIFF | [2] Solo JPEG | [3] Ambos (TIFF + JPEG) | [Esc] Cancelar`.
- [ ] Al presionar `1`, `2` o `3`:
  - `1`: Encolar exportación únicamente con perfil TIFF 16-bit para cada variante seleccionada.
  - `2`: Encolar exportación únicamente con perfil JPEG 100% para cada variante seleccionada.
  - `3`: Encolar ambos perfiles (TIFF 16-bit + JPEG 100%) para cada variante seleccionada.
- [ ] Al presionar `Esc`: cancelar la acción y restaurar la barra de estado normal sin exportar.
- [ ] Notificar en la barra de estado el progreso o inicio de las exportaciones por lote y deseleccionar las variantes procesadas al finalizar.

---

## Functional Requirements
- **FR-1**: Las teclas `Left`, `Right`, `h`, `l` no deben surtir ningún efecto sobre las variantes cuando el foco esté posicionado en `PaneId::Library`.
- **FR-2**: Cada slot de variante en `ModeAView::VariantMatrix` debe reflejar su estado mediante colores normalizados de borde: Naranja (Enfocado / Sin render), Rojo (Renderizando), Verde (Completado listo).
- **FR-3**: El color Verde de variante completada debe persistir indefinidamente hasta que cambie la foto activa o se re-ejecute el render.
- **FR-4**: La barra espaciadora en `PaneId::PreviewCanvas` debe alternar la selección múltiple de la variante enfocada, cambiando su borde a Blanco y agregando la marca `[X]`.
- **FR-5**: La tecla `F` / `f` debe alternar la condición de favorito (`★`) de la variante enfocada.
- **FR-6**: Al presionar `E`, el sistema debe capturar las opciones `1`, `2`, `3` o `Esc` desde la barra de estado para determinar los formatos de exportación y procesar en lote todas las variantes seleccionadas.

---

## Non-Goals (Fuera de Alcance)
- No se creará una vista completa dedicada de configuración de exportación ni ventanas superpuestas pesadas; se usará la barra de estado rápida y eficiente (Opción 2C).
- No se alterará el formato del archivo RAW maestro original en disco (se preserva la inmutabilidad fotográfica de entrada).
- No se modificará el comportamiento del Modo B (Batch Queue) existente.

---

## Technical Considerations
- **Aislamiento de teclas**: Modificación puntual y de bajo impacto en `src/app.rs` dentro de `handle_mode_a_key`.
- **Arquitectura de estado no bloqueante**: La captura del prompt de formato de exportación en la barra de estado debe mantenerse dentro del bucle de eventos asíncronos de Tokio/Ratatui sin congelar el render loop a 60 FPS.
- **Formatos compatibles**: `OutputFormat::Tiff` y `OutputFormat::Jpeg` ya están integrados en `src/export/profile.rs` y el pipeline de exportación existente.

---

## Success Metrics
- Cero desbordes de eventos de navegación horizontal desde el Panel 1 al Panel 2.
- 100% de consistencia visual en la progresión cromática Naranja -> Rojo -> Verde en el render de variantes.
- Capacidad verificada de seleccionar 2 o más variantes y exportarlas simultáneamente en el formato seleccionado (TIFF, JPEG o ambos) con una sola interacción de teclado.
