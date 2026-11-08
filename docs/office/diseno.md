# Office: diseño

Estado: **propuesta, pendiente de visto bueno**. No hay código escrito.

Office es un panel de UACode con una oficina de agentes especializados (orquestador, PM/analista, diseñador, arquitecto, scrum master, tech lead/revisor, frontend, backend, QA, devops, seguridad, documentador). Cada puesto usa una IA (Claude Code o Codex vía ACP) con una cuenta concreta.

Base: la prueba de [cuentas.md](cuentas.md) (agentes `custom` con `CLAUDE_CONFIG_DIR` / `CODEX_HOME`, login desde el panel, sesiones en paralelo).

Versiones revisadas: `agent-client-protocol` 5.0.0 (schema 1.15.1), `claude-agent-acp` 0.94.0, `codex-acp` 2.5.0.

## Resumen de viabilidad

| Pregunta | Respuesta |
| --- | --- |
| ¿Se puede hacer como crate aparte? | Sí. El código de Zed se toca en pocos puntos (ver 1.3). |
| ¿Se puede leer el estado de cada agente? | Sí, desde `AcpThread` (estado, permisos pendientes, errores, plan, mensajes). |
| ¿Instrucciones de rol por sesión? | Sí, con el campo estándar ACP `systemPrompt` en modo **append**: lo aceptan los dos adaptadores y el crate de UACode ya lo trae. Hay que añadir el paso de UACode al adaptador. |
| ¿MCP propio solo para el orquestador? | Sí. ACP admite `mcpServers` por sesión y los dos adaptadores lo usan (Claude: stdio/http/sse; Codex: stdio/http). UACode no expone hoy ningún servidor MCP: hay que añadirlo. |
| ¿Un worktree por programador? | Sí. Ya existe creación de worktrees y el cwd de la sesión es configurable (`work_dirs`). No existe `git merge` en el backend. |

## 1. Arquitectura

### 1.1 Crate nuevo `office`

`crates/office/` con `[lib] path = "src/office.rs"`, siguiendo el modelo de `crates/acp_tools`. Módulos previstos (un archivo por componente lógico, sin `mod.rs`):

| Módulo | Contenido |
| --- | --- |
| `office.rs` | `init(cx)`, acciones (`office::ToggleFocus`, `office::NewPost`, `office::ManageAccounts`), re-exports |
| `office_panel.rs` | `OfficePanel` (implementa `workspace::Panel`), lista de tarjetas |
| `post_card.rs` | Tarjeta de puesto (`RenderOnce`) |
| `post_state.rs` | Cálculo del estado de un puesto a partir de `AcpThread` (funciones puras, con tests) |
| `accounts.rs` | Modelo de cuentas, carpetas, estado de login |
| `accounts_modal.rs` | Gestor de cuentas (modal) |
| `office_store.rs` | Carga/guarda `office.json` y `accounts.json`, sincroniza `agent_servers` |
| `roles.rs` | Plantillas de rol (Fase 2) |
| `office_mcp.rs` | Servidor MCP del orquestador (Fase 3) |
| `worktrees.rs` | Worktree por puesto e integración (Fase 4) |

### 1.2 Piezas existentes que se reutilizan

