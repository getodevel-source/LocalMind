# LocalMind — respuestas SRS (P1–P32)

- Fecha: 2026-09-25
- Respondidas: 32/32
- Compacto: P1:C P2:A P3:A P4:A P5:A P6:A P7:A P8:A P9:B P10:A P11:A P12:A P13:A P14:A P15:A P16:A P17:C P18:C P19:A P20:A P21:A P22:A P23:A P24:A P25:A P26:A P27:A P28:A P29:A P30:A P31:A P32:A

## Producto y alcance

- **P1** ¿Para quién es LocalMind? → Producto para terceros (NO recomendada; recomendada: sólo esta PC). Implica: instalador, presets por hardware y soporte multiplican F2/F3.
- **P2** ¿Cuántos modelos cargados a la vez? → Uno por vez.
- **P3** ¿En qué plataformas corre? → Sólo Windows.
- **P4** ¿Backend de GPU? → Medir ROCm vs Vulkan y decidir con números.
- **P5** ¿Cómo se entrega? → Instalador con versión visible (modelos aparte; cierra D-15).

## Modelos

- **P6** ¿Gestión de modelos dentro de la app? → Descarga integrada desde HuggingFace con progreso y checksum.
- **P7** ¿Visión y voz en el chat? → Sólo texto.
- **P8** ¿Un modelo para todo o varios por caso de uso? → Un modelo para todo.
- **P9** ¿Política de cuantización? → Permitir Q4/Q5/Q6 con benchmark automático por perfil (NO recomendada; recomendada: fijar IQ4_XS).
- **P10** ¿Meta dura de t/s por perfil? → Definir metas por perfil y medirlas con llama-bench. Referencia: objetivo 30 t/s a 262K vs 12.91 medido en `bench-20260924-234539.json`.

## Rendimiento y energía

- **P11** MTP y cache-reuse: ¿velocidad o estabilidad? → Estabilidad primero, con MTP/cache-reuse por perfil y visibles.
- **P12** ¿Política anti-apagón? → Mantener y hacerlo visible en la UI. Nota textual del dueño: "lo unico que necesito en este aspecto es que tratemos de tener en cuenta todo lo que nos puede bloquear o generar un apagon inesperado, ya paso varias veces y no se puede volver a permitir." → requisito duro, no optimización.
- **P13** ¿Auto-stop por inactividad? → Configurable desde la UI (hoy 1500 s fijos).
- **P14** ¿Arranque del motor al abrir la app? → Manual.
- **P15** ¿Qué hacemos con dashboard.html y chat.html? → Eliminarlos.

## Interfaz

- **P16** ¿Parámetros de generación ajustables? → Panel avanzado con preset por perfil (hoy temperature fija 0.7).
- **P17** ¿Accesibilidad e idioma? → Bilingüe es/en con conmutador (NO recomendada; recomendada: español + a11y básica). Implica duplicar textos y pruebas en F2.
- **P18** ¿Estética? → Rediseño completo con sistema de diseño propio (NO recomendada; recomendada: rediseño sobre estructura actual). Es el ítem de mayor coste de F2.
- **P19** ¿Adjuntos en el chat embebido? → Texto + imágenes del portapapeles. ⚠️ CONFLICTO con P7 (sólo texto): requiere mmproj, que P7 deja apagado. Registro: objetivo condicionado a revisar P7 en F2; por defecto manda P7.
- **P20** ¿Notificaciones del sistema? → Avisar al quedar listo y antes del auto-stop.

## Lanzadores y CLIs

- **P21** ¿Cómo tocamos las configs de OMP y Pi? → Merge no destructivo + backup antes de escribir. Nota: el árbol actual ya escribe configs privadas en `%APPDATA%/LocalMind/agents/<agent>/` sin tocar `~/.pi`/`~/.omp`.
- **P22** ¿Qué puerto ven los CLIs? → Puerto estable reservado para CLIs, reescrito al arrancar el motor (cierra D-6).
- **P23** ¿El modelo elegido en la UI se respeta al lanzar? → Sí, respetar el modelo seleccionado (cierra D-2). Aclaración dada: hoy lanzar OMP/Pi usa un nombre fijo del código e ignora el desplegable; debe usar lo elegido/cargado.
- **P24** ¿Cuántos CLIs soportamos? → OMP, Pi y Orca alcanzan. Nota: el árbol ya trae `/v1/messages` (Claude Code) y `/v1/responses` (Codex) como superficie API, no como CLIs soportados.
- **P25** ¿Qué debe abrir «Interfaz Web Externa»? → La UI de llama.cpp.

## Configuración

- **P26** ¿Se editan los perfiles desde la UI? → Editor de perfiles en la UI (cierra D-1 del perfil `turbo` fantasma).
- **P27** ¿Dónde vive la configuración? → APPDATA + opción portable.
- **P28** ¿Qué pasa si la config está corrupta? → Avisar en la UI y ofrecer reparar (con backup). Cierra D-9.

## Seguridad y mantenimiento

- **P29** ¿La API local lleva autenticación y CORS restringido? → Token simple para `/api/*` y CORS restringido a loopback. Nota: el árbol actual ya exige `gateway.key` en `/api/*` y `/v1/*`.
- **P30** ¿Los logs se guardan en disco? → Persistir a archivo con rotación y botón de exportar.
- **P31** ¿Tests, CI y remoto git? → Suite mínima + CI + publicar remoto. Nota: `tests/` ya existe sin commitear (desactualiza §3.11/§8 del SRS v1.1).
- **P32** ¿El SRS es la fuente de verdad versionada? → Sí: versionado en el repo y actualizado al cerrar cada fase.

Sin pendientes: las 32 decisiones están tomadas, salvo resolver el conflicto P7↔P19 en F2.
