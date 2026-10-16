# Guía de Uso, Manual de Usuario y Casos de Prueba — LightCraft TUI

Documentación técnica y manual operativo integral del sistema **LightCraft MCP Rust TUI** (`lightcraft-tui`). Esta guía cubre la arquitectura del sistema, manual de usuario paso a paso, navegación por paneles (con teclado y ratón), resolución de eventos de teclado, gestión asíncrona de variantes con timeouts de seguridad, exportación secuencial dual en máxima calidad (TIFF 16-bit + JPEG/PNG), presets fotográficos (incluyendo estilos para Instagram y Blanco/Negro), y una suite completa de casos de prueba.

---

## 1. Visión General y Arquitectura

El sistema es un procesador y automatizador fotográfico profesional desarrollado en **Rust**, diseñado para interactuar de forma nativa mediante **Model Context Protocol (MCP)** con el motor de revelado de **LightCraft**.

```
┌─────────────────────────────────────────────────────────────────┐
│                     TUI Interface (ratatui)                     │
│   [Header: Modos]  │  [Panel 1: Library] │  [Panel 2: Canvas]   │
│   [Panel 3: Inspector] │  [Bottom: Status Bar & Dynamic Guide]  │
└───────────────────────────────▲─────────────────────────────────┘
                                │ Event Loop 60 FPS (crossterm)
┌───────────────────────────────▼─────────────────────────────────┐
│                    Application Core Engine                      │
│  ┌──────────────────────┐               ┌────────────────────┐  │
│  │ Recipe/Preset Engine │               │  Job & Batch Queue │  │
│  └──────────────────────┘               └────────────────────┘  │
│  ┌──────────────────────┐               ┌────────────────────┐  │
│  │ Image Preview Cache  │               │   Export Pipeline  │  │
│  └──────────────────────┘               └────────────────────┘  │
└───────────────────────────────▲─────────────────────────────────┘
                                │ JSON-RPC / stdio MCP
┌───────────────────────────────▼─────────────────────────────────┐
│                     LightCraft MCP Server                       │
│    (query_photos, render_preview, set_develop, export, ...)     │
└─────────────────────────────────────────────────────────────────┘
```

### Pilares Fundamentales:
1. **Inmutabilidad Cero-Destrucción (RAW Baseline):** Los archivos fotográficos maestros en `./DRAW/` (como Canon RAW `.CR3` o fuentes de alta resolución) permanecen estrictamente inmutables con comprobación criptográfica SHA-256.
2. **Arquitectura TUI Desacoplada (60 FPS):** El renderizado nunca se bloquea por la descompresión de imágenes ni por llamadas RPC. Las tareas pesadas corren en subprocesos con canales Tokio MPSC.
3. **Modos Duales Sin Pérdida de Estado:** Alternancia fluida con `F10` o `m` entre el **Modo A** (Inspección y variantes individuales) y el **Modo B** (Cola de procesamiento por lotes).

---

## 2. Novedades y Mejoras Recientes en el Repositorio

A raíz de los requerimientos de usabilidad e incidencias detectadas en Windows, el repositorio incorpora las siguientes optimizaciones críticas:

### 2.1. Eliminación del "Doble Salto" de Teclado en Windows
* **Causa Raíz:** En consolas de Windows (PowerShell, Windows Terminal, CMD), la API de terminal emite dos eventos por tecla: `KeyEventKind::Press` (al presionar) y `KeyEventKind::Release` (al soltar). Esto provocaba que una sola pulsación disparara dos acciones consecutivas.
* **Solución Implementada:** El bucle de eventos (`handle_key`) filtra y descarta explícitamente cualquier evento `KeyEventKind::Release`. Ahora cada pulsación se procesa de forma **estrictamente unitaria**, permitiendo una navegación suave y predecible.

### 2.2. Resaltado de Foco y Scroll Sincronizado 1:1
* **Panel de Biblioteca:** Se implementó una ventana de desplazamiento (*viewport window*). El elemento seleccionado (`selected_index`) permanece siempre visible y centrado en pantalla, evitando que el foco quede oculto fuera del marco.
* **Indicadores Visuales Claros:**
  * **Elemento Seleccionado + Panel Enfocado:** Resaltado en **Amarillo Negrita** con cursor `►`.
  * **Elemento Seleccionado (Panel sin Foco):** Resaltado en **Cian Negrita** (sabes qué foto está activa mientras ajustas controles en otro panel).
  * **Marco de Paneles Activos:** El panel enfocado muestra un borde grueso de doble línea (`BorderType::Thick`) en color **Cian** con la etiqueta `❯ [ACTIVO] ❮`.

