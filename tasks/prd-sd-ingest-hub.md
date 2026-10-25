# PRD: Menú Launcher y Sistema de Ingesta Inteligente de Tarjetas SD con Identificación de Cámara en Modo Read-Only

## Overview
Implementación de un **Menú Launcher Modular (Hub de Herramientas)** previo en `lightcraft-tui` y un sistema de **Ingesta y Extracción Automatizada de Tarjetas SD** optimizado para fotógrafos que formatean continuamente sus tarjetas en cámara.

El sistema implementa la **Opción B**:
1. **100% Read-Only en la tarjeta SD:** No escribe archivos marcadores en la memoria extraíble, garantizando compatibilidad con tarjetas con interruptor físico de protección contra escritura ("Lock switch") y respetando la integridad inmutable del medio.
2. **Reconocimiento por Serial de Cámara (EXIF):** Identifica la tarjeta tras ser formateada en la cámara mediante la extracción ultra-rápida del `BodySerialNumber` (número de serie del cuerpo de la cámara) grabado en las fotos `.CR3`, `.ARW`, `.JPG`, etc.
3. **Persistencia y Automatización Escalable:** La primera vez solicita interactivamente la carpeta de almacenamiento de destino. En las siguientes inserciones, consulta la ruta registrada e inicia automáticamente la transferencia organizada por fecha, omitiendo duplicados mediante hashes criptográficos SHA-256.
4. **Soporte de Tarjeta Vacía:** Si la tarjeta viene formateada de la cámara sin fotos, ofrece confirmación de 1 tecla basada en la capacidad física.

---

## Goals
- **Menú Launcher Previo (Hub Extensible):** Presentar un selector interactivo al ejecutar `cargo run` para elegir entre `[1] LightCraft Studio (Revelado RAW)` y `[2] SD Ingest & Storage (Extracción y Análisis)`, dejando la arquitectura lista para agregar más herramientas en el futuro.
- **Detección Automática de Unidades en Windows:** Escaneo nativo Win32 de unidades extraíbles (`DRIVE_REMOVABLE`), detectando capacidad, sistema de archivos (exFAT/FAT32) y carpetas `DCIM/`.
- **Cero Modificación en la Tarjeta SD:** Lectura estricta sin escribir marcadores ni metadatos en la memoria.
- **Identificación Biométrica por Cámara:** Extracción eficiente del número de serie físico de la cámara (`BodySerialNumber`) desde los metadatos EXIF de la primera imagen sin decodificar el archivo completo en memoria.
- **Automatización Primera Vez vs Recurrente:**
  - *Primera inserción:* Asistente rápido para seleccionar y validar la carpeta de almacenamiento.
  - *Siguientes inserciones:* Reconocimiento instantáneo y copia directa a la ruta configurada.
- **Pipeline de Ingesta Escalable y No Destructivo:** Copia asíncrona en streaming por chunks (búfer de 1 MB) con cálculo de SHA-256 en vuelo y omisión instantánea de archivos ya transferidos sin saturar la memoria RAM.
- **Soporte de Tarjeta Vacía Recién Formateada:** Prompt de 1 toque (`[Enter]`) si la tarjeta no tiene fotos, basado en la capacidad en sectores.

---

## Quality Gates

These commands must pass for every user story:
- `cargo check` - Compilación limpia y estricta verificación de tipos en cada historia de usuario

Al finalizar la implementación completa del PRD:
- `cargo test` - Suite completa de pruebas unitarias y de integración sin errores

---

## User Stories

### US-001: Menú Launcher Inicial y Selector de Herramientas Modular
**Description:** Como usuario, quiero un menú interactivo previo al iniciar la aplicación para seleccionar entre el revelador de fotos existente (LightCraft Studio) y la nueva herramienta de extracción de tarjetas SD, con capacidad de sumar futuras opciones.

**Acceptance Criteria:**
- [ ] Crear el módulo `src/hub/mod.rs` con el enum `ToolOption`:
  - `Studio` (LightCraft Studio / Modo A y B actual).
  - `SdIngest` (Analizador e Ingesta de Tarjetas SD).
  - `Exit` (Salir).
