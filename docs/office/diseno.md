# Office: diseño

Estado: **aprobado; Fase 0 completada y Fase 1 pendiente**. Orden: 1 (panel y cuentas), 2 (roles), 3 (worktrees), 4 (orquestador). Las decisiones vigentes del spike están incorporadas aquí; [spike.md](spike.md) conserva los resultados históricos. Seguimiento en [estado.md](estado.md).

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
| `policy.rs` | Permisos por rol y límite de concurrencia (Fase 1) |
| `worktrees.rs` | Worktree por puesto, revisión y merge con el CLI de git (Fase 3) |
| `office_mcp.rs` | Servidor MCP del orquestador (Fase 4) |

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
| Worktrees | `crates/git/src/repository.rs` (`create_worktree`, `remove_worktree`), `crates/project/src/git_store.rs` | Worktree por programador (Fase 3). |
| Revisión de rama | `crates/git_ui/src/branch_diff.rs` (`review_diff`, `ReviewBranchDiff`) | Paso de revisión (Fase 3). |
| `context_server::listener::McpServer` | `crates/context_server/src/listener.rs` | Servidor MCP en proceso, existe pero nadie lo usa (Fase 4). |

### 1.3 Puntos de enganche en código de Zed

Mínimos e imprescindibles:

| Fase | Archivo | Cambio |
| --- | --- | --- |
| 1 | `Cargo.toml`, `crates/zed/Cargo.toml` | Añadir el crate `office`. |
| 1 | `crates/zed/src/main.rs` (~722) | `office::init(cx)` tras `agent_ui::init`. |
| 1 | `crates/zed/src/zed.rs` (`initialize_panels`, ~778) | Cargar `OfficePanel` con `setup_or_teardown_ai_panel` para que desaparezca con `disable_ai`, igual que `AgentPanel`. `activation_priority` libre: 4 u 8+ (se usan 0-3 y 5-7; repetir una prioridad hace `panic` en debug). |
| 1 | `crates/acp_thread/src/acp_thread.rs` (peticiones de permiso, `fs/write_text_file`, `terminal/create`) | Gancho de política: antes de mostrar una petición de permiso o ejecutar una escritura/terminal, consultar una política registrable (aprobar, rechazar o pedir al usuario). Sin política registrada, comportamiento actual. Ver 2.7. |
| 2 | `crates/acp_thread/src/connection.rs` | Opciones de sesión (`SessionOptions { system_prompt, extra_mcp_servers }`) en `new_session` / `load_session` / `resume_session`, con valor por defecto vacío para no romper a los demás implementadores. |
| 2 | `crates/agent_servers/src/acp/v1.rs` (~1209) y `v2.rs` (~1296) | Poner `system_prompt` en `NewSessionRequest` (y load/resume) si el agente anuncia `sessionCapabilities.systemPrompt`; añadir `extra_mcp_servers` a los del proyecto (los usa la Fase 4). |
| 2 | `crates/agent_ui/src/agent_panel.rs` (`CreateThreadOptions`, 935) y `conversation_view.rs` (`initial_state`, 1317) | Llevar `SessionOptions` desde la creación del hilo hasta `new_session`, y guardarlas para reanudar. |
| 3 | `crates/agent_ui/src/agent_panel.rs`, `thread_metadata_store.rs`, `crates/acp_thread/src/acp_thread.rs` y despacho ACP de terminales en `crates/agent_servers/src/acp/v1.rs` | Tres cambios aislados: respetar los `work_dirs` explícitos, persistirlos y restaurarlos al reanudar, y usarlos para autorizar `fs/*` y como cwd por defecto de `terminal/create`. Comentar el motivo de cada adaptación para facilitar merges con upstream. |

El merge (Fase 3) no toca código de Zed: lo ejecuta `crates/office` con el CLI de git (ver 4.3).

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
- En Fase 1 se evaluará primero registrar `office-*` dinámicamente en `AgentServerStore`, sin escribir ajustes. Si no es viable, se explicará el motivo al usuario antes de usar la alternativa persistente siguiente.
- Alternativa persistente: Office escribe esas entradas `custom` en `agent_servers` de los ajustes globales con `update_settings_file` (mismo esquema que en cuentas.md: `npx.cmd`/`npx`, versión fijada, `CLAUDE_CONFIG_DIR`/`CODEX_HOME`, API keys vacías). Las entradas `office-*` las gestiona Office: se regeneran desde `accounts.json`.
- Ventaja: cero cambios en `AgentServerStore`, y el mecanismo ya está probado. Inconveniente: aparecen también en el menú del panel de agentes. Alternativa futura: API de registro en memoria en `AgentServerStore` (hoy `reregister_agents` vacía el mapa en cada cambio de ajustes, habría que mantener un mapa aparte).

