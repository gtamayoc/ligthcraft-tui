# PRD: Refactorización del Flujo Interactivo CLI con Asistente Superior y Matriz de Proporciones

## Overview
Reorganización integral del flujo interactivo de procesamiento y exportación fotográfica en `lightcraft-tui`. La interacción actual en la barra inferior es poco visible y no ofrece selección estructurada de proporciones ni encuadres. Este cambio introduce un **Asistente Superior (Top Banner Wizard)** de alta visibilidad en la cabecera de la ventana y un flujo estricto de 6 etapas:
1. **Entrada de imágenes**: Selección de fotos desde la biblioteca (Panel 1).
2. **Variantes de procesamiento**: Selección múltiple de filtros/recetas en el grid (Panel 2).
3. **Proporciones geométricas**: Selección de proporciones de encuadre (`1:1`, `3:2`, `4:3`, `16:9`, `4:5`, original) con cálculo de recorte centrado sin distorsión.
4. **Formatos de archivo**: Selección desacoplada de formatos de codificación (`TIFF`, `JPEG`, `PNG`).
5. **Resumen y confirmación**: Cálculo visible de la matriz de combinaciones en la cabecera con navegación bidireccional y opción de guardar como nuevo Preset.
6. **Ejecución y desacoplamiento**: Despacho asíncrono hacia la cola de exportación con control de progreso.

---

## Goals
- **Máxima visibilidad en cabecera**: Ubicar el asistente y los mensajes de ayuda interactiva en el área superior (`header_area`) de la pantalla, dejando libres los paneles centrales de fotos.
- **Flujo secuencial ordenado**: Variantes → Proporciones → Formatos → Resumen/Preset → Ejecución.
- **Catálogo de proporciones fotográficas estándar**: Soporte nativo para `1:1`, `3:2`, `4:3`, `16:9`, `4:5` y `Original` mediante Center-Crop proporcional.
- **Navegación bidireccional con memoria**: Permitir retroceder con `B` o `Esc` para corregir selecciones previas sin perder datos.
- **Cero colisiones de nombres**: Identificación unívoca de cada archivo exportado mediante tokens `{aspect}` en el motor de nombres.
- **Guardado de Presets**: Permitir almacenar configuraciones nuevas de proporciones y formatos para su reutilización.

---

## Quality Gates

These commands must pass for every user story:
- `cargo check` - Compilación limpia y estricta verificación de tipos
- `cargo test` - Suite de pruebas unitarias y de integración sin errores
- `cargo fmt --check` - Formato de código Rust idiomático

---

## User Stories

### US-001: Modelos de dominio tipados y matemática de recorte (Aspect Ratio)
**Description:** Como desarrollador, quiero definir los tipos de datos en Rust para proporciones geométricas, formatos de exportación y la geometría de corte proporcional, garantizando que el cálculo de dimensiones no distorsione la imagen original.

**Acceptance Criteria:**
- [ ] Definir en `src/export/mod.rs` el enum `AspectRatio`:
  - Variantes: `Square1x1` (1:1), `Photo3x2` (3:2), `Classic4x3` (4:3), `Widescreen16x9` (16:9), `Social4x5` (4:5), `Original`, `Custom(u32, u32)`.
  - Método `aspect_label(&self) -> &'static str` (`"1x1"`, `"3x2"`, `"4x3"`, `"16x9"`, `"4x5"`, `"orig"`).
- [ ] Implementar la función de cálculo de recorte centrado:
  `calculate_center_crop(orig_w: u32, orig_h: u32, ratio: AspectRatio) -> (u32, u32, u32, u32)` (x, y, target_w, target_h).
  - Preservar el área máxima posible de la imagen sin introducir bordes negros ni deformación de píxeles.
- [ ] Definir en `src/app.rs` la estructura `InteractiveFlowState`:
  - `step`: Enum `WizardStep` (`Variants`, `AspectRatios`, `FileFormats`, `Summary`, `SavePreset`).
  - `selected_variants`: `Vec<usize>`.
  - `selected_ratios`: `Vec<AspectRatio>`.
  - `selected_formats`: `Vec<ExportFormat>`.
  - `is_active: bool`.
- [ ] Pruebas unitarias en `tests/` que verifiquen las coordenadas de corte para imágenes horizontales (`6000x4000`) y verticales en todas las proporciones estándar.

---

### US-002: Rediseño del Header de Ratatui para el Asistente Superior (Top Banner)
**Description:** Como usuario, quiero que las instrucciones del flujo interactivo y los atajos de teclado se muestren en la parte superior de la ventana de forma destacada, para ver claramente el paso actual sin que quede oculto en la barra inferior.

