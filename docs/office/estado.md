# Office: estado de desarrollo

Actualizado: 2026-11-09.

## Estado actual

- Rama de desarrollo: `office`, basada en `rebrand`.
- Rebranding y prueba de cuentas completados: login sin API keys, aislamiento, concurrencia y persistencia comprobados en Windows.
- Fase 0 completada. `office-spike` contiene código desechable y no se fusiona.
- Fase 1, paso 1: implementado el crate office con panel vacío, acción `office: toggle focus`, botón de dock y respeto de disable_ai. Compilación y arranque comprobados; apertura por paleta, cierre/reapertura por botón y traslado derecha → izquierda → derecha verificados visualmente.
- Fase 1, paso 2: puestos persistentes en `.uacode/office.json` (commit `d0539a0d60`). Crear, editar y archivar puestos; JSON inválido o versión futura en solo lectura con «Abrir archivo» y «Recargar»; posición del dock persistida en la base de datos local.
- Fase 1, paso 3: catálogo global de cuentas en `accounts.json` (commit `15beb0e1aa`). Alta con nombre único por proveedor, entradas virtuales «Por defecto», importación explícita de carpetas existentes y selector de cuenta en el puesto. Todas las cuentas «Sin probar».
- Fase 1, paso 4: registro, login y comprobación de cuentas (commits `e5e8ca9371`, `9c1992fc03`, `040f12e913`). Un agente `office-<proveedor>-<id corto>` por cuenta, registrado dinámicamente sin escribir ajustes; «Iniciar sesión» crea la carpeta y lanza el login de suscripción; «Comprobar» sondea `_auth/status_update`. Aprobado en el mensaje #7 tras la prueba de login real del usuario.
- Fase 1, paso 5: gancho de política (commits `7672740417` en `acp_thread` + `agent_servers`, `b09deb699c` en `office`). `ThreadPolicy` por `AgentId` para permisos, escrituras y terminales del cliente, modo y `_meta` de sesión; Codex de Office arranca en `read-only`. Verificado a mano con prompts reales y una política de prueba en la rama `office-step5-test` (sin fusionar). Aprobado en el mensaje #9.
- Fase 1, paso 6: política de Office (commits `b889f4f367` y `f5ec3d7830` en `acp_thread`/`agent_servers`, `95f568000e` y `e2178eeb4f` en `office`). Solo consulta para todos los puestos, guarda de versión de adaptador, aprobaciones y bloqueos en la tarjeta del puesto. Verificado a mano. Aprobado en el mensaje #11.
- Fase 1, paso 7: hilos por puesto (commits `32e385fb09`, `521a13de70`, `17b3083437`, `9cedc971aa`, `5090e52652`, `a73cd5518c`, `80b5ee0dde`). Clic en el resumen de la tarjeta abre o crea el hilo del puesto; enlace puesto → hilo en la base local; política y tarjeta por puesto; «Nuevo hilo» y «Cancelar»; los agentes de Office no leen ni escriben preferencias en settings.json. Aprobado en el mensaje #12 salvo el fallo de «Aprobar»/«Rechazar», tratado en el paso 8.
- Fase 1, paso 8: estado y mensajes del puesto (commits `a160906647`, `3bf14eedf4`, `edbc7018e6`, `2631087fe6`, `ad735dd35f`, `d7b4990341`). Test de UI de «Aprobar»/«Rechazar» y respuestas sin fallos silenciosos; estados Requiere login, Error, Esperando aprobación, Trabajando y Libre; tarea actual y último mensaje con refresco limitado; título del puesto en el hilo nuevo; adaptadores v1 muertos al cerrar. Pendiente de revisión.

## Decisiones vigentes

