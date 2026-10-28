# Codex en quill (`quill-agent-codex`)

Plan para sumar el CLI de OpenAI Codex como segundo agente, con la misma interfaz de quill. Todo lo de Codex vive en `crates/quill-agent-codex`: el código genérico no lo nombra (AGENTS.md).

## Fuente de verdad

- **Código abierto:** [`openai/codex`](https://github.com/openai/codex), Apache-2.0, en Rust. No hace falta REA. El protocolo se lee del propio CLI con `codex app-server generate-json-schema --out <dir>`, que escribe el contrato completo (105 pedidos del cliente, 85 notificaciones, 10 pedidos del servidor en `codex-cli 0.163.0`). Las capturas de la fase 1 confirman cómo se usa.
- **Versión probada:** `codex-cli 0.163.0`. La última release es la 0.165.0. `app-server` figura como `[experimental]` en el `--help`, así que el adaptador declara su rango en `tested()` y quill avisa fuera de él, como con Claude.
- **Observado el 2026-10-28** con `initialize` + `model/list` + `account/read`:
  - la respuesta de `initialize` trae `userAgent`, `codexHome`, `platformFamily` y `platformOs`;
  - `model/list` devuelve 8 modelos, cada uno con sus esfuerzos (`low` … `max`, y `ultra` en algunos) y el esfuerzo por defecto;
  - la cuenta es `chatgpt`, plan `plus`.
- quill se presenta como `clientInfo { name: "quill" }`, nunca como otro cliente.

## Transporte

Se usa `codex app-server` (JSON-RPC por stdio, una línea por mensaje), el que usa la extensión de VS Code. Encaja en el contrato del adaptador sin cambios: `encode` escribe pedidos con `id` y respuestas a pedidos del servidor, y `decode` lee respuestas, notificaciones y pedidos del servidor. El adaptador guarda qué `id` espera cada respuesta.

Se descartan:
- `codex exec --json`: corre un solo turno y no tiene permisos interactivos.
- `codex mcp-server`: no expone el transcript.

Un proceso por sesión de quill, con un thread cada uno, igual que con Claude.

## Mapa del contrato

| quill (`AgentCommand` / `AgentEvent`) | Codex app-server |
|---|---|
| `Start` → `Ready` + `Session{id}` | `initialize` → `initialized`. Después `thread/start {cwd, model, approvalPolicy, sandbox}`, o `thread/resume {threadId}`, o `thread/fork`. El `thread.id` es el id de sesión. Las capacidades salen de `model/list` |
| `Prompt` | `turn/start {threadId, input:[{type:"text",text}], model, effort, approvalPolicy, sandboxPolicy}`. Con un turno en curso, `turn/steer` (`capabilities.steer`) |
| `Interrupt` | `turn/interrupt {threadId, turnId}` → `turn/completed` interrumpido |
| `TextDelta` / `Text` | `item/agentMessage/delta`, y `item/completed` con `agentMessage` |
| `ThinkingDelta` / `ThinkingText` | `item/reasoning/summaryTextDelta`, `item/reasoning/textDelta` |
| `ToolUse` / `ToolResult` | `item/started` / `item/completed` según el tipo de ítem (ver abajo). La salida en vivo llega con `item/commandExecution/outputDelta` |
| `PermissionRequest` → `Decide` | Pedidos del servidor `item/commandExecution/requestApproval` y `item/fileChange/requestApproval`. Se contestan con `accept`, `acceptForSession`, `decline` o `cancel` |
| Pregunta al usuario (tarjeta de AskUserQuestion) | `item/tool/requestUserInput`: preguntas con `header`, `options{label, description}` e `isOther`, la misma forma que la tarjeta de quill |
| `TurnFinished` | `turn/completed` |
| `Tokens` / `ContextUsage` | `thread/tokenUsage/updated`: lo usado sobre la ventana del modelo, sin desglose por categoría |
| `Limits` | `account/rateLimits/updated` / `account/rateLimits/read`: ventana primaria y semanal, igual que la tarjeta Usage |
| `TaskStarted` / `TaskEnded` (subagentes) | `collabAgentToolCall`, `subAgentActivity` |
| `McpServers` / `McpStatus` | `mcpServerStatus/list`, `mcpServer/startupStatus/updated` |
| `ModeChanged` | `thread/settings/updated` |
| `AuthRequired` | `account/read` con `requiresOpenaiAuth` sin cuenta, o el `error` del turno (a confirmar en la fase 1) |
| Desconocido | Se guarda como `Unknown` y nunca rompe (AGENTS.md) |

### Ítems → las herramientas que quill ya dibuja

El adaptador traduce cada ítem al nombre de herramienta que la interfaz ya conoce. Así se reusan tal cual las filas, los cuerpos desplegados, los diffs, la búsqueda web y la lista de tareas, sin pantallas nuevas.

| Ítem de Codex | Se muestra como | Notas |
|---|---|---|
| `commandExecution` | `Bash` (`command`, `description`) | `commandActions` clasifica el comando. `read` pasa a `Read`, `search` a `Grep`, `listFiles` a `LS`, y el resto queda en `Bash` |
| `fileChange` | `Edit` / `Write` por archivo (`add` → creado, `update` → editado, `delete`) | Codex da un diff unificado. El antes y el después de `ToolResult.files` se reconstruyen con el repo sombra del motor (a confirmar) |
| `mcpToolCall` | `mcp__{server}__{tool}` | Etiqueta y logo del conector como hoy |
| `webSearch` | `WebSearch` con `WebSearchRun` | El bloque de búsqueda web actual |
| `plan` / `turn/plan/updated` | La lista de tareas (forma de `TodoWrite`) | El panel de tareas sobre el composer |
| `collabAgentToolCall` | `Agent` | Abre el panel del subagente |
| `imageView` | `Read` de una imagen | |
| `contextCompaction` | La marca de compactación | |
| `enteredReviewMode` / `exitedReviewMode`, `imageGeneration`, `dynamicToolCall`, `sleep` | Paso genérico "Used …" | Diseño propio en una fase posterior |

## Modos de permiso

Codex no tiene un modo único sino dos ejes: `approvalPolicy` (`untrusted`, `on-request`, `never`) y `sandbox` (`read-only`, `workspace-write`, `danger-full-access`). El adaptador los ofrece como tres modos, los mismos presets que muestra Codex:

| Modo en quill | `approvalPolicy` | `sandbox` |
|---|---|---|
| Read only | `on-request` | `read-only` |
| Auto (por defecto) | `on-request` | `workspace-write` |
| Full access (inseguro, en `unsafe_modes`) | `never` | `danger-full-access` |

El selector del composer ya se arma desde `AgentCapabilities`. La exención del servidor de desarrollo hoy mira los nombres de Claude (`bypassPermissions`, `auto`); con Codex pasa a ser una pregunta al adaptador ("¿este modo exime?"), para que el motor no nombre modos de ningún proveedor.

## Cuenta

Para la v1 se reusa el camino del motor (`auth.rs`) con los subcomandos:
- `codex login status` → `AuthState`;
- `codex login` (navegador, con la vuelta en localhost a cargo de Codex) → `login_url` lee la URL que imprime;
- `codex logout`.

Métodos de inicio de sesión: "ChatGPT account" (`codex login`) y "Device code" (`codex login --device-auth`), si su salida se puede leer. La clave de API queda fuera de la v1, porque necesita un campo seguro. La salida exacta de los tres comandos se verifica en la fase 1, solo con `status` contra la configuración real.

Mejora posterior: el login por el protocolo (`account/login/start` → `authUrl`, `account/login/completed`).

## Lo que Codex no tiene o se degrada

Se declara en `AgentCapabilities` y la interfaz lo muestra degradado, como dice `agents.md`:
- **Atribución por herramienta con hooks:** se hace por turno con el repo sombra. `fileChange` ya trae los archivos.
- **Desglose del contexto por categoría:** solo el total.
- **Rewind del CLI:** existe `thread/revert`. Se evalúa después; el rewind de archivos de quill es del motor.
- **Comandos `/`:** Codex tiene skills y prompts propios (`skills/list`). Se muestran en el menú `/` en una fase posterior.

## Elegir el agente

Hoy el motor registra solo `claude` (`quill-engine/src/lib.rs`, `adapter()`). Con dos agentes, la página de sesión nueva necesita elegir uno: un chip más junto a "Local" / carpeta, hecho con los componentes de limn. Es lo único visual nuevo; el resto reusa lo que hay.

## Fases

1. **Capturas.** `tools/capture` suma `--agent codex`: un driver JSON-RPC que graba `{t_ms, dir, msg}` como el de Claude. Cada escenario corre en un repo git nuevo, con `gpt-6-luna` y esfuerzo `low`, con la cuenta de la persona. Escenarios:
   - simple, comando + edición con aprobación, denegar, aceptar por sesión;
   - plan/tareas, steer, interrumpir, reanudar, fork;
   - razonamiento, búsqueda web, una herramienta MCP, un subagente;
   - pregunta al usuario, modelos y cuenta.

   Las capturas se anonimizan (`/work/`, email) y quedan en `crates/quill-agent-codex/fixtures/0.163.0/`. Codex guarda esos threads en `~/.codex/sessions` como cualquier sesión suya.
2. **Adaptador** `quill-agent-codex`: `launch`, `encode` y `decode`, capacidades, la traducción de ítems y los modos. Tests que reproducen cada captura, como los de Claude.
3. **Motor:** registrar `codex` y hacer que la exención de modo sea del adaptador. El agente falso ya reproduce cualquier captura (no depende del protocolo).
4. **Cuenta:** `login status` / `login` / `logout` con el agente falso para los tests.
5. **Interfaz:** el chip para elegir agente, el nombre "Codex" desde el adaptador y la verificación en pantalla de cada escenario, en tema claro y oscuro.
6. **Después:** el login por el protocolo, la historia de sesiones (`thread/list`), skills en el menú `/`, `thread/revert`, y los diseños de review, imágenes y herramientas dinámicas.

## Para confirmar en la fase 1

- Si `item/started` de un comando llega antes o después de su pedido de aprobación.
- Si `fileChange` trae el contenido completo o solo el diff.
- Cómo responde `turn/steer` y qué devuelve `thread/resume`: si repite los ítems o hay que leerlos con `thread/turns/list`.
- Qué emite un turno sin cuenta.
- La salida de `codex login status`, `codex login` (sin completarlo) y `codex logout --help`.