| Pieza | Dónde | Uso en Office |
| --- | --- | --- |
| `AgentServerStore` | `crates/project/src/agent_server_store.rs` | Lista de agentes (`external_agents()`), evento `AgentServersUpdated`. Los agentes de Office se registran como `custom` en `agent_servers` (ver 2.4). |
| `AgentPanel::create_thread_with_options` | `crates/agent_ui/src/agent_panel.rs:3233` | Crear el hilo de un puesto: `agent: Agent::Custom { id }`, `title`, `work_dirs`, `initial_content` con `auto_submit`. Devuelve `ThreadId`. |
| `AgentPanel::load_agent_thread` | `agent_panel.rs:4502` | Clic en tarjeta: abrir el hilo (si no está en memoria lo reanuda por `session_id`). Es lo que hace la barra lateral de hilos (`crates/sidebar/src/sidebar.rs:3698`). |
| `AgentPanel::retained_threads()` / `ConversationView::root_thread_view()` | `agent_panel.rs:4042`, `conversation_view.rs:896` | Llegar al `Entity<AcpThread>` de cada puesto para leer su estado. |
| `AgentConnectionStore` | `crates/agent_ui/src/agent_connection_store.rs` | Una conexión (un proceso) por agente; varias sesiones encima. `connection_status()` para el estado "conectando". |
| `AcpThread` | `crates/acp_thread/src/acp_thread.rs` | Estado, eventos, permisos, plan, mensajes (ver 2.2). |
| `ThreadMetadataStore` | `crates/agent_ui/src/thread_metadata_store.rs` | `session_id`, `agent_id`, título y rutas persistidos: reabrir el hilo de un puesto tras reiniciar. |
| Flujo de login ACP | `conversation_view.rs` (`authenticate`) | Login de cuentas sin código nuevo (probado en cuentas.md). |
| Worktrees | `crates/git/src/repository.rs` (`create_worktree`, `remove_worktree`), `crates/project/src/git_store.rs` | Worktree por programador (Fase 4). |
| Revisión de rama | `crates/git_ui/src/branch_diff.rs` (`review_diff`, `ReviewBranchDiff`) | Paso de revisión (Fase 4). |
| `context_server::listener::McpServer` | `crates/context_server/src/listener.rs` | Servidor MCP en proceso, existe pero nadie lo usa (Fase 3). |

### 1.3 Puntos de enganche en código de Zed

Mínimos e imprescindibles:

| Fase | Archivo | Cambio |
| --- | --- | --- |
| 1 | `Cargo.toml`, `crates/zed/Cargo.toml` | Añadir el crate `office`. |
| 1 | `crates/zed/src/main.rs` (~722) | `office::init(cx)` tras `agent_ui::init`. |
| 1 | `crates/zed/src/zed.rs` (`initialize_panels`, ~778) | Cargar `OfficePanel` con `setup_or_teardown_ai_panel` para que desaparezca con `disable_ai`, igual que `AgentPanel`. `activation_priority` libre: 4 u 8+ (se usan 0-3 y 5-7; repetir una prioridad hace `panic` en debug). |
| 2 | `crates/acp_thread/src/connection.rs` | Opciones de sesión (`SessionOptions { system_prompt, extra_mcp_servers }`) en `new_session` / `load_session` / `resume_session`, con valor por defecto vacío para no romper a los demás implementadores. |
| 2 | `crates/agent_servers/src/acp/v1.rs` (~1209) y `v2.rs` (~1296) | Poner `system_prompt` en `NewSessionRequest` (y load/resume) si el agente anuncia `sessionCapabilities.systemPrompt`; añadir `extra_mcp_servers` a los del proyecto. |
| 2 | `crates/agent_ui/src/agent_panel.rs` (`CreateThreadOptions`, 935) y `conversation_view.rs` (`initial_state`, 1317) | Llevar `SessionOptions` desde la creación del hilo hasta `new_session`, y guardarlas para reanudar. |
| 3 | `crates/zed/src/main.rs` | Puente stdio ↔ socket para el MCP del orquestador (antes existía como `--nc`). |
| 4 | `crates/git/src/repository.rs` + `git_store.rs` | `merge` (no existe) si la integración la hace UACode y no el agente integrador (ver 5.3). |

Fuera de esto, todo vive en `crates/office`. Cada cambio en Zed debe ir en un commit aislado para facilitar futuras sincronizaciones con Zed.

## 2. Fase 1: panel y puestos

### 2.1 Panel

Panel acoplable (por defecto a la derecha, junto al de agentes), con:

- Cabecera: nombre del proyecto, botón **Nuevo puesto**, botón **Cuentas**.
- Una tarjeta por puesto:

```
┌──────────────────────────────────────────────┐
│ ● Backend                       Trabajando   │
│ Claude · cuenta "empresa" · sonnet           │
│ Tarea: Añadir endpoint /users                │
│ «Creando la migración de la tabla users…»    │
└──────────────────────────────────────────────┘
```

  rol, proveedor, cuenta, modelo, estado, tarea actual y último mensaje (una línea, truncada).
- Clic en la tarjeta: `AgentPanel::load_agent_thread(Agent::from(agent_id), thread_id, …)` y `workspace.focus_panel::<AgentPanel>()`. Si el puesto aún no tiene hilo, se crea con `create_thread_with_options`.
- Menú contextual: editar ficha, nuevo hilo, cancelar (`AgentPanel::cancel_thread`), archivar.

