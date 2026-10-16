# Guía de Uso, Configuración y Casos de Prueba — LightCraft TUI

Documentación operativa integral del sistema **LightCraft MCP Rust TUI** (`lightcraft-tui`). Esta guía detalla el funcionamiento del sistema, cómo interactuar con su interfaz de terminal (TUI) y su backend por CLI, cómo configurar recetas/presets (incluyendo B/N y estilos para Instagram en PNG), dónde se guardan las exportaciones, cómo verificar el término de procesos y cómo diagnosticar anomalías en los controles de teclado.

---

## 1. Visión General del Sistema

El sistema es una herramienta de automatización fotográfica profesional desarrollada en **Rust** que opera como cliente nativo del motor **LightCraft** a través del protocolo **MCP (Model Context Protocol)**.

### Características Principales:
* **Integridad Garantizada (Inmutabilidad de Fuentes):** Lectura estricta no destructiva. Los archivos originales en `./DRAW/` (como RAW Canon `.CR3` o imágenes base) nunca se modifican ni se sobrescriben (verificación SHA-256).
* **Dos Modos de Operación:**
  * **Modo A (Single Photo):** Inspección detallada, matriz de variantes lado a lado (2, 4 o 6 estilos), vista antes/después (Split/A-B Toggle), histograma y lupa 1:1.
  * **Modo B (Batch Queue):** Cola de procesamiento por lotes para carpetas completas con control de pausas, reintentos y barras de progreso concurrentes.
* **Motor de Exportación Multiformato:** Conversión a **PNG**, **JPEG** y **TIFF** con perfiles de escala (resolución completa, borde largo, bounding box o porcentaje), control de calidad, espacio de color y políticas anticolisión.

---

## 2. Dónde se Guardan las Fotos y Cómo Saber Cuándo Terminó

### 2.1. Rutas de Almacenamiento
* **Directorio por defecto de producción:**  
  `./dist/client_gallery/` (relativo a la raíz del proyecto).
