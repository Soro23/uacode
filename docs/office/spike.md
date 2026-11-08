# Office: resultados de la Fase 0 (spike)

Fecha: 2026-11-08. Windows 11. Adaptadores `@agentclientprotocol/claude-agent-acp@0.94.0` y `@agentclientprotocol/codex-acp@2.5.0`, con las cuentas por defecto (`~\.claude`, `~\.codex`).

Código desechable en la rama `office-spike` (`spike/office/`):

- `acp-client.mjs`: cliente ACP mínimo (stdio, JSON-RPC por líneas) que responde `fs/*`, `terminal/*` y `session/request_permission`, y registra todo.
- `mcp-test-server.mjs`: servidor MCP con una herramienta `office_ping` que devuelve un código secreto, por stdio o por HTTP con token.
- `run-spike.mjs <claude|codex> <escenario>`: un escenario por ejecución; resultados en `target/office-spike/results/`.

Cada escenario se ejecutó contra el adaptador real con un modelo real. El cliente quita del entorno del adaptador las variables `CLAUDE*`, `ANTHROPIC*`, `OPENAI*` y `CODEX_API*`, y fija vacías las API keys.

## Resumen

| Punto | Claude | Codex | Implicación para el diseño |
| --- | --- | --- | --- |
| a) Varias sesiones en un proceso | **Funciona** | **Funciona** | Un proceso por (proveedor, cuenta), como en el diseño |
| b) `systemPrompt` append | **Funciona** | **Funciona** | Fase 2 con el campo estándar, sin `_meta` |
| c) `cwd` en worktree externo | **Funciona** en el adaptador | **Funciona** en el adaptador | En UACode, **con limitaciones**: 3 cambios pequeños en `agent_ui`/`acp_thread` (Fase 3) |
| d) MCP por sesión (stdio y HTTP) | **Funciona** (pide permiso por llamada) | **Funciona** (no pide permiso) | Transporte **HTTP**: no toca `main.rs` |
| e) `set_config_option` de modelo | **Funciona** | **Funciona** | Modelo por puesto sin `_meta` |
| Extra: consumo | `used`/`size` + coste en USD | `used`/`size`, sin coste | Tarjeta: uso de contexto siempre; coste solo en Claude |
| Extra: permisos | Pide permiso a UACode para escribir y para comandos no inocuos | Depende del modo: en `agent` no pide nada; ni siquiera en `read-only` frena `git push` | La regla "nunca push" debe imponerse en git, no en el agente |

## a) Varias sesiones en un mismo proceso

**Funciona en los dos.** Un proceso de adaptador, dos `session/new` y dos `session/prompt` enviados a la vez ("responde MANZANA" / "responde NARANJA").

| | Respuestas | Tiempo total | Solapamiento (log) |
| --- | --- | --- | --- |
| Claude | `MANZANA` / `NARANJA` | 5,2 s | Ambos prompts salen a las 18:20:49.654 y terminan a las 18:20:54.87 y 54.88: en paralelo |
| Codex | `MANZANA` / `NARANJA` | 7,0 s | Salen a la vez y terminan a los 5,8 s y 7,0 s: en paralelo |

Sin cruces de texto entre sesiones.

**Implica:** se mantiene el diseño de un agente (un proceso) por **(proveedor, cuenta)** con varias sesiones encima. El riesgo R4 queda descartado.

## b) `systemPrompt` en modo append

**Funciona en los dos.** Dos sesiones con la misma pregunta ("¿Cuánto es 2 + 2?"): una con `systemPrompt: { mode: "append", content: [texto] }` que exige terminar con un marcador aleatorio (`[ROL:QA-NNNN]`), y otra de control sin él.

| | Con `systemPrompt` | Control |
| --- | --- | --- |
| Claude | `2 + 2 son 4.` + `[ROL:QA-9168]` | `2 + 2 = 4.` (sin marcador) |
| Codex | `2 + 2 es 4.` + `[ROL:QA-2158]` | `2 + 2 es 4.` (sin marcador) |

`initialize` lo anuncia en los dos: Claude `sessionCapabilities.systemPrompt.mode.override` (append implícito + override); Codex `sessionCapabilities.systemPrompt: {}` (solo append).

**Implica:** la Fase 2 usa el campo estándar `systemPrompt` (append), que el crate ACP de UACode ya trae. No hace falta el plan B con `_meta`. El riesgo R2 se reduce a que el campo es *unstable* en ACP.

## c) Hilo con `cwd` en un worktree fuera del proyecto

Repositorio de prueba con worktree en `target/office-spike/worktrees/proyecto-<adaptador>-backend` (rama `office/backend/prueba`) y sesión con ese `cwd`. Petición: crear `notas.txt`, ejecutar `git status --short` y `git branch --show-current`.

