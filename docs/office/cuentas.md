# Varias cuentas por agente en UACode (fase de prueba)

Estado: **configuración aplicada; login y verificación desde la interfaz pendientes** (ver "Resultados").

Objetivo: usar a la vez Claude Code (x2) y Codex vía ACP, cada uno con login de cuenta (no API key) y su propia carpeta de configuración, haciendo todo desde UACode.

## Cómo funciona (revisado en el código y en los adaptadores)

- `agent_servers` es un mapa `nombre → ajustes` ([settings_content/src/agent.rs](../../crates/settings_content/src/agent.rs)).
  - `"type": "custom"`: `command`, `args`, `env`. El nombre es libre, así que permite varias instancias.
  - `"type": "registry"`: solo `env`; el nombre debe ser el id del registro ACP (`claude-acp`, `codex-acp`). Solo cabe una instancia por agente, por eso no se usa aquí.
- Para agentes `custom`, el entorno final es: entorno del proyecto → `env` de settings → extras de UACode (proxy, `NO_BROWSER`). UACode no fuerza ninguna API key en agentes `custom`, hay que vaciarlas en `env` ([project/src/agent_server_store.rs](../../crates/project/src/agent_server_store.rs), `LocalCustomAgent`).
- Adaptadores:
  - Claude: `@agentclientprotocol/claude-agent-acp`. Lee `CLAUDE_CONFIG_DIR` (`dist/paths.js` del paquete).
  - Codex: `@agentclientprotocol/codex-acp`. Arranca el app-server de Codex, que hereda el entorno y por tanto `CODEX_HOME`. **Se comprueba en la verificación.**

## Flujo de autenticación ACP del panel de agentes

Código: [conversation_view.rs](../../crates/agent_ui/src/conversation_view.rs) (`authenticate`), [agent_servers/src/acp.rs](../../crates/agent_servers/src/acp.rs) y [acp/v1.rs](../../crates/agent_servers/src/acp/v1.rs).

Cuando el agente responde que necesita autenticación, el panel muestra un botón por cada método que el adaptador anuncia. Hay tres caminos según el tipo de método, y **todos son independientes de si el agente es Custom o de registro**:

| Tipo de método | Qué hace UACode | ¿Respeta el `env` del agente? |
| --- | --- | --- |
| `terminal` (de primera clase) | Abre una terminal con **el mismo comando del agente** (`get_command`: `command` + `env` de settings) más los `args` del método | **Sí** |
| `agent` con `_meta["terminal-auth"]` (formato antiguo) | Abre una terminal con el `command`, `args` y `env` que da el adaptador, **sin mezclar el `env` de settings** | **No** (se pierde `CLAUDE_CONFIG_DIR`) |
| `agent` sin terminal | Envía la petición ACP `authenticate` al proceso del agente, que ya arrancó con el `env` de settings | **Sí** |

Qué anuncia cada adaptador (versiones fijadas):

- **Claude 0.94.0**: métodos `terminal` con `args` `--cli auth login --claudeai` ("Claude Subscription") y `--cli auth login --console` ("Anthropic Console", facturación por API: **no usar**). Además adjunta `_meta["terminal-auth"]` como respaldo. Si UACode recibe el método como `terminal` (lo previsto), se usa el camino 1 y el login va a la carpeta del agente.
- **Codex 2.5.0**: métodos `agent` sin terminal: "API Key" (**no usar**), "ChatGPT" (navegador) y "ChatGPT (device code)". Camino 3: el login lo hace el proceso del agente, que tiene `CODEX_HOME`. Si `NO_BROWSER` está activo en UACode, "ChatGPT" no se ofrece y queda el device code.

Conclusión por lectura de código: **debería funcionar para Custom y guardar el login en la carpeta de cada agente**. Falta confirmarlo ejecutando UACode (ver "Verificación"). Riesgo: si Claude llegase como método `agent` (camino 2), el login iría a `~/.claude` y no a `claude-b`; se detecta en la prueba 1 de abajo.

Limitaciones del flujo actual:

- El botón de login solo aparece si el agente responde "autenticación requerida"; no hay una acción "cambiar de cuenta" ni un estado "conectada/caducada".
- Cada agente tiene su botón, pero no existe una pantalla que liste cuentas ni las asocie a agentes.

## Carpetas de cuentas

| Sistema | Ruta |
| --- | --- |
| Windows | `%APPDATA%\UACode\accounts\{claude-default, claude-b, codex-default}` |
| Linux | `~/.config/uacode/accounts/{claude-default, claude-b, codex-default}` |

`claude-default` es una carpeta aislada; **no** es `~/.claude`.

## settings.json (Windows)

Fichero: `%APPDATA%\UACode\settings.json` (copia previa en `settings.backup.json`). Sustituir `<USUARIO>` por el usuario real; `env` no expande `%APPDATA%`.