### 2.3. Generación Asíncrona de Variantes con Timeout Watchdog (Anti-Cuelgues)
* **Antes:** La matriz de variantes podía quedar congelada en "cargando variantes..." si la llamada gráfica tardaba o fallaba silenciosamente.
* **Ahora:** Cada variante en la matriz se procesa en una tarea asíncrona dedicada (`tokio::spawn`) con un **temporizador de seguridad de 10 segundos**.
* **Estados Visuales en Pantalla:**
  * `⏳ Pendiente`: En cola para renderizado.
  * `⟳ Generando...`: Procesando activamente con medición de tiempo.
  * `✔ Listo`: Renderizado completado con éxito.
  * `✖ Error`: Muestra el mensaje exacto de fallo o timeout si supera los 10 segundos, sin congelar la interfaz.
* **Insignia en Barra de Estado:** Muestra en tiempo real el progreso de las variantes: `[Var: X/Y]` (ej. `[Var: 4/4]`).

### 2.4. Exportación Inmediata en Máxima Calidad con Tecla `[E]`
* Al pulsar `e` o `E` en el Modo A, el sistema exporta de inmediato la variante o foto activa con los perfiles maestros de máxima fidelidad sin necesidad de configurar diálogos intermedios:
  1. **TIFF de 16 bits:** Calidad 100%, escala nativa (archival master sin compresión destructiva).
  2. **JPEG / PNG:** Calidad 100%, escala nativa para previsualización y entrega rápida.
* **Retroalimentación Inmediata:** La barra de estado reporta el tamaño en megabytes escritos en disco:  
  `Guardado OK: IMG_0588.CR3 [Variante 2 (Cinema)] (TIFF 16b: 42.1MB, JPEG 100%: 3.8MB)`.

---

## 3. Dónde se Guardan las Fotos y Cómo Saber Cuándo Terminó

### 3.1. Rutas de Almacenamiento

| Tipo de Exportación | Ubicación en Disco | Propósito |
| :--- | :--- | :--- |
| **Exportación Rápida Interactiva (`[E]`)** | `./export/interactive/` | Archivos maestros generados al presionar `E` en la TUI (TIFF 16-bit + JPEG/PNG). |
| **Exportación de Producción / Lotes** | `./dist/client_gallery/` | Salida predeterminada para el procesado masivo o por CLI (`--production-workload`). |
| **Ruta Personalizada** | Configurable vía `--output-dir <DIR>` | Cualquier carpeta que elijas (ej. `./dist/instagram_png/`). |
| **Archivos Originales Protegidos** | `./DRAW/` | Fuentes originales (CR3/JPEG). **Nunca se modifican ni se escriben allí exportaciones.** |

### 3.2. Estructura y Plantilla de Nombres
Los archivos siguen el patrón estructurado:
```text
{name}__{preset}.{ext}
```
*Ejemplos concretos generados en disco:*
* `IMG_0588__B&W High Contrast.png`
* `IMG_0588__Cinema.jpg`
* `IMG_0588__Natural_TIFF16.tif`

Tokens admitidos por el motor de nombrado:
* `{name}`: Nombre base del archivo original (sin extensión).
* `{preset}`: Nombre descriptivo de la receta o preset aplicado.
* `{seq:2}`: Número secuencial formateado a $N$ dígitos con ceros iniciales.
* `{date}`: Fecha de captura EXIF UTC (`YYYY-MM-DD`).
* `{ext}`: Extensión según el contenedor (`png`, `jpg`, `tif`).

### 3.3. ¿Cómo Saber Cuándo Terminó el Proceso?
1. **En la TUI (Modo A - Foto o Variante Individual):**
   * Al pulsar `E`, la barra inferior mostrará transitoriamente:  
     `Exportando IMG_0588.CR3 [Variante X] en máxima calidad...`
   * Al terminar (generalmente 1 a 3 segundos), cambiará a:  
     `Guardado OK: IMG_0588.CR3 [Variante X] (TIFF 16b: XX.XMB, JPEG 100%: X.XMB)`