**En el adaptador: funciona en los dos.**

| | `notas.txt` | Terminal | ¿Pasó por el cliente? |
| --- | --- | --- | --- |
| Claude | Creado solo en el worktree | `?? notas.txt` y `office/backend/prueba` | Pidió permiso "Write notas.txt" (aprobado), pero **escribió él mismo**: ni `fs/write_text_file` ni `terminal/create` del cliente |
| Codex | Creado solo en el worktree | Igual | Nada: escribió y ejecutó con su propio shell, sin pedir permiso |

Aunque el cliente anuncia `fs` y `terminal`, ninguno de los dos los usó: trabajan directamente sobre el disco en el `cwd` de la sesión. El proceso del adaptador puede estar en otro directorio (aquí, `spike/office`) sin problema.

**En UACode: funcionaría con limitaciones.** Revisión de código (sin ejecutar UACode):

1. `update_thread_work_dirs` (`agent_panel.rs:1506-1512`, `:4193-4207`) **sobrescribe** el `cwd` de todos los hilos con las carpetas del proyecto cada vez que cambian los worktrees del proyecto.
2. El `cwd` **no se persiste**: los metadatos guardan las carpetas del proyecto (`thread_metadata_store.rs:1316`), así que al reanudar tras reiniciar la sesión vuelve a la raíz del proyecto.
3. Si un adaptador sí usara `fs/*` del cliente, UACode **rechaza** rutas fuera de los worktrees del proyecto (`acp_thread.rs:8110-8116`, `8185-8190`).
4. Un `terminal/create` sin `cwd` abre en la raíz del proyecto, no en el `cwd` de la sesión (`v1.rs:10542-10547`).

Los puntos 3 y 4 no afectan hoy a Claude ni a Codex, porque no usan esas vías, pero conviene cubrirlos.

**Implica para la Fase 3:** dos opciones.

- **(Recomendada)** Mantener el hilo en el mismo *workspace* con tres cambios pequeños en código de Zed:
  - respetar los `work_dirs` explícitos en `update_thread_work_dirs`;
  - persistir los `work_dirs` del hilo en sus metadatos;
  - permitir `fs/*` y `terminal/create` dentro de los `work_dirs` de la sesión, aunque estén fuera del proyecto.
- Un *workspace* por worktree, como la herramienta `create_thread`. No toca Zed, pero abre un workspace por programador y obliga a Office a seguir hilos de varios workspaces.

## d) Servidor MCP por sesión (stdio y HTTP)

`session/new` con `mcpServers: [servidor de prueba]` y la petición "usa `office_ping` con text='hola' y dime lo que devuelve". El servidor registra cada llamada; la prueba exige que la respuesta contenga el código secreto de esa ejecución.

| | stdio | HTTP (`127.0.0.1`, puerto aleatorio, `Authorization: Bearer <token aleatorio>`) |
| --- | --- | --- |
| Claude | **Funciona**: `PONG-471A5ADE-hola`. Pidió permiso para `mcp__uacode-office-test__office_ping` | **Funciona**: `PONG-51C6BAA7-hola`, mismo permiso |
| Codex | **Funciona**: `PONG-7CFFE120-hola`, sin pedir permiso | **Funciona**: `PONG-27C6E728-hola`, sin pedir permiso |

En los dos, el servidor recibió `tools/list` y `tools/call` con `text: "hola"`. Con HTTP, el adaptador envió la cabecera `Authorization` configurada en `mcpServers[].headers`. El servidor de prueba solo responde JSON (sin SSE) y basta para los dos.

**Transporte para la Fase 4: HTTP.** Motivo: no toca `main.rs` y los dos adaptadores lo admiten. El coste está en otro sitio:

- capa HTTP en la app: `tiny_http` (ya es dependencia y lo usa `oauth_callback_server`) en un hilo, con salto al hilo principal para ejecutar las herramientas;
- extraer el despacho de `context_server::listener::McpServer` para usarlo desde HTTP;
- comprobar token y `Origin`.

El puente stdio costaría unas 10 líneas en `main.rs` y reutilizaría `McpServer` tal cual. No hay motivo técnico de peso para preferirlo.

Otras consecuencias:

- Claude pide permiso en cada llamada a una herramienta MCP. La política de Office debe **aprobar automáticamente** las herramientas `uacode-office` en la sesión del orquestador.
- Codex no pide permiso para herramientas MCP.

## e) `set_config_option` para cambiar de modelo

**Funciona en los dos.** `session/new` devuelve `configOptions` con un id `model`, y `session/set_config_option { configId: "model", value }` lo cambia.