- [ ] Implementar la pantalla interactiva del Launcher con Ratatui en `src/hub/view.rs`:
  - Selector con navegación por flechas `[↑/↓]` o atajos de teclado numéricos directos (`[1]`, `[2]`, `[Q]`).
  - Tarjetas visuales con título, descripción y estado del sistema.
- [ ] Modificar `src/main.rs`:
  - Si se ejecuta `cargo run` en terminal interactiva sin flags directos, mostrar el Launcher Hub.
  - Preservar compatibilidad hacia atrás: flags como `--tui`, `--sd-ingest`, `--verify-integrity` o `--sequential-export` inician directamente su modo respectivo sin pasar por el Launcher.

---

### US-002: Escaneo de Unidades Removibles y Análisis de Tarjetas en Windows
**Description:** Como usuario, quiero que el sistema detecte automáticamente las tarjetas SD conectadas a la PC y analice su capacidad, formato y estructura de directorios fotográficos.

**Acceptance Criteria:**
- [ ] Crear el módulo `src/ingest/scanner.rs` con la estructura `SdCardVolume`:
  - `drive_letter: char` (ej. `'E'`).
  - `volume_label: String`.
  - `file_system: String` (ej. `"exFAT"`, `"FAT32"`).
  - `total_bytes: u64`.
  - `free_bytes: u64`.
  - `has_dcim: bool`.
  - `camera_folders: Vec<PathBuf>` (rutas como `E:\DCIM\100CANON`).
- [ ] Implementar escaneo nativo de unidades para Windows (`GetLogicalDrives`, `GetDriveTypeW`, `GetVolumeInformationW` o consulta Win32 equivalente):
  - Filtrar solo unidades con tipo `DRIVE_REMOVABLE` o discos externos con estructura `DCIM/`.
- [ ] Contabilizar archivos fotográficos en la tarjeta: cantidad de RAWs (`.CR3`, `.CR2`, `.ARW`, `.NEF`), JPEGs y videos (`.MP4`, `.MOV`), y volumen total a transferir.

---

### US-003: Extractor Ligero de Huella Digital EXIF de Cámara
**Description:** Como usuario, quiero que el sistema identifique la cámara utilizada leyendo el número de serie físico del cuerpo de la cámara en los metadatos EXIF de las fotos, sin cargar gigabytes de imagen a memoria.

**Acceptance Criteria:**
- [ ] Crear el módulo `src/ingest/exif_fingerprint.rs` con la estructura `CameraFingerprint`:
  - `make: String` (ej. `"Canon"`).
  - `model: String` (ej. `"Canon EOS R6 Mark II"`).
  - `body_serial: Option<String>` (número de serie extraído del tag EXIF `BodySerialNumber`).
  - `lens_model: Option<String>`.
- [ ] Implementar la función `extract_camera_fingerprint(photo_path: &Path) -> Result<CameraFingerprint>`:
  - Usar `kamadak-exif` leyendo únicamente los primeros 64 KB a 128 KB del contenedor de metadatos (evitando cargar el archivo RAW completo de 30-50 MB).
  - Extraer de forma segura los tags `Tag::Make`, `Tag::Model` y `Tag::BodySerialNumber`.
- [ ] Manejar fallback ordenado: si el archivo no contiene `BodySerialNumber`, utilizar el identificador compuesto `Model` + `Make`.

---

### US-004: Registro Local Escalable de Cámaras y Tarjetas (100% Read-Only en SD)
**Description:** Como usuario, quiero que la configuración y vinculación de las cámaras y tarjetas se almacene únicamente en el almacenamiento local de la PC, garantizando cero escrituras en la tarjeta SD.

**Acceptance Criteria:**
- [ ] Crear el módulo `src/ingest/registry.rs` con la estructura `CameraProfile`:
  - `camera_id: String` (clave primaria: `body_serial` o hash de modelo).
  - `display_name: String` (ej. `"Canon R6 Juan"`).
  - `target_directory: PathBuf` (carpeta de almacenamiento de destino).
  - `known_card_capacities: Vec<u64>` (capacidades en bytes de las tarjetas asociadas).
  - `last_ingest_timestamp: Option<DateTime<Utc>>`.
  - `total_photos_ingested: u64`.