- UACode es un proyecto independiente y no se sincroniza con Zed upstream (mensaje #10). El código de Zed se modifica cuando es la solución más limpia; la compatibilidad con upstream no es criterio. La lógica de Office vive en `crates/office` (raíz `src/office.rs`), con commits pequeños, tests y comentarios donde el motivo no sea obvio. Las marcas `UACode:` ya no son obligatorias.
- No renombrar por ahora identificadores internos (`zed::`, crates, `ZED_*`, `.zed/`): coste alto sin beneficio inmediato; se valorará más adelante.
- Orden: Fase 1 panel/cuentas, 2 roles, 3 worktrees, 4 orquestador.
- Un proceso por (proveedor, cuenta), varias sesiones por proceso. Adaptadores fijados: Claude 0.94.0, Codex 2.5.0.
- Roles con ACP `systemPrompt` append; modelo por puesto con `set_config_option` en Fase 2.
- Tres cambios de cwd en Fase 3: respetar work_dirs explícitos, persistir/restaurar al reanudar, y respetarlos en fs/terminales del cliente ACP.
- Durante Fase 1, solo consulta: Codex read-only y Claude con aprobación obligatoria para cualquier escritura o comando. Desde Fase 3, Codex workspace-write con cwd en su worktree; pushurl inválido para todos los remotos de cada worktree; pre-push solo como aviso adicional. No alterar remotos del árbol principal.
- Destructivos siempre con aprobación humana desde Office. Pendiente validar una vía efectiva para operaciones internas de Codex/Claude que no solicitan permiso ACP; bloquear tareas afectadas mientras no se garantice.
- Contexto en todos los puestos; USD solo Claude, etiquetado estimado. Concurrencia por número de agentes activos, con cola.
- MCP HTTP en 127.0.0.1, puerto y token por sesión; validar Origin. Solo el orquestador recibe el endpoint; solo herramientas de Office se autoaprueban. Aprobaciones humanas de destructivos e integración se mantienen.
- Merge local ejecutado por UACode mediante CLI de git tras aprobación; agente solo para conflictos.
- Proyecto en `.uacode/office.json`; catálogo global en `%APPDATA%\UACode\office\accounts.json`; credenciales aisladas en `%APPDATA%\UACode\accounts\…`. «Por defecto» es virtual por proveedor y apunta a `accounts\claude-default` / `accounts\codex-default`.
- Cuentas: id UUID, carpeta derivada del id (`<proveedor>-<8 caracteres>`) salvo las importadas, que guardan su carpeta. El puesto referencia la cuenta por id; si falta, «Sin cuenta» sin borrar la referencia. accounts.json nunca guarda tokens ni correos; Office no lee el contenido de las carpetas de credenciales.

- office.json y accounts.json incluyen `version` desde el inicio; tests unitarios de serialización, JSON inválido y referencias inexistentes en pasos 2, 3 y 11.
- Paso 4: evaluar registro dinámico en AgentServerStore; explicar inviabilidad antes de escribir settings.json. Paso 5: gancho mínimo aislado con test de hilos ajenos a Office. Pasos 7 y 8 pueden unirse si son pequeños.

- office.json vive en el primer worktree visible que sea carpeta local; proyectos remotos muestran un aviso y no crean nada. Campos desconocidos se rechazan (solo lectura) para no perderlos al guardar.

## Siguiente paso

Revisión del paso 8 por Claude y decisión sobre la propuesta de disco (en `para-claude.md`, sin aplicar).

Pendientes conocidos:
- «Aprobar»/«Rechazar» en la app: el test de UI con clic real pasa, así que la causa del fallo del paso 7 no se reprodujo. Los caminos sospechosos ya no fallan en silencio (warn en el log y la entrada sale de la tarjeta). Si se repite, el log dirá cuál de los dos avisos salta.
- Un hilo vacío creado con «Nuevo hilo» pierde el título al reiniciar UACode: el borrador restaurado vuelve a «New Agent Thread» y su fila en `sidebar_threads` queda con título vacío. Hipótesis sin comprobar: el adaptador manda `session_info_update` con título nulo al crear la sesión restaurada y pisa el título pedido. Con el primer mensaje el hilo recibe su título normal.
- Las entradas `agent_servers.office-*` que quedaron en el `settings.json` aislado de las pruebas (modo y modelo por defecto) no se borran: Office las ignora porque sus agentes no leen preferencias guardadas.
- «Trabajando» y «Esperando aprobación» solo están comprobados por tests; no se han visto en vivo porque requieren un prompt.
- Las builds debug activan `acp-beta` (flags de equipo activos en debug) y Codex usa ACP v2; en release usaría v1. Las decisiones de política de sesión (inicio, modo por defecto, cambio del usuario y del agente) viven en `crates/agent_servers/src/acp/session_policy.rs` y v1 y v2 las llaman igual; los tests `consult_only_modes_on_v1_connection` y `v2_consult_only_modes_on_v2_connection` comprueban los mismos casos en ambas rutas.
- **Riesgo abierto:** el sandbox read-only de Codex en Windows no ha quedado demostrado (en el paso 5 Codex pidió escalada en vez de intentar escribir, así que la barrera efectiva fue el gancho). Se vuelve a probar en la Fase 3, cuando workspace-write sea relevante.
- En codex-acp el único rechazo de un cambio de archivo es la decisión `cancel`, que termina el turno (diseno.md 2.7 prefería no usar `cancel` como rechazo ordinario; aquí no hay alternativa).

## Reglas de trabajo

- Confirmar cada paso con el usuario antes de continuar; commits pequeños y claros en `office`.
- Nunca hacer git push ni tocar remotos durante este desarrollo.
- No escribir fuera del repositorio salvo en `%APPDATA%\UACode` cuando el paso lo requiera, avisando antes. No acceder a cuentas reales para pruebas sin el paso correspondiente.
- Ciclo de desarrollo debug incremental (`cargo build -p zed`), `cargo check -p office` y `cargo test -p office`; comprobar arranque tras cada paso. Release solo al cerrar hitos o por petición expresa. Registrar tiempos reales de debug.
- Actualizar este archivo en cada paso con lo hecho, pendiente y decisiones nuevas.
- AGENTS.md es un enlace simbólico a .rules, adaptado a UACode como proyecto independiente: no añadir avisos de revisión upstream al README; seguir el protocolo del buzón.
- Preferir codebase-memory-mcp para descubrir código. Usar `./script/clippy` si se necesita clippy.
- Las pruebas manuales de política deben cubrir la ruta ACP v1 y la v2: en debug los flags de equipo (`acp-beta`) están activos y en release no, así que el mismo agente puede ir por rutas distintas. Codex va por v2 en debug y por v1 en release; Claude siempre por v1.
- La app de prueba se lanza siempre con `--user-data-dir` aislado. No usar computer-use para traer la ventana al frente si eso puede lanzar otra instancia; si ocurre, cerrarla y avisar.

## Historial

- 2026-11-08: incorporadas al diseño las decisiones del spike y creado este estado inicial. Pendiente aprobación del plan e implementación del panel vacío.

- 2026-11-08: recibido mensaje #1; plan y paso 1 aprobados con ajustes. AGENTS.md adaptado antes de implementar.

## Verificación del paso 1

- `cargo fmt -p office` y `git diff --check`: correctos.
- Compilación correcta: `cargo rustc -p zed --bin uacode --release --offline -- -C opt-level=0` (LTO conservado; optimización reducida solo para el ejecutable de comprobación, sin cambios de configuración versionada). Log: `target/office-step1-build.log`.
- El intento release normal se interrumpió por tiempo/recursos; el intento con LTO desactivado falló al enlazar dependencias release existentes (LNK1136/símbolos sin resolver). El reintento compatible terminó con avisos LNK4217 de Wasmtime, sin errores.
- Prueba visual mediante computer-use: UACode arrancó, `office: toggle focus` apareció en la paleta, Office abrió vacío, cerró y reabrió desde el botón, y se movió entre docks. Queda abierto a la derecha.
- `disable_ai` reutiliza el mismo helper y observador de ajustes del panel de agentes; su cambio dinámico no se probó manualmente en esta sesión.
- No se añadieron avisos al README, ni se escribieron ajustes de cuentas, ni se hizo push ni se alteraron remotos.
- Limitación del panel mínimo: posición inicial derecha; el traslado no persiste su preferencia tras reiniciar.

- 2026-11-08: completado el paso 1 de Fase 1 (panel vacío). Próximo paso sujeto a confirmación: puestos persistentes.

- 2026-11-08, mensaje #2: aislado el error de carga de Office en zed.rs: se registra con log::error y no impide cargar AgentPanel. Paso 2 autorizado; pendiente prueba manual de disable_ai y persistencia de puestos.

- 2026-11-08: Codex se detuvo a mitad del paso 2; lo retoma Claude. El trabajo sin commit no compilaba (dos errores de préstamo en office_panel.rs) y los tests asíncronos descartaban sus errores (`gpui::test` ignora el `Result`). Corregido, añadido test de campos desconocidos, aviso de proyecto remoto y de puestos todos archivados.

## Verificación del paso 2

- `cargo test -p office`: 7 tests correctos. `cargo clippy -p office --all-targets -- --deny warnings` (debug): sin avisos. `cargo fmt -p office` aplicado.
- Tiempos del ciclo debug en esta máquina:
  - `cargo check -p office`, incremental: ~12 s.
  - `cargo test -p office`: primera compilación de tests 10 min 14 s; incremental ~15 s.
  - `cargo clippy -p office` (debug): 2 min 10 s la primera vez.
  - `cargo build -p zed` debug incremental: 1 min 49 s.
- `script/clippy` usa `--release`; para iterar se usa clippy en debug con las mismas opciones.
- Con la app debug (`target/debug/uacode.exe`) y carpeta de prueba en `target/office-step2-test/proyecto`:
  - Al abrir el panel no se crea `.uacode`; se crea al guardar el primer puesto.
  - Crear (con proveedor Codex y modelo), archivar, mostrar archivados y restaurar: correcto, JSON escrito como se espera.
  - JSON inválido: aviso con línea y columna, «Abrir archivo» abre office.json, «Nuevo puesto» deshabilitado. Versión futura: solo lectura con aviso.
  - disable_ai (en `.uacode/settings.json` del proyecto) activado y desactivado en caliente: el panel y su botón desaparecen y vuelven.
  - Dock movido a la izquierda, app reiniciada: se mantiene a la izquierda. Devuelto a la derecha.
- Con la app debug, abrir una carpeta por argumento avisa de que falta `cli.exe` (solo afecta a askpass); no impide el arranque.

- 2026-11-08, mensaje #3: los commits ya estaban en LF (`core.autocrlf=true` en el git de sistema normaliza al commitear; `git diff --stat --ignore-cr-at-eol` coincide con el normal). El CRLF solo estaba en la copia de trabajo y un git sin esa configuración lo ve como cambio de archivo completo. `.gitattributes` no tenía regla general; se añade `text eol=lf` solo para `crates/office/**` y `docs/office/**`, sin `* text=auto` global para no renormalizar archivos de upstream (hay uno con CRLF en el índice). Copias de trabajo de Office refrescadas a LF. Dependencias de `crates/office/Cargo.toml` ordenadas.

## Verificación del paso 3

- `cargo test -p office`: 19 tests (7 de office.json y 12 de cuentas: ida y vuelta, JSON inválido, versión futura, campos desconocidos como tokens, nombre duplicado, carpeta derivada del id, cuenta inexistente o de otro proveedor, cambio de proveedor, importación, carpetas reservadas o inseguras, archivo inexistente y edición externa). Clippy debug sin avisos. `cargo build -p zed` debug incremental: 48 s.
- Prueba manual con `--user-data-dir target/office-step3-test/data` y carpetas vacías `claude-default`, `claude-b` y `codex-default`, sin escribir en `%APPDATA%`:
  - Sin catálogo se ven las dos entradas «Por defecto».
  - «Importar carpetas existentes» importó solo `claude-b`, con `"folder": "claude-b"`.
  - Alta «Empresa»: carpeta derivada `claude-031a5164`, sin crear carpeta. «empresa» repetida: error claro.
  - Selector del puesto: «Por defecto», `claude-b` y «Empresa» (solo Claude). Guardado con el id.
  - Cuenta borrada a mano y «Recargar»: el puesto muestra «Sin cuenta» y conserva el id.
  - Cambio de proveedor a Codex: la cuenta vuelve a «Por defecto».
- Todavía no ha habido ninguna escritura real en `%APPDATA%\UACode\office`. La primera ocurrirá cuando el usuario añada o importe una cuenta en su UACode.

- 2026-11-09, mensaje #6: Fase A aprobada con ajustes (prefijo `office-` reservado, arreglo del `_meta terminal-auth`, sondeo bajo demanda con máximo 2, correo y plan solo en memoria). Fase B implementada en tres commits aislados.

## Verificación del paso 4

- Tests: `project` 18 de `agent_server_store` (3 nuevos: el agente dinámico sobrevive a cambios de ajustes y refrescos del registro, unregister emite `AgentServersUpdated`, gana a una entrada `office-…` de ajustes); `agent_ui` 3 de `agent_connection_store` (1 nuevo: la conexión del agente dinámico sigue tras recargar ajustes y se descarta al darlo de baja); `agent_servers` 8 de terminal-auth (3 nuevos: el `_meta` hereda el env del agente con el suyo encima, y sin agente registrado no cambia); `office` 28 (nombres, comandos y env por cuenta con claves vacías; registrar no crea carpetas; lectura de `_auth/status_update`; el sondeo mata el árbol de procesos tras el plazo; carpeta derivada de un id inválido rechazada).
- `cargo clippy -p office --all-targets -- --deny warnings` (debug) sin avisos. El repo prohíbe `smol::Timer`; el sondeo usa el timer del executor de GPUI.
- Tiempos debug: tests de `project` 4 min 41 s (primera vez), `agent_servers` 5 min, `agent_ui` 14 min 19 s; `cargo test -p office` 3 min 58 s; `cargo build -p zed` incremental 42 s – 5 min 37 s.
- Prueba manual con `--user-data-dir target/office-step4-test/data`: el panel muestra «Por defecto» de Claude y Codex con «Iniciar sesión» y «Comprobar». «Comprobar» en Claude sin carpeta: «Requiere login» al instante y la carpeta no se crea. En Codex con carpeta vacía: «Comprobando…» y luego «Requiere login» (sondeo real con codex-acp 2.5.0); después no queda ningún proceso `codex-acp`.
- Incidencia: al traer la ventana al frente con computer-use se lanzó una segunda instancia debug sin argumentos (datos reales). Se cerró sin tocar nada; no se creó `%APPDATA%\UACode\office`.
- No se ha hecho ningún login real ni se ha leído ningún archivo de credenciales.
- Prueba manual del usuario (mensaje #7) con `--user-data-dir` aislado: OK. Dos cuentas de Claude distintas (personal y PREICO) y Codex conectadas, cada una con su correo y su plan; se mantienen tras reiniciar; sin procesos `claude-agent-acp` ni `codex-acp` huérfanos tras cerrar UACode; el menú del panel de agentes muestra solo «Office · Claude · Por defecto», «Office · Claude · Prueba» y «Office · Codex · Por defecto».

- 2026-11-09, mensaje #8: diseño del paso 5 aprobado (clave `AgentId` con sesión en el contexto, `_meta` de sesión incluido, `AskUser` en modo solo para el usuario, protección contra bucles, `INITIAL_AGENT_MODE` y prueba manual autorizada). Fase B implementada y verificada; ver «Verificación del paso 5».

## Verificación del paso 5

- Tests: `acp_thread` 310 (7 nuevos en `policy.rs`: sin política espera al usuario y la política de otro agente no afecta; denegar resuelve con `reject_once` sin pedir nada, bloquea escritura y terminal y deja aviso; sin `reject_once` cancela; permitir nunca elige `allow_always`; preguntar deja permiso, escritura y terminal en manos del usuario; el segundo cambio de modo denegado del agente cancela el turno; semántica de `AskUser` en modo); `agent_servers` 183 (2 nuevos: `_meta` solo se envía con política; el modo del usuario denegado no se envía y el del agente se revierte con `session/set_mode`); `office` 28 (el env de Codex incluye `INITIAL_AGENT_MODE=read-only` y el de Claude no). `cargo check -p agent_ui -p agent` sin errores. Clippy de `acp_thread`, `agent_servers` y `office` (`--all-targets --deny warnings`) sin avisos.
- Tiempos debug: tests de `acp_thread` 3 min 55 s; `cargo build -p zed` incremental 2 min 39 s.
- Prueba manual con prompts reales (`--user-data-dir target\office-login-test\data`, proyecto desechable `target\office-step5-test\proyecto` con git init y sin remotos, rama `office-step5-test` con una política de solo consulta). Registro detallado en `target\office-step5-test\registro.md`.
  - a) Codex (5.6 Luna, Low), arranca en Read-only. Prompt 1, crear un archivo y ejecutar `echo`: permiso de «Edit files» denegado por la política; el rechazo de codex-acp para archivos es `cancel` y terminó el turno. Prompt 2, solo el comando: escalada de «Run command» denegada. Prompt 3, ejecutar dentro del sandbox sin pedir permisos: Codex volvió a pedir escalada y se denegó. `git status` limpio tras cada prompt. El sandbox de Windows no llegó a ejercitarse.
  - b) Claude (Haiku 4.5), arranca en Manual. El selector no ofrece «Bypass permissions» y elegir «Accept edits» no cambia el modo. Pedir editar `notas.txt`: Edit, Write y Bash «No such tool available … disabled for this session». `git status` limpio.
  - c) Claude con `.claude/settings.json` en el proyecto que preautoriza Write, Edit y Bash y fija `defaultMode: acceptEdits`: la sesión arranca en Manual (ajustes del proyecto ignorados) y Write/Bash siguen sin existir. `git status` limpio. Como `disallowedTools` quita las herramientas de raíz, la prueba de que se ignoran las reglas `allow` del proyecto es indirecta (el `defaultMode` ignorado).
  - Prompts enviados: 3 a Codex y 2 a Claude. Ningún caso escribió en disco.
  - Al cerrar UACode no quedó ningún proceso `uacode`, `claude-agent-acp` ni `codex-acp`. Elegir modelo escribió entradas `office-…` en el `settings.json` de los datos aislados (pendiente del paso 7).
  - Computer-use no lanzó otra instancia: la app se abrió desde la terminal y solo se pulsó dentro de su ventana.

