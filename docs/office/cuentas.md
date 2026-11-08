# Varias cuentas por agente en UACode (fase de prueba)

Estado: **configuración aplicada, login y verificación pendientes** (ver "Resultados").

Objetivo: usar a la vez Claude Code (x2) y Codex vía ACP, cada uno con login de cuenta (no API key) y su propia carpeta de configuración.

## Cómo funciona (revisado en el código)

- `agent_servers` es un mapa `nombre → ajustes` ([settings_content/src/agent.rs](../../crates/settings_content/src/agent.rs)).
  - `"type": "custom"`: `command`, `args`, `env`. El nombre es libre, así que permite varias instancias.
  - `"type": "registry"`: solo `env`; el nombre debe ser el id del registro ACP (`claude-acp`, `codex-acp`). Solo cabe una instancia por agente, por eso no se usa aquí.
- Para agentes `custom`, el entorno final es: entorno del proyecto → `env` de settings → extras de UACode (proxy, `NO_BROWSER`). UACode no fuerza ninguna API key en agentes `custom`, hay que vaciarlas en `env` ([project/src/agent_server_store.rs](../../crates/project/src/agent_server_store.rs), `LocalCustomAgent`).
- Adaptadores:
  - Claude: `@agentclientprotocol/claude-agent-acp`. Respeta `CLAUDE_CONFIG_DIR` (el test e2e de `acp.rs` ya lo usa).
  - Codex: `@agentclientprotocol/codex-acp` (el registro ACP lo declara como paquete npx). `CODEX_HOME` lo lee el propio Codex CLI; **se comprueba en la verificación**.

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

En Linux: `"command": "npx"` y rutas bajo `~/.config/uacode/accounts/` (en absoluto).

Versiones: Claude fijado en 0.94.0 (el del test e2e); en npm y en el registro ACP la última es 0.101.0. Codex fijado en 2.5.0 (versión del registro ACP); npm tiene 2.6.0. No se han subido sin decisión explícita.

## Login (PowerShell, a mano)

Cada bloque define la variable solo en esa terminal.

Claude A:

```powershell
$env:CLAUDE_CONFIG_DIR="$env:APPDATA\UACode\accounts\claude-default"; $env:ANTHROPIC_API_KEY=""; claude
# dentro: /login
```

Claude B:

```powershell
$env:CLAUDE_CONFIG_DIR="$env:APPDATA\UACode\accounts\claude-b"; $env:ANTHROPIC_API_KEY=""; claude
# dentro: /login
```

Codex A:

```powershell
$env:CODEX_HOME="$env:APPDATA\UACode\accounts\codex-default"; codex login
```

Verificar la cuenta activa:

```powershell
# Claude (A o B, con su CLAUDE_CONFIG_DIR definido)
claude auth status
# o dentro de claude: /status

# Codex
$env:CODEX_HOME="$env:APPDATA\UACode\accounts\codex-default"; codex login status
```

## Verificación

1. **Codex respeta CODEX_HOME**: tras `codex login`, debe existir `accounts\codex-default\auth.json` y `~\.codex\auth.json` no debe cambiar.
   Línea base anterior a la prueba (`~\.codex\auth.json` ya existía): modificado `2026-10-26T17:14:34Z`, SHA-256 `89D706F2…3977F9`.

   ```powershell
   Test-Path "$env:APPDATA\UACode\accounts\codex-default\auth.json"
   Get-Item "$HOME\.codex\auth.json" | Select-Object LastWriteTimeUtc
   (Get-FileHash "$HOME\.codex\auth.json").Hash
   ```

2. Abrir **Claude A** y **Claude B** a la vez en UACode (panel de agentes, selector de agente) y ejecutar `/status` en cada uno: deben mostrar cuentas distintas.
3. Lanzar una tarea en cada uno a la vez (por ejemplo "di tu cuenta y espera 30 s") y comprobar que cada sesión mantiene su historial y no aparecen sesiones cruzadas. Las sesiones deben escribirse en `claude-default\projects` y `claude-b\projects` respectivamente.
4. Cerrar y reabrir UACode, abrir de nuevo cada agente y confirmar que mantiene su cuenta.

## Resultados

Pendiente de rellenar tras las pruebas.

| Prueba | Resultado |
| --- | --- |
| Login Claude A | |
| Login Claude B | |
| Login Codex A | |
| `auth.json` en `codex-default`, `~/.codex` intacto | |
| Claude A y B a la vez, cuentas distintas | |
| Tareas simultáneas sin pisarse | |
| Persistencia tras reiniciar | |

## Problemas conocidos / a vigilar

- `npx.cmd` es obligatorio en Windows; `npx` es un script `.ps1`.
- `env` no expande variables (`%APPDATA%`), hay que usar rutas absolutas.
- Si un agente aparece como no registrado, revisar el nombre y que `"type": "custom"` esté presente.