**Acceptance Criteria:**
- [ ] Modificar `AppLayout::compute` en `src/tui/layout.rs`:
  - Cuando el asistente esté activo (`flow_state.is_active`), asignar altura dinámica al `header_area` (4 a 5 líneas) para alojar el banner interactivo.
- [ ] En `src/app.rs` (`render_header`), si el asistente está activo, renderizar el componente `TopWizardBanner`:
  - Línea 1: Breadcrumb del paso activo (`[1. Fotos] ❯ [2. Variantes] ❯ [3. Proporciones*] ❯ [4. Formatos] ❯ [5. Resumen]`).
  - Línea 2: Instrucción clara del paso y selecciones acumuladas.
  - Línea 3: Barra de teclas de ayuda (`[1..6] Alternar | [Enter] Continuar | [B] Volver | [Esc] Cancelar`).
- [ ] Garantizar contraste visual alto (borde Cyan o Amarillo con texto blanco/negro resaltado).

---

### US-003: Panel superior interactivo para Selección de Proporciones (Paso 3)
**Description:** Como fotógrafo, quiero seleccionar una o varias proporciones de imagen mediante atajos rápidos en el banner superior, pudiendo regresar al selector de variantes si deseo modificar mi elección.

**Acceptance Criteria:**
- [ ] Al pulsar `E` en el grid de variantes (Paso 2), validar que haya al menos una variante seleccionada y transicionar `InteractiveFlowState.step` a `WizardStep::AspectRatios`.
- [ ] Renderizar en el banner superior las opciones:
  - `[1] 1:1 Cuadrado` | `[2] 3:2 Tradicional` | `[3] 4:3 Clásico` | `[4] 16:9 Panorámico` | `[5] 4:5 Retrato` | `[6] Original` | `[A] Todas`.
- [ ] Teclas `1` a `6` o `Space` alternan la selección mostrando `[X]` en las elegidas.
- [ ] Tecla `Enter` valida que exista al menos una proporción seleccionada y avanza al Paso 4.
- [ ] Tecla `B` o `Esc` regresa al Paso 2 (Variantes) conservando la selección de variantes intacta.
- [ ] Impedir duplicados en el listado de proporciones seleccionadas.

---

### US-004: Panel superior interactivo para Selección de Formatos (Paso 4)
**Description:** Como usuario, quiero seleccionar los formatos de archivo de exportación (TIFF, JPEG, PNG) desde la cabecera antes de ver el resumen final.

**Acceptance Criteria:**
- [ ] En `WizardStep::FileFormats`, mostrar en el banner superior las opciones disponibles:
  - `[1] Solo TIFF (16-bit)` | `[2] Solo JPEG (Web)` | `[3] Solo PNG` | `[4] TIFF + JPEG` | `[5] Todos (TIFF+JPEG+PNG)`.
- [ ] Teclas `1` a `5` seleccionan el conjunto de formatos deseado.
- [ ] Tecla `Enter` avanza al Paso 5 (Resumen).
- [ ] Tecla `B` regresa al Paso 3 (Proporciones) manteniendo las proporciones seleccionadas.
- [ ] Tecla `Esc` cancela el flujo y vuelve a la vista normal de fotos sin exportar.

---

### US-005: Panel de Resumen, Guardado de Presets y Confirmación (Paso 5)
**Description:** Como fotógrafo, quiero ver el cálculo total de imágenes a generar en la cabecera, poder guardar la configuración como preset si es nueva, o saltar directamente a modificar cualquier paso previo antes de procesar.

**Acceptance Criteria:**
- [ ] En `WizardStep::Summary`, mostrar en el área superior el cálculo matemático:
  - `Imágenes (N) × Variantes (V) × Proporciones (P) × Formatos (F) = TOTAL archivos`.
- [ ] Menú de opciones de confirmación:
  - `[1/Enter] Confirmar e iniciar exportación`.
  - `[2] Modificar Variantes` (salto al Paso 2).
  - `[3] Modificar Proporciones` (salto al Paso 3).
  - `[4] Modificar Formatos` (salto al Paso 4).
  - `[P] Guardar como nuevo Preset de Exportación` (solicita nombre y persiste en `preset_store`).
  - `[Esc/5] Cancelar`.
- [ ] Si el usuario salta a modificar un paso y pulsa `Enter`, debe retornar directamente al Resumen con las cifras actualizadas.

---

### US-006: Pipeline de Recorte en Exportación y Nomenclatura Anti-Colisión
**Description:** Como usuario, quiero que el motor de exportación aplique físicamente el recorte centrado correspondiente a cada proporción elegida y genere nombres de archivo que incluyan el token de aspecto para que los archivos no se sobreescriban entre sí.

**Acceptance Criteria:**
- [ ] Incorporar el token `{aspect}` en `src/export/naming.rs`:
  - Añadir `aspect_ratio: &'a str` a `NamingContext`.
  - Plantilla por defecto: `{name}__{preset}__{aspect}.{ext}`.
