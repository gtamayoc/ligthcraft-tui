# PRD: Manejo Confiable de Cancelación con Confirmación para Exportación Interactiva

## 1. Overview
Durante una exportación por lotes en el asistente interactivo (Modo A -> Selección de variantes -> Proporciones -> Formatos -> Exportación), el usuario visualiza en la interfaz `[Esc] Interrumpir`. Sin embargo, al pulsar la tecla `Esc` en `WizardStep::Exporting`, el sistema únicamente oculta la barra visual del asistente (`self.flow_state.close()`) mientras la tarea asíncrona de fondo creada con `tokio::spawn` continúa procesando y escribiendo en disco el resto de fotos (ej. 72 archivos) sin verificar ningún token de cancelación. Además, la cancelación carecía de un diálogo de confirmación para prevenir pulsaciones accidentales de `Esc`.

Este feature introduce:
1. Un diálogo de confirmación visual no bloqueante en la TUI al presionar `Esc` durante la exportación ("¿Desea cancelar la exportación en curso? [S/N]").
2. Un mecanismo de cancelación cooperativa estructurado (`tokio_util::sync::CancellationToken` o señal atómica) que aborta ordenadamente la cola de exportación en segundo plano tras concluir o descartar la foto en vuelo.
3. Limpieza de archivos temporales de staging (`.lc_export_tmp_*`).
4. Notificación clara y concisa en la barra de estado indicando el total de archivos exportados con éxito y el total de archivos omitidos.

## 2. Goals
- Prevenir ejecuciones huérfanas de procesamiento de imágenes cuando el usuario solicita interrumpir la exportación.
- Evitar cancelaciones involuntarias mediante un paso de confirmación ágil (`[S]í / [N]o` o `[Enter]/[Esc]`).
- Garantizar la integridad del sistema de archivos, asegurando que ninguna foto quede a medio escribir ni con archivos temporales huérfanos.
- Proporcionar feedback visual inmediato y exacto en la barra de estado tras la interrupción.

## 3. Quality Gates

These commands must pass for every user story:
- `cargo check --tests` - Verificación de tipos y compilación limpia de la suite completa
- `cargo test --test tier1_mode_a_tests` - Tests de regresión de Modo A y asistente interactivo
- `cargo clippy -- -D warnings` - Linter de Rust sin advertencias ni regresiones

## 4. User Stories

### US-001: Token de Cancelación Cooperativa para Tareas de Exportación
**Description:** Como usuario de LightCraft, quiero que la tarea asíncrona en segundo plano verifique un token de cancelación antes de iniciar cada foto/variante/formato para que el proceso se detenga efectivamente cuando yo lo decida.

**Acceptance Criteria:**
- [ ] Incorporar un mecanismo de señal de cancelación (`tokio_util::sync::CancellationToken` o `Arc<AtomicBool>`) vinculado al ciclo de vida de la exportación en `AppController`.
- [ ] En `execute_flow_export_batch`, verificar el token antes de comenzar cada iteración del bucle de jobs.
- [ ] Si se activa la señal de cancelación, romper el bucle de exportación inmediatamente sin despachar nuevos `pipeline.execute_export`.
- [ ] Emitir un evento `AppEvent::FlowExportCancelled { processed: usize, total: usize, cancelled: usize }` hacia el bucle de eventos principal.

### US-002: Diálogo de Confirmación de Cancelación en TUI (Paso Exporting)
**Description:** Como usuario, quiero que al presionar `Esc` durante una exportación se me pregunte si realmente deseo cancelar para evitar pérdidas involuntarias de progreso por una pulsación accidental.

**Acceptance Criteria:**
- [ ] Añadir un estado de confirmación en `InteractiveFlowState` (ej. `is_confirming_cancel: bool`).
- [ ] Al presionar `Esc` en `WizardStep::Exporting`, pausar la navegación habitual y mostrar en la barra del asistente el mensaje de confirmación: `¿Desea cancelar la exportación en curso? [S]í / [N]o (o [Esc] para continuar)`.
- [ ] Si el usuario presiona `s`, `S` o `Enter` sobre "Sí", activar la señal de cancelación y cerrar el estado de confirmación.
- [ ] Si el usuario presiona `n`, `N` o `Esc` sobre "No", descartar la confirmación y reanudar la visualización estándar de progreso y spinner sin detener el trabajo.