2. **En la TUI (Modo B - Cola de Lotes):**
   * La barra de estado indica el porcentaje global y archivo actual:  
     `Batch: IMG_0589.CR3 (80.0%) - [4/5]`
   * Cuando todos los ítems están procesados, la cola se detiene y la barra reporta `Ready.`
3. **Vía Línea de Comandos (CLI Headless):**
   * Finaliza con un reporte en consola indicando cantidad de archivos y megabytes totales:
     ```text
     SUCCESS: Generated 60 production images (184.20 MB) into dist/client_gallery
     SUCCESS: 100% byte-for-byte immutability confirmed across all 5 master files.
     ```
   * Código de retorno del sistema: `ExitCode::SUCCESS` (`0`).

---

## 4. Manual de Usuario: Interfaz TUI Paso a Paso

### 4.1. Distribución de los Paneles (Layout)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ LightCraft TUI v0.1.0 │ Modo: SINGLE PHOTO (A) │ Protocolo: Auto (Halfblocks)│
├─────────────────────┬───────────────────────────────┬────────────────────────┤
│ 1. BIBLIOTECA       │ 2. PREVISUALIZADOR            │ 3. INSPECTOR           │
│ (RAW/Imágenes)      │ (Lienzo Central de Variantes) │ (Metadatos y Sliders)  │
│                     │                               │                        │
│ ► [x] IMG_0588.CR3  │   [Hero / 2x2 Grid / Split]   │ Foto: IMG_0588.CR3     │
│   [ ] IMG_0589.CR3  │                               │ 6000x4000 (24.0 MP)    │
│   [ ] IMG_0590.CR3  │   Variant 1  │  Variant 2     │ 1/500s  f/2.8  ISO 400 │
│   [ ] IMG_0969.CR3  │   ───────────┼───────────     │ Preset: Cinema         │
│   [ ] IMG_0981.CR3  │   Variant 3  │  Variant 4     │ Intensidad: 100%       │
├─────────────────────┴───────────────────────────────┴────────────────────────┤
│ Foto 1/5 │ Panel Activo: [1. BIBLIOTECA] │ [Tab]: Ir a Canvas │ [E]: Exportar │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 4.2. Cómo Cambiar el Foco entre Paneles
Puedes cambiar el foco de dos maneras:
1. **Teclado:** Pulsa `Tab` para avanzar cíclicamente entre `1. BIBLIOTECA` ➔ `2. PREVISUALIZADOR` ➔ `3. INSPECTOR`. Pulsa `Shift + Tab` (`BackTab`) para ir en sentido inverso.
2. **Ratón:** Haz clic izquierdo dentro de cualquiera de los 3 paneles. El sistema detecta las coordenadas exactas y activa el panel clicado inmediatamente.

### 4.3. Flujo Operativo Completo en Modo A (Inspección y Variantes)

#### Paso 1: Seleccionar la Imagen de Trabajo
* Haz foco en el panel **1. BIBLIOTECA** (`Tab` o clic).
* Usa `↑` o `k` para subir, y `↓` o `j` para bajar.
* Nota cómo el cursor `►` y el texto en amarillo siguen exactamente tu movimiento, y la lista realiza auto-scroll cuando hay más archivos de los que caben en pantalla.
* *(Opcional)* Pulsa `Espacio` en la biblioteca para marcar la foto para procesamiento posterior en lote (`[x]`).

#### Paso 2: Explorar Diferentes Variantes Visuales
* Pasa el foco al **2. PREVISUALIZADOR** (`Tab` o clic).
* Elige el modo de visualización:
  * Pulsa `1`: Modo **Hero** (imagen en solitario a pantalla completa).
  * Pulsa `4` (o `v`): Modo **Matriz 2x2** (4 variantes simultáneas).
  * Pulsa `6`: Modo **Matriz 3x2** (6 variantes simultáneas).
  * Pulsa `2`: Modo **Matriz de 2 variantes**.
* El sistema dispara en segundo plano el renderizado de cada estilo. Verás el estado pasar de `⟳ Generando...` a `✔ Listo`.

