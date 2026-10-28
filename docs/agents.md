# Adaptadores de agente (`quill-agent`)

quill maneja cualquier agente de código por CLI mediante un adaptador (spec.md D11). Este documento fija el contrato común y cómo se proyecta cada CLI sobre él. Solo el adaptador de Claude Code está verificado con capturas (engine.md §11.4). Las filas de otros CLIs son **hipótesis de diseño a verificar** con su propia fase de captura antes de implementarlos. La columna de Codex ya está verificada con capturas de `codex-cli 0.163.0` (`app-server` v2); el detalle está en [codex.md](codex.md).

## Contrato

El adaptador es **puro**: no abre procesos ni sockets. El motor (`quill-engine`) lanza el proceso según el `LaunchPlan`, y por cada línea que llega llama a `decode`:

```text
LaunchPlan  = programa, args, env (a poner y a quitar), cwd
decode(line) → Decoded { events: Vec<AgentEvent>, replies: Vec<String> }   // replies: respuestas automáticas (p. ej. hooks)
encode(cmd)  → Vec<String>                                                  // líneas a escribir
```

| Concepto | `AgentEvent` / `AgentCommand` | Claude Code (verificado) | Codex 0.163.0 (verificado) | OpenCode (hipótesis) |
|---|---|---|---|---|
| Arranque | `Command::Start` → `Event::Ready{session, capabilities}` | `control_request initialize` → respuesta con `models`, `commands`, modo actual; `system/init` con `session_id` y `capabilities` | `codex app-server` (JSON-RPC por stdio): `initialize` → `initialized` → `thread/start` / `thread/resume` / `thread/fork`; los modelos salen de `model/list` | `opencode serve` (HTTP + SSE): crear sesión por API |
| Prompt | `Command::Prompt{request, text}` | `user` con `uuid` = `RequestId` | `turn/start` (con un turno en curso, `turn/steer`) | `POST /session/:id/message` |
| Texto en streaming | `Event::TextDelta` / `Event::Text` | `stream_event` `text_delta`; `assistant` completo | `item/agentMessage/delta` y `item/completed` | evento SSE de parte de mensaje |
| Herramienta | `Event::ToolUse{call, name, input}` / `Event::ToolResult{call, output, is_error, files, command}` | `assistant.tool_use`; `user.tool_result` + `tool_use_result` estructurado | `item/started` / `item/completed` de `commandExecution`, `fileChange`, `mcpToolCall`, `webSearch`; el plan, con `turn/plan/updated` | partes `tool` del mensaje |
| Permiso | `Event::PermissionRequest{id, call, tool, input, suggestions}` → `Command::Decide{id, Decision}` | `control_request can_use_tool` → `control_response` allow/deny | pedidos del servidor `item/commandExecution/requestApproval` y `item/fileChange/requestApproval`; se contestan `accept` / `acceptForSession` / `decline` | evento `permission.updated` → responder por API |
| Pregunta al usuario | `PermissionRequest` con `interactive: true`, respuesta en `Decision::Answer` | AskUserQuestion vía `can_use_tool` + `updatedInput.answers` | `item/tool/requestUserInput`, contestado con `{answers:{id:{answers:[…]}}}` | — |
| Interrumpir | `Command::Interrupt` → `Event::TurnFinished{interrupted}` | `control_request interrupt` | `turn/interrupt` → `turn/completed` interrumpido | `POST /session/:id/abort` |
| Fin de turno | `Event::TurnFinished{ok, usage, cost}` | `result` sin `parent_tool_use_id` | `turn/completed` | sesión idle |
| Atribución por herramienta | `capabilities.tool_hooks` | `hook_callback` PostToolUse con `prompt_id` | no hay hooks: atribución por turno | partes `patch` |
| Cambios de archivo estructurados | `ToolResult.files` | Write/Edit: `structuredPatch`, `originalFile` | `fileChange`: solo el diff por archivo (el hunk de un cambio, el contenido de un archivo nuevo) | parte `patch` |
| Reanudar / fork | `capabilities.resume/fork` | `--resume`, `--fork-session` | `thread/resume`, `thread/fork` | sesiones persistidas del servidor |
| Rewind propio del CLI | `capabilities.rewind` | `rewind_files` | — | `revert` |
| Desglose del contexto | `Command::ContextUsage` → `Event::ContextUsage{used, max, categories, groups}` (sin respuesta si el agente no lo sabe) | `control_request get_context_usage` (`detail: "full"`) | solo el total: `thread/tokenUsage/updated`, sin categorías | — |

Lo que un CLI no tenga se declara en `AgentCapabilities` y la UI lo muestra degradado. Por ejemplo, sin hooks la atribución se hace por turno con el repo sombra, y sin permisos interactivos no hay tarjeta de permiso.

## Transportes

El contrato cubre CLIs por **stdio con líneas JSON**: Claude Code (stream-json) y Codex (`app-server`, JSON-RPC). Un CLI que expone un **servidor HTTP** (OpenCode) necesita otro transporte en el motor, que entrega al adaptador cada evento SSE como una "línea". Ese segundo transporte se agrega cuando se implemente el primer adaptador HTTP.

## Cuenta (`sign_in_methods`, `auth_query`, `login_run`, `logout_run`)

El adaptador declara cómo se conecta la persona; el motor corre esos comandos fuera del hilo de la UI (`quill-engine/src/auth.rs`) y la UI sólo ve `AuthState` y los eventos `Auth`, `SignInUrl` y `SignInFailed`. Opcionales: un agente sin ellos no muestra pantalla de inicio de sesión.

| Concepto | Contrato | Claude Code 2.1.312 (observado, `cli-auth.md`) |
|---|---|---|
| Métodos | `sign_in_methods()` → (clave, etiqueta) | `claudeai` "Claude subscription", `console` "Anthropic Console" |
| Estado | `auth_query()` + `auth_state(stdout, exit)` | `auth status --json`; sale con 1 sin sesión pero imprime el mismo JSON (`loggedIn`, `authMethod`, `email`, `orgName`, `subscriptionType`) |
| Login | `login_run(método)`, `login_url(línea)`, `login_code(código)` | `auth login --claudeai\|--console`: imprime la URL dentro de un hipervínculo OSC 8 (se quita) y espera en stdin `CODE#STATE`; espera sin límite, quill corta a los 600 s |
| Logout | `logout_run()` | `auth logout` (cierra también la sesión del CLI en la terminal) |
| Turno sin sesión | `AgentEvent::AuthRequired` | `assistant` con `error: "authentication_failed"` antes del `result` |

El motor abre la URL con `xdg-open` (como la de un servidor MCP) y lanza el login con `BROWSER=true` para que el CLI no abra otra pestaña. El estado se pregunta con el mismo entorno que las sesiones (las variables del agente en Settings incluidas): con `ANTHROPIC_API_KEY` el CLI dice `api_key`. Cancelar mata el proceso hijo por su PID.

Codex 0.163.0 (verificado, ver [codex.md](codex.md)): un solo método, `chatgpt` "ChatGPT account"; `auth_query` es `login status` (escribe en stderr, el motor entrega el texto de la salida que tenga algo), `login` imprime la URL en stderr y vuelve por un servidor local, sin código; `logout` es `logout`. `--device-auth` no se ofrece: pide escribir un código que quill no muestra.
