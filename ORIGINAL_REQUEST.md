# Original User Request

## Initial Request — 2026-10-24T02:29:12Z

Implementar en LightCraft TUI (`hydroid`) un Menú Launcher interactivo modular y un sistema automatizado de ingesta segura de fotos desde tarjetas SD en modo 100% solo lectura, identificando cámaras por número de serie EXIF, recordando de forma estricta y persistente la ruta de almacenamiento de cada tarjeta y enviando notificaciones Toast nativas en Windows.

Working directory: C:\Users\user\orca\workspaces\0001\hydroid
Integrity mode: development
Reference specification: tasks/prd-sd-ingest-hub.md

## Requirements

### R1. Modular Tool Launcher Hub
Implementar un menú previo interactivo en terminal (TUI) que aparezca al ejecutar la aplicación sin argumentos y permita al usuario elegir entre el revelador existente (LightCraft Studio) y el nuevo módulo de Ingesta SD, preservando el bypass directo de todas las banderas CLI existentes (`--tui`, `--verify-integrity`, `--sequential-export`).

### R2. Windows Removable Drive & DCIM Scanner
Detectar de forma automática unidades extraíbles y tarjetas SD conectadas en Windows, analizando su capacidad, sistema de archivos (exFAT/FAT32), presencia de directorios `DCIM/` y conteo estructurado de archivos RAW, JPEG y videos.

### R3. Camera EXIF Fingerprint & 100% Read-Only SD Protocol
Extraer el número de serie único del cuerpo de la cámara (`BodySerialNumber`) y modelo desde los metadatos EXIF de las fotos en menos de 100 ms sin cargar el archivo completo en memoria. El sistema debe operar en modo estricto de solo lectura respecto a la tarjeta SD, sin escribir ningún archivo ni marcador en la memoria física.

### R4. Scalable Local Camera Registry & Strict Folder Binding
Mantener una base de datos local en `~/.lightcraft/camera_registry.toml`. La vinculación entre cada cámara/tarjeta y su carpeta de almacenamiento de destino debe ser estrictamente persistente y confiable:
- La primera vez que se detecte una tarjeta/cámara se debe solicitar interactivamente la carpeta de almacenamiento de destino al usuario.
- En todas las siguientes inserciones, el sistema DEBE consultar inmediatamente esa ruta guardada y realizar la transferencia directamente y sin volver a preguntarle al usuario.
- Si la tarjeta está recién formateada y vacía, debe permitir re-vincularla con 1 sola pulsación (`[Enter]`) basada en su capacidad física registrada.

### R5. Asynchronous Streaming Ingest & Windows Toast Notification
Transferir archivos de forma concurrente y asíncrona mediante streaming por búferes de 1 MB, computando el hash criptográfico SHA-256 en vuelo sin doble lectura de disco y omitiendo duplicados ya transferidos. Al finalizar la extracción con éxito, emitir una notificación Toast nativa de Windows con el resumen de archivos transferidos y ruta de almacenamiento.

## Acceptance Criteria

### Launcher Hub
- [ ] Ejecutar `cargo run` en terminal interactiva despliega el Menú Launcher con navegación por flechas y selección numérica directa.
- [ ] Banderas CLI directas (`--tui`, `--sd-ingest`, `--verify-integrity`, `--sequential-export`) ejecutan directamente su funcionalidad sin mostrar el Launcher.

### Detección y No-Modificación de SD
- [ ] Las unidades extraíbles se detectan correctamente en Windows identificando si contienen estructura de cámara `DCIM/`.
- [ ] Cero operaciones de escritura hacia la unidad de la tarjeta SD durante todo el ciclo de ejecución.
- [ ] El número de serie de la cámara (`BodySerialNumber`) y modelo se extraen mediante lectura parcial de metadatos con `kamadak-exif`.

### Persistencia y Flujo de Ingesta
- [ ] Al detectar una cámara por primera vez, el sistema solicita la ruta de destino y la persiste atómicamente en `~/.lightcraft/camera_registry.toml`.
- [ ] En la segunda inserción de la misma cámara, el sistema recupera la ruta e inicia la transferencia de forma automática y desatendida.
- [ ] Las fotos duplicadas (mismo tamaño y hash SHA-256 en destino) son omitidas de la transferencia.
- [ ] Al terminar la copia, se emite una notificación Toast nativa en Windows informando la cantidad de archivos transferidos.
- [ ] Si se inserta una tarjeta vacía de capacidad coincidente, se presenta un diálogo de 1 tecla para re-asociarla.

### Quality Gates
- [ ] `cargo check` compila limpiamente sin errores de tipos en cada componente.
- [ ] `cargo test` ejecuta y pasa la suite completa de pruebas unitarias y de integración.
