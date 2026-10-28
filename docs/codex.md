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
| Pregunta al usuario (tarjeta de AskUserQuestion) | `item/tool/requestUserInput`: preguntas con `id`, `header`, `question`, `options{label, description}`, `isOther` e `isSecret`, la misma forma que la tarjeta de quill. Se contesta `{answers:{<id>:{answers:[<label>]}}}`. Solo existe en modo Plan o con la feature `default_mode_request_user_input` |
| `TurnFinished` | `turn/completed` |
| `Tokens` / `ContextUsage` | `thread/tokenUsage/updated`: lo usado sobre la ventana del modelo, sin desglose por categoría |
| `Limits` | `account/rateLimits/updated` / `account/rateLimits/read`: ventana primaria y semanal, igual que la tarjeta Usage |
| `TaskStarted` / `TaskEnded` (subagentes) | `collabAgentToolCall`, `subAgentActivity` |
| `McpServers` / `McpStatus` | `mcpServerStatus/list`, `mcpServer/startupStatus/updated` |
| `ModeChanged` | `thread/settings/updated` (no observado en las capturas) |
| `AuthRequired` | `account/read` con `account: null` y `requiresOpenaiAuth: true`, o `turn/completed` fallido con `codexErrorInfo: unauthorized` (inferido de la fuente, ver Fase 1) |
| Desconocido | Se guarda como `Unknown` y nunca rompe (AGENTS.md) |

### Ítems → las herramientas que quill ya dibuja

El adaptador traduce cada ítem al nombre de herramienta que la interfaz ya conoce. Así se reusan tal cual las filas, los cuerpos desplegados, los diffs, la búsqueda web y la lista de tareas, sin pantallas nuevas.

| Ítem de Codex | Se muestra como | Notas |
|---|---|---|
| `commandExecution` | `Bash` (`command`, `description`) | `commandActions` clasifica el comando. `read` pasa a `Read`, `search` a `Grep`, `listFiles` a `LS`, y el resto queda en `Bash` |
| `fileChange` | `Edit` / `Write` por archivo (`add` → creado, `update` → editado, `delete`) | Codex da un diff unificado. `add` trae el contenido completo como `diff`; `update` trae solo el hunk. El antes y el después de `ToolResult.files` se reconstruyen con el repo sombra del motor (confirmado, ver Fase 1) |
| `mcpToolCall` | `mcp__{server}__{tool}` | Etiqueta y logo del conector como hoy |
| `webSearch` | `WebSearch` con `WebSearchRun` | El bloque de búsqueda web actual |
| `turn/plan/updated` | La lista de tareas (forma de `TodoWrite`) | El panel de tareas sobre el composer. No hay ítem `plan`, solo la notificación, y el thread debe activar `update_plan` (ver Fase 1) |
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

Métodos de inicio de sesión: "ChatGPT account" (`codex login`) y "Device code" (`codex login --device-auth`), si su salida se puede leer (resuelto en la fase 2: solo el primero, ver "Fases 2 y 3"). La clave de API queda fuera de la v1, porque necesita un campo seguro. La salida exacta de los tres comandos se verifica en la fase 1, solo con `status` contra la configuración real.

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

## Fase 1: lo observado

Capturas de `codex-cli 0.163.0` (2026-10-28) con `quill-capture --agent codex <escenario>`, modelo `gpt-6-luna`, esfuerzo `low`, cuenta ChatGPT plus. Los fixtures están en `crates/quill-agent-codex/fixtures/0.163.0/<escenario>.jsonl` (`archivo:línea` abajo). Se gastaron 25 turnos. El formato es el de Claude: `{t_ms, dir, msg}` con `dir` = `out`, `in`, `stderr` o `exit`. Los mensajes JSON-RPC no llevan `"jsonrpc"`. Las claves de cada línea están ordenadas alfabéticamente por la anonimización, no en el orden del cable.

### Escenarios

| Escenario | Resultado |
|---|---|
| `simple`, `edit-bash`, `deny-patch`, `deny`, `always`, `steer`, `interrupt`, `resume`, `fork`, `websearch`, `ask`, `plan`, `account` | Comportamiento buscado |
| `think` | **No emitió razonamiento.** Con `gpt-6-luna` en `low` y `summary: "detailed"` no hubo ítems `reasoning` ni `item/reasoning/*` (dos intentos). Las formas de `summaryTextDelta` / `textDelta` salen solo del esquema. Hace falta repetirlo en un esfuerzo mayor o con otro modelo |
| MCP y subagente | Omitidos: no hay un servidor MCP trivial ni un modo de subagente sin configuración extra. `account` sí registra `mcpServerStatus/list` |