```jsonc
"agent_servers": {
  "Claude A": {
    "type": "custom",
    "command": "npx.cmd",
    "args": ["-y", "@agentclientprotocol/claude-agent-acp@0.94.0"],
    "env": {
      "CLAUDE_CONFIG_DIR": "C:\\Users\\<USUARIO>\\AppData\\Roaming\\UACode\\accounts\\claude-default",
      "ANTHROPIC_API_KEY": "",
      "CLAUDE_CODE_OAUTH_TOKEN": ""
    }
  },
  "Claude B": {
    "type": "custom",
    "command": "npx.cmd",
    "args": ["-y", "@agentclientprotocol/claude-agent-acp@0.94.0"],
    "env": {
      "CLAUDE_CONFIG_DIR": "C:\\Users\\<USUARIO>\\AppData\\Roaming\\UACode\\accounts\\claude-b",
      "ANTHROPIC_API_KEY": "",
      "CLAUDE_CODE_OAUTH_TOKEN": ""
    }
  },
  "Codex A": {
    "type": "custom",
    "command": "npx.cmd",
    "args": ["-y", "@agentclientprotocol/codex-acp@2.5.0"],
    "env": {
      "CODEX_HOME": "C:\\Users\\<USUARIO>\\AppData\\Roaming\\UACode\\accounts\\codex-default",
      "OPENAI_API_KEY": "",
      "OPEN_AI_API_KEY": "",
      "CODEX_API_KEY": ""
    }
  }
}
```

En Linux: `"command": "npx"` y rutas absolutas bajo `~/.config/uacode/accounts/`.

Versiones: Claude fijado en 0.94.0 (el del test e2e); en npm y en el registro ACP la última es 0.101.0. Codex fijado en 2.5.0 (versión del registro ACP); npm tiene 2.6.0. No se han subido sin decisión explícita.

## Login desde UACode (camino principal)

1. Abrir el panel de agentes y elegir **Claude A** en el selector de agente. Al iniciar un hilo, si no hay sesión, aparece el aviso de autenticación con botones.
2. Pulsar **Claude Subscription** (no "Anthropic Console"). Se abre una terminal dentro de UACode y el navegador para el OAuth; completar el login.
3. Repetir con **Claude B** (otra cuenta) y con **Codex A**, donde se pulsa **ChatGPT** (no "API Key").
4. Comprobar la cuenta activa. `/status` **no existe** en el agente ACP de Claude (es de la TUI de Claude Code); se lee la cuenta guardada en cada carpeta, en la terminal integrada de UACode (muestra correo y organización, no tokens):

   ```powershell
   foreach ($name in "claude-default", "claude-b") { $file = "$env:APPDATA\UACode\accounts\$name\.claude.json"; if (Test-Path $file) { $account = (Get-Content $file -Raw | ConvertFrom-Json).oauthAccount; "{0}: {1} ({2})" -f $name, $account.emailAddress, $account.organizationName } else { "${name}: sin login" } }
   ```

   En Codex, ver el punto 1 de la verificación.

### Respaldo: login en la terminal integrada de UACode

Solo si el botón no aparece o el login va a una carpeta equivocada. Abrir la terminal integrada de UACode (no una consola externa):

```powershell
# Claude A
$env:CLAUDE_CONFIG_DIR="$env:APPDATA\UACode\accounts\claude-default"; $env:ANTHROPIC_API_KEY=""; claude
# dentro: /login  y luego /status

# Claude B
$env:CLAUDE_CONFIG_DIR="$env:APPDATA\UACode\accounts\claude-b"; $env:ANTHROPIC_API_KEY=""; claude

# Codex A
$env:CODEX_HOME="$env:APPDATA\UACode\accounts\codex-default"; codex login
$env:CODEX_HOME="$env:APPDATA\UACode\accounts\codex-default"; codex login status
```

## Verificación

1. **El login cae en la carpeta del agente.** Tras cada login desde la interfaz:

   ```powershell
   Get-ChildItem "$env:APPDATA\UACode\accounts\claude-default" -Force | Select-Object Name   # credenciales de Claude A
   Get-ChildItem "$env:APPDATA\UACode\accounts\claude-b" -Force | Select-Object Name         # credenciales de Claude B
   Test-Path "$env:APPDATA\UACode\accounts\codex-default\auth.json"                          # debe ser True
   ```

   Si el login de Claude B aparece en `~\.claude` y no en `claude-b`, el método llegó por el camino 2 (ver tabla): anotarlo en "Problemas".
2. **Codex respeta CODEX_HOME.** `~\.codex\auth.json` ya existía antes de la prueba (modificado `2026-10-26T17:14:34Z`, SHA-256 `89D706F2…3977F9`); no debe cambiar:

   ```powershell
   Get-Item "$HOME\.codex\auth.json" | Select-Object LastWriteTimeUtc
   (Get-FileHash "$HOME\.codex\auth.json").Hash
   ```