* **Directorio de exportación en TUI o CLI personalizada:**  
  Configurable mediante el parámetro `--output-dir <RUTA>`.  
  *Ejemplo:* `C:\discolocal\PROYECTOS\PYTHON\LIGTHCRAFT\0001\dist\instagram_png\`
* **Estructura de nombres generada:**  
  Sigue la plantilla predeterminada `{name}__{preset}.{ext}` o `{name}__{preset}__{seq:2}.{ext}`.  
  *Ejemplo:* `IMG_0588__B&W High Contrast.png`.
* **Archivos originales protegidos:**  
  Ubicados en `./DRAW/` (permanecen 100% inmutables con hash SHA-256 verificado antes y después).

### 2.2. ¿Cómo Saber Cuándo Terminó el Proceso?
1. **En la TUI (Interfaz de Terminal):**
   * En la **barra de estado inferior** se muestra el indicador en tiempo real:  
     `Batch: IMG_0588.CR3 (100.0%) - [X/X]` o mensaje `Ready. [Tab] Focus | [F10] Mode A/B | [Space] Split | [E]xport`.
   * En el panel de **Modo B (Cola de Lotes)**, el estado de cada trabajo cambia de `Queued` -> `Processing { progress_pct: 100 }` -> `Done`.
2. **Vía Línea de Comandos (CLI / Automatización):**
   * En la terminal aparecerá la confirmación final:
     ```text
     ------------------------------------------------------------
     SUCCESS: Generated X production images (Y.YY MB) into dist/client_gallery
     Executing post-flight source immutability verification...
       [PASS] IMG_0588.CR3 | sha256 verified
     SUCCESS: 100% byte-for-byte immutability confirmed across all 5 master files.
     ```
   * El código de salida del proceso (`ExitCode`) es `0` (Success).

---

## 3. Guía de Interfaz (TUI): Atajos de Teclado y Comportamiento Especial

La interfaz está construida con **Ratatui** y **Crossterm** con un loop a 60 FPS.

> [!WARNING]
> ### Explicación del "Bug" de Pulsar 2 Veces (Doble Evento de Teclado en Windows)
> En consolas de Windows (PowerShell / Windows Terminal / CMD), Crossterm reporta por defecto eventos tanto de **`KeyEventKind::Press`** (tecla pulsada) como de **`KeyEventKind::Release`** o repeticiones de entrada si no se filtran estrictamente por tipo de evento en el loop asíncrono.
> * **Efecto observado:** Al presionar una tecla, el sistema puede capturar dos eventos seguidos (presionar y soltar), haciendo que una acción toggle (como A/B o selección) cambie y regrese inmediatamente al estado anterior, o que avance 2 posiciones.
> * **Solución operativa mientras se usa la TUI:** Pulsa las teclas con un toque breve y seco. Si alternas modos (`Tab`, `F10` o `\`), presiona con firmeza o utiliza las opciones CLI automáticas para pruebas repetitivas sin fricción de entrada.

### 3.1. Navegación Global
| Tecla / Atajo | Acción |
| :--- | :--- |
| `Tab` | Cambiar foco entre paneles: **Biblioteca** (izquierda) ➔ **Lienzo Preview** (centro) ➔ **Presets** (derecha) |
| `Shift + Tab` (`BackTab`) | Foco en reversa |
| `F10` o `m` / `M` | Alternar entre **Modo A** (Foto individual) y **Modo B** (Cola de lotes) |
| `q` o `Ctrl + C` | Salir de la aplicación de forma segura |

### 3.2. Controles en Modo A (Foto Individual)
* **Con foco en la Biblioteca (panel izquierdo):**
  * `↑` / `k`: Foto anterior.
  * `↓` / `j`: Siguiente foto.
  * `Espacio`: Marcar/desmarcar foto seleccionada para lote.
* **Con foco en el Lienzo de Previsualización (panel central):**
  * `1`: Vista **Hero** (imagen principal a pantalla completa).
  * `2`: Matriz de **2 variantes** simultáneas.
  * `4` o `v` / `V`: Matriz de **4 variantes** (2x2 cuadrícula comparativa).
  * `6`: Matriz de **6 variantes** simultáneas.
  * `s` / `S`: Vista dividida Antes/Después (**Split View** lado a lado).
  * `\` (Barra invertida): Alternar instantáneamente **A/B** (Antes vs Después con receta activa).
  * `z` / `Z`: Zoom Lupa 1:1 (**Loupe View** para comprobar nitidez de enfoque).
  * `h`, `j`, `k`, `l` (o flechas): Desplazar el visor (panorámica) dentro del zoom 1:1.
  * `←` / `→` (en matriz de variantes): Mover el selector entre las recetas calculadas.
  * `Enter`: Promover la variante seleccionada como receta maestra activa.
  * `[` / `]`: Disminuir o aumentar intensidad del preset en pasos de 5% (de 0% a 150%).
  * `e` / `E`: Exportar foto actual con la receta activa.
  * `p` / `P`: Marcar foto como Aceptada (**Pick**).
  * `x` / `X`: Marcar foto como Rechazada (**Reject**).
  * `u` / `U`: Quitar marcas (**Unflagged**).

### 3.3. Controles en Modo B (Procesamiento por Lotes)
* `↑` / `k` y `↓` / `j`: Navegar por la lista de trabajos en cola.
* `Espacio`: Pausar / Reanudar la ejecución de la cola de exportación.
* `c` / `C`: Cancelar la cola activa.
* `r` / `R`: Reintentar trabajos fallidos.

---

## 4. Caso Práctico: Exportar para Instagram (10 Estilos en PNG)

Para publicar en Instagram necesitas imágenes optimizadas en formato PNG, con recorte o límite de borde largo (usualmente **1080px** o **2048px** para mantener la máxima fidelidad) y una variedad estética que incluya Blanco y Negro y otros acabados estilizados.

### 4.1. Catálogo de Estilos y Presets Disponibles

El sistema integra recetas maestras de revelado (y admite presets personalizados en TOML/JSON):

1. **`bw-high-contrast` (B&W High Contrast):** Blanco y negro de alto impacto con curva tonal profunda, negros densos y claridad acentuada.
2. **`bw-soft-portrait` (B&W Soft / Tonos Medios):** Tratamiento monocromático suave con grano analógico sutil y reducción de microcontraste.
3. **`bw-film-noir` (B&W Film Noir):** Viñeteado clásico, sombras comprimidas y altas luces controladas.
4. **`natural` (Natural):** Colorimetría limpia, fidelidad de tonos de piel, microcontraste y enfoque de exportación.
5. **`cinema` (Cinematic Look):** Verdes desaturados, tonos cálidos en piel, sombras azuladas y rolloff suave en altas luces.
6. **`teal-orange` (Teal & Orange):** Separación complementaria icónica: sombras cian/turquesa y pieles doradas cálidas.
7. **`golden-hour` (Golden Hour):** Temperatura cálida (+1000K), sombras levantadas con resplandor dorado en reflejos.
8. **`vintage-fade` (Vintage / Retro Fade):** Punto negro elevado (sombras lavadas estilo película química) y ligero tinte sepia.
9. **`urban-moody` (Urban Moody):** Darks profundos, saturación selectiva (resaltando amarillos y rojos) y textura marcada.
10. **`clean-minimal` (Clean Minimal / Modern Pastel):** Tonos pasteles, altas luces aireadas y baja saturación general.

### 4.2. Cómo crear un Preset Personalizado en TOML
Los presets de usuario se guardan en el directorio local de presets (`~/.config/lightcraft-tui/presets/` o `--presets-dir ./presets/`).

Ejemplo de archivo de preset (`presets/bw-film-noir.toml`):
```toml
[metadata]
id = "bw-film-noir"
name = "B&W Film Noir"
description = "Monocromo con negros profundos y viñeteado de época para redes sociales"
version = 1
author = "Usuario"
category = "Monochrome"
created_at = "2026-10-15T12:00:00Z"
modified_at = "2026-10-15T12:00:00Z"
is_builtin = false
recommended_intensity = 100.0