#### Paso 3: Comparar y Elegir una Variante
* En la matriz de variantes, usa las flechas `←`, `→`, `↑`, `↓` (o `h`, `j`, `k`, `l`) para moverte entre las miniaturas.
* El recuadro enfocado se marcará con borde grueso en **Amarillo/Cian**.
* Pulsa `Enter`: La variante enfocada se convierte en la **receta maestra activa**.
* Pulsa `\` (Barra invertida): Realiza un cambio instantáneo **A/B** (Antes vs Después) para evaluar el impacto del revelado sobre la foto original.
* Pulsa `s`: Activa la pantalla dividida (**Split View** lado a lado).

#### Paso 4: Ajustar Intensidad o Calificar
* Pulsa `[` para bajar la intensidad del preset (de 5% en 5%).
* Pulsa `]` para subir la intensidad (hasta 150% para un efecto más dramático).
* Pulsa `Espacio` (con foco en el Canvas) para marcar la variante como favorita (`★`).
* Pulsa `p` para marcar como Aceptada (**Pick**), `x` para Rechazar (**Reject**), o `u` para quitar marcas (**Unflagged**).

#### Paso 5: Inspeccionar Enfoque y Nitidez (Lupa 1:1)
* Pulsa `z`: Entra al modo **1:1 Loupe Zoom**.
* Usa `h`, `j`, `k`, `l` o las flechas de dirección para desplazarte (panorámica) sobre la imagen a escala real y comprobar los detalles en ojos, texturas o grano.
* Pulsa `z` o `1` nuevamente para regresar a la vista general.

#### Paso 6: Exportación Inmediata
* Con la foto o variante deseada seleccionada, pulsa `e` o `E`.
* El sistema generará los archivos en máxima calidad en `./export/interactive/` y te confirmará el éxito en la barra de estado.

---

### 4.4. Flujo Operativo en Modo B (Procesamiento por Lotes)

1. **Entrar al Modo B:** Pulsa `F10` o la tecla `m`.
2. **Revisar la Cola:** Verás todos los trabajos en cola con su ID, archivo origen, receta y estado.
3. **Pausar / Reanudar:** Pulsa `Espacio` si necesitas liberar CPU momentáneamente.
4. **Cancelar:** Pulsa `c` para detener la cola.
5. **Reintentar:** Pulsa `r` si algún trabajo dio error y deseas reintentarlo.
6. **Volver a Modo A:** Pulsa `F10` o `m` en cualquier momento (todo el estado de la sesión se preserva intacto).

---

## 5. Tabla Maestra de Atajos de Teclado (Cheat Sheet)

### Globales
| Atajo | Función |
| :--- | :--- |
| `Tab` | Avanzar foco: Biblioteca ➔ Lienzo Previsualizador ➔ Inspector |
| `Shift + Tab` | Retroceder foco de panel |
| `F10` o `m` / `M` | Alternar entre Modo A (Single) y Modo B (Batch) |
| `q` o `Ctrl + C` | Salir de LightCraft TUI |

### En Panel 1: Biblioteca
| Atajo | Función |
| :--- | :--- |
| `↑` o `k` | Foto anterior (con scroll automático de lista) |
| `↓` o `j` | Siguiente foto (con scroll automático de lista) |
| `Espacio` | Marcar / desmarcar foto para procesamiento por lote (`[x]`) |

### En Panel 2: Previsualizador (Modo A)
| Atajo | Función |
| :--- | :--- |
| `1` | Vista Hero (previsualización principal completa) |
| `2` | Matriz de 2 variantes |
| `4` o `v` / `V` | Matriz 2x2 (4 variantes simultáneas) |
| `6` | Matriz 3x2 (6 variantes simultáneas) |
| `←`, `→`, `↑`, `↓` | Navegación entre las celdas de la matriz de variantes |
| `Enter` | Promover variante seleccionada como receta maestra activa |
| `s` / `S` | Vista dividida Antes/Después (Split View lado a lado) |
| `\` | Alternar instantáneamente A/B (Original vs Con Receta) |
| `z` / `Z` | Alternar Lupa de Enfoque 1:1 (Loupe Zoom) |
| `h`, `j`, `k`, `l` | Desplazar viewport (panorámica) en la Lupa 1:1 |
| `Espacio` | Alternar favorita (`★`) en la variante seleccionada |
| `p` / `P` | Clasificar foto como Aceptada (`[P]`) |
| `x` / `X` | Clasificar foto como Rechazada (`[X]`) |
| `u` / `U` | Limpiar clasificación (`Unflagged`) |
| `[` / `]` | Modificar intensidad de la receta (-5% / +5%) |
| `e` / `E` | **Exportar en máxima calidad (TIFF 16b + JPEG/PNG) a `./export/interactive/`** |

### En Modo B (Cola de Lotes)
| Atajo | Función |
| :--- | :--- |
| `↑` / `k` y `↓` / `j` | Navegar por la tabla de trabajos encolados |
| `Espacio` | Alternar Pausa / Reanudación de la cola |
| `c` / `C` | Cancelar todos los trabajos pendientes de la cola |
| `r` / `R` | Reintentar únicamente los trabajos que hayan fallado |

---

## 6. Caso de Uso Práctico: Exportación para Instagram (Solo PNG y 10 Estilos)

Para Instagram se busca mantener máxima nitidez en compresión de feeds y stories, evitando artefactos JPEG mediante **PNG**, escalando a un ancho óptimo de **1080px** o **2048px** en el borde largo.

### 6.1. Los 10 Estilos Recomendados (B/N y Color)

1. **`bw-high-contrast` (B&W High Contrast):** Blanco y negro de alto impacto con curva tonal en S, negros puros y textura pronunciada en ropa y piel.
2. **`bw-soft-portrait` (B&W Suave Tonos Medios):** Retrato clásico con negros suaves, altas luces aireadas y grano fino cinematográfico.
3. **`bw-film-noir` (B&W Film Noir):** Viñeteado denso, sombras profundas y contraste angular para fotografía callejera y arquitectura.
4. **`natural` (Natural Master):** Revelado limpio y neutro, respetando la colorimetría de pieles y paisaje con corrección cromática.
5. **`cinema` (Cinematic Look):** Verdes oliva atenuados, tonos de piel cobrizos y sombras frías azuladas estilo largometraje.
6. **`teal-orange` (Teal & Orange):** Clásico contraste complementario: cielos y sombras turquesa, reflejos y tonos cálidos en el sujeto.
7. **`golden-hour` (Golden Hour / Atardecer):** Desplazamiento de temperatura cálida (+1000K), sombras abiertas con halo dorado en los bordes.
8. **`vintage-fade` (Vintage Matte):** Curva tonal con punto negro levantado (sombras lavadas y mate) con toque cálido analógico.
9. **`urban-moody` (Urban Moody):** Desaturación selectiva de verdes y azules, resaltando acentos amarillos y rojos para street photography.
10. **`clean-minimal` (Clean Pastel):** Tonos pasteles, altas luces muy limpias, bajo contraste general para composiciones arquitectónicas y minimalistas.

### 6.2. Creación de Preset en Formato TOML
Puedes añadir cualquiera de estos estilos creando un archivo `.toml` en la carpeta de presets (por defecto `~/.config/lightcraft-tui/presets/` o pasando `--presets-dir <DIR>`):

Ejemplo: `presets/instagram-bw-noir.toml`
```toml
[metadata]
id = "instagram-bw-noir"
name = "Instagram B&W Noir"
description = "Monocromo con sombras profundas y textura optimizado para feed vertical de IG"
version = 1
author = "Fotografo"
category = "BlackAndWhite"
created_at = "2026-10-16T12:00:00Z"
modified_at = "2026-10-16T12:00:00Z"
is_builtin = false
recommended_intensity = 100.0

