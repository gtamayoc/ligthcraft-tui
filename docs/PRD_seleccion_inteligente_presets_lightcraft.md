# PRD — Selección inteligente de ajustes preestablecidos para fotografías en LightCraft

**Versión:** 0.1  
**Estado:** Borrador para validación  
**Alcance:** Una mejora puntual de la automatización fotográfica existente  
**Integración existente:** Aplicación propia conectada al MCP de LightCraft  
**Formato prioritario:** Canon RAW / C-RAW `.CR3`  

---

## 1. Resumen

La aplicación actual permite seleccionar una fotografía y generar o presentar un número reducido de ajustes preestablecidos —por ejemplo, 2, 4 o 6— disponibles en LightCraft. El usuario todavía tiene que decidir manualmente cuáles utilizar y probar opciones para encontrar un resultado que le guste.

La mejora propuesta consiste en añadir una **selección inteligente de presets**: después de elegir una fotografía, la aplicación examina los ajustes preestablecidos que realmente están disponibles en LightCraft, analiza las características visibles de la imagen, identifica los candidatos más adecuados y presenta únicamente las mejores opciones en forma de vistas previas comparables.

El usuario podrá elegir cuántas opciones desea ver —2, 4 o 6—, pero ya no tendrá que escoger manualmente los presets de origen. La automatización hará la búsqueda, priorización y preparación de las variantes. De forma opcional, podrá habilitarse un modo para aplicar directamente el primer candidato, siempre que se mantenga la edición no destructiva y exista una forma clara de deshacer el cambio.

Este PRD se limita a esa mejora. No propone rehacer la aplicación, crear un editor fotográfico nuevo, reemplazar LightCraft, construir otro servidor MCP ni modificar la automatización de otras tareas.

## 2. Contexto y problema actual

El flujo actual, según la descripción del usuario, funciona de forma aproximada así:

1. Se selecciona una fotografía dentro de la aplicación existente.
2. La herramienta conectada a LightCraft ofrece un conjunto pequeño de opciones, como 2, 4 o 6 ajustes.
3. El usuario tiene que seleccionar manualmente los presets que desea probar o decidir cuáles aplicar.
4. Si la biblioteca tiene muchos presets, la selección manual obliga a invertir tiempo probando combinaciones que pueden no corresponder con la fotografía.

El problema no es que falten presets, sino que **la aplicación no reduce automáticamente una biblioteca extensa a un conjunto breve y pertinente para la imagen seleccionada**.

## 3. Objetivo del producto

Cuando el usuario seleccione una fotografía, la aplicación debe localizar los presets disponibles en LightCraft, evaluar cuáles tienen mayor probabilidad de producir un buen resultado para esa imagen y generar un conjunto pequeño de candidatos visuales, ordenados por compatibilidad.

El resultado debe reducir pasos manuales y facilitar la comparación, sin prometer que una puntuación automática pueda determinar de forma universal cuál es el mejor estilo artístico. La recomendación se debe tratar como una clasificación útil, no como una verdad absoluta.

## 4. Experiencia esperada

### Flujo principal

1. El usuario selecciona una fotografía, preferiblemente un RAW `.CR3` de su Canon EOS R100.
2. La aplicación obtiene de LightCraft la identidad y los metadatos accesibles de la foto, así como una previsualización de referencia.
3. La aplicación consulta los presets que están disponibles en la instalación actual de LightCraft, en vez de utilizar una lista fija escrita en el código.
4. Un mecanismo de selección inteligente estima qué presets encajan mejor con la imagen.
5. La aplicación prepara y compara vistas previas de los candidatos mejor posicionados.
6. La interfaz muestra las 2, 4 o 6 mejores opciones, ordenadas de mayor a menor compatibilidad, junto a la imagen original o una referencia sin el preset.
7. El usuario elige un resultado para aplicarlo, o activa una preferencia explícita para aplicar automáticamente el candidato mejor clasificado.
8. La aplicación confirma el resultado y permite deshacerlo o volver a la edición anterior.

### Principio de experiencia de usuario

**El usuario elige la fotografía y, si lo desea, cuántas alternativas ver; el sistema elige qué presets merece la pena probar.** No debe ser necesario seleccionar a mano los identificadores de los presets de la biblioteca.