- [ ] En `src/export/mod.rs` (`ExportPipeline`):
  - Antes de la codificación final de la imagen (`encode_tiff`, `encode_jpeg`, `encode_png`), aplicar `imageops::crop_imm(&img, x, y, w, h).to_image()` según las dimensiones calculadas en US-001.
- [ ] Garantizar la estricta inmutabilidad de los archivos RAW/CR3 de origen (verificación SHA-256 intacta).
- [ ] Añadir prueba de integración que ejecute un export de prueba con una imagen real en formatos `1:1`, `3:2` y `16:9`, verificando que se generan 3 archivos distintos con las dimensiones en píxeles correctas.

---

### US-007: Despacho Asíncrono a la Cola y Retroalimentación en Barra de Progreso
**Description:** Como usuario, quiero que al confirmar el resumen el trabajo se envíe a la cola de exportación en segundo plano sin congelar la interfaz, cerrando el asistente superior y mostrando el progreso general.

**Acceptance Criteria:**
- [ ] Al pulsar `Confirmar`, convertir el `InteractiveFlowState` en una colección de `ExportJob` y enviarlos a la cola de procesamiento en segundo plano (`tokio::spawn`).
- [ ] Restaurar la altura estándar del `header_area` y devolver el foco a la vista principal.
- [ ] Mostrar en la barra de estado el progreso en tiempo real (`Exportando lote: 14/72 archivos...`).
- [ ] Registrar en el log cualquier error de exportación individual (ej. permiso o disco) sin detener el resto de las tareas del lote.

---

## Functional Requirements
- **FR-1**: El asistente debe desplegarse en la parte superior (`header_area`), expandiendo su área para mantener legibles las instrucciones y atajos.
- **FR-2**: El flujo debe seguir estrictamente el orden: Entrada → Variantes → Proporciones → Formatos → Resumen → Ejecución.
- **FR-3**: El Paso 2 no debe solicitar proporciones ni formatos de archivo.
- **FR-4**: El Paso 3 debe admitir selección única o múltiple de proporciones (`1:1`, `3:2`, `4:3`, `16:9`, `4:5`, `Original`).
- **FR-5**: El recorte debe realizarse mediante Center-Crop simétrico conservando la máxima resolución útil de la imagen.
- **FR-6**: El Paso 4 debe admitir uno o varios formatos simultáneos (`TIFF`, `JPEG`, `PNG`).
- **FR-7**: El usuario debe poder regresar al paso anterior mediante la tecla `B` o `Esc` sin perder los datos seleccionados.
- **FR-8**: El Paso 5 debe calcular y mostrar el total de combinaciones antes de iniciar la exportación.
- **FR-9**: El Paso 5 debe ofrecer la opción de guardar la configuración activa como Preset de Exportación reutilizable.
- **FR-10**: Cada archivo exportado debe incluir el token de proporción (`{aspect}`) para evitar colisiones.
- **FR-11**: Los archivos fuente RAW originales no deben ser alterados bajo ninguna circunstancia.
- **FR-12**: La cancelación en cualquier punto previo a la confirmación debe cerrar el asistente sin escribir archivos.
- **FR-13**: La ejecución de los trabajos de exportación no debe bloquear el hilo principal de renderizado de Ratatui.

---

## Non-Goals
- No implementar herramientas de reencuadre o paneo manual con ratón en esta fase (el recorte es automático y centrado).
- No añadir formatos de compresión no estándar en el catálogo (solo TIFF, JPEG y PNG soportados por el motor).
- No eliminar ni reemplazar las opciones de línea de comandos no interactivas existentes.

---

## Technical Considerations
- **Cálculo de Layout Dinámico**: Ajustar la restricción del layout superior de `Constraint::Length(3)` a `Constraint::Length(5)` únicamente mientras `InteractiveFlowState.is_active == true`.
- **Rendimiento de Recorte**: El recorte con `image::imageops::crop_imm` opera sobre vistas del búfer en memoria antes de la codificación, evitando accesos adicionales a disco.
- **Persistencia de Presets**: Reutilizar el `PresetStore` existente para guardar perfiles de exportación en formato TOML/JSON en la carpeta de configuración.

---

## Success Metrics
- Asistente 100% visible y cómodo de operar en la parte superior de la ventana.
- Generación exitosa de archivos recortados en proporciones `1:1`, `3:2`, `16:9` sin deformación.
- 0 colisiones o sobreescrituras accidentales en exportaciones combinadas.
- Pasan sin errores `cargo check`, `cargo test` y `cargo fmt --check`.

---

## Open Questions
- Ninguna pregunta abierta pendiente. El flujo, la ubicación superior, las proporciones estándar y la opción de presets han sido definidos y confirmados.