- [ ] Persistencia atómica en formato TOML en la ruta local del usuario: `~/.lightcraft/camera_registry.toml`.
- [ ] Índices en memoria con `HashMap<String, CameraProfile>` para búsquedas instantáneas $O(1)$ sin importar la cantidad de perfiles registrados.
- [ ] Veto estricto a nivel de código: ninguna función de este módulo intentará abrir descriptores de archivo en modo escritura hacia la unidad de la tarjeta SD.

---

### US-005: Flujo Interactivo de Destino con Auto-Guardado y Memoria
**Description:** Como usuario, quiero que la primera vez que se detecte una cámara se me solicite interactivamente la carpeta de destino, y que las siguientes veces el sistema recuerde esa ruta e inicie la transferencia automáticamente.

**Acceptance Criteria:**
- [ ] Crear la vista TUI interactiva en `src/tui/ingest_setup.rs`:
  - **Paso Primera Vez:** Si `CameraFingerprint` no está registrado en `camera_registry.toml`:
    - Mostrar pantalla modal: *"Nueva cámara detectada: [Modelo] (Serial: [Serial])"*.
    - Campo interactivo para ingresar o seleccionar la ruta de destino deseada (con autocompletado y validación de existencia en disco).
    - Guardar inmediatamente en `camera_registry.toml`.
  - **Paso Siguientes Veces:** Si `CameraFingerprint` ya existe en el registro:
    - Recuperar automáticamente `target_directory`.
    - Mostrar banner informativo: *"Cámara reconocida: [Nombre]. Destino: [target_directory]"*.
    - Iniciar la ingesta inmediatamente o tras un timeout de confirmación de 2 segundos.

---

### US-006: Motor de Ingesta Asíncrono en Streaming con SHA-256 y Control de Duplicados
**Description:** Como fotógrafo, quiero que la transferencia de fotos sea ultra-rápida, asíncrona, segura y escalable para miles de fotos, calculando el hash SHA-256 en vuelo y omitiendo fotos duplicadas.

**Acceptance Criteria:**
- [ ] Crear `src/ingest/pipeline.rs` con la estructura `IngestPipeline`:
  - Copia asíncrona mediante búferes de streaming de 1 MB (`tokio::io::AsyncReadExt` y `AsyncWriteExt`).
  - Cálculo simultáneo del hash criptográfico `Sha256` en vuelo sin necesidad de releer el archivo desde la tarjeta.
- [ ] Control de Duplicados Escalable:
  - Mantener un registro local ligero de transferencias en `~/.lightcraft/ingest_history.db` o JSONL con pares `(file_name, file_size, sha256)`.
  - Antes de copiar, verificar si el archivo de origen ya fue transferido previamente; si existe en el destino con idéntico tamaño y hash, omitirlo (`SkipDuplicate`).
- [ ] Organización de Destino:
  - Guardar las fotos en la estructura: `{target_directory}/{YYYY}/{YYYY-MM-DD}/` basada en la fecha de captura EXIF.
- [ ] Generar un archivo de manifiesto `ingest_manifest_{timestamp}.json` en la carpeta de destino con el resumen de la sesión, archivos transferidos y verificación de integridad.

---

### US-007: Detección y Asociación Rápida de Tarjeta Vacía Recién Formateada
**Description:** Como usuario, si inserto una tarjeta que acabo de formatear en la cámara y aún no tiene fotos, quiero que el sistema me ofrezca vincularla con 1 sola tecla según su capacidad física.

**Acceptance Criteria:**
- [ ] Crear en `src/ingest/empty_handler.rs` la lógica de verificación de tarjeta vacía:
  - Comprobar si la unidad removible tiene `free_bytes == total_bytes` (o dentro del 99% por tablas FAT/exFAT) y 0 fotos en `DCIM/`.