## 5. Alcance funcional

### 5.1 Descubrir los presets disponibles

- Consultar dinámicamente el catálogo de presets que expone la versión instalada de LightCraft.
- Considerar todos los presets que la integración permita utilizar, no solo 2, 4 o 6 opciones codificadas de antemano.
- Distinguir, cuando la API lo permita, entre presets integrados y presets personalizados del usuario.
- Mantener el origen del preset y su identificador real para poder aplicarlo después.
- No asumir que todos los presets están disponibles en todas las versiones de LightCraft. Si el MCP instalado difiere de la documentación pública, la implementación deberá adaptarse a las capacidades reales detectadas.

### 5.2 Analizar la fotografía seleccionada

La aplicación debe aprovechar primero los datos que LightCraft ya expone. El análisis podrá considerar, cuando estén disponibles:

- El tipo de escena o sujeto: retrato, paisaje, animal, arquitectura, calle, macro, interior, noche u otros.
- La iluminación aparente: luz suave o dura, contraluz, escena oscura, altas luces fuertes o iluminación mixta.
- Las características generales del color: temperatura aparente, predominio de tonos cálidos o fríos, saturación y contraste.
- Las características técnicas visibles: exposición aparente, sombras empastadas, altas luces muy intensas o bajo contraste.
- Los metadatos de captura disponibles, sin tratarlos como sustitutos del análisis visual.

El informe técnico debe indicar qué datos son realmente accesibles a través de la integración existente y cuáles requerirían un componente adicional. No se debe inventar información ni dar por hecho que LightCraft ya clasifica escenas automáticamente.

Para imágenes RAW `.CR3`, se debe comprobar si LightCraft está desarrollando el archivo RAW o si solo tiene disponible la previsualización incrustada. Si la foto está en estado de solo previsualización (`previewOnly` o equivalente), se debe indicar que la recomendación se basa en esa representación y podría tener menor fiabilidad.

### 5.3 Clasificar y ordenar candidatos

La selección debe combinar dos tipos de evaluación:

**Primera etapa: selección preliminar.** Utilizar los metadatos disponibles de cada preset —como nombre, descripción, categoría o etiquetas, si existen— y las características de la fotografía para descartar opciones claramente poco pertinentes y priorizar un conjunto de candidatos.

**Segunda etapa: comparación visual.** Probar candidatos de la primera etapa mediante previsualizaciones de resolución reducida y comparar los resultados con la imagen de referencia. El objetivo es comprobar el efecto real del preset en esa foto, no confiar exclusivamente en el nombre del preset.

La clasificación deberá considerar, en la medida en que la arquitectura existente lo permita:

- Compatibilidad con el tipo de escena y su intención visual.
- Mejora o conservación razonable de exposición y detalle.
- Control de altas luces y sombras.
- Tratamiento del color, saturación, temperatura y contraste.
- Conservación de detalles del sujeto y ausencia de efectos claramente excesivos.
- Diversidad visual entre las alternativas para evitar mostrar seis resultados prácticamente idénticos.

La puntuación debe servir para ordenar candidatos. No debe presentarse como una medición científica ni garantizar que la opción con mayor puntuación sea la preferida por el usuario.

Si no hay información semántica en los presets, la aplicación no debe inventarla. Deberá depender más de la comparación de las previsualizaciones y explicar la limitación en sus registros de diagnóstico.

### 5.4 Número de opciones

Conservar la idea de mostrar 2, 4 o 6 alternativas, pero cambiar su significado:

- **2 opciones:** mostrar los dos candidatos mejor clasificados.
- **4 opciones:** mostrar los cuatro candidatos mejor clasificados.
- **6 opciones:** mostrar los seis candidatos mejor clasificados.

La lista completa de presets se evalúa como universo de candidatos; el número 2/4/6 define cuántos resultados finales se presentan, no cuántos presets se consultan.

Debe existir una opción para solicitar más alternativas cuando las primeras no sean satisfactorias. Si la biblioteca contiene menos candidatos válidos que el número solicitado, mostrar solo los disponibles e informar claramente del motivo.

### 5.5 Aplicar el resultado

Modo recomendado por defecto:

- Mostrar las previsualizaciones de los candidatos.
- Permitir que el usuario pulse **Aplicar este preset** sobre una alternativa.
- Mantener disponible el estado anterior de la edición y la acción de deshacer.