Relación puesto ↔ hilo: cada puesto guarda el `ThreadId` de su hilo activo en `office.json`. Tras reiniciar, `ThreadMetadataStore::entry(thread_id)` da `session_id` y el hilo se reanuda al abrirlo.

### 2.2 Estado a partir de ACP

Se calcula en `post_state.rs` observando el `AcpThread` del puesto (`cx.subscribe` a `AcpThreadEvent`) y la conexión.

| Estado | Condición (en orden de prioridad) | Fuente |
| --- | --- | --- |
| **Error** | Evento `Error`, `ConnectionLost` o `LoadError`; o `had_error()` | `AcpThreadEvent`, `AcpThread::had_error` |
| **Sin sesión** | La conexión pide autenticación | estado de autenticación de `ConversationView` (ver riesgo R3) |
| **Esperando aprobación** | `is_waiting_for_confirmation()` o hay `pending_permission_requests()`; también `ElicitationRequested` sin respuesta | `ToolAuthorizationRequested/Received`, `ElicitationRequested/Responded` |
| **Trabajando** | `status() == ThreadStatus::Generating` (o `foreground_activity() == Running`) | `StatusChanged` |
| **Conectando** | `AgentConnectionStore::connection_status == Connecting` | store de conexiones |
| **Libre** | `ThreadStatus::Idle` o el puesto no tiene hilo | `Stopped`, `ActivityConfirmedIdle` |

- **Tarea actual**: entrada en curso del plan (`plan()?.stats().in_progress_entry`); si no hay plan, `title()` del hilo; si no, el texto del último mensaje del usuario.
- **Último mensaje**: último `AgentThreadEntry::AssistantMessage`, solo chunks `Message` (no `Thought`), texto plano en una línea. Se recalcula con `NewEntry` / `EntryUpdated` con un pequeño *throttle* para no repintar en cada token.

### 2.3 Gestor de cuentas

Según los requisitos de cuentas.md:

- **Añadir cuenta**: proveedor (Claude / Codex) + nombre. Crea `<config>/accounts/<proveedor>-<nombre>` (`paths::config_dir()`, en Windows `%APPDATA%\UACode\accounts\…`), registra el agente de esa cuenta (2.4) y abre un hilo con él para que salga el botón de login del adaptador (flujo ya probado).
- **Estado** de cada cuenta:
  - *Conectada*: el último intento de sesión con esa cuenta fue bien.
  - *Sin sesión / caducada*: el adaptador pidió autenticación.
  - *Sin probar*: no hay datos.
  - Identidad mostrada: Claude, `oauthAccount.emailAddress` de `<carpeta>/.claude.json`; Codex, correo del `id_token` de `<carpeta>/auth.json`. Solo se leen campos de identidad, nunca tokens, y no se registran en logs.
- **Por defecto**: opción especial que no fija `CLAUDE_CONFIG_DIR`/`CODEX_HOME` (usa `~/.claude` o `~/.codex`).
- **Selección por puesto**: desplegable de cuentas filtrado por el proveedor del puesto, con "Por defecto" como valor inicial.
- **Borrar cuenta**: quita el registro; borrar la carpeta pide confirmación aparte.

### 2.4 Registro de agentes

Un agente por **(proveedor, cuenta)**, no por puesto: los puestos que comparten cuenta comparten proceso y se distinguen por sesión (instrucciones de rol por sesión, Fase 2). Así doce puestos con dos cuentas son dos procesos, no doce.

- Id del agente: `office-<proveedor>-<cuenta>` (por ejemplo `office-claude-empresa`).
- Office escribe esas entradas `custom` en `agent_servers` de los ajustes globales con `update_settings_file` (mismo esquema que en cuentas.md: `npx.cmd`/`npx`, versión fijada, `CLAUDE_CONFIG_DIR`/`CODEX_HOME`, API keys vacías). Las entradas `office-*` las gestiona Office: se regeneran desde `accounts.json`.
- Ventaja: cero cambios en `AgentServerStore`, y el mecanismo ya está probado. Inconveniente: aparecen también en el menú del panel de agentes. Alternativa futura: API de registro en memoria en `AgentServerStore` (hoy `reregister_agents` vacía el mapa en cada cambio de ajustes, habría que mantener un mapa aparte).