- 2026-11-09, mensajes #9 y #10: paso 5 aprobado; paso 6 aprobado; UACode pasa a ser proyecto independiente (documentado en `05c26116e7`). Paso 6 implementado y verificado; ver «Verificación del paso 6».

## Verificación del paso 6

- Gancho ampliado (`b889f4f367`): la política recibe la versión que anuncia el agente, puede rechazar `session/new`, `load` y `resume` con motivo, y se le notifica cada petición que deja al usuario con el hilo; `AcpThread::answer_permission_request` y `permission_request_title` permiten responderla desde el panel.
- Política de Office (`95f568000e`): Claude con `disallowedTools` (Edit, Write, MultiEdit, NotebookEdit, Bash, PowerShell, Monitor), `allowDangerouslySkipPermissions: false` y `settingSources: []`; modos Claude `default`/`plan`, Codex `read-only`. Permisos de Claude: se deniegan escritura, borrado, movimiento, ejecución, red y cambio de modo; lectura, búsqueda y lo no clasificable van al usuario. Codex: toda petición es una escalada del sandbox y se deniega. Versión distinta de 0.94.0 / 2.5.0 o desconocida: sesión rechazada («Versión de adaptador no verificada») y todo denegado.
- `settingSources: []` funciona: Claude se autentica con la carpeta de la cuenta y responde (caso c).
- Tests: `acp_thread` 311, `agent_servers` 187 (v1: sesión rechazada, modo por opción de configuración; v2: `_meta`, modo del usuario rechazado, cambio del agente revertido, sesión rechazada), `office` 34 (decisión por tipo de petición y modo, guarda de versión, opciones de sesión de Claude, eventos al panel, solo los agentes `office-*` reciben política). Clippy sin avisos.
- Prueba manual (datos aislados, proyecto desechable; registro en `target\office-step5-test\registro.md`): c) ajustes del proyecto ignorados (arranca en Manual), Write/Bash no existen; b) Edit/Write no existen; aprobación: lectura fuera del proyecto → «Esperando aprobación (1)» en la tarjeta → «Aprobar» → Claude leyó el archivo y la petición desapareció del hilo y de la tarjeta; Codex: escalada denegada y bloqueo en la tarjeta. `git status` limpio tras cada prompt. Rechazo de modo del usuario visible en la tarjeta («Bloqueado: …» con «Descartar»).
- Fallo encontrado en la prueba y corregido (`f5ec3d7830`): en debug Codex usa ACP v2 y el modo no pasaba por la política (se pudo poner «Full access»; se devolvió a Read-only sin enviar prompts). Recomprobado tras la corrección.
- Maquetación (`e2178eeb4f`): títulos y motivos largos dejaban los botones fuera de la tarjeta; ahora van truncados y con los botones en su propia fila.
- Prompts: 3 a Claude (b, c y lectura de aprobación) y 1 a Codex. Sin procesos huérfanos al cerrar.