[recipe]
name = "B&W Film Noir"
treatment = "blackAndWhite"
exposure = -0.15
contrast = 35.0
highlights = -10.0
shadows = -20.0
whites = 15.0
blacks = -30.0
clarity = 15.0
dehaze = 10.0

[recipe.grain]
amount = 25.0
size = 30.0
roughness = 50.0

[recipe.vignette]
amount = -35.0
midpoint = 40.0
roundness = 0.0
feather = 60.0
```

---

## 5. Batería de Pruebas (Test Cases) para Validar el Sistema

Para descartar fallos, confirmar que los módulos funcionan y verificar que las exportaciones se generan correctamente en disco, ejecuta la siguiente suite de pruebas sistemáticas.

### Caso de Prueba 1: Prueba de Integridad Cero-Destrucción (RAW Baseline)
* **Objetivo:** Verificar que el sistema lee los archivos RAW (`.CR3` o fuentes) sin tocarlos ni corromperlos.
* **Comando:**
  ```powershell
  cargo run -- --verify-integrity
  ```
* **Resultado Esperado:**
  Todos los archivos de `./DRAW/` deben mostrar `[PASS]` y un hash SHA-256 inmutable de 64 caracteres hexadecimales.

### Caso de Prueba 2: Verificación de Handshake y Conexión MCP
* **Objetivo:** Confirmar que el cliente Rust puede comunicarse con el motor LightCraft y listar sus herramientas fotográficas.
* **Comando:**
  ```powershell
  cargo run -- --check-mcp --mock
  ```
* **Resultado Esperado:**
  Imprime `Discovered 14 MCP photographic tools:` listando `import`, `query_photos`, `render_preview`, `cmd_develop_set`, `export`, etc., finalizando con `SUCCESS`.

### Caso de Prueba 3: Consulta y Serialización de Presets
* **Objetivo:** Comprobar la carga del catálogo de estilos y serialización en TOML/JSON.
* **Comandos:**
  ```powershell
  # Listar presets disponibles
  cargo run -- --list-presets

  # Consultar detalle en TOML
  cargo run -- --show-preset bw-high-contrast --format toml

  # Consultar detalle en JSON
  cargo run -- --show-preset teal-orange --format json
  ```
* **Resultado Esperado:**
  Muestra una tabla con los IDs de presets (`bw-high-contrast`, `cinema`, etc.) y el volcado de la receta sin errores de parseo.

### Caso de Prueba 4: Carga de Producción Masiva (Workload Real con RAW Canon CR3)
* **Objetivo:** Procesar las imágenes de `./DRAW/` con 4 recetas y en 3 formatos distintos (JPEG, PNG y TIFF de 16 bits), verificando dimensiones y bytes escritos.
* **Comando:**
  ```powershell
  cargo run -- --production-workload --output-dir dist/test_run
  ```
* **Resultado Esperado:**
  1. Supera el pre-flight check de integridad.
  2. Genera 60 archivos en `dist/test_run/` con nombres formateados (`{name}__{preset}.{ext}`).
  3. Supera el post-flight check verificando que ningún archivo en `./DRAW/` fue alterado.

### Caso de Prueba 5: Prueba de Formato Único (Solo PNG para Instagram)
* **Objetivo:** Validar la generación de un único formato (PNG) para una foto individual con resolución ajustada a 2048px en el borde largo.
* **Comportamiento en Pipeline:**
  * Formato: `OutputFormat::Png`
  * Profundidad: `BitDepth::Eight` (8 bits por canal sRGB estándar)
  * Escalado: `ScalingProfile::LongEdge { max_long_edge: 2048, dont_enlarge: true }`
* **Verificación de archivos:**
  ```powershell
  Get-ChildItem -Path dist\test_run\*.png
  ```
  Comprobar que las imágenes se abren correctamente y tienen relación de aspecto idéntica a la original.

### Caso de Prueba 6: Suite Completa de Tests Automatizados de Rust
* **Objetivo:** Ejecutar la batería de pruebas unitarias y de integración del proyecto (módulos de naming, collision, MCP, presets y export).
* **Comando:**
  ```powershell
  cargo test
  ```
* **Resultado Esperado:**
  Todos los tests deben reportar `test result: ok`.

---

## 6. Diagnóstico de Problemas Frecuentes (Troubleshooting)

| Síntoma | Causa Raíz | Solución |
| :--- | :--- | :--- |
| Al presionar una tecla en la TUI salta 2 elementos | En terminales Windows, Crossterm procesa tanto `KeyPress` como `KeyRelease` en ráfaga rápida. | Efectúa pulsaciones breves. Para pruebas automatizadas de exportación, utiliza los comandos directos por CLI (`--production-workload`). |
| Error `PathTraversal` al exportar | La ruta solicitada contiene `..` o caracteres inválidos de Windows. | Especifica rutas limpias y absolutas o relativas directas (ejemplo: `dist/export_ig`). |
| "Live lightcraft-cli.exe not found on system" | El binario ejecutable externo de LightCraft no está en el PATH del sistema. | La aplicación pasa automáticamente a modo **Mock**, permitiendo renderizar previews y exportar usando el motor interno de Rust sin interrumpir la sesión. |
| La imagen en la terminal se ve como caracteres de bloque | La terminal no soporta Kitty Graphics Protocol ni Sixel. | Es el comportamiento normal de fallback mediante **Halfblocks Unicode**. Para gráficos de alta resolución nativos en terminal, utiliza emuladores compatibles como Ghostty o WezTerm. |

---

## 7. Resumen de Comandos Rápidos de Verificación

```powershell
# 1. Verificar integridad de las fotos de origen
cargo run -- --verify-integrity

# 2. Comprobar catálogo de presets disponibles
cargo run -- --list-presets

# 3. Lanzar TUI interactiva
cargo run -- --tui

# 4. Lanzar TUI forzando modo simulación (offline)
cargo run -- --tui --mock

# 5. Ejecutar exportación de lote a carpeta personalizada
cargo run -- --production-workload --output-dir dist/instagram_png
```