### US-003: Finalización Ordenada y Limpieza de Recursos Temporales
**Description:** Como usuario, quiero que la cancelación sea limpia y segura para que los archivos ya guardados queden intactos y no queden archivos temporales `.lc_export_tmp_*` en la carpeta de destino.

**Acceptance Criteria:**
- [ ] Si una foto ya inició su fase de escritura atómica en archivo temporal, permitir que complete o asegurar que el archivo temporal sea eliminado sin dejar residuos en `export/interactive/`.
- [ ] Al recibir `AppEvent::FlowExportCancelled`, actualizar la barra de estado con el formato: `Exportación cancelada por el usuario. {X} de {Total} fotos guardadas, {Y} omitidas.`
- [ ] Cerrar el asistente (`self.flow_state.close()`) y restaurar la interacción y foco en el panel de Variantes/Librería sin congelar la interfaz.
- [ ] Ignorar eventos de progreso huérfanos que pudieran llegar después de haberse consolidado la cancelación.

### US-004: Tests Automatizados de Regresión y Cancelación
**Description:** Como desarrollador, quiero pruebas unitarias y de integración que verifiquen el ciclo de cancelación para evitar regresiones futuras en la gestión de tareas de fondo.

**Acceptance Criteria:**
- [ ] Añadir un test en `tests/tier1_mode_a_tests.rs` (o suite equivalente) que simule una cola de exportación con múltiples tareas y dispare una señal de cancelación tras N items procesados.
- [ ] Verificar que el número total de archivos generados en disco sea exactamente igual al número de tareas procesadas antes de la cancelación.
- [ ] Verificar que no queden archivos temporales con prefijo `.lc_export_tmp_` en el directorio de exportación.
- [ ] Verificar que la transición de estados `WizardStep::Exporting` -> confirmación -> cancelado funcione adecuadamente ante las teclas `Esc` y `S`.

## 5. Functional Requirements
- FR-1: Al pulsar `Esc` mientras `WizardStep::Exporting` está activo, el sistema NO debe cerrar la exportación silenciosamente; debe entrar en estado de confirmación visual.
- FR-2: Durante el estado de confirmación, la exportación de fondo puede continuar en progreso o en pausa cooperativa hasta que el usuario decida.
- FR-3: Confirmar con tecla `S` / `s` debe disparar el cese inmediato de la cola de tareas pendientes.
- FR-4: Rechazar con tecla `N` / `n` o `Esc` debe reanudar la vista regular de progreso.
- FR-5: Ninguna tarea en background debe continuar ejecutándose tras recibir `FlowExportCancelled`.
- FR-6: La barra de estado debe reportar con exactitud cuántos archivos fueron procesados y cuántos fueron omitidos.
- FR-7: La interfaz debe responder a 60 FPS sin bloqueos durante todo el proceso de interrupción.

## 6. Non-Goals
- Modificar el pipeline de decodificación RAW de bajo nivel para permitir cancelaciones a nivel de píxel individual (la cancelación se realiza a nivel de límites de archivo/tarea fotográfica).
- Modificar la lógica de cancelación de la cola de Modo B (`BatchWorkerPool`), la cual se rige por su propio ciclo de vida.
- Implementar pausa/reanudación persistente en disco entre sesiones del ejecutable.

## 7. Technical Considerations
- `tokio_util::sync::CancellationToken` es compatible con Tokio 1.x (ya presente en el `Cargo.toml` como dependencia de Tokio).
- La gestión de eventos en `AppController` utiliza `EventManager` y canales mpsc de Tokio; los eventos de cancelación deben tener prioridad o descartar eventos antiguos de progreso.
- La escritura de archivos en `ExportPipeline::execute_export` utiliza `tempfile::Builder` con prefijo `.lc_export_tmp_`; al salir de alcance con error o cancelación, estos archivos deben limpiarse adecuadamente.

## 8. Success Metrics
- Tiempo de respuesta ante la cancelación: < 500 ms desde que el usuario confirma con 'S' hasta que la tarea se detiene.
- 0 procesos o hilos huérfanos consumiendo CPU tras la cancelación.
- 0 archivos corruptos o temporales residuales en la carpeta de destino.

## 9. Open Questions
- ¿Se desea en el futuro agregar la misma confirmación con `Esc` para la cola de Modo B? (Queda fuera del alcance actual, pero la arquitectura del token podrá ser reutilizada).