- 2026-11-09, mensaje #11: paso 6 aprobado; rama `office-step5-test` borrada; política de sesión centralizada; paso 7 implementado. Ver «Verificación del paso 7».

## Verificación del paso 7

- Centralización (`32e385fb09`): decisiones de inicio de sesión, modo por defecto, cambio del usuario y del agente en `acp/session_policy.rs`; v1 comprueba también cambios de modo por opción de configuración cuando la sesión no tiene modos heredados. Tests `consult_only_modes_on_v1_connection` (con control: sin política el modo por defecto sí se aplica) y `v2_consult_only_modes_on_v2_connection`.
- Preferencias (`521a13de70`, `17b3083437`): `ThreadPolicy::uses_saved_preferences`; para los agentes de Office no se leen ni escriben modo, modelo, opciones por defecto ni favoritos en settings.json (lecturas y escrituras de `CustomAgentServer` y valores por defecto de v1 y v2). Test con FakeFs y control con un agente normal.
- Hilos por puesto (`9cedc971aa`, `5090e52652`, `a73cd5518c`): `post_threads.rs` guarda puesto → hilo en la base local (`office-post-thread:<carpeta del proyecto>:<id de puesto>`), nunca en office.json; sesión → hilo → puesto con `ThreadMetadataStore::entry_by_session`. La política resuelve el puesto en `rules_for` y sus eventos llevan el puesto; los hilos fuera de puestos tienen las reglas de Fase 1 y no salen en ninguna tarjeta. Si el puesto cambió de cuenta, se crea un hilo nuevo con el agente nuevo.
- Tests: `acp_thread` 311, `agent_servers` 191, `office` 38. Clippy sin avisos. Un test inestable del paso 4 (orden del id aleatorio) corregido en `6f0c6e0e55`.
- Prueba manual (datos aislados, proyecto desechable, 3 prompts; registro en `target\office-step5-test\registro.md`):
  - Dos puestos con la misma cuenta (Claude · Por defecto): cada clic creó su propio hilo; la lectura pendiente del puesto «Revisión Claude» apareció solo en su tarjeta.
  - Reinicio: ambas tarjetas conservan su hilo y cada clic abre su historial.
  - «Cancelar» desde la tarjeta detuvo el turno en curso.
  - Elegir modelo (Haiku) y cambiar de modo (Plan → Manual) en hilos de Office: settings.json con el mismo hash al empezar y al terminar. Un hilo nuevo vuelve a mostrar el modelo por defecto (las preferencias no se guardan).
  - Fallo: «Aprobar»/«Rechazar» de la tarjeta no responden (ver pendientes). Un primer intento con la tarjeta entera clicable se tragaba los clics; se corrigió en `a73cd5518c`, pero el fallo siguió.
  - «Nuevo hilo» no se probó con prompt; usa la misma ruta de creación que el primer clic.