3. Abrir **Claude A** y **Claude B** a la vez; el comando del paso 4 del login debe mostrar cuentas distintas.
4. Lanzar una tarea en cada uno a la vez (por ejemplo "di tu cuenta y espera 30 s") y comprobar que cada sesión mantiene su historial. Las sesiones deben escribirse en `claude-default\projects` y `claude-b\projects` respectivamente.
5. Cerrar y reabrir UACode, abrir de nuevo cada agente y confirmar que mantiene su cuenta.

## Resultados

Prueba del 2026-11-08 en Windows 11, adaptador Claude 0.94.0.

| Prueba | Resultado |
| --- | --- |
| Botón de login visible en Claude A / B / Codex A | OK en Claude A y B ("Claude Subscription"). Codex A pendiente |
| Login Claude A guardado en `claude-default` | OK: cuenta personal |
| Login Claude B guardado en `claude-b` | OK: `.credentials.json` y `.claude.json` en `claude-b`, cuenta de empresa; `~\.claude\.credentials.json` sin cambios |
| Login Codex A guardado en `codex-default` (`auth.json`), `~/.codex` intacto | |
| Claude A y B a la vez, cuentas distintas | OK |
| Tareas simultáneas sin pisarse | OK: dos hilos en el mismo panel; cada uno creó su archivo (`prueba-B.txt` 18:49:28, `prueba-A.txt` 18:49:31) y siguió con una espera de 30 s, sin cruzarse respuestas |
| Persistencia tras reiniciar | |

## Problemas conocidos / a vigilar

- **Virtualización MSIX al configurar desde la app de Claude (Windows)**: la app de escritorio de Claude está empaquetada, y los procesos que lanza no escriben en el `%APPDATA%` real. Sus escrituras se desvían a `%LOCALAPPDATA%\Packages\Claude_<id>\LocalCache\Roaming\...`, y esos procesos leen la copia desviada como si fuera la real. UACode no ve esos cambios. Las carpetas y el `settings.json` hay que crearlos desde un proceso no empaquetado, por ejemplo la terminal integrada de UACode. En la prueba se usó un script que inserta los tres agentes dentro del `agent_servers` existente (conservando `claude-acp` y `codex-acp` de registro) tras copiar `settings.json` a `settings.backup-<fecha>.json`.
- **`disable_ai` en ajustes de proyecto**: un `.uacode/settings.json` con `"disable_ai": true` en el proyecto abierto oculta el panel de agentes y todos los comandos `agent:`, aunque el ajuste global diga `false`. El gestor de cuentas de la fase Office evitará el problema, porque lo hará la propia UACode.
- `npx.cmd` es obligatorio en Windows; `npx` es un script `.ps1`.
- `env` no expande variables (`%APPDATA%`), hay que usar rutas absolutas.
- Si un agente aparece como no registrado, revisar el nombre y que `"type": "custom"` esté presente.
- Las entradas de registro `claude-acp` y `codex-acp` ("Claude Agent" y "Codex" en el menú) usan las cuentas por defecto (`~\.claude`, `~\.codex`); se quitaron de `agent_servers` para no abrir un agente con la cuenta equivocada.
- La app de escritorio de Codex renueva el token de `~\.codex\auth.json` (cambia `last_refresh`, no la cuenta), así que su fecha o hash no sirven para probar que Codex A no lo tocó mientras esa app esté abierta.
- Los métodos "Anthropic Console" (Claude) y "API Key" (Codex) usan facturación por API; no usarlos en esta prueba.
- Camino 2 del login (formato `_meta["terminal-auth"]`): ignora el `env` del agente. Solo afectaría si un adaptador anuncia únicamente ese formato.

## Requisitos para la fase Office (solo anotado, sin implementar)

Gestor de cuentas dentro de la app:

- **Añadir cuenta**: el usuario elige proveedor (Claude, Codex, …) y escribe un nombre. La app crea la carpeta `accounts/<proveedor>-<nombre>`, lanza el login contra esa carpeta y guarda la cuenta.
- **Estado por cuenta**: conectada, caducada o sin sesión, y cuál es la cuenta activa.
- **Selección por agente**: cada agente elige su cuenta en un desplegable, con "Por defecto" como opción inicial. La app inyecta `CLAUDE_CONFIG_DIR` / `CODEX_HOME` y vacía las variables de API key, en lugar de pedir al usuario que edite `settings.json`.
- Debe funcionar con agentes `custom` y de registro, y tratar igual los tres caminos de login de la tabla (incluido el que hoy pierde el `env`).
- Mostrar la cuenta activa en el hilo: el agente ACP de Claude no tiene `/status`.

Observado en la prueba, a valorar para Office: el panel de agentes muestra un solo hilo a la vez; para ver dos agentes lado a lado hay que abrir otra ventana.