Modo opcional:

- **Aplicar automáticamente el mejor:** aplicar el candidato de mayor clasificación después de terminar la evaluación.
- El modo automático debe ser configurable y estar desactivado inicialmente, hasta confirmar la calidad de las recomendaciones.
- Si la clasificación tiene poca confianza o los candidatos están prácticamente empatados, es preferible mostrar las opciones y no aplicar ningún cambio de forma automática.

El proceso de evaluación no debe dejar todos los presets superpuestos en la fotografía. Cada candidato debe evaluarse desde la misma base de ajustes, para que los resultados sean comparables y no se acumulen efectos por error.

## 6. Requisitos de interfaz

La interfaz existente debe mantenerse en lo posible. La mejora se integra en el flujo actual de selección de fotografía y generación de variantes.

Elementos propuestos:

- Indicador breve de análisis: «Analizando fotografía y comparando presets…».
- Selector de cantidad final: 2, 4 o 6 resultados.
- Tarjetas o filas con previsualización, nombre del preset y una explicación breve de por qué fue recomendado, cuando exista evidencia suficiente.
- Orden de recomendación visible, como «Opción 1», «Opción 2», etc.
- Acción principal **Aplicar** en cada resultado.
- Acción **Ver más opciones** para consultar el siguiente grupo de candidatos.
- Acción **Volver al original / deshacer**.
- Estado claro cuando LightCraft no responde, no se puede recuperar la biblioteca o la fotografía no tiene una vista previa utilizable.

Las explicaciones de recomendación deben ser concretas y relacionadas con la imagen, por ejemplo: «priorizado por su tratamiento moderado de contraste y color para esta escena». Solo deberán generarse si el sistema tiene información que las respalde; no mostrar descripciones genéricas como si fueran análisis precisos.

## 7. Integración con LightCraft MCP

La documentación pública de LightCraft describe operaciones MCP para consultar presets disponibles, aplicar presets y generar previsualizaciones. Entre las interfaces documentadas aparecen `cmd_presets_list`, `apply_preset` y herramientas de renderizado como `render_preview` o `render_photo`.

La implementación debe descubrir y verificar las herramientas disponibles en la versión de LightCraft conectada. No debe depender de nombres o argumentos asumidos sin consultar la respuesta real del servidor MCP.

### Reglas de integración

- Utilizar la conexión MCP existente; no crear un segundo MCP para realizar esta selección.
- Consultar la biblioteca de presets dinámicamente.
- Asociar cada candidato con el identificador válido que LightCraft requiere para aplicarlo.
- Obtener imágenes de previsualización de baja resolución para la clasificación, si la API lo permite.
- Evitar exportaciones a resolución completa durante la fase de evaluación.
- No cambiar accidentalmente la fotografía activa ni la selección de LightCraft al consultar o renderizar candidatos, cuando la API permita operar por identificador de foto.
- Validar el comportamiento real de `apply_preset` y la gestión del historial de edición antes de diseñar el proceso de evaluación.

### Evaluación aislada de candidatos

Antes de probar múltiples presets, el agente de desarrollo debe comprobar cómo mantener un estado base de la fotografía y cómo restaurarlo entre candidatos. Puede utilizarse un mecanismo de previsualización independiente, un estado temporal o una secuencia controlada de guardar/restaurar ajustes, según lo que permita el motor. No se debe elegir una estrategia sin inspeccionar la API y sus efectos.

El usuario no debería ver cambios parpadeantes ni acumulativos en la fotografía mientras se calculan recomendaciones. Si no es posible evaluar candidatos de manera aislada con la integración actual, se debe documentar esa limitación y plantear la alternativa técnica más segura.

## 8. Requisitos no funcionales

