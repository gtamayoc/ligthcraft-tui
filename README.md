# LightCraft TUI 📷⚡

[![CI](https://github.com/gtamayoc/ligthcraft-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/gtamayoc/ligthcraft-tui/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg?logo=rust)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/edition-2021-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2021/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green.svg)](LICENSE-MIT)
[![Protocol](https://img.shields.io/badge/MCP-JSON--RPC%202.0-purple.svg)](https://modelcontextprotocol.io/)
[![Architecture](https://img.shields.io/badge/TUI-Ratatui%200.30-red.svg)](https://github.com/ratatui/ratatui)

**LightCraft TUI** es una terminal gráfica interactiva de alto rendimiento y grado profesional para revelado, graduación de color y exportación por lotes de fotografía digital (incluyendo Canon CR3 RAW). Combina un motor de renderizado asíncrono en Rust con integración mediante el protocolo **Model Context Protocol (MCP)** para automatización con agentes de Inteligencia Artificial.

---

## 🌟 Características Principales

- **🖼️ Interfaz TUI de 3 Paneles Sincronizados:**
  - **1. Biblioteca (Library):** Exploración de catálogo con scroll viewport activo y selección de archivos.
  - **2. Previsualizador (Preview Canvas):**
    - Vista Hero principal.
    - Matriz de Variantes 2×2 (`[4]`) y 3×2 (`[6]`) con navegación bidimensional `[↑/↓/←/→]`.
    - Modo Split-View Wiper (`[S]`) para comparativa Antes/Después (Before & After).
    - Modo Lupa 1:1 (`[Z]`) con traslación guiada para inspección de píxeles al 100%.
  - **3. Inspector de Receta y Metadatos:** Histograma de 4 canales en tiempo real (<2ms) y ajuste fino de intensidad de presets.
- **🚀 Exportación Secuencial de Máxima Calidad (`[E]`):**
  - Exportación inmediata de la variante seleccionada con un solo toque:
    - **Archival Master:** TIFF de 16 bits sin pérdida.
    - **Full-Resolution:** JPEG a 100% de calidad.
- **🔒 Integridad Criptográfica Inmutable (Non-Destructive RAW):**
  - Verificación SHA-256 pre-flight y post-flight.
  - Veto estricto a nivel de motor para garantizar **cero modificaciones o sobreescritura accidental** en los archivos RAW maestros (`DRAW/*.CR3`).
- **⚡ Motor Asíncrono Tokio Multihilo:**
  - Despacho reactivo de variantes en segundo plano (`tokio::spawn_blocking`).
  - Protección de timeout preventivo para evitar bloqueos en decodificación de imagen.
- **🤖 MCP Server / Client Bridge:**
  - Soporte para operar conectado a un servidor MCP o en modo autónomo / mock offline.

---

## 🛠️ Comandos de Compilación y Ejecución

### 1. Modo Debug (Desarrollo y Pruebas Rápidas)
Ideal mientras se realizan cambios en el código, con símbolos de depuración completos:

```powershell
# Compilar en modo debug
cargo build

# Ejecutar el TUI interactivo en modo debug
cargo run -- --tui

# Ejecutar con protocolo gráfico Halfblocks (universalmente compatible en Windows Terminal)
cargo run -- --tui --graphics halfblocks

# Ejecutar en modo offline/mock (sin depender del servidor MCP externo)
cargo run -- --tui --mock
```

### 2. Modo Release (Máximo Rendimiento / Producción)
Aplica optimizaciones `opt-level = 3`, LTO y vectorización:

```powershell
# Compilar binario optimizado para release
cargo build --release

# Ejecutar el binario compilado en release
cargo run --release -- --tui

# O ejecutar directamente el ejecutable generado:
.\target\release\lightcraft-tui.exe --tui
```

### 3. Ejecución por CLI (Sin TUI / Automatización Batch)

```powershell
# Verificar la integridad SHA-256 de los archivos RAW en DRAW/
cargo run --release -- --verify-integrity

# Listar presets disponibles (Built-in y de usuario)
cargo run --release -- --list-presets

# Ver detalles de un preset en formato TOML o JSON
cargo run --release -- --show-preset Cinema --format toml

# Exportación secuencial batch de máxima calidad por línea de comandos
cargo run --release -- --sequential-export --output-dir dist/galeria_maxima
```

### 4. Quality Gates y Suite de Pruebas

```powershell
# Verificación rápida de tipos y compilación
cargo check --all-targets

# Ejecutar toda la suite de pruebas unitarias, de integración y adversariales
cargo test

# Ejecutar pruebas específicas del TUI y navegación
cargo test --test tier1_mode_a_tests
cargo test --test tier1_tui_layout_tests

# Análisis de linter (Clippy)
cargo clippy --all-targets
```

---

## ⌨️ Atajos de Teclado en el TUI

| Tecla | Contexto / Panel | Acción |
| :--- | :--- | :--- |
| `[Tab]` | Global | Alternar foco entre paneles (`Biblioteca` ↔ `Previsualizador` ↔ `Inspector`) |
| `[↑] / [↓]` | Biblioteca | Navegar por la lista de fotos del catálogo |
| `[1]` | Previsualizador | Modo **Hero** (Vista principal completa) |
| `[2]` / `[4]` / `[6]` | Previsualizador | Modo **Matriz de Variantes** (2, 4 o 6 slots en cuadrícula) |
| `[↑/↓/←/→]` | Matriz de Variantes | Navegar entre las variantes disponibles (borde resaltado amarillo) |
| `[Enter]` | Matriz de Variantes | Promover la variante seleccionada a receta de trabajo activa |
| `[E]` | Previsualizador / Matriz | **Exportar Variante Enfocada a Máxima Calidad** (TIFF 16b + JPEG 100%) |
| `[S]` | Previsualizador | Modo **Split-View** Antes / Después con divisor wiper |
| `[Z]` | Previsualizador | Modo **Lupa 1:1** (Zoom de inspección de detalle) |
| `[[]` / `[]]` | Inspector | Disminuir / Aumentar intensidad de la receta activa (±5%) |
| `[Q]` | Global | Salir de la aplicación limpiamente |

---

## 🏛️ Arquitectura del Sistema

```
lightcraft-tui/
├── src/
│   ├── main.rs            # Entry point y CLI parser (Clap)
│   ├── app.rs             # Controlador principal y bucle de eventos Ratatui
│   ├── engine/            # Motor de presets, recetas fotográficas y colorimetría
│   ├── export/            # Pipeline de exportación multi-formato (TIFF 16b, JPEG, PNG)
│   ├── graphics/          # Adaptadores de renderizado terminal (Halfblocks, Sixel, Kitty)
│   ├── mcp/               # Cliente/servidor Model Context Protocol (JSON-RPC 2.0)
│   ├── queue/             # Cola de procesamiento por lotes concurrente y resiliente
│   ├── storage/           # Almacenamiento atómico de presets (TOML / JSON)
│   └── tui/               # Componentes visuales, layout responsivo y gestión de eventos
├── DRAW/                  # Directorio protegido de negativos digitales Canon CR3 RAW
└── tests/                 # Suites de pruebas estructuradas por Tiers (1 a 4) y Adversariales
```

---

## 📄 Licencia y Comunidad

- **Licencia:** Distribuido bajo licencia dual [MIT](LICENSE-MIT) o [Apache-2.0](LICENSE-APACHE).
- **Guía de Contribución:** Consulta [CONTRIBUTING.md](CONTRIBUTING.md) para configurar el entorno y enviar PRs.
- **Seguridad:** Revisa nuestra política de divulgación en [SECURITY.md](SECURITY.md).
- **Código de Conducta:** Adherido a [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
- **Historial de Cambios:** Consulta [CHANGELOG.md](CHANGELOG.md).