Cómo se llegó a cada uno:
- `deny-patch` es el rechazo de un `apply_patch` (`fileChange`). `deny` y `always` son de comandos y usan `approvalPolicy: untrusted`.
- `plan` y `ask` fijan una `config` por thread en `thread/start` (no tocan `~/.codex/config.toml`): `{"tools":{"update_plan":{"enabled":true}}}` y `{"features":{"default_mode_request_user_input":true}}`.
- `account` está recortado: `mcpServerStatus/list` trae las herramientas de todos los conectores (4 MB), así que quedan 2 herramientas y 1 recurso por servidor.
- Las instrucciones globales de la persona (`~/.codex/AGENTS.md` → `rtk …`) se cuelan en los comandos y en las `instructionSources` (`edit-bash.jsonl:54`). Es ruido de la cuenta, no del protocolo.

### Preguntas del plan

1. **`item/started` de un comando, ¿antes o después de su pedido de aprobación?** Antes. El ítem llega `inProgress`, después el pedido del servidor, la respuesta, `serverRequest/resolved` y por último `item/completed` (`deny.jsonl:26` started, `:27` requestApproval, `:28` respuesta, `:29` resolved, `:30` completed con `status: "declined"`). Igual con `fileChange` (`edit-bash.jsonl:58` started, `:60` requestApproval). `serverRequest/resolved` llega antes que `item/completed`.
2. **`fileChange`: ¿diff o contenido completo?** Solo diff por archivo, sin el contenido de antes. `changes[{path, kind, diff}]`: con `kind.type: "add"` el `diff` es el contenido entero del archivo nuevo (`"hello\n"`, `edit-bash.jsonl:58`); con `update` (`kind.move_path` puede ser null) es un hunk sin cabecera (`"@@ -1 +1 @@\n-hello\n+hello world\n"`, `:69`). El antes y el después hay que reconstruirlos con el repo sombra. Además `turn/diff/updated` manda el diff git acumulado de todo el turno, con cabecera `diff --git` (`:65`, `:76`).
3. **`turn/steer`.** Pedido `{threadId, expectedTurnId, input}` y respuesta `{turnId}` (el mismo turno, `steer.jsonl:30` y `:32`). No hay `turn/started` nuevo: llega otro ítem `userMessage` con el texto del steer dentro del mismo turno (`:47`, `:48`) y el modelo lo acata (`1..5`).
4. **`thread/resume`.** La respuesta repite la historia: `thread.turns` trae el turno anterior con `itemsView: "full"` y los ítems completos, y `initialTurnsPage` es null (`resume.jsonl:52`). Antes manda `deprecationNotice` ("Full-history hydration is deprecated for paginated threads; use `excludeTurns: true`, then page with `thread/turns/list`", `resume.jsonl:41`). Con `excludeTurns: true` más `thread/turns/list {threadId}` la historia llega paginada (`data`, `nextCursor`, `backwardsCursor`), pero cada turno viene con `itemsView: "summary"` (el `userMessage` y la respuesta final; no se vio si incluye comandos, porque el turno no tenía herramientas, `resume.jsonl:56`). Para la historia con herramientas hay `thread/items/list` (no probado). `thread/fork` devuelve la misma forma, con `forkedFromId` y los turnos completos (`fork.jsonl:42`), y emite `thread/started` del thread nuevo (`:45`).
5. **Un turno sin cuenta.** No se probó (no se cerró la sesión). Por el esquema y la fuente: `account/read` devuelve `account: null` y `requiresOpenaiAuth: true` (con cuenta, `requiresOpenaiAuth` también es true y `account` viene lleno, `account.jsonl:9`; así que la señal es `account == null`). Un turno falla con una notificación `error` cuyo `codexErrorInfo` puede ser `unauthorized` y un `turn/completed` con `status: "failed"` y `turn.error`; el esquema de `TurnStatus` es `completed | interrupted | failed | inProgress`. Es inferido; confirmarlo en la fase 4 con una `CODEX_HOME` vacía, sin tocar la sesión real.
6. **`codex login status`.** Escribe en **stderr** (stdout vacío): `Logged in using ChatGPT` y sale 0. Sin sesión, la fuente imprime `Not logged in` y sale 1 (`codex-rs/cli/src/login.rs`, `run_login_status`). Otras variantes: `API key configured - <clave recortada>`, `Logged in using workload identity`. No incluye el email: el email sale de `account/read`.
7. **`codex login` / `--device-auth`.** No se ejecutaron. `codex login --help` y `codex login --device-auth --help` imprimen la misma ayuda (`--device-auth` es un flag de `login`, sin ayuda propia); opciones: `--with-api-key` y `--with-access-token` (leen de stdin), `--device-auth`, `-c`, `--enable`, `--disable`, subcomando `status`. Por la fuente, el login por navegador imprime en **stderr** `Starting local login server on http://localhost:<puerto>.` / `If your browser did not open, navigate to this URL to authenticate:` / la URL / `On a remote or headless machine? Use \`codex login --device-auth\` instead.`, y al terminar `LOGIN_SUCCESS_MESSAGE`. `login_url` debe leer la línea que sigue a esa frase. Falta ver la salida real del código de dispositivo.
8. **`codex logout --help`.** `Remove stored authentication credentials`, sin argumentos propios más allá de `-c`, `--enable`, `--disable`. No se ejecutó `logout`.

