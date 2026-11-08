# Office: estado de desarrollo

Actualizado: 2026-11-08.

## Estado actual

- Rama de desarrollo: `office`, basada en `rebrand`.
- Rebranding y prueba de cuentas completados: login sin API keys, aislamiento, concurrencia y persistencia comprobados en Windows.
- Fase 0 completada. `office-spike` contiene código desechable y no se fusiona.
- Fase 1, paso 1: implementado el crate office con panel vacío, acción `office: toggle focus`, botón de dock y respeto de disable_ai. Compilación y arranque comprobados; apertura por paleta, cierre/reapertura por botón y traslado derecha → izquierda → derecha verificados visualmente.
- Fase 1, paso 2: puestos persistentes en `.uacode/office.json` (commit `d0539a0d60`). Crear, editar y archivar puestos; JSON inválido o versión futura en solo lectura con «Abrir archivo» y «Recargar»; posición del dock persistida en la base de datos local.
- Sin lógica de cuentas ni procesos ACP todavía.

## Decisiones vigentes

- Todo código nuevo en `crates/office`, con raíz `src/office.rs`; enganches mínimos en Zed y cambios de upstream aislados y comentados por su motivo.
- No renombrar identificadores internos (`zed::`, crates, `ZED_*`, `.zed/`).
- Orden: Fase 1 panel/cuentas, 2 roles, 3 worktrees, 4 orquestador.
- Un proceso por (proveedor, cuenta), varias sesiones por proceso. Adaptadores fijados: Claude 0.94.0, Codex 2.5.0.
- Roles con ACP `systemPrompt` append; modelo por puesto con `set_config_option` en Fase 2.
- Tres cambios de cwd en Fase 3: respetar work_dirs explícitos, persistir/restaurar al reanudar, y respetarlos en fs/terminales del cliente ACP.
- Durante Fase 1, solo consulta: Codex read-only y Claude con aprobación obligatoria para cualquier escritura o comando. Desde Fase 3, Codex workspace-write con cwd en su worktree; pushurl inválido para todos los remotos de cada worktree; pre-push solo como aviso adicional. No alterar remotos del árbol principal.
- Destructivos siempre con aprobación humana desde Office. Pendiente validar una vía efectiva para operaciones internas de Codex/Claude que no solicitan permiso ACP; bloquear tareas afectadas mientras no se garantice.
- Contexto en todos los puestos; USD solo Claude, etiquetado estimado. Concurrencia por número de agentes activos, con cola.
- MCP HTTP en 127.0.0.1, puerto y token por sesión; validar Origin. Solo el orquestador recibe el endpoint; solo herramientas de Office se autoaprueban. Aprobaciones humanas de destructivos e integración se mantienen.
- Merge local ejecutado por UACode mediante CLI de git tras aprobación; agente solo para conflictos.
- Proyecto en `.uacode/office.json`; catálogo global en `%APPDATA%\UACode\office\accounts.json`; credenciales aisladas en `%APPDATA%\UACode\accounts\…`. Por defecto usa la cuenta habitual del proveedor.

- office.json y accounts.json incluyen `version` desde el inicio; tests unitarios de serialización, JSON inválido y referencias inexistentes en pasos 2, 3 y 11.
- Paso 4: evaluar registro dinámico en AgentServerStore; explicar inviabilidad antes de escribir settings.json. Paso 5: gancho mínimo aislado con test de hilos ajenos a Office. Pasos 7 y 8 pueden unirse si son pequeños.

- office.json vive en el primer worktree visible que sea carpeta local; proyectos remotos muestran un aviso y no crean nada. Campos desconocidos se rechazan (solo lectura) para no perderlos al guardar.

## Siguiente paso

Paso 2 y ajustes 1–4 del mensaje #2 completados. Pendiente revisión y autorización del paso 3. No avanzar sin visto bueno.

## Reglas de trabajo

- Confirmar cada paso con el usuario antes de continuar; commits pequeños y claros en `office`.
- Nunca hacer git push ni tocar remotos durante este desarrollo.
- No escribir fuera del repositorio salvo en `%APPDATA%\UACode` cuando el paso lo requiera, avisando antes. No acceder a cuentas reales para pruebas sin el paso correspondiente.
- Ciclo de desarrollo debug incremental (`cargo build -p zed`), `cargo check -p office` y `cargo test -p office`; comprobar arranque tras cada paso. Release solo al cerrar hitos o por petición expresa. Registrar tiempos reales de debug.
- Actualizar este archivo en cada paso con lo hecho, pendiente y decisiones nuevas.
- AGENTS.md es un enlace simbólico a .rules, adaptado al fork: no añadir avisos de revisión upstream al README; seguir el protocolo del buzón.
- Preferir codebase-memory-mcp para descubrir código. Usar `./script/clippy` si se necesita clippy.

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