### 2.5 Ficha de puesto y almacenamiento

| Archivo | Ámbito | Contenido | ¿Secretos? |
| --- | --- | --- | --- |
| `<config>/office/accounts.json` | Global (usuario) | Cuentas: id, proveedor, nombre, carpeta, último estado conocido | No (solo rutas) |
| `<proyecto>/.uacode/office.json` | Proyecto (se puede versionar) | Puestos del proyecto | No |
| `<proyecto>/.uacode/office/roles/<rol>.md` | Proyecto | Instrucciones de cada rol | No |
| `<config>/office/roles/<rol>.md` | Global | Plantillas por defecto de los roles | No |

Ficha de puesto en `office.json`:

```jsonc
{
  "posts": [
    {
      "id": "backend",
      "role": "backend",                 // clave de rol → roles/backend.md
      "name": "Backend",                 // nombre visible
      "provider": "claude",              // claude | codex
      "account": "empresa",              // id de cuenta o "default"
      "model": "sonnet",                 // opcional; vacío = el del agente
      "instructions": "roles/backend.md",// ruta relativa a .uacode/office/
      "extra_instructions": "",          // texto corto específico del proyecto
      "worktree": true,                  // programador: trabaja en su worktree (Fase 4)
      "thread_id": null                  // lo rellena Office
    }
  ]
}
```

La cuenta va por id, nunca por ruta, para que `office.json` sea portable entre máquinas: cada usuario resuelve el id en su `accounts.json`. Si la cuenta no existe en la máquina, el puesto aparece como "Sin cuenta".

## 3. Fase 2: roles y directrices

### 3.1 Qué admite cada adaptador

| Mecanismo | Claude (`claude-agent-acp` 0.94.0) | Codex (`codex-acp` 2.5.0) |
| --- | --- | --- |
| ACP estándar `systemPrompt` en `session/new` | **Sí**, `append` y `override` (`client-system-prompt.js:15-39`); prevalece sobre `_meta.systemPrompt` | **Sí, solo `append`** → `developerInstructions` de `thread/start` (`index.js:41741-41759`); `override` da error |
| `_meta.systemPrompt` | Sí: string sustituye, objeto `{append}` añade | No |
| `_meta.claudeCode.options` | Sí: pasa opciones al SDK (`model`, `settingSources`…) | No |
| `_meta.jetbrains.air.customInstructions` | Sí, se añade al final | Sí, como `developerInstructions` |
| Archivos de instrucciones | `CLAUDE.md` vía `settingSources` | `AGENTS.md` y `config.toml` (`developer_instructions`) en `CODEX_HOME` |
| Al reanudar (`load`/`resume`) | Se reaplica (reconstruye la sesión) | Solo si el hilo está inactivo y persistido |

Lado UACode: `agent-client-protocol` 5.0.0 trae `NewSessionRequest::system_prompt(SystemPrompt)` (y en load/resume) bajo la feature `unstable_session_system_prompt`, que ya está activa (la incluye `unstable`, que UACode activa en `Cargo.toml`). Hoy UACode no rellena ese campo.

### 3.2 Propuesta

- Usar **solo** `systemPrompt` en modo **append**, el denominador común de los dos adaptadores y el mecanismo estándar.
- Contenido = `roles/<rol>.md` + `extra_instructions` + un bloque fijo de Office (nombre del puesto, a quién reporta, convenciones del proyecto).
- Enviarlo solo si el agente anuncia `sessionCapabilities.systemPrompt` (los dos lo hacen); si no, el puesto muestra un aviso y funciona sin rol.
- **Modelo por puesto**: tras crear la sesión, `session/set_config_option` con id `model` (los dos adaptadores lo admiten). Sin `_meta` específico de proveedor.

Alternativas descartadas como mecanismo principal:

- `_meta.jetbrains.air.customInstructions`: funciona en los dos, pero es una extensión privada de JetBrains; queda como plan B si `systemPrompt` diese problemas.
- `CLAUDE.md` / `AGENTS.md` por puesto: afectan a todas las sesiones de la cuenta y del proyecto, no a un puesto.

## 4. Fase 3: orquestador

### 4.1 Viabilidad

