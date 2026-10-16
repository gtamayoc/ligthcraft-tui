# PRD: Motor Nativo Autónomo de Lectura e Inspección de Metadatos Fotográficos

## Overview
Implementar un sistema de extracción y lectura de metadatos fotográficos autónomo y nativo en Rust para LightCraft TUI. Actualmente, la aplicación depende de payloads MCP externos o recurre a valores simulados/hardcodeados en la interfaz (como forzar "Canon EOS R6" y parámetros estáticos). Esta funcionalidad dotará a la aplicación de un extractor local capaz de leer directamente desde el archivo en disco los encabezados EXIF y metadatos de disparo (cámara, lente, apertura, velocidad, ISO, longitud focal, dimensiones y fecha), sincronizándolos automáticamente en la interfaz TUI durante la navegación entre fotos.

## Goals
- Proporcionar lectura autónoma y nativa de metadatos EXIF directamente desde el sistema de archivos (sin depender de herramientas MCP externas).
- Eliminar todo el código simulado/hardcodeado en `app.rs` y `mode_a.rs` (dimensiones estáticas de 6000x4000, fallbacks a "Canon EOS R6" y valores de disparo fijos).
- Sincronizar dinámicamente la ficha técnica completa en el panel "Shooting Metadata" al cambiar o navegar entre fotos.
- Diseñar una arquitectura resiliente con fallbacks limpios ("Unknown Camera", "1/---s", "f/--", "ISO ---") cuando un archivo no contenga etiquetas EXIF.
- Establecer una suite completa de pruebas unitarias y de integración que certifique la extracción nativa en múltiples formatos (JPEG, TIFF, CR3/RAW).

## Quality Gates

These commands must pass for every user story:
- `cargo check` - Verificación de tipos y compilación limpia sin errores
- `cargo test` - Suite completa de tests unitarios y de integración pasando al 100%

## User Stories

### US-001: Integración del motor de extracción nativa de metadatos
**Description:** Como desarrollador, quiero un lector nativo en Rust (`kamadak-exif` o parser EXIF autónomo) para extraer etiquetas EXIF directamente desde los bytes/streams de los archivos de imagen sin depender de servidores MCP externos.

**Acceptance Criteria:**
- [ ] Incorporar dependencia en `Cargo.toml` para lectura pura de EXIF en Rust (ej. `kamadak-exif = "0.6"`)
- [ ] Crear módulo `src/storage/metadata.rs` con función `pub fn read_native_metadata(path: &Path) -> Result<PhotoMetadata, MetadataError>`
- [ ] Extraer campos clave estándar: Make, Model, LensModel, FocalLength, FNumber, ExposureTime, PhotographicSensitivity (ISO), DateTime y dimensiones (PixelXDimension / PixelYDimension o reader header)
- [ ] Tests unitarios en `src/storage/metadata.rs` verificando la extracción nativa con fixtures sintéticos/muestras de prueba

### US-002: Desacople de valores hardcodeados en el estado de la aplicación
**Description:** Como usuario de la TUI, quiero que la vista de metadatos en Mode A muestre los datos reales del archivo seleccionado y no valores fijos de prueba.

**Acceptance Criteria:**
- [ ] Eliminar en `src/app.rs` la inicialización estática de `mode_a.metadata` que fija 6000x4000, Canon EOS R6 y parámetros simulados
- [ ] En la función `sync_selected_photo` de `src/app.rs`, invocar `read_native_metadata(&item.path)` para actualizar el struct `mode_a.metadata` en tiempo real
- [ ] Si la lectura nativa encuentra campos vacíos o la foto no tiene EXIF, mostrar fallbacks legibles y neutros ("Unknown Camera", "1/---s", "f/--", "ISO ---")
- [ ] Garantizar que cambiar de una foto con metadatos a una foto sin metadatos no retenga información residual de la foto anterior

### US-003: Robustecimiento de formateadores y cálculo de métricas
**Description:** Como fotógrafo, quiero que el resumen de exposición, la relación de aspecto y los megapíxeles se calculen de manera exacta y sin panics ante resoluciones atípicas.

**Acceptance Criteria:**
- [ ] `aspect_ratio_str()` debe soportar relaciones comunes (3:2, 2:3, 16:9, 4:3, 1:1, 5:4) y calcular formato decimal exacto para proporciones personalizadas
- [ ] `exposure_summary()` debe manejar de forma elegante fracciones estándar ("1/500s"), velocidades largas ("2.5s", "30s") y valores ausentes
- [ ] Si las dimensiones no vienen en los tags EXIF, obtenerlas de forma eficiente mediante inspección de encabezados (`image::io::Reader::new(File::open(...)).into_dimensions()`)
- [ ] Tests unitarios en `mode_a.rs` probando casos de límites (dimensiones 0x0, aperturas decimales, velocidades fraccionarias)

### US-004: Suite de tests de integración para el pipeline autónomo
**Description:** Como ingeniero de calidad, quiero una suite de integración que valide el flujo completo desde el escaneo del directorio hasta la visualización en la TUI.

**Acceptance Criteria:**
- [ ] Crear test de integración `tests/tier1_native_metadata_tests.rs`
- [ ] Simular carga de un catálogo con múltiples imágenes de diferentes formatos y verificar que la selección de cada una actualiza fielmente `mode_a.metadata`
- [ ] Verificar que no existan dependencias con procesos o respuestas mock de MCP para la lectura de metadatos básicos
- [ ] Documentar en el log de tests la validación exitosa de los módulos de metadatos

## Functional Requirements
- FR-1: La aplicación debe poder inspeccionar y extraer metadatos EXIF directamente del archivo en disco de forma local e independiente de MCP.
- FR-2: `sync_selected_photo` en `app.rs` debe ser reactivo y actualizar `self.mode_a.metadata` en cada cambio de índice.
- FR-3: El widget de visualización de Shooting Metadata (`app.rs`) nunca debe recurrir a valores hardcodeados de una cámara específica.
- FR-4: La extracción nativa de metadatos debe ejecutarse de forma no bloqueante o con latencia mínima (<5ms por archivo) para mantener la fluidez a 60 FPS en la navegación TUI.

## Non-Goals
- Escritura y modificación destructiva de metadatos en archivos RAW/CR3 originales.
- Soporte para etiquetas propietarias binarias indocumentadas (MakerNotes complejos de fabricantes).
- Dependencia obligatoria de daemons o servicios MCP externos para la inspección básica de fotos en biblioteca.

## Technical Considerations
- Usar una biblioteca ligera en Rust puro (como `kamadak-exif`) compatible con compilación cruzada en Windows y sin dependencias C/CMake.
- Aprovechar las funciones seguras de `image::ImageReader` para extraer dimensiones (ancho y alto) de forma ultra rápida sin decodificar el mapa de bits completo.

## Success Metrics
- 100% de tests pasando en `cargo test`.
- Compilación limpia con `cargo check`.
- Ficha técnica de disparo 100% autónoma, dinámica y fiel a las imágenes cargadas.

## Open Questions
- Ninguna. Alcance cerrado para ejecución autónoma.
