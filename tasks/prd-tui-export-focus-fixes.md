# PRD: TUI Export Controls, Focus Synchronization & Variant Generation Fixes

## Overview
El TUI de Lightcraft presenta inconsistencias de usabilidad y sincronización en la gestión de variantes fotográficas:
1. Las combinaciones de teclas para exportar variantes individuales (especialmente perfiles de máxima calidad) no están unificadas, documentadas ni visibles en la interfaz.
2. Existe un desajuste visual en el foco: el estilo de borde/color de elemento enfocado se mantiene en el primer elemento del grid/lista mientras el cursor o scroll activo se encuentra navegando en el medio.
3. La generación de variantes en el TUI queda congelada en estado de carga indefinida ("cargando variantes...") debido a falta de manejo de eventos asíncronos y retroalimentación de errores/progreso, a pesar de que la invocación manual funciona correctamente.

Este PRD define los requerimientos para añadir controles claros y configurables de exportación en máxima calidad, sincronizar estrictamente el cursor y foco visual del TUI, y desacoplar la generación de variantes con manejo robusto de estado asíncrono y timeouts informativos.

---

## Goals
- **Exportación controlada**: Permitir exportar la variante activa en máxima calidad mediante un atajo directo documentado en el footer y barra de estado.
- **Sincronización del foco visual**: Garantizar que el elemento visualmente enfocado coincida siempre de forma exacta con el índice del elemento activo en la navegación por teclado.
- **Ciclo de vida robusto de variantes**: Eliminar el spinner infinito en la generación de variantes en el TUI asegurando recepción de eventos de progreso, finalización y errores.
- **Claridad de atajos de teclado**: Documentar todos los shortcuts en el footer del TUI y mantener compatibilidad con configuración externa.

---

## Quality Gates

These commands must pass for every user story:
- `cargo check` - Verificación de tipos y compilación limpia
- `cargo test` - Suite completa de tests unitarios e integración
- `cargo clippy --all-targets -- -D warnings` - Linter de Rust sin advertencias

---

## User Stories

### US-001: Documentar e implementar atajo de exportación en máxima calidad
**Description:** Como usuario del TUI, quiero presionar una tecla claramente indicada para exportar la variante activa con el perfil de máxima calidad preconfigurado, de modo que tenga control rápido sin tener que adivinar las teclas.

**Acceptance Criteria:**
- [ ] Definir atajo predeterminado (ej. `e` / `E`) para exportar la variante seleccionada con perfil de máxima calidad en `src/tui/event.rs` y `src/tui/mode_a.rs` (o `mode_b.rs`).
- [ ] Mostrar explícitamente en el footer de ayuda (`src/tui/layout.rs`) la tecla asignada junto a su función: `[E] Exportar Max Calidad`.
- [ ] Encolar el trabajo con los parámetros de máxima calidad en `src/queue/` o `src/export/` y mostrar notificación no bloqueante de exportación iniciada.

### US-002: Sincronización estricta de foco y cursor en el Grid/Lista de Variantes
**Description:** Como usuario del TUI, quiero que el resaltado visual de color de foco siga exactamente al elemento activo en el medio de la lista, para evitar la confusión de pensar que el foco sigue en el inicio.

**Acceptance Criteria:**
- [ ] Corregir la lógica de renderizado en `src/tui/mode_a.rs` / `src/tui/mode_b.rs` donde el estilo `selected` o `active_border` calcula el offset visible versus el índice global.
- [ ] Asegurar que sólo el elemento con `index == active_variant_index` reciba el color/estilo de foco activo.
- [ ] El scroll del viewport debe mantener centrado o visible el elemento activo sin desfasar el índice enfocado.
- [ ] Agregar test de regresión unitario en `tests/tier1_mode_a_tests.rs` o `tests/tier1_tui_layout_tests.rs` que verifique que el índice renderizado con foco coincide con el estado interno tras navegar.

### US-003: Manejo asíncrono y feedback visual de generación de variantes
**Description:** Como usuario del TUI, quiero ver el progreso real de generación de variantes o el error exacto si falla, para que la UI nunca quede en estado de carga infinita.

**Acceptance Criteria:**
- [ ] Conectar los canales de comunicación / receptores de eventos entre el motor en segundo plano y el bucle del TUI (`src/tui/event.rs`).
- [ ] Actualizar el estado de cada variante de `Loading` a `Ready` o `Failed(err)` una vez recibido el evento del pipeline.
- [ ] Implementar un temporizador de timeout preventivo en el TUI que marque la tarea con advertencia si el worker no responde, desbloqueando la UI.
- [ ] Renderizar estado visual diferenciado en la miniatura o etiqueta de la variante: icono de cargando (con contador/progreso si está disponible), éxito o error.

### US-004: Notificaciones y barra de estado de procesos en cola
**Description:** Como usuario del TUI, quiero una sección visible en la interfaz que liste las exportaciones y variantes en proceso o completadas, de manera que pueda verificar qué se ha generado correctamente.

**Acceptance Criteria:**
- [ ] Incorporar un indicador de cola / estado en la barra inferior que muestre tareas activas (ej. `[Variantes: 2/4] [Export: Listo]`).
- [ ] Registrar mensajes legibles en el log visible del TUI cuando finalice una exportación o variante.

---

## Functional Requirements
- **FR-1**: El sistema debe mapear una tecla configurable para disparar la exportación de la variante seleccionada con el preset de máxima calidad.
- **FR-2**: El footer del TUI debe actualizarse dinámicamente según la pantalla/modo activo, exhibiendo las teclas vigentes de navegación y exportación.
- **FR-3**: El índice visual de foco (`focused_item`) debe corresponder 1:1 con el elemento sobre el cual se ejecutan las acciones de teclado.
- **FR-4**: Cuando se solicite la generación de una o varias variantes, el componente de lista debe recibir eventos por cada variante completada y re-renderizar inmediatamente su miniatura/estado.
- **FR-5**: Si una variante falla en el proceso de renderizado interno, el TUI debe reflejar el error y no permanecer en estado cargando indefinido.

---

## Non-Goals
- Modificar el pipeline de exportación por línea de comandos (CLI headless o invocación manual ya verificada).
- Reescribir el motor gráfico de renderizado de imágenes.
- Implementar diálogos modales complejos de configuración de renderizado 3D/HDR en esta iteración.

---

## Technical Considerations
- **Lenguaje y arquitectura**: Proyecto desarrollado en Rust con arquitectura TUI basada en `ratatui` o `crossterm`.
- **Archivos clave**:
  - `src/tui/mode_a.rs` / `src/tui/mode_b.rs`: Renderizado de vistas, cálculo de rects de variantes y lógica de selección.
  - `src/tui/event.rs`: Distribución de eventos de teclado y canal de mensajería asíncrona.
  - `src/tui/layout.rs`: Footer y distribución de barras de estado.
  - `src/queue/` y `src/export/`: Gestión del despacho de tareas de variantes y exportaciones.

---

## Success Metrics
- Cero desincronizaciones visuales entre el elemento seleccionado por teclado y el elemento resaltado con color de foco.
- 100% de finalización clara (éxito o error explícito) en el TUI al generar variantes, eliminando el cuelgue visual.
- Acceso inmediato en 1 tecla a la exportación de máxima calidad con confirmación en la UI.
