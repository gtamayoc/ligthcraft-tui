# PRD: Estabilización de Consola TUI, Foco de Paneles y Pipeline de Exportación Secuencial Inmediata

## 1. Overview
El automatizador fotográfico en Rust (`lightcraft-tui`) presenta problemas de usabilidad y rendimiento durante la interacción en consola y el procesamiento de lotes:
1. **Confusión en el foco de la interfaz:** Existen 3 paneles interactivos (1: Biblioteca de fotos, 2: Visor de previsualización, 3: Inspector de ajustes y presets) cuyo foco cambiaba de forma inesperada ante clics de ratón erráticos y carecía de señalización clara sobre el propósito de cada panel y los atajos de teclado disponibles.
2. **Cuelgues y retención en procesamiento por lotes (>3600s):** La cola de procesamiento intentaba paralelizar cargas pesadas o quedaba retenida esperando respuestas sin persistir de inmediato los resultados en disco, provocando la sensación de bloqueo a mitad de proceso.

Esta funcionalidad rediseña el motor de eventos de la TUI y el pipeline de exportación para operar en modo **secuencial estricto foto a foto**, garantizando que cada imagen se decodifique, se le aplique la receta correspondiente y se **guarde de inmediato en disco en máxima calidad (TIFF archival de 16 bits sin pérdida + JPEG al 100%)** antes de avanzar a la siguiente, liberando memoria de forma continua y reportando progreso en tiempo real.

---

## 2. Goals
- **Claridad de navegación:** Explicar explícitamente en la TUI el propósito de los 3 paneles (Biblioteca, Visor, Inspector de recetas) e incorporar una barra de navegación/ayuda permanente que indique el panel activo y las teclas de cambio (`Tab`, `Shift+Tab`, flechas).
- **Manejo controlado de eventos de ratón:** Corregir el cálculo de coordenadas de clics en la terminal para evitar cambios de foco no deseados o accidentales.
- **Pipeline secuencial de guardado inmediato (Streaming):** Eliminar tiempos muertos y cuelgues procesando de forma estrictamente secuencial ($N \rightarrow N+1$), guardando en disco al terminar cada foto individualmente.
- **Máxima calidad garantizada:** Generar de inmediato para cada fotografía procesada tanto el archivo maestro **TIFF de 16 bits sin pérdida** (`Rgb16Image`) como una versión **JPEG al 100%** de calidad.
- **Inmutabilidad absoluta:** Mantener intactos e inalterables los archivos RAW Canon `.CR3` originales en `DRAW/`.

---

## 3. Quality Gates

These commands must pass for every user story:
- `cargo check --all-targets` - Comprobación de tipos y sintaxis sin advertencias.
- `cargo test` - Todas las suites de pruebas unitarias, de integración y adversariales deben pasar (0 errores, 0 fallos).
- Verificación empírica de archivos exportados en disco (existencia de archivos TIFF 16-bit y JPEG 100% generados de inmediato por cada foto procesada).

---

## 4. User Stories

### US-001: Clarificación visual y barra de ayuda permanente para los 3 paneles TUI
**Description:** Como fotógrafo/usuario de la consola, quiero que los 3 paneles de la TUI (1: Biblioteca, 2: Previsualizador, 3: Inspector) tengan títulos explicativos claros y una barra inferior de ayuda permanente que indique cuál panel está enfocado y qué teclas usar, para no desorientarme al interactuar con la terminal.

**Acceptance Criteria:**
- [ ] El panel izquierdo muestra claramente el título "1. BIBLIOTECA (Archivos RAW/Imágenes)" con indicador de foco resaltado.
- [ ] El panel central muestra claramente el título "2. PREVISUALIZADOR (Comparación / Zoom)" con indicador de foco resaltado.
- [ ] El panel derecho muestra claramente el título "3. INSPECTOR (Presets y Controles de Revelado)" con indicador de foco resaltado y descripción de ajustes.
- [ ] Se añade una barra de ayuda inferior fija que muestra: `[Tab / Shift+Tab]: Cambiar panel | [↑/↓]: Navegar lista | [1..6]: Variantes | [Espacio]: Favorito | [E]: Exportar foto actual`.
- [ ] El estado del panel enfocado se refleja de manera unívoca mediante bordes gruesos y títulos activos en color cian/resaltado.

### US-002: Corrección y estabilización del cambio de foco por clic de ratón
**Description:** Como usuario de la terminal, quiero que al hacer clic en cualquier área de la consola no se produzcan saltos erráticos ni cambios de foco inesperados, calculando con precisión las coordenadas de la ventana.

**Acceptance Criteria:**
- [ ] En `src/app.rs` (`handle_mouse`), se corrige el cálculo de `AppLayout::compute` utilizando el tamaño real de la terminal (`terminal.size()`) en lugar de dimensiones sintéticas basadas en el cursor.
- [ ] El cambio de foco mediante clic izquierdo solo se activa cuando el puntero cae estrictamente dentro de los límites rectangulares interiores de uno de los 3 paneles principales.
- [ ] Clics fuera de las áreas interactivas (cabecera, bordes o márgenes) no alteran el foco actual.

### US-003: Motor de procesamiento secuencial foto a foto con guardado inmediato
**Description:** Como usuario, quiero que el procesamiento por lotes procese estrictamente una foto a la vez y la guarde de inmediato en disco, para que no ocurran bloqueos de más de 3600s y pueda ver los resultados generándose progresivamente.