### 2.5 Ficha de puesto y almacenamiento

| Archivo | Ámbito | Contenido | ¿Secretos? |
| --- | --- | --- | --- |
| `<config>/office/accounts.json` | Global (usuario) | Cuentas: id, proveedor, nombre, carpeta, último estado conocido | No (solo rutas) |
| `<proyecto>/.uacode/office.json` | Proyecto (se puede versionar) | Puestos del proyecto | No |
| `<proyecto>/.uacode/office/roles/<rol>.md` | Proyecto | Instrucciones de cada rol | No |
| `<config>/office/roles/<rol>.md` | Global | Plantillas por defecto de los roles | No |

Ambos archivos JSON (`office.json` y `accounts.json`) incluyen un campo `version` desde el inicio, con validación y tests de serialización, JSON inválido y referencias inexistentes.

En Fase 1, paso 2, la ubicación es el primer worktree visible que sea una carpeta local, siguiendo el orden del proyecto. Sin carpeta local no se crea nada. `.uacode/office.json` se crea solo al guardar el primer puesto. Las escrituras usan `Fs::atomic_write` (temporal en el mismo volumen y reemplazo atómico), con comprobación del contenido original para detectar cambios externos. JSON inválido: solo lectura, error con línea/columna y «Abrir archivo». Versión futura: solo lectura y aviso. Los campos desconocidos se rechazan para no perderlos al guardar. «Recargar» vuelve a leer tras una corrección externa.

La ficha inicial del paso 2 contiene `id` estable, `name`, `role`, `provider`, `account` opcional (sin asignar), `model`, `instructions` y `archived`. Modelo e instrucciones se guardan sin aplicarse hasta Fase 2; el resto de campos se añadirá en su fase. La posición del dock se guarda en la base de datos local de UACode, sin escribir settings.json.

Ficha completa prevista en `office.json`:

```jsonc
{
  "version": 1,
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
      "worktree": true,                  // programador: trabaja en su worktree (Fase 3)
      "thread_id": null                  // lo rellena Office
    }
  ]
}
```

La cuenta va por id, nunca por ruta, para que `office.json` sea portable entre máquinas: cada usuario resuelve el id en su `accounts.json`. Si la cuenta no existe en la máquina, el puesto aparece como "Sin cuenta".

Ajustes generales de la oficina, también en `office.json`:

```jsonc
{
  "version": 1,
  "max_working_agents": 3,   // límite de puestos en "Trabajando" a la vez
  "posts": [ /* … */ ]
}
```

### 2.6 Límite de concurrencia y consumo

- **Límite de agentes trabajando**: `max_working_agents` (configurable desde el panel). Al enviar una tarea con el límite alcanzado, la tarea queda **en cola** en la tarjeta ("En cola, posición N") y se envía cuando un puesto pasa a Libre. El usuario puede forzar el envío.
- El límite cuenta agentes activos, nunca coste monetario. La cola debe coordinar todos los envíos de los hilos de Office, incluidos los abiertos desde el panel de agentes.
- **Consumo por puesto**: `usage_update` aporta `used`/`size` en ambos proveedores. Mostrar uso de contexto en todos los puestos; si falta información, mostrar «—». Mostrar USD solo para Claude cuando llegue `cost`, con la etiqueta **estimado** (no facturación de la suscripción). Mantener el acumulado por puesto en `office.json`, evitando contar dos veces al reanudar.

### 2.7 Permisos por rol

Los programadores solo editan dentro de su worktree; QA puede ejecutar tests; ningún agente hace push. Las acciones destructivas siempre requieren aprobación del usuario desde Office.

La protección frente a push tiene **tres capas**:

1. Durante Fase 1 todos los puestos son solo de consulta: Codex usa sandbox `read-only` y Claude requiere aprobación para cualquier escritura o comando. La programación se habilita en Fase 3; desde entonces todos los puestos Codex usan sandbox `workspace-write`, con cwd en su worktree; nunca `agent`, `agent-full-access` ni excepciones por rol. El sandbox no impide por sí solo el acceso a red ni solicita permiso para todos los comandos internos.
2. En cada worktree de agente, configurar un `pushurl` inválido para todos los remotos. La configuración debe quedar aislada al worktree, sin modificar la del árbol principal; comprobarla antes de habilitar tareas y ante cambios de remotos. No cambiar las URL de fetch.
3. Hook `pre-push` como aviso adicional, conservando los hooks existentes. No constituye la barrera principal.

Además, la política de Office se aplica por sesión a las peticiones ACP, con gancho registrable y comportamiento original para hilos ajenos a Office:

| Petición | Regla |
| --- | --- |
| `fs/write_text_file` | Fuera del cwd/worktree: rechazar para programadores; para el resto, pedir aprobación. |
| `terminal/create` | Rechazar push; pedir aprobación desde Office para acciones destructivas; permitir tests a QA y programadores según su política. |
| `session/request_permission` | Aplicar las reglas de rutas y comandos por rol. Lo ambiguo requiere aprobación. Rechazar con `reject_once` cuando exista, sin usar `cancel` como rechazo ordinario. |

Claude usa modo `default`, nunca `bypassPermissions`; durante Fase 1 ninguna escritura o comando se autoaprueba. Las reglas `allow` de la cuenta pueden evitar peticiones ACP. Codex puede ejecutar acciones destructivas dentro del sandbox sin pedir permiso: **el gancho ACP por sí solo no garantiza la aprobación**. En Fase 1 se debe validar una vía efectiva de aprobación para estas operaciones; hasta entonces, bloquear la ejecución de tareas afectadas y mostrar el motivo en Office. No presentar una instrucción de rol ni un detector de comandos como garantía suficiente.

La autoaprobación de herramientas MCP queda limitada a las herramientas del servidor Office de la sesión del orquestador (5.3); no se extiende a otros servidores. Una herramienta de Office nunca sustituye la aprobación humana requerida por una acción destructiva o integración.

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

## 4. Fase 3: worktree por programador

El orquestador (Fase 4) no lanza programadores en paralelo hasta que esta fase esté hecha: sin worktrees, varios programadores en el mismo árbol se pisan.

### 4.1 Qué existe

- `Repository::create_worktree(CreateWorktreeTarget::NewBranch { branch_name, base_sha }, path)` y `remove_worktree` (`git_store.rs:9245`, `:9547`); carpeta según `git.worktree_directory` (por defecto `../worktrees`).
- La herramienta de agente `create_thread` ya crea hilos en worktree nuevo, abriendo un *workspace* aparte (`agent_panel.rs:4905-4992`).
- El cwd de la sesión sale de `CreateThreadOptions.work_dirs` (primer path = `cwd`; `conversation_view.rs:1339`, `v2.rs:625-635`). Hoy todos pasan `None` (raíz del proyecto), pero acepta cualquier ruta absoluta.
- Revisión: `BranchDiff` + `ReviewBranchDiff` envían el diff de una rama a un agente.
- Archivado: `thread_worktree_archive.rs` guarda el trabajo en `refs/archived-worktrees/<id>` y quita el worktree.
- **No existe `git merge`** en el backend ni en la UI.

### 4.2 Propuesta

1. Al asignar tarea a un puesto programador (`worktree: true`): crear rama `office/<puesto>/<tarea-corta>` desde `HEAD` de la rama de integración y su worktree en `<worktree_directory>/<proyecto>/office-<puesto>`.
2. Crear el hilo con `work_dirs = [ruta del worktree]`, en el mismo *workspace*. Aplicar los tres cambios de 1.3: no sobrescribir el cwd explícito, guardarlo y restaurarlo al reanudar, y respetarlo en `fs/*` y `terminal/create`. Los adaptadores ya validaron el cwd externo; falta verificar la integración en UACode. Antes de ejecutar tareas, activar las protecciones de 2.7.
3. El programador hace commits en su rama (instrucción de rol).
4. Al terminar: la tarjeta pasa a "Pendiente de revisión"; Office envía el diff de la rama al puesto **tech lead/revisor** (`ReviewBranchDiff` con `base_ref` = rama de integración).
5. Revisor aprueba → la tarjeta pasa a "Lista para integrar" y espera **tu aprobación** (botón "Integrar" con el resumen del diff y del informe del revisor).
6. Con tu aprobación, **UACode** ejecuta el merge (4.3).
7. Tras el merge: quitar el worktree (o archivarlo con el mecanismo existente) y borrar la rama.