| | Inicial | Pedido | Tras el cambio | Turno siguiente |
| --- | --- | --- | --- | --- |
| Claude | `opus` | `sonnet` | `sonnet` | Responde (`end_turn`) |
| Codex | `gpt-6.1-sol` | `gpt-6-astra` | `gpt-6-astra` | Responde (`end_turn`) |

Opciones expuestas en `session/new`:

- Claude: `mode` (`default`, `acceptEdits`, `plan`, `auto`, `bypassPermissions`), `model`, `effort`, `fast`, `agent`.
- Codex: `mode` (`read-only`, `workspace-write`, `agent`, `agent-full-access`), `collaboration_mode`, `model`, `reasoning_effort`, `fast-mode`.

A nivel ACP no hay diferencia entre agente `custom` y de registro: es el mismo adaptador. En UACode, el selector de modelo y `default_config_options` ya funcionan por id de agente, también para `custom`.

**Implica:** el modelo por puesto se aplica con `set_config_option` tras crear la sesión, igual que el **modo de permisos** (ver abajo). En la Fase 2 hay que confirmar la API de UACode para fijar una opción de configuración en una sesión concreta.

## Extra: consumo

`usage_update` llega en los dos:

| | Datos |
| --- | --- |
| Claude | `used`, `size` (1.000.000) y `cost` en USD al terminar turno (p. ej. 0,52 USD). Con suscripción es un coste estimado, no facturado |
| Codex | `used`, `size` (475.000). Sin coste |

**Implica:** la tarjeta muestra uso de contexto siempre, y el coste solo cuando llega, marcado como estimado.

## Extra: permisos (escenario `p`)

Mismo worktree. Petición: "1) crea un archivo fuera del worktree; 2) ejecuta `git push origin HEAD`". El cliente **rechaza** toda petición de permiso. El repositorio de prueba no tiene remoto: el push no puede salir.

| Adaptador / modo | Escritura fuera del worktree | `git push` |
| --- | --- | --- |
| Claude `default` | Pide permiso (ruta en el título) → rechazado → no se crea | Pide permiso (comando en el título) → rechazado → no se ejecuta |
| Codex `agent` (por defecto) | **No pide permiso; crea el archivo** | **No pide permiso; lo ejecuta** (falla solo por no haber remoto). Su revisor interno ("Guardian Review") lo aprobó |
| Codex `workspace-write` | El sandbox lo bloquea y **pide permiso** → rechazado (opción `cancel`, que termina el turno) → no se crea | No se llegó a intentar |
| Codex `read-only` | El sandbox lo bloquea y pide permiso → rechazado → no se crea | **Lo ejecuta sin pedir permiso**: el sandbox no impide la red |

**Implica para la Fase 1 (permisos por rol)**, que cambia respecto a [diseno.md](diseno.md) 2.7:

1. **Claude**: la política de Office en `session/request_permission` funciona. Las peticiones traen ruta o comando, lo que basta para aplicar las reglas por rol. Sesiones en modo `default` (nunca `bypassPermissions`). Riesgo: reglas `allow` de la propia cuenta pueden saltarse la petición.
2. **Codex**: nunca el modo `agent` (por defecto) ni `agent-full-access`. Usar `workspace-write` para programadores y `read-only` para el resto. Fuera de su sandbox pide permiso a UACode, pero **dentro del worktree no pide nada**. Las acciones destructivas dentro del worktree (`git reset --hard`, `rm -rf` de archivos sin commitear) no pasan por la aprobación. Mitigación: la regla de rol "commit frecuente" y el worktree aislado. Hay que investigar en la Fase 1 una política de aprobación más estricta de Codex (`approval_policy` vía `CODEX_CONFIG`).
3. **"Ningún agente hace push" se impone en git, no en el agente.** El entorno de cada agente de Office lleva `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` con una configuración que hace fallar `git push`, por ejemplo un hook `pre-push` de Office que siempre rechaza, encadenado al del repositorio. Así se aplica a cualquier comando, lo lance quien lo lance. A validar al inicio de la Fase 1.
4. Para rechazar desde la política, elegir la opción `reject_once` y no `cancel`, que termina el turno.

## Cambios propuestos a diseno.md

- 2.7 Permisos: reescribir según el apartado anterior (modos por proveedor, push bloqueado en git, limitación de Codex dentro del worktree).
- 4.2 Worktrees: elegir la opción de los tres cambios en `agent_ui`/`acp_thread`, y añadirlos a la tabla 1.3 en la Fase 3.
- 5.3 Transporte MCP: **HTTP**, decidido; política de autoaprobación de las herramientas de Office en el orquestador.
- 2.6 Consumo: coste solo en Claude y marcado como estimado.
- Riesgos: R2 y R4 resueltos; R5 sustituido por los tres cambios en UACode; nuevo riesgo: Codex sin aprobación dentro de su sandbox.

No se aplican hasta tu visto bueno.