### Forma exacta de las aprobaciones

- **Comando**, pedido del servidor `item/commandExecution/requestApproval` (`deny.jsonl:27`): `params {threadId, turnId, itemId, command, cwd, commandActions, availableDecisions, proposedExecpolicyAmendment, kind: "command", environmentId, startedAtMs}`. `availableDecisions` fue `["accept", {"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["touch","denied.txt"]}}, "cancel"]`: **no ofrece `acceptForSession` ni `decline`**. Aun así `decline` funcionó (`status: "declined"`, `deny.jsonl:30`; stderr: `rejected by user`) y `acceptForSession` también: no hubo segundo pedido para el mismo comando (`always.jsonl:43` pedido, `:44` respuesta, y el segundo `item/started` sin pedido). El adaptador no debería limitarse a `availableDecisions`.
- **Archivo**, `item/fileChange/requestApproval` (`edit-bash.jsonl:60`): `params {threadId, turnId, itemId, reason, grantRoot, startedAtMs}`. El `itemId` es el del ítem `fileChange`. Rechazado, el ítem termina `status: "declined"` y el modelo sigue con el turno (`deny-patch.jsonl:64`); el turno cierra `completed` (`:100`).
- **Respuesta** a ambos: `{"id": <mismo id>, "result": {"decision": "accept" | "acceptForSession" | "decline" | "cancel"}}`. Los `id` de los pedidos del servidor son enteros desde 0, aparte de los del cliente. `cancel` además interrumpe el turno (esquema).
- **Pregunta**, `item/tool/requestUserInput` (`ask.jsonl:30`): `params {threadId, turnId, itemId, questions[{id, header, question, options[{label, description}], isOther, isSecret}], isBlocking: false, autoResolutionMs: null}`. Respuesta `{"result":{"answers":{"color_choice":{"answers":["Red"]}}}}` (`:31`). No hay ítem aparte para la pregunta: solo el pedido, y `serverRequest/resolved` (`:32`). El modelo la usó con `gpt-6-luna`, pero **solo con la feature activada** (la CLI avisa con `warning` en `ask.jsonl:9`).
- `item/permissions/requestApproval` y `mcpServer/elicitation/request` no aparecieron.

### Otros hallazgos que cambian el diseño