### 4.3 Merge

Lo ejecuta UACode directamente con el CLI de git, desde `crates/office/src/worktrees.rs`, sin tocar `crates/git`:

1. Comprobar que el árbol principal está limpio (`git status --porcelain`) y en la rama de integración; si no, no se hace nada y se avisa.
2. `git merge --no-ff --no-edit office/<puesto>/<tarea>` en el worktree principal.
3. **Sin conflictos**: se registra el commit de merge, se quita el worktree (`git worktree remove`) y se borra la rama (`git branch -d`, nunca `-D`).
4. **Con conflictos**: el merge queda abierto y se muestra la vista de conflictos existente. Solo entonces interviene un agente: el botón "Resolver con agente" usa `ResolveConflictsWithAgent` (o envía la tarea al puesto revisor). Para abortar: `git merge --abort`.
5. Nunca `push`: integrar es local. Publicar la rama sigue siendo cosa tuya.

Los comandos se lanzan con el ejecutable de git que ya usa UACode, con `cwd` explícito y argumentos separados (sin pasar por shell), y su salida se muestra en la tarjeta si fallan.

## 5. Fase 4: orquestador

### 5.1 Viabilidad

- **ACP permite `mcpServers` por sesión**: es un campo de `NewSessionRequest` (también en load/resume), y los dos adaptadores lo usan (`acp-agent.js:8303-8328`, `index.js:39490-39531`).
- Transportes: Claude anuncia `http` y `sse` (stdio implícito); Codex acepta `stdio` y `http`, **no** `sse`. Común: **stdio** y **http**.
- Hoy UACode solo reenvía los servidores MCP del proyecto (`mcp_servers_for_project`, `v1.rs:9985`). Hace falta añadir `extra_mcp_servers` por sesión (la misma `SessionOptions` de la Fase 2), y solo el puesto orquestador los recibe.
- Codex ignora un servidor si ya existe uno con el mismo nombre en su `config.toml`: usar un nombre propio (`uacode-office`).

### 5.2 Servidor MCP de Office

Herramientas:

| Herramienta | Entrada | Efecto |
| --- | --- | --- |
| `listar_agentes` | — | Puestos con rol, proveedor, estado y tarea actual |
| `asignar_tarea` | `puesto`, `tarea`, `contexto?` | Crea o reutiliza el hilo del puesto y envía la tarea (`initial_content` con `auto_submit`). Respeta el límite de concurrencia y, para programadores, crea su worktree (Fase 3) |
| `ver_estado` | `puesto` | Estado, tarea actual, último mensaje y plan |
| `pedir_aprobacion` | `resumen`, `detalle` | Muestra la petición en el panel Office y espera la respuesta del usuario (aprobar / rechazar + comentario) |

### 5.3 Transporte

Decidido y validado en la Fase 0: **HTTP en `127.0.0.1`**, puerto aleatorio y token aleatorio por sesión en `Authorization: Bearer`. Lo arranca `crates/office`, sin añadir un puente stdio ni tocar `main.rs` por este transporte.

El endpoint solo vive mientras UACode está abierto. Valida el token y `Origin`; sus datos de conexión solo se entregan a la sesión del orquestador. Autoaprobar únicamente llamadas a las herramientas conocidas del servidor `uacode-office` de esa sesión; el nombre de una herramienta por sí solo no acredita su procedencia. Los demás MCP conservan su flujo normal de permisos.

`asignar_tarea` no puede cambiar cuenta, rol ni permisos. Las aprobaciones de acciones destructivas y merges siempre las da el usuario desde el panel, incluso si la llamada MCP que solicita la aprobación se ha autoaprobado.

## 6. Plan de implementación