[recipe]
name = "Instagram B&W Noir"
treatment = "blackAndWhite"
exposure = -0.10
contrast = 38.0
highlights = -15.0
shadows = -25.0
whites = 20.0
blacks = -35.0
clarity = 18.0
dehaze = 12.0

[recipe.grain]
amount = 22.0
size = 28.0
roughness = 45.0

[recipe.vignette]
amount = -30.0
midpoint = 45.0
roundness = 0.0
feather = 65.0
```

---

## 7. Batería de Pruebas y Diagnóstico (Test Cases)

Ejecuta estas pruebas para descartar fallos y validar que la instalación y el motor de revelado funcionan correctamente en cada escenario.

### Caso de Prueba 1: Verificación de Inmutabilidad de Fuentes (RAW Check)
* **Objetivo:** Asegurar que ninguna operación de lectura o renderizado altera los binarios fuente en `./DRAW/`.
* **Comando:**
  ```powershell
  cargo run -- --verify-integrity
  ```
* **Salida esperada:**
  ```text
  [PASS] IMG_0588.CR3 | 26527598 bytes | sha256: 09cfa35882260ff0...
  [PASS] IMG_0589.CR3 | 28369647 bytes | sha256: ecfcbf34289bbdc7...
  SUCCESS: All 5 Canon CR3 master files verified byte-for-byte against SHA-256 baseline.
  ```

### Caso de Prueba 2: Handshake y Detección de Herramientas MCP
* **Objetivo:** Verificar que el cliente Rust puede conectarse al motor LightCraft y listar sus 14 herramientas.
* **Comando:**
  ```powershell
  cargo run -- --check-mcp --mock
  ```
* **Salida esperada:**
  Lista las herramientas fotográficas (`import`, `query_photos`, `set_develop`, `render_preview`, `export`, etc.) y termina en `SUCCESS: MCP connection and tool discovery verified`.

### Caso de Prueba 3: Consulta y Validación de Presets
* **Objetivo:** Comprobar que el motor lee y serializa correctamente las recetas en TOML y JSON.
* **Comandos:**
  ```powershell
  # Listar todos los presets registrados
  cargo run -- --list-presets

  # Consultar preset en TOML
  cargo run -- --show-preset bw-high-contrast --format toml

  # Consultar preset en JSON
  cargo run -- --show-preset cinema --format json
  ```
* **Salida esperada:**
  Muestra la tabla del catálogo y las especificaciones numéricas de sliders sin errores de parsing.

### Caso de Prueba 4: Carga de Producción Masiva (Workload Real)
* **Objetivo:** Procesar los archivos Canon CR3 en 4 presets y 3 formatos simultáneos (JPEG, PNG y TIFF 16-bit).
* **Comando:**
  ```powershell
  cargo run -- --production-workload --output-dir dist/test_instagram
  ```
* **Salida esperada:**
  Genera 60 imágenes en `dist/test_instagram/` y ejecuta inmediatamente la verificación SHA-256 posterior confirmando 100% de inmutabilidad en los archivos de `./DRAW/`.

### Caso de Prueba 5: Prueba de la Suite de Tests Automatizados de Rust
* **Objetivo:** Ejecutar las pruebas unitarias y de integración del motor, verificando el layout, eventos de teclado y filtros contra el doble trigger.
* **Comandos recomendados:**
  ```powershell
  # 1. Tests unitarios del núcleo (librería)
  cargo test --lib

  # 2. Tests de layout, foco y eventos de teclado
  cargo test --test tier1_tui_layout_tests

  # 3. Tests de integridad y sandboxing
  cargo test --test tier2_boundary_tests
  ```
* **Salida esperada:**
  Todos los casos deben mostrar `test result: ok`.

---

## 8. Diagnóstico de Problemas Frecuentes (FAQ & Troubleshooting)

### P: ¿Por qué antes parecía que tenía que pulsar dos veces una tecla o saltaba dos fotos?
**R:** En Windows, la consola enviaba el evento de soltar la tecla (`KeyRelease`). Con la versión actualizada en el repositorio, la aplicación ignora explícitamente los eventos `Release`. Ahora solo responde a pulsaciones (`Press`), eliminando cualquier comportamiento errático.

### P: ¿Qué pasa si al generar variantes una imagen se demora o se bloquea?
**R:** El sistema cuenta con un *watchdog* que monitorea cada variante. Si el renderizado supera los 10 segundos, cancela la tarea, libera el hilo y marca la variante como `✖ Error: Tiempo de espera agotado (>10s)`. Tu interfaz nunca se congelará.

### P: ¿Dónde encuentro exactamente las imágenes que exporté con la tecla `[E]`?
**R:** Se guardan en la carpeta `./export/interactive/` en la raíz de este proyecto. Se crean dos archivos por cada exportación: un archivo `.tif` (TIFF sin compresión de 16 bits para archivo fotográfico profesional) y un archivo `.jpg` (o `.png` según la configuración del preset) al 100% de calidad.

### P: ¿Puedo usar el ratón para seleccionar cosas en la TUI?
**R:** Sí. Puedes hacer clic izquierdo dentro del panel de **Biblioteca**, del **Previsualizador** o del **Inspector** para transferir el foco inmediatamente a ese panel sin necesidad de usar `Tab`.

### P: ¿Por qué en mi terminal las fotos se ven como rectángulos de colores o caracteres Unicode?
**R:** Si utilizas el CMD clásico de Windows o PowerShell sin soporte gráfico nativo, la TUI utiliza el protocolo de fallback seguro **Halfblocks Unicode** (bloques de medios caracteres con color de 24 bits). Si deseas gráficos nativos de alta resolución en la terminal, ejecuta la TUI en terminales modernas que soporten Kitty Graphics Protocol o Sixel (como Ghostty o WezTerm).
