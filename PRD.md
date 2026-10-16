# PRD — Automatizador Fotográfico Profesional en Rust (LightCraft MCP TUI)

**Versión:** 1.0.0  
**Fecha:** 2026-10-15  
**Documento base:** `1.txt`  
**Estado:** Listo para Implementación (Fase P0)

---

## 1. Visión y Objetivos

Construir una aplicación de terminal (TUI) profesional y de alto rendimiento en Rust, interactuando nativamente mediante **Model Context Protocol (MCP)** con el motor de revelado fotográfico **LightCraft**.

La herramienta permite a fotógrafos y editores:
1. Inspeccionar y procesar imágenes individuales o colecciones masivas (JPEG, PNG, TIFF y RAW Canon CR3).
2. Generar múltiples recetas visuales y variaciones estéticas de forma no destructiva.
3. Comparar visualmente las variantes directamente en la terminal (`ratatui-image` con Kitty, Sixel y fallback Halfblocks).
4. Exportar lotes con perfiles estandarizados, plantillas de nombres y control de colisiones.

---

## 2. Roles del Equipo de Ingeniería

* **Arquitecto de Software:** Diseño desacoplado en Rust, gestión de memoria eficiente, cliente cliente-servidor JSON-RPC MCP robusto.
* **Ingeniero de Automatización Fotográfica:** Mapeo de parámetros de revelado (exposición, contraste, curvas, HSL, balance de blancos, etc.), manejo de recetas/presets y formatos RAW.
* **Diseñador UX/UI:** Layout responsive y ergonómico en terminal con Ratatui (paneles, split views, atajos de teclado, barras de progreso).
* **Implementador Principal:** Implementación limpia en Rust, manejo de errores exhaustivo, suite de tests unitarios y de integración.
* **Revisor Adversarial:** Validación de integridad de archivos fuente (inmutabilidad de originales), concurrencia segura y manejo de caídas de conexión MCP.

---

## 3. Arquitectura del Sistema

```
┌─────────────────────────────────────────────────────────────────┐
│                     TUI Interface (ratatui)                     │
│   [Header: Modos]  │  [Left: Library]  │  [Center: Preview]     │
│   [Right: Presets] │  [Bottom: Queue & Action Bar]              │
└───────────────────────────────▲─────────────────────────────────┘
                                │ Event Loop / UI State
┌───────────────────────────────▼─────────────────────────────────┐
│                    Application Core Engine                      │
│  ┌──────────────────────┐               ┌────────────────────┐  │
│  │ Recipe/Preset Engine │               │  Job & Batch Queue │  │
│  └──────────────────────┘               └────────────────────┘  │
│  ┌──────────────────────┐               ┌────────────────────┐  │
│  │ Image Preview Cache  │               │   Export Manager   │  │
│  └──────────────────────┘               └────────────────────┘  │
└───────────────────────────────▲─────────────────────────────────┘
                                │ JSON-RPC / stdio MCP
┌───────────────────────────────▼─────────────────────────────────┐
│                     LightCraft MCP Client                       │
│    (query_photos, render_preview, cmd_develop_set, export, ...) │
└─────────────────────────────────────────────────────────────────┘
```

### Componentes de Software:
1. **`src/tui/`**: Renderizado con `ratatui` y `ratatui-image`. Manejo de eventos de teclado y redibujado sin parpadeos.
2. **`src/mcp/`**: Cliente de transporte MCP asíncrono sobre stdio (`tokio`), serialización de payloads JSON-RPC y tipado seguro de respuestas de LightCraft.
3. **`src/engine/`**: Motor de recetas (composición y combinación de estilos fotográficos), gestión del estado no destructivo.
4. **`src/queue/`**: Cola de trabajos concurrentes para procesamiento en lote, pausar, cancelar y reintentar.
5. **`src/export/`**: Generador de perfiles de exportación (resolución, formato, compresión, plantillas de nombres).
6. **`src/storage/`**: Persistencia local en TOML/JSON de presets personalizados y configuración de usuario.

---

## 4. Requisitos Funcionales

### Modo A: Fotografía Individual
* **RF-A1:** Carga e inspección de imagen (dimensiones, metadatos EXIF, formato).
* **RF-A2:** Generación simultánea de variantes de previsualización (Natural, Cinematográfica, B&W, Teal & Orange, Golden Hour, etc.).
* **RF-A3:** Modos de visualización:
  * Cuadrícula de variantes (Grid View).
  * Vista partida Antes/Después (Split View).
  * Zoom/Detalle del recorte 1:1.