- [ ] Si la capacidad en bytes coincide con las `known_card_capacities` de algún perfil de cámara registrado en `camera_registry.toml`:
  - Mostrar en pantalla el prompt interactivo de 1 toque:
    `[TARJETA VACÍA DETECTADA: 128 GB] ¿Pertenece a Canon EOS R6? [Enter = Sí / Esc = Ignorar]`
- [ ] Al presionar `[Enter]`:
  - Marcar la tarjeta como vinculada a esa cámara para la sesión actual, dejando listo el destino configurado para cuando se guarden fotos.

---

## Functional Requirements
- **FR-1:** El sistema debe arrancar en el Launcher Hub cuando no se provean banderas específicas en el CLI.
- **FR-2:** La detección de tarjetas SD debe identificar volúmenes extraíbles en Windows con sistema de archivos exFAT o FAT32.
- **FR-3:** La tarjeta SD debe ser tratada como medio de solo lectura estricto; el sistema no debe escribir ningún archivo en la memoria física.
- **FR-4:** La lectura de metadatos EXIF para el fingerprint debe limitarse al bloque inicial de metadatos del archivo para garantizar latencia inferior a 50 ms por consulta.
- **FR-5:** El registro de cámaras debe persistir de manera atómica en el directorio local de configuración del usuario (`~/.lightcraft/camera_registry.toml`).
- **FR-6:** La primera inserción de una cámara desconocida debe solicitar al usuario la ruta de destino y guardarla en su perfil.
- **FR-7:** Las inserciones subsecuentes de la misma cámara deben cargar la ruta guardada sin volver a solicitarla al usuario.
- **FR-8:** El pipeline de copia debe calcular el checksum SHA-256 en streaming durante la copia y guardarlo en el manifiesto de sesión.
- **FR-9:** Los archivos que ya existan en destino con idéntico tamaño y hash deben ser ignorados para evitar duplicación innecesaria.
- **FR-10:** Las tarjetas formateadas sin fotos deben ofrecer un diálogo de 1 tecla para re-asociarlas con la cámara registrada de igual capacidad.
- **FR-11:** Al finalizar la extracción de archivos, el sistema debe emitir una notificación nativa Toast de Windows informando la cantidad de fotos transferidas y la ruta de destino.

---

## Non-Goals (Out of Scope)
- No se implementarán drivers de kernel propietarios para tarjetas SD ni se intentará leer el registro físico CID vía comandos de bajo nivel SCSI/SFFDISK.
- No se formateará la tarjeta SD desde la PC (el formateo queda delegado exclusivamente a la cámara).
- No se editarán metadatos EXIF ni se rotarán los archivos RAW originales durante la transferencia.
- No se sincronizarán las fotos a la nube en este milestone (el destino siempre es un directorio local o de red montado en el sistema de archivos).

---

## Technical Considerations
- **Cero Impacto en Memoria RAM:** Al usar transferencias en streaming por chunks de 1 MB, transferir 100 GB de fotos RAW utiliza menos de 50 MB de memoria RAM residente.
- **Crates Rust Utilizados:**
  - `kamadak-exif` (ya presente en el proyecto) para la extracción de tags EXIF.
  - `tokio` (con soporte asíncrono y fs) para la copia concurrente sin congelar la interfaz TUI.
  - `sha2` y `hex` para la verificación criptográfica.
  - `ratatui` y `crossterm` para el renderizado del Launcher Hub y la vista de ingesta.
  - `windows-sys` para las llamadas de detección de volúmenes en Windows y notificaciones.

---

## Success Metrics
- El Launcher Hub se inicializa en menos de 10 ms.
- La identificación de cámara mediante EXIF se completa en menos de 100 ms tras insertar la tarjeta.
- Cero escrituras en la tarjeta SD durante todo el ciclo de vida del proceso.
- En la segunda inserción, la transferencia comienza automáticamente sin requerir intervención del usuario.
- 100% de los archivos copiados pasan la verificación de integridad SHA-256.
- Todos los quality gates (`cargo check` y `cargo test`) pasan sin advertencias ni errores.

---

## Open Questions
- Ninguna pregunta abierta pendiente (resuelto: notificación nativa Toast de Windows al finalizar la ingesta).