- **`commandActions`** viaja en el ítem `commandExecution` (y en el pedido): `{type: "read", command, name, path}` para `cat <archivo>` simple (`deny-patch.jsonl:53`), `{type: "listFiles", command, path}` para `ls` (`plan.jsonl:69`), y `{type: "unknown", command}` para el resto: `touch`, `pwd`, y también `cat … && pwd` o `rtk cat …` (`edit-bash.jsonl:54`, `:80`). `search` no apareció. El comando completo viene envuelto: `/run/current-system/sw/bin/bash -lc '<cmd>'`; el texto útil está en `commandActions[].command`.
- **Aprobaciones y el modo Read only.** Con `on-request` + `read-only` Codex **no** pide aprobación para un comando que el sandbox bloquea: el modelo recibe el fallo y lo cuenta. Solo `apply_patch` pide aprobación (`edit-bash`, `deny-patch`). Para ver aprobaciones de comandos hubo que usar `untrusted`. Hay que revisar los presets de "Modos de permiso": con `on-request` el modo "Read only" nunca preguntaría por comandos.
- **Resume, fork y los valores por defecto del usuario.** `thread/resume` y `thread/fork` **sin** `approvalPolicy` / `sandbox` volvieron con `sandbox: {type: "dangerFullAccess"}`, tomado de la configuración de la persona (primer intento, descartado y no incluido). El adaptador debe mandar `approvalPolicy` y `sandbox` en `thread/start`, `thread/resume` y `thread/fork`, siempre. Lo mismo vale para el esfuerzo: `thread/start` respondió `reasoningEffort: "max"` (el de la persona), y el `effort` sale en cada `turn/start`.
- **`update_plan` está apagado por defecto.** Sin config, el modelo dice que no existe la herramienta (dos intentos). Con `tools.update_plan.enabled` en la `config` del thread llega `turn/plan/updated` `{turnId, explanation, plan[{step, status}]}` con `status` `pending | inProgress | completed`; la primera notificación llega antes de los comandos (`plan.jsonl:52`, `:59`, `:66`, `:73`). Es la lista completa en cada notificación.
- **Razonamiento.** Ver `think`: con `gpt-6-luna` en `low` no hay resumen. Además, `turn/start.summary` hay que mandarlo (`auto` | `concise` | `detailed`) para tenerlos, y `model/list` da los esfuerzos válidos por modelo (`low` … `max`; `gpt-6-luna` ofrece `low, medium, high, xhigh, max`, `account.jsonl:7`). `model/list` vino en una sola página (`nextCursor: null`), 8 modelos.
- **Orden del arranque de un turno.** La respuesta de `turn/start` (`{turn:{id, status:"inProgress", items:[], itemsView:"notLoaded"}}`, `simple.jsonl:15`) llega antes que `turn/started` (`:20`). Los ítems del turno vienen con `item/started` / `item/completed` y `startedAtMs`; el primero es siempre el `userMessage`. Los `agentMessage` traen `phase`: `commentary` (los avisos del modelo entre herramientas) o `final_answer`. `item/agentMessage/delta` da `{delta, itemId}`.
- **`turn/interrupt`.** Pedido `{threadId, turnId}`, respuesta `{}`, y `turn/completed` con `status: "interrupted"` y `items: []` (`interrupt.jsonl:30`, `:34`, `:36`). `turn/completed.turn.items` siempre viene vacío (`itemsView: "notLoaded"`): los ítems se arman con `item/*`.
- **`webSearch`.** `item/started` con `query: ""` y `action: null`; `item/completed` con `query`, `action {type: "search", query, queries}` y `results[{title, url, snippet, domain, ref_id, type: "text_result"}]` (`websearch.jsonl:44`, `:45`). Funcionó con la búsqueda cacheada que viene por defecto.
- **Uso de tokens.** `thread/tokenUsage/updated {tokenUsage:{last, total, modelContextWindow}}`, cada uno con `inputTokens`, `cachedInputTokens`, `outputTokens`, `reasoningOutputTokens`, `totalTokens` (`simple.jsonl:31`; ventana de 258400 para `gpt-6-luna`).
- **Ruido que el adaptador debe tolerar** (queda como `Unknown` o se ignora): `remoteControl/status/changed`, `mcpServer/startupStatus/updated` (uno por servidor, antes y durante el primer turno), `account/updated`, `account/rateLimits/updated`, `thread/status/changed`, `thread/goal/cleared`, `hook/started` / `hook/completed` (hooks de la configuración de la persona, `interrupt.jsonl`), `warning`, `deprecationNotice`, y `emittedAtMs` en casi toda notificación.
- **Cuenta.** `account/read` → `{account:{type:"chatgpt", email, planType}, requiresOpenaiAuth, workspaceRouting{...}}`; `account/rateLimits/read` → `rateLimits{primary{usedPercent, windowDurationMins:300, resetsAt}, secondary{…10080…}, credits, planType}` (`account.jsonl:9`, `:11`). El CLI sale con código 0 al cerrar stdin (`exit` en cada fixture).
- **Repetir un proceso** (`resume`): el fixture es un solo archivo con el `exit` del primer proceso a mitad, seguido de otro `initialize` con `id: 1` (el cliente reinicia su contador). El agente falso ignora el `exit`, así que el adaptador de la fase 2 tiene que volver a mandar `initialize` en la misma conversación para reproducirlo.

