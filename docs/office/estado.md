# Office: estado de desarrollo

Actualizado: 2026-11-08.

## Estado actual

- Rama de desarrollo: `office`, basada en `rebrand`.
- Rebranding y prueba de cuentas completados: login sin API keys, aislamiento, concurrencia y persistencia comprobados en Windows.
- Fase 0 completada. `office-spike` contiene código desechable y no se fusiona.
- Fase 1, paso 1: implementado el crate office con panel vacío, acción `office: toggle focus`, botón de dock y respeto de disable_ai. Compilación y arranque comprobados; apertura por paleta, cierre/reapertura por botón y traslado derecha → izquierda → derecha verificados visualmente.
- Fase 1, paso 2: puestos persistentes en `.uacode/office.json` (commit `d0539a0d60`). Crear, editar y archivar puestos; JSON inválido o versión futura en solo lectura con «Abrir archivo» y «Recargar»; posición del dock persistida en la base de datos local.
- Fase 1, paso 3: catálogo global de cuentas en `accounts.json` (commit `15beb0e1aa`). Alta con nombre único por proveedor, entradas virtuales «Por defecto», importación explícita de carpetas existentes y selector de cuenta en el puesto. Todas las cuentas «Sin probar».
- Sin procesos ACP ni login desde Office todavía; añadir una cuenta no crea su carpeta.

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
- Proyecto en `.uacode/office.json`; catálogo global en `%APPDATA%\UACode\office\accounts.json`; credenciales aisladas en `%APPDATA%\UACode\accounts\…`. «Por defecto» es virtual por proveedor y apunta a `accounts\claude-default` / `accounts\codex-default`.
- Cuentas: id UUID, carpeta derivada del id (`<proveedor>-<8 caracteres>`) salvo las importadas, que guardan su carpeta. El puesto referencia la cuenta por id; si falta, «Sin cuenta» sin borrar la referencia. accounts.json nunca guarda tokens ni correos; Office no lee el contenido de las carpetas de credenciales.

- office.json y accounts.json incluyen `version` desde el inicio; tests unitarios de serialización, JSON inválido y referencias inexistentes en pasos 2, 3 y 11.
- Paso 4: evaluar registro dinámico en AgentServerStore; explicar inviabilidad antes de escribir settings.json. Paso 5: gancho mínimo aislado con test de hilos ajenos a Office. Pasos 7 y 8 pueden unirse si son pequeños.

- office.json vive en el primer worktree visible que sea carpeta local; proyectos remotos muestran un aviso y no crean nada. Campos desconocidos se rechazan (solo lectura) para no perderlos al guardar.

## Siguiente paso

Paso 3 (mensaje #4) completado. Pendiente revisión y autorización del paso 4. No avanzar sin visto bueno.

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
