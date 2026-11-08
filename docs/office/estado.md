# Office: estado de desarrollo

Actualizado: 2026-11-08.

## Estado actual

- Rama de desarrollo: `office`, basada en `rebrand`.
- Rebranding y prueba de cuentas completados: login sin API keys, aislamiento, concurrencia y persistencia comprobados en Windows.
- Fase 0 completada. `office-spike` contiene código desechable y no se fusiona.
- Diseño actualizado con las decisiones aprobadas del spike. Fase 1 todavía sin implementar; no existe aún el panel Office.
- Este cambio solo modifica documentación; no cambia el ejecutable ni requiere compilar o arrancar una nueva versión.

## Decisiones vigentes

- Todo código nuevo en `crates/office`, con raíz `src/office.rs`; enganches mínimos en Zed y cambios de upstream aislados y comentados por su motivo.
- No renombrar identificadores internos (`zed::`, crates, `ZED_*`, `.zed/`).
- Orden: Fase 1 panel/cuentas, 2 roles, 3 worktrees, 4 orquestador.
- Un proceso por (proveedor, cuenta), varias sesiones por proceso. Adaptadores fijados: Claude 0.94.0, Codex 2.5.0.
- Roles con ACP `systemPrompt` append; modelo por puesto con `set_config_option` en Fase 2.
- Tres cambios de cwd en Fase 3: respetar work_dirs explícitos, persistir/restaurar al reanudar, y respetarlos en fs/terminales del cliente ACP.
- Codex siempre workspace-write con cwd en su worktree; pushurl inválido para todos los remotos de cada worktree; pre-push solo como aviso adicional. No alterar remotos del árbol principal.
- Destructivos siempre con aprobación humana desde Office. Pendiente validar una vía efectiva para operaciones internas de Codex/Claude que no solicitan permiso ACP; bloquear tareas afectadas mientras no se garantice.
- Contexto en todos los puestos; USD solo Claude, etiquetado estimado. Concurrencia por número de agentes activos, con cola.
- MCP HTTP en 127.0.0.1, puerto y token por sesión; validar Origin. Solo el orquestador recibe el endpoint; solo herramientas de Office se autoaprueban. Aprobaciones humanas de destructivos e integración se mantienen.
- Merge local ejecutado por UACode mediante CLI de git tras aprobación; agente solo para conflictos.
- Proyecto en `.uacode/office.json`; catálogo global en `%APPDATA%\UACode\office\accounts.json`; credenciales aisladas en `%APPDATA%\UACode\accounts\…`. Por defecto usa la cuenta habitual del proveedor.

## Siguiente paso

Presentar y aprobar el plan de Fase 1. Primer paso: panel Office vacío acoplable que se abre y se cierra, para validar el enganche con Zed. No comenzar implementación sin visto bueno.

## Reglas de trabajo

- Confirmar cada paso con el usuario antes de continuar; commits pequeños y claros en `office`.
- Nunca hacer git push ni tocar remotos durante este desarrollo.
- No escribir fuera del repositorio salvo en `%APPDATA%\UACode` cuando el paso lo requiera, avisando antes. No acceder a cuentas reales para pruebas sin el paso correspondiente.
- Compilar y comprobar que arranca después de cada paso; ejecutar verificaciones pertinentes. En esta preparación documental solo se ha revisado el diff; no se ha compilado ni comprobado el arranque.
- Actualizar este archivo en cada paso con lo hecho, pendiente y decisiones nuevas.
- Antes de modificar fuentes, anteponer al README.md las dos líneas obligatorias de revisión indicadas en AGENTS.md; nunca retirarlas automáticamente.
- Preferir codebase-memory-mcp para descubrir código. Usar `./script/clippy` si se necesita clippy.

## Historial

- 2026-11-08: incorporadas al diseño las decisiones del spike y creado este estado inicial. Pendiente aprobación del plan e implementación del panel vacío.