- **ACP permite `mcpServers` por sesión**: es un campo de `NewSessionRequest` (también en load/resume), y los dos adaptadores lo usan (`acp-agent.js:8303-8328`, `index.js:39490-39531`).
- Transportes: Claude anuncia `http` y `sse` (stdio implícito); Codex acepta `stdio` y `http`, **no** `sse`. Común: **stdio** y **http**.
- Hoy UACode solo reenvía los servidores MCP del proyecto (`mcp_servers_for_project`, `v1.rs:9985`). Hace falta añadir `extra_mcp_servers` por sesión (la misma `SessionOptions` de la Fase 2), y solo el puesto orquestador los recibe.
- Codex ignora un servidor si ya existe uno con el mismo nombre en su `config.toml`: usar un nombre propio (`uacode-office`).

### 4.2 Servidor MCP de Office

Herramientas:

| Herramienta | Entrada | Efecto |
| --- | --- | --- |
| `listar_agentes` | — | Puestos con rol, proveedor, estado y tarea actual |
| `asignar_tarea` | `puesto`, `tarea`, `contexto?` | Crea o reutiliza el hilo del puesto y envía la tarea (`initial_content` con `auto_submit`) |
| `ver_estado` | `puesto` | Estado, tarea actual, último mensaje y plan |
| `pedir_aprobacion` | `resumen`, `detalle` | Muestra la petición en el panel Office y espera la respuesta del usuario (aprobar / rechazar + comentario) |

Transporte recomendado: **stdio con puente**, reutilizando `context_server::listener::McpServer` (servidor MCP en proceso sobre socket local; en Windows usa AF_UNIX del crate `net`). Al orquestador se le pasa `McpServer::Stdio { command: <uacode.exe>, args: ["--office-mcp", <socket>] }`, y ese modo de `uacode.exe` solo copia stdio ↔ socket. Es el diseño que Zed tuvo para Claude Code (`ClaudeZedMcpServer` con `--nc`, retirado en `930189ed83`).

Alternativa: endpoint HTTP local (`127.0.0.1`, puerto aleatorio, token por sesión en cabecera). Evita el puente, pero requiere servidor HTTP en UACode (dependencia nueva) y cuidar la exposición local. Queda como plan B.

Seguridad: el socket solo vive mientras UACode está abierto, va en el directorio temporal del usuario y solo se pasa a la sesión del orquestador. `asignar_tarea` no puede cambiar la cuenta ni las instrucciones de un puesto. Las aprobaciones siempre las da el usuario.

## 5. Concurrencia: worktree por programador

### 5.1 Qué existe

- `Repository::create_worktree(CreateWorktreeTarget::NewBranch { branch_name, base_sha }, path)` y `remove_worktree` (`git_store.rs:9245`, `:9547`); carpeta según `git.worktree_directory` (por defecto `../worktrees`).
- La herramienta de agente `create_thread` ya crea hilos en worktree nuevo, abriendo un *workspace* aparte (`agent_panel.rs:4905-4992`).
- El cwd de la sesión sale de `CreateThreadOptions.work_dirs` (primer path = `cwd`; `conversation_view.rs:1339`, `v2.rs:625-635`). Hoy todos pasan `None` (raíz del proyecto), pero acepta cualquier ruta absoluta.
- Revisión: `BranchDiff` + `ReviewBranchDiff` envían el diff de una rama a un agente.
- Archivado: `thread_worktree_archive.rs` guarda el trabajo en `refs/archived-worktrees/<id>` y quita el worktree.
- **No existe `git merge`** en el backend ni en la UI.

### 5.2 Propuesta

1. Al asignar tarea a un puesto programador (`worktree: true`): crear rama `office/<puesto>/<tarea-corta>` desde `HEAD` de la rama de integración y su worktree en `<worktree_directory>/<proyecto>/office-<puesto>`.
2. Crear el hilo con `work_dirs = [ruta del worktree]`, en el mismo *workspace* (no abrir ventana nueva). Hay que validar que las ediciones y terminales del agente funcionan fuera de los worktrees del proyecto (riesgo R5); si no, abrir un *workspace* por worktree como hace `create_thread`.
3. El programador hace commits en su rama (instrucción de rol).
4. Al terminar: la tarjeta pasa a "Pendiente de revisión"; Office envía el diff de la rama al puesto **tech lead/revisor** (`ReviewBranchDiff` con `base_ref` = rama de integración).
5. Revisor aprueba → **integración**: `git merge --no-ff office/<puesto>/<tarea>` en la rama de integración.
6. Conflictos: vista de conflictos existente y `ResolveConflictsWithAgent`.
7. Tras el merge: quitar el worktree (o archivarlo con el mecanismo existente) y borrar la rama.