- **No destructivo:** la evaluación no debe modificar el archivo RAW original ni sobrescribir el archivo fuente.
- **Reversible:** aplicar un preset debe conservar un camino claro para deshacerlo o recuperar la edición anterior.
- **Eficiencia:** no renderizar cien versiones a resolución completa; usar una etapa preliminar y previsualizaciones reducidas para un conjunto acotado de candidatos.
- **Comparabilidad:** cada candidato debe evaluarse sobre la misma base de edición.
- **Robustez:** gestionar fallos de MCP, presets obsoletos, errores de renderizado y fotos no compatibles sin bloquear la aplicación.
- **Trazabilidad:** registrar, para depuración, cuántos presets se descubrieron, cuáles se evaluaron y por qué un candidato fue descartado, sin generar logs innecesariamente enormes.
- **Extensibilidad:** poder cambiar el criterio de selección o añadir nuevos atributos sin reescribir el flujo de LightCraft.
- **Control del usuario:** la aplicación debe permitir volver a comparar opciones y aplicar manualmente otra alternativa.

## 9. Fuera del alcance

Esta mejora no incluye:

- Reemplazar el motor de revelado de LightCraft.
- Crear presets nuevos automáticamente.
- Descargar catálogos externos o comprar presets.
- Crear una nueva interfaz completa de edición fotográfica.
- Cambiar procesos de recorte, exportación, redimensionado, formatos de salida o publicación en redes sociales.
- Procesamiento por lotes de carpetas enteras. Primero se valida el flujo de una sola foto; el lote puede analizarse más adelante como extensión independiente.
- Entrenar un modelo propio de visión por computador desde cero.
- Garantizar que una IA identifique siempre la intención artística exacta del fotógrafo.

## 10. Plan por fases

### Fase 0 — Auditoría de la automatización existente

- Inspeccionar el flujo actual desde la selección de la fotografía hasta la presentación de 2, 4 o 6 opciones.
- Identificar los archivos, funciones, prompts, reglas y llamadas MCP involucradas.
- Verificar cómo se consulta hoy la lista de presets y si se está usando una lista fija.
- Consultar las capacidades MCP de la versión instalada de LightCraft.
- Confirmar cómo se puede renderizar una previsualización y restaurar el estado de edición.
- No modificar todavía el código ni instalar dependencias.

**Resultado:** diagnóstico con los componentes que se reutilizarán, las limitaciones detectadas y la estrategia propuesta.

### Fase 1 — Selección automática de candidatos

- Consultar el catálogo real de presets.
- Analizar los metadatos accesibles de la fotografía y, si ya existe soporte, la previsualización visual.
- Implementar una clasificación inicial que seleccione los mejores 2, 4 o 6 candidatos.
- Mantener el mismo aspecto de la interfaz actual siempre que sea posible.

**Resultado:** el usuario deja de seleccionar manualmente los identificadores de los presets a probar; recibe una lista priorizada.

### Fase 2 — Previsualizaciones comparables

- Generar vistas previas de los candidatos sin acumular sus efectos.
- Mostrar los resultados ordenados con nombres y explicación breve.
- Añadir aplicación manual, deshacer y recuperación del estado anterior.
- Medir tiempos y reducir candidatos si el proceso resulta demasiado lento.

**Resultado:** el usuario compara visualmente alternativas ya filtradas y puede aplicar la elegida.

### Fase 3 — Aplicación automática opcional

- Incorporar la preferencia «Aplicar el mejor automáticamente».
- Definir umbral de confianza y comportamiento ante resultados ambiguos.
- Mantener el historial y la reversibilidad.

**Resultado:** cuando esté habilitado y la clasificación sea suficientemente clara, el sistema puede aplicar el candidato mejor clasificado sin intervención manual.

## 11. Criterios de aceptación

La mejora se considera aceptada cuando:

1. Al seleccionar una foto, la aplicación consulta la lista real de presets disponibles en LightCraft, en lugar de limitarse a una lista fija de 2, 4 o 6 identificadores.
2. El número 2, 4 o 6 controla cuántos candidatos finales se muestran.
3. Los candidatos aparecen ordenados por una clasificación generada a partir de datos disponibles y/o previsualizaciones, no por un orden arbitrario de la lista.
4. El sistema conserva el identificador válido de cada preset para aplicarlo.
5. Las comparaciones se hacen desde una misma base de edición y no acumulan efectos de candidatos anteriores.
6. La evaluación no sobrescribe la fotografía RAW original.
7. El usuario puede aplicar cualquier alternativa mostrada, deshacer el resultado y volver al estado anterior.
8. El modo de aplicación automática es opcional y no se activa por defecto sin validación.
9. Si falla el MCP, la lista de presets o la previsualización, se muestra un error comprensible y no se deja la foto en un estado de edición inesperado.
10. Cuando una imagen RAW no puede revelarse y solo existe una vista previa incrustada, el sistema lo detecta si LightCraft expone ese estado y evita presentar la recomendación como un análisis RAW completo.
11. Se prueba el flujo con fotografías de distintos tipos y condiciones de luz para verificar que las recomendaciones no se limitan a un único estilo.
12. Se documentan los tiempos reales para una biblioteca pequeña y otra extensa de presets, sin renderizar todos a resolución completa.