**Acceptance Criteria:**
- [ ] Se implementa en `src/queue/worker.rs` o en el exportador un modo secuencial estricto: Cargar foto $i$ $\rightarrow$ Decodificar $\rightarrow$ Aplicar receta $\rightarrow$ Guardar a disco $\rightarrow$ Liberar buffers de memoria $\rightarrow$ Pasar a foto $i+1$.
- [ ] Cada imagen procesada se escribe de forma atómica en disco inmediatamente antes de iniciar la siguiente tarea, eliminando la retención de imágenes en RAM.
- [ ] Se elimina cualquier dependencia o bloqueo asíncrono que pueda dejar colgado el proceso a la mitad.
- [ ] La barra de estado de la TUI actualiza en tiempo real el porcentaje exacto y el nombre de la foto que se acaba de guardar en disco.

### US-004: Pipeline de doble salida en máxima calidad (TIFF 16-bit + JPEG 100%)
**Description:** Como editor fotográfico, quiero que cada exportación genere siempre y de inmediato tanto el master TIFF archival en 16 bits sin pérdida como el JPEG al 100% de calidad, asegurando la máxima fidelidad de color y nitidez.

**Acceptance Criteria:**
- [ ] Para cada fotografía procesada, el pipeline escribe de inmediato dos archivos en el directorio de salida:
  1. `<nombre>_<preset>.tif`: Codificado nativamente en 16 bits por canal (`Rgb16Image` con Tag 258 `(16, 16, 16)`).
  2. `<nombre>_<preset>.jpg`: Codificado con calidad JPEG al 100% y submuestreo de croma 4:4:4.
- [ ] Ambas imágenes mantienen la resolución nativa original completa (o escala máxima solicitada) y relación de aspecto correcta.
- [ ] Se verifica en disco la existencia y el tamaño no vacío de ambos archivos inmediatamente tras el procesamiento de cada foto.

### US-005: Verificación de inmutabilidad y pruebas de regresión
**Description:** Como responsable de calidad, quiero pruebas automatizadas que certifiquen el flujo secuencial de guardado inmediato y verifiquen que los archivos originales en `DRAW/` permanecen intactos.

**Acceptance Criteria:**
- [ ] Prueba automatizada que valida la secuencia de guardado inmediato de 5 fotos Canon `.CR3`, verificando la existencia de los 10 archivos resultantes (5 TIFFs de 16 bits + 5 JPEGs 100%) en disco.
- [ ] Auditoría de hash criptográfico NIST SHA-256 previa y posterior certificando que los archivos originales en `DRAW/` permanecen 100% byte por byte idénticos.
- [ ] Todas las pruebas de regresión pasan limpiamente con `cargo test`.

---

## 5. Functional Requirements
- **FR-1:** La interfaz TUI debe renderizar los 3 paneles con numeración explícita (1. Biblioteca, 2. Previsualizador, 3. Inspector) y bordes diferenciados según el panel activo.
- **FR-2:** Al presionar `Tab`, el foco debe rotar en orden: Biblioteca $\rightarrow$ Previsualizador $\rightarrow$ Inspector $\rightarrow$ Biblioteca. `Shift+Tab` rota en sentido inverso.
- **FR-3:** Los eventos de ratón deben validar las coordenadas contra el layout calculado a partir del tamaño real del frame de la terminal.
- **FR-4:** El procesamiento de imágenes debe realizarse en un bucle secuencial desacoplado de la interfaz gráfica para no congelar la pantalla.
- **FR-5:** Cada archivo debe ser guardado en disco en formato TIFF 16-bit y JPEG 100% antes de avanzar a la siguiente imagen de la lista.
- **FR-6:** En caso de interrupción del proceso por el usuario (`Ctrl+C` o `q`), las fotos ya guardadas deben permanecer intactas en disco sin archivos temporales corruptos.

---

## 6. Non-Goals (Out of Scope)
- No se sustituirá el motor de revelado existente ni se alterarán los 5 presets fotográficos base ya verificados.
- No se implementarán algoritmos externos de inteligencia artificial o deep learning.
- No se eliminará ninguno de los 3 paneles de la TUI, sino que se mantendrán clarificando su navegación y propósito.
- No se modificarán nunca los archivos fuente en `DRAW/`.

---

## 7. Technical Considerations
- **Uso de memoria:** Procesar una por una permite mantener en memoria solo una trama de 24MP descomprimida a la vez (~144 MB en 16 bits), liberándola inmediatamente después de la escritura en disco.
- **Codificación TIFF 16-bit:** Utilizar el codec nativo en Rust de `image::codecs::tiff::TiffEncoder` para `Rgb16Image`.
- **Codificación JPEG 100%:** Utilizar `image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 100)`.
- **Compatibilidad Windows:** Evitar cualquier bloqueo de descriptor de archivo mediante apertura y cierre atómico de ficheros de exportación.

---

## 8. Success Metrics
- Tiempo de espera entre el inicio del proceso y la aparición del primer archivo terminado en disco: < 5 segundos por imagen.
- Cero cuelgues o bloqueos prolongados (>10 segundos sin actividad registrada en la barra de estado).
- Generación confirmada del 100% de los archivos pares (TIFF 16-bit + JPEG 100%) para todas las fotos solicitadas.
- 100% de coincidencia en los hashes SHA-256 de los archivos originales en `DRAW/`.

---

## 9. Open Questions
- Ninguna pregunta abierta tras la confirmación de preferencias del usuario (1A+1C, 2A, 3C, 4C).