### Pendiente

- Razonamiento en vivo (otro modelo o esfuerzo), un turno sin cuenta, `thread/items/list`, `search` en `commandActions`, `item/permissions/requestApproval`, MCP y subagentes.
- El esquema usa `cancel` y `acceptWithExecpolicyAmendment`: no se probaron.

## Fases 2 y 3: lo que cambió respecto del plan

El adaptador está en `crates/quill-agent-codex` (`Codex`, con `tested() = ["0.163.0"]`, exacto como el de Claude). Cada test reproduce un fixture: lo que el adaptador escribe tiene que ser el mismo número de líneas y los mismos métodos, ids y respuestas que los `out` de la captura, y lo que decodifica no deja ningún `Unknown`. El motor registra `codex` junto a `claude` y hay un test del motor que reproduce `simple` y `deny` con el agente falso.

- **Arranque en cuatro tiempos.** `encode(Start)` escribe solo `initialize`. Con su respuesta, `decode` devuelve en `replies` `initialized`, `model/list` y `thread/start` | `thread/resume` | `thread/fork` (`SessionSpec.resume` / `fork`). Así no se manda nada al servidor antes de que se inicialice. El contador de ids vuelve a 1 en cada `Start` (el `resume` tiene dos procesos). El `SessionSpec` entra por `launch` (que guarda una copia): es la única forma de que `Start` sepa el cwd, el modelo y la sesión a reanudar.
- **`model/list` es la única línea que las capturas no tienen** (salvo `account`). Va con el id de texto `"quill-models"` para no correr los números de las demás, y el agente falso la contesta solo (sin modelos) y no la cuenta como la línea que espera la captura. Mientras el adaptador no recibe esa respuesta, `Ready` trae el modelo del thread con los esfuerzos `low`…`xhigh`; cuando llega, se vuelve a emitir `Ready` con los modelos (el predeterminado primero, sin los `hidden`) y sus esfuerzos.
- **`thread/resume` pide `excludeTurns: true`** (evita la hidratación completa, `resume.jsonl:41`) y escribe después un `thread/turns/list {limit: 1}` cuya respuesta se ignora. Es la línea que la captura tiene en `resume.jsonl:54`: solo mantiene el paso del agente falso. Se puede quitar si se regraba `resume`.
- **Siempre `approvalPolicy` y `sandbox`**, en `thread/start`, `thread/resume` y `thread/fork`, y `approvalPolicy` + `sandboxPolicy` en cada `turn/start` (así un cambio de modo vale desde el turno siguiente), más la `config` del thread con `update_plan` y `default_mode_request_user_input` (la CLI avisa con un `warning`, ignorado). `full-access` solo se acepta si `SessionSpec.allow_unsafe`; si no, el modo vuelve a `auto`.
- **Los modos son los tres presets del plan, sin cambios**, con los nombres `read-only`, `auto` y `full-access` (`unsafe_modes`). Pendiente de decidir: con `on-request` el modo `read-only` nunca pide permiso para un comando (Codex le deja al modelo el fallo del sandbox, ver Fase 1) y `auto` solo pregunta por lo que sale del sandbox o por un `apply_patch` fuera de él. Para ver tarjetas de aprobación de comandos hace falta `untrusted`. Propuesta: que `read-only` sea `untrusted` + `read-only`. No se cambió. Además, `capabilities.modes` son solo ids: la interfaz arma sus etiquetas con los nombres de Claude (`view.rs`), así que los de Codex se muestran crudos hasta la fase 5.
- **`ModeChanged`.** `encode` solo devuelve líneas, y Codex no tiene un pedido que cambie los ajustes de un thread (viajan en el `turn/start`). `SetMode` escribe un `account/read` (verificado en `account.jsonl`) y cuenta el cambio con su respuesta. `Ready.mode` trae el inicial.
- **Un `Prompt` antes de que exista el thread** se encola y sale como `turn/start` con la respuesta de `thread/start`. `steer: true` con un turno en curso es `turn/steer`; el `Running` de la respuesta lista todos los pedidos del turno.
- **Mapa de ítems** (`commandExecution`, `fileChange`, `mcpToolCall`, `webSearch`, `turn/plan/updated`, `agentMessage`, `reasoning`) como en la tabla de arriba. Precisiones: un comando con una sola acción `read`, `listFiles` o `search` es `Read` / `LS` / `Grep`, y todo lo demás (incluidas cadenas de acciones) es `Bash` con el comando sin la envoltura `bash -lc '…'`; el `Edit` / `Write` de un `fileChange` toma la llamada del ítem para el primer archivo y `<id>#<n>` para los siguientes; `webSearch` se dibuja al completarse (empieza con `query` vacía); el plan es un `TodoWrite` con su `ToolResult` al instante (`inProgress` → `in_progress`). `ToolDone` no se emite (`tool_hooks: false`: la atribución es por turno con el repo sombra).
- **`ToolResult.files` de una edición** sale del hunk: `before` / `after` son las líneas viejas y nuevas del diff con su contexto, **no el archivo entero** (comentado en el código como atajo). Un `add` es todo `after`; un `delete` es todo `before` (inferido: ninguna captura lo tiene). No hay conteo de agregadas / quitadas en el evento: `FileChange` no lo tiene, la interfaz lo saca de `before`/`after`.
- **Aprobaciones.** Comando: `PermissionRequest` con tool `Bash` e input `{command}`; archivo: la herramienta y el input del ítem (`Edit` / `Write`); pregunta: `AskUserQuestion` con `{questions:[{question, header, options, multiSelect: false}]}`. Se ofrece una sugerencia (`acceptForSession`) para que la tarjeta muestre "Siempre". `Allow` → `accept`, `AllowAlways` → `acceptForSession`, `Deny` → `decline` (el mensaje se pierde: Codex no lo lleva). `cancel` no se usa. La tarjeta contesta por el texto de la pregunta y Codex quiere el `id`: el adaptador los empareja (una pregunta es de opción única; un "Other" escrito va tal cual). Lo que quill no sabe contestar (`item/permissions/requestApproval`, `mcpServer/elicitation/request`, `item/tool/call`, refresco de tokens…) se contesta con un error JSON-RPC `-32601` y queda como `Unknown`, para que Codex no espere.
- **Tokens.** `Tokens` es lo escrito por el modelo en el turno (suma de `last.outputTokens`, inferido), `ContextUsage` es `last.totalTokens` sobre la ventana, sin categorías, y `TurnFinished.usage` suma entrada y salida del turno. No hay costo.
- **Cuenta.** Solo "ChatGPT account": `codex login --device-auth` imprime en stdout un código que la persona tiene que escribir en la página, y quill solo sabe mostrar una URL. `login_url` toma la línea que empieza con `https://` (la URL de autenticación viene sola en su línea y en stderr; la de `http://localhost:<puerto>` no sirve). `auth_query = ["login", "status"]`; `auth_state` lee `Logged in using …` / `Not logged in` (sin email: eso sale de `account/read`). `logout_run = ["logout"]`. **El motor ahora pasa las dos salidas:** `auth_state` recibe stdout, o stderr si stdout está vacío (no se concatenan: el JSON de Claude no tiene que ensuciarse con avisos de stderr), y el login lee también las líneas de stderr para encontrar la URL.
- **Motor.** `waives_server_consent` ya no nombra modos: pregunta al adaptador (`AgentAdapter::mode_waives_prompts`). Claude: `bypassPermissions` y `auto`; Codex: solo `full-access` (su `auto` sí pregunta, no es el de Claude). El mapa de modos del motor ahora guarda (agente, modo).
- **Fuera de alcance, queda `Unknown` o sin respuesta:** `collabAgentToolCall`, `subAgentActivity`, `imageView`, `contextCompaction`, `plan` (ítem), `dynamicToolCall`, `imageGeneration`, review, `sleep`, `functionCallOutput`; `Rewind`, `ContextUsage`, `McpStatus` y compañía, `Skills`, herramientas del anfitrión (`HostTool`, navegador) y `SessionSpec.resume_at` (fork desde un mensaje: `thread/fork` tiene `lastTurnId`, pero el evento `Text` no lleva el id del turno). Las imágenes del prompt van como `{type:"image", url:"data:…;base64,…"}` (inferido del esquema).
- **Replay.** `fork` pasa dentro de un solo proceso en la captura, pero quill forkea lanzando uno nuevo (`fork: true`): el test lo comprueba como dos corridas. Por el mismo motivo, el agente falso no puede reproducir `fork` entero (el segundo `thread/fork` de la captura no sale nunca). `resume` sí, porque el motor relanza el programa.