## 12. Métricas para validar la mejora

- Número de pasos manuales necesarios desde seleccionar una fotografía hasta obtener candidatos útiles.
- Tiempo desde la selección hasta que se muestran las primeras recomendaciones.
- Porcentaje de ocasiones en que el usuario aplica una de las primeras opciones sugeridas.
- Frecuencia con que el usuario solicita ver más alternativas.
- Número de fallos o cambios de edición no deseados durante la comparación.

La calidad artística se debe validar con uso real: registrar si el usuario elige las recomendaciones y permitir ajustar preferencias según la experiencia. No se debe declarar éxito únicamente porque el algoritmo genere una puntuación.

## 13. Riesgos y mitigaciones

**Riesgo: elegir por el nombre del preset sin ver su resultado.**  
Mitigación: usar nombres y etiquetas solo como filtro preliminar; comparar visualmente las mejores opciones cuando sea posible.

**Riesgo: aplicar un preset sobre el efecto de otro.**  
Mitigación: restaurar o aislar el estado base al renderizar cada candidato.

**Riesgo: evaluar demasiados presets y demorar la interacción.**  
Mitigación: clasificación en dos etapas y renderizado de baja resolución únicamente para los candidatos más prometedores.

**Riesgo: una puntuación que no representa el gusto del usuario.**  
Mitigación: explicar el motivo cuando sea justificable, permitir comparar y aplicar manualmente, y no autoaplicar con poca confianza.

**Riesgo: interpretación incompleta de un archivo RAW.**  
Mitigación: detectar si LightCraft está usando los datos RAW o una vista previa incrustada y comunicarlo.

**Riesgo: cambios entre versiones del MCP.**  
Mitigación: descubrir las herramientas disponibles al inicio y validar sus esquemas en tiempo de ejecución o durante la inicialización.

## 14. Preguntas que debe resolver la auditoría técnica

Antes de implementar, el agente de desarrollo debe determinar:

1. ¿Dónde se limita actualmente el conjunto de opciones a 2, 4 o 6 presets?
2. ¿La aplicación ya recupera todos los presets de LightCraft o solo utiliza una lista seleccionada manualmente?
3. ¿Qué metadatos reales de presets están disponibles además del identificador y el nombre?
4. ¿Qué mecanismo de análisis visual existe en la aplicación y se puede reutilizar?
5. ¿Qué herramienta MCP permite obtener la previsualización sin cambiar la selección activa?
6. ¿Cómo se puede aplicar cada preset sobre un estado base idéntico y restaurar la edición después?
7. ¿Cómo se detecta si un archivo RAW se está revelando completamente o solo se muestra su vista previa incrustada?
8. ¿Cuál es el número real de presets disponibles y cuánto tarda evaluar una muestra representativa?

Las respuestas deben basarse en el código actual y la versión real de LightCraft conectada, no solo en esta propuesta o en nombres de herramientas supuestos.

## 15. Referencias técnicas

- Documentación pública de LightCraft MCP: https://github.com/storytold/lightcraft/blob/main/docs/mcp.md
  - Describe la consulta de presets, la aplicación de ajustes y la generación de previsualizaciones. La compatibilidad exacta debe comprobarse contra la versión instalada.
- Manual oficial de Canon EOS R100 — especificaciones: https://cam.start.canon/es/C015/manual/html/UG-11_Reference_0080.html
  - La cámara registra fotografías RAW en `.CR3` y también ofrece C-RAW.

**Nota sobre formato:** para la Canon EOS R100, el término correcto es **RAW `.CR3`** o **C-RAW `.CR3`**, no «R3». CR3 es la extensión/formato del archivo de imagen RAW de Canon; no significa que la cámara sea el modelo EOS R3.