Tamaños: S ≤ 2 días, M 3-5 días, L 1-2 semanas (una persona, incluye tests).

| Fase | Contenido | Tamaño | Toca Zed |
| --- | --- | --- | --- |
| **0. Spike (completada)** | Validado: varias sesiones en un proceso; `systemPrompt` append; hilo con `cwd` en worktree externo; MCP por sesión (HTTP y stdio); `set_config_option` de modelo; qué consumo envía cada adaptador | S | No (rama `office-spike`, desechable) |
| **1. Panel y puestos** | Crate `office`, `OfficePanel`, tarjetas con estado, `office.json`/`accounts.json`, gestor de cuentas, sincronización de `agent_servers`, clic → hilo, reanudar tras reiniciar, límite de concurrencia con cola, consumo por puesto, permisos por rol (gancho de política) | L+ | Cargo, `main.rs`, `zed.rs`, `acp_thread` (gancho) |
| **2. Roles** | `SessionOptions` (system prompt) de la creación del hilo a `session/new`/load/resume; plantillas de los 12 roles; modelo por puesto | M | `acp_thread`, `agent_servers` v1/v2, `agent_ui` |
| **3. Worktrees e integración** | Worktree y rama por programador; tres ajustes de cwd; bloqueo de push; flujo revisor; aprobación del usuario; merge con el CLI de git; conflictos con agente; limpieza | M | `agent_ui`, `acp_thread`, despacho de terminales ACP |
| **4. Orquestador** | `extra_mcp_servers` por sesión; servidor MCP con las 4 herramientas; UI de aprobaciones en el panel | L | No (HTTP) |

Orden: 0 → 1 → 2 → 3 → 4. Las fases 1 y 2 aportan valor por sí solas. La 3 depende de 1; la 4 depende de 2 (`extra_mcp_servers`) y de 3 (no hay programadores en paralelo sin worktrees).

### Riesgos

| # | Riesgo | Mitigación |
| --- | --- | --- |
| R1 | APIs internas de `agent_ui` cambian al sincronizar con Zed (muchas son `pub(crate)` o recientes) | Usar solo API pública (`create_thread_with_options`, `load_agent_thread`, `retained_threads`); cambios en Zed en commits aislados |
| R2 | `systemPrompt` es *unstable* en ACP: puede cambiar de forma | Append validado en ambos adaptadores; encapsular en `SessionOptions` para cambios futuros |
| R3 | El estado "sin sesión" vive en `ConversationView` y puede no ser público | Exponer un getter mínimo, o deducirlo de errores de conexión/auth |
| R4 | Varias sesiones en el mismo proceso del adaptador (puestos con la misma cuenta) | Resuelto en el spike: varias sesiones concurrentes validadas en ambos proveedores |
| R5 | Hilo con `cwd` en un worktree fuera del proyecto: ediciones/terminales del agente | Aplicar y verificar los tres ajustes de cwd de 1.3 en el mismo workspace |
| R6 | Límites de uso de las cuentas con muchos puestos en paralelo | Mostrar el estado de la cuenta; límite de puestos trabajando a la vez por cuenta |
| R7 | Seguridad del MCP de Office (cualquier proceso local podría hablar con el endpoint) | HTTP solo en `127.0.0.1`, token aleatorio por sesión y validación de Origin; solo mientras UACode está abierto; aprobaciones siempre humanas |
| R10 | Los permisos por rol no cubren lo que el agente ejecuta sin pedir permiso | Tres capas contra push y política por sesión (2.7); validar aprobación efectiva de destructivos y bloquear tareas si no puede garantizarse |
| R11 | Merge sobre un árbol principal con cambios sin commitear | Comprobar `git status` antes; no integrar si no está limpio |
| R8 | Coste y ruido: doce agentes generando a la vez | Puestos creados bajo demanda; el orquestador asigna y no todos trabajan a la vez |
| R9 | Virtualización de `%APPDATA%` (cuentas.md) | No aplica: Office escribe desde UACode, que no está empaquetado |

### Fuera de alcance de este diseño

- Gestión de costes y cuotas por cuenta.
- Puestos en proyectos remotos (SSH/colaboración): Office será solo local al principio.
- Ver varios hilos lado a lado (el panel de agentes muestra uno; hoy se resuelve con otra ventana).