- Disco: la compilación agotó el disco D; se borró solo `target\debug\incremental` (98,5 GB de caché regenerable).

- 2026-11-09, mensaje #12: paso 7 aprobado salvo el fallo de la tarjeta; paso 8 implementado. Ver «Verificación del paso 8».

## Verificación del paso 8

- Test de UI `approving_and_rejecting_from_the_card_answer_the_thread` (OfficePanel en `VisualTestContext`, hilo sobre `StubAgentConnection` con agente `office-claude-default` 0.94.0, `PolicyEvent::Pending` real, `simulate_click` sobre `.debug_selector`): verde a la primera. El clic llega; la causa del fallo en la app no se reprodujo.
- Robustez (`a160906647`, `3bf14eedf4`): `answer_permission_request` devuelve `bool` y avisa con `log::warn`; una petición legacy cuya tool call ya no apunta a ella se resuelve directamente; la tarjeta quita la entrada si no se pudo responder, si la petición desaparece del hilo (cancelada, turno cancelado) o con «Nuevo hilo». Tests `a_request_cancelled_in_the_thread_leaves_the_card` y `an_unanswerable_approval_is_removed_from_the_card`.
- Estado del puesto (`ad735dd35f`, `post_activity.rs`): Requiere login > Error > Esperando aprobación > Trabajando > Libre; tarea actual (paso del plan en curso o tool call activa) y último mensaje del agente, truncados a 140 caracteres; refresco como máximo cada 500 ms y solo si cambia. Test `cards_show_state_and_last_message_with_limited_refresh`.
- Título (`edbc7018e6`, `d7b4990341`): `CreateThreadOptions` aplica el título desde la creación; la cabecera del panel de agente muestra el título de un hilo vacío en vez de «New … Thread».
- Cierre (`2631087fe6`): la conexión ACP v1 no implementaba `shutdown()` y el adaptador vivía hasta que se soltaba la última referencia. Ahora suelta el hijo y el job object mata el árbol. Con Claude y Codex abiertos colgaban 8 procesos de `uacode.exe`; a los 3–5 s de cerrar no quedaba ninguno.
- Prueba manual sin prompts (datos aislados): tarjetas con «Libre» y último mensaje; «Nuevo hilo» en «Consulta Codex» con título en tarjeta y cabecera; cierre limpio dos veces. Encontrado el título perdido del borrador al reiniciar (ver pendientes).
- Tests: `acp_thread` 311, `agent_servers` 191, `agent_ui` 586, `office` 44. Clippy sin avisos.
- Disco: `target/debug` 112 GB (incremental 46,8, deps 59, build 4,1). Propuesta en el buzón, sin aplicar.

- 2026-11-09, mensaje #7: paso 4 aprobado. Regla nueva sobre instancias de prueba. Paso 5 aprobado en dos fases; entregado el diseño de la Fase A (puntos de permiso, escritura, terminal y modo; interfaz `ThreadPolicy` por `AgentId`; límites de cada adaptador leídos en su código fijado).