* **RF-A4:** Marcado de favoritas y exportación individual o en conjunto.
* **RF-A5:** Garantía absoluta de no destrucción de la imagen fuente.

### Modo B: Procesamiento por Lote
* **RF-B1:** Selección de archivos múltiples o escaneo recursivo de directorios (e.g. `./DRAW/`).
* **RF-B2:** Filtrado por extensión (`.CR3`, `.jpg`, `.png`, etc.), ratio y orientación.
* **RF-B3:** Dos estrategias de procesamiento:
  * **Uniforme:** Aplicación de una receta maestra a todo el conjunto.
  * **Variaciones:** Generación de un conjunto de $N$ variantes por cada foto.
* **RF-B4:** Cola de trabajo con estimación de tiempo, control de concurrencia y barras de progreso por archivo y global.
* **RF-B5:** Gestión de colisiones de archivos: `Rename`, `Skip`, `Overwrite` (requiere confirmación expresa).

### Gestión de Presets y Recetas
* **RF-P1:** Detección y reutilización de presets nativos del servidor LightCraft.
* **RF-P2:** Biblioteca local de recetas complementarias persistidas en disco (`~/.config/lightcraft-tui/presets/`).
* **RF-P3:** Ajuste de intensidad de preset (slider de 0% a 150%).

---

## 5. Requisitos No Funcionales y Restricciones

* **RNF-1 (Rendimiento):** La interfaz TUI debe responder a 60 FPS sin bloquearse durante llamadas MCP gracias a tareas asíncronas con Tokio channels.
* **RNF-2 (Uso de Memoria):** Previsualizaciones escaladas a resolución de terminal antes de almacenar en memoria; liberación de buffers tras renderizado.
* **RNF-3 (Compatibilidad de Terminal):**
  * Auto-detección de terminal: Kitty Graphics Protocol > Sixel > Halfblocks Unicode fallback.
  * Compatible con Windows Terminal, Ghostty, WezTerm, Alacritty y VS Code integrated terminal.
* **RNF-4 (Seguridad):** Validaciones estrictas de rutas en disco; sin ejecución arbitraria de subprocesos fuera de los binarios autorizados.

---

## 6. Plan de Ejecución (Fases P0 a P6)

| Fase | Alcance | Entregable |
| :--- | :--- | :--- |
| **P0: Diagnóstico & Entorno** | Verificación de Cargo/Rust, auditoría de herramientas MCP de LightCraft y soporte de terminal. | Documento de diagnóstico y esquema de tipos MCP. |
| **P1: Conector MCP & Core** | Cliente MCP en Rust, carga de imagen de prueba y render de preview inicial. | CLI capaz de solicitar y recibir una previsualización vía MCP. |
| **P2: Motor de Variaciones** | Definición de recetas (Cinematográfico, B&W, etc.), aplicación no destructiva. | Generación de múltiples variantes en memoria. |
| **P3: Interfaz TUI Base** | Implementación del layout de 3 paneles con `ratatui` y `ratatui-image`. | TUI navegable con teclado para explorar variantes. |
| **P4: Modo Lote & Cola** | Procesamiento de listas de archivos (ej. carpeta `DRAW/`), cola asíncrona. | Lote en ejecución con barra de progreso interactiva. |
| **P5: Presets & Exportación** | Guardado de presets en TOML, motor de nombrado de archivos y exportador. | Exportaciones a disco verificadas con diferentes perfiles. |
| **P6: Validación & Polish** | Suite de tests, manejo adversarial de errores, documentación final. | Binario final compilado y verificado. |

---

## 7. Criterios de Aceptación

1. **Compilación Limpia:** `cargo build --release` y `cargo test` completan sin warnings críticos ni errores.
2. **Prueba Canon CR3:** Procesa correctamente los archivos de prueba en `./DRAW/` sin alterar los binarios RAW originales.
3. **Persistencia Verificada:** Crear un preset personalizado, reiniciar la aplicación y comprobar que se mantiene en la lista.
4. **Resistencia a Fallos:** Desconectar o pausar el servidor MCP resulta en un mensaje de error claro y reintento en la TUI sin pánico (*panic recovery*).