### 5.3 Quién hace el merge

- **Opción A (recomendada para empezar)**: el revisor o un puesto integrador ejecuta el merge con su terminal en el worktree principal, tras `pedir_aprobacion`. Cero código nuevo en `crates/git`.
- **Opción B**: `merge` nativo en `crates/git` + `git_store` + botón en Office. Más control y mejor UX, pero toca código de Zed (y RPC para proyectos remotos).

## 6. Plan de implementación

Tamaños: S ≤ 2 días, M 3-5 días, L 1-2 semanas (una persona, incluye tests).

| Fase | Contenido | Tamaño | Toca Zed |
| --- | --- | --- | --- |
| **0. Spike** | Validar: varias sesiones en un proceso del adaptador; `systemPrompt` append real en los dos adaptadores; `set_config_option` de modelo; hilo con `work_dirs` fuera del proyecto | S | No (rama de prueba) |
| **1. Panel y puestos** | Crate `office`, `OfficePanel`, tarjetas con estado, `office.json`/`accounts.json`, gestor de cuentas, sincronización de `agent_servers`, clic → hilo, reanudar tras reiniciar | L | Cargo, `main.rs`, `zed.rs` |
| **2. Roles** | `SessionOptions` (system prompt) de la creación del hilo a `session/new`/load/resume; plantillas de los 12 roles; modelo por puesto | M | `acp_thread`, `agent_servers` v1/v2, `agent_ui` |
| **3. Orquestador** | `extra_mcp_servers` por sesión; servidor MCP con las 4 herramientas; puente stdio; UI de aprobaciones en el panel | L | `main.rs` (puente) + lo de la Fase 2 |
| **4. Worktrees e integración** | Worktree y rama por programador; flujo revisor; merge (opción A); limpieza | M (A) / L (B) | No (A) / `crates/git` (B) |

Orden: 0 → 1 → 2 → 3 → 4. Las fases 1 y 2 aportan valor por sí solas; 3 y 4 dependen de 2.

### Riesgos

| # | Riesgo | Mitigación |
| --- | --- | --- |
| R1 | APIs internas de `agent_ui` cambian al sincronizar con Zed (muchas son `pub(crate)` o recientes) | Usar solo API pública (`create_thread_with_options`, `load_agent_thread`, `retained_threads`); cambios en Zed en commits aislados |
| R2 | `systemPrompt` es *unstable* en ACP: puede cambiar de forma | Encapsular en `SessionOptions`; plan B `_meta.jetbrains.air.customInstructions` |
| R3 | El estado "sin sesión" vive en `ConversationView` y puede no ser público | Exponer un getter mínimo, o deducirlo de errores de conexión/auth |
| R4 | Varias sesiones en el mismo proceso del adaptador (puestos con la misma cuenta) | Validar en el spike; si falla, un agente por puesto (más procesos) |
| R5 | Hilo con `cwd` en un worktree fuera del proyecto: ediciones/terminales del agente | Validar en el spike; alternativa: *workspace* por worktree |
| R6 | Límites de uso de las cuentas con muchos puestos en paralelo | Mostrar el estado de la cuenta; límite de puestos trabajando a la vez por cuenta |
| R7 | Seguridad del MCP de Office (cualquier proceso local podría hablar con el socket) | Socket en directorio temporal del usuario, solo mientras UACode está abierto, nombre aleatorio; aprobaciones siempre humanas |
| R8 | Coste y ruido: doce agentes generando a la vez | Puestos creados bajo demanda; el orquestador asigna y no todos trabajan a la vez |
| R9 | Virtualización de `%APPDATA%` (cuentas.md) | No aplica: Office escribe desde UACode, que no está empaquetado |

### Fuera de alcance de este diseño

- Gestión de costes y cuotas por cuenta.
- Puestos en proyectos remotos (SSH/colaboración): Office será solo local al principio.
- Ver varios hilos lado a lado (el panel de agentes muestra uno; hoy se resuelve con otra ventana).
