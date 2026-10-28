# Portabilidad: Linux, Windows y macOS

Estado al hacer esta pasada (rama `wip/portable`). Leyenda: **anda** (código con camino
real), **degradado** (compila y no rompe, pero hace menos), **falta** (no existe o no
compila).

## Qué se verificó y qué sólo se leyó

Desde Linux, con `rustup target add x86_64-pc-windows-msvc` (único componente instalado,
en espacio de usuario; `x86_64-pc-windows-gnu` y `aarch64-apple-darwin` ya estaban):

| Verificación | Resultado |
|---|---|
| `cargo check --target x86_64-pc-windows-gnu` | **compila**: quill-model, -agent, -agent-claude, -agent-codex, -store, -git, -files, -link, -pty, -update, -components, -ui, quill-ide |
| `cargo check --target aarch64-apple-darwin` | **compila**: quill-model, -agent*, -git, -files, -link, -pty. No se pudo: -store (sqlite), -update (ring), -components y -ui (tree-sitter) necesitan el compilador C de Apple |
| `quill-browser`, `quill-engine`, `quill` (la app) en Windows y macOS | **sólo leído, no compilado**: `cef-dll-sys` no soporta `windows-gnu` y con `windows-msvc` quiere bajar el CEF de Windows (cientos de MB); en macOS necesita el SDK. Los cambios en esos tres crates están auditados a ojo |
| Linux | `scripts/check.sh` verde (fmt, clippy `-D warnings`, tests) |

El job nuevo de CI (`.github/workflows/check.yml`) es lo que va a compilar de verdad esos
tres crates en Windows y macOS. Es la primera vez que corre: espere errores que acá no se
vieron.

## Subsistemas

| Subsistema | Linux | Windows | macOS | Qué se arregló / notas |
|---|---|---|---|---|
| Enlace quill ↔ quill-ide (`quill-link`) | anda (socket Unix) | anda, **sólo leído en runtime**: TCP en 127.0.0.1 con secreto en un archivo | anda (socket Unix, compila) | Antes no compilaba fuera de Unix. Windows: puerto efímero + secreto de 128 bits en el archivo de la carpeta del usuario; el secreto sale de `RandomState`, no de un CSPRNG. Un archivo viejo cuyo puerto reutilice otra app recibiría el secreto (riesgo bajo, local) |
| Terminal (`quill-pty`) | anda | degradado: usa `%COMSPEC%`; `busy()` siempre `false` (no avisa al cerrar con un programa corriendo) `quill-pty/src/lib.rs:135` | anda | `portable-pty` cubre ConPTY; no se probó |
| Archivos: copiar, mover, papelera (`quill-files`) | anda (`renameat2` atómico) | degradado: copiar un enlace simbólico da error `lib.rs:273`; mover sin reemplazar no es atómico `lib.rs:320` | anda salvo lo atómico (mismo `shortcut`) | La papelera es el crate `trash`, multiplataforma |
| Abrir en el sistema (`open_in_system`) | `xdg-open` | `rundll32` | `open` | Ya estaba |
| Carpeta de datos, config y home | `XDG_*`/`~/.local/share` | `%LOCALAPPDATA%`, `%APPDATA%` | `~/Library/Application Support` | Un solo lugar: `quill_files::{home_dir, data_dir, config_dir}` (`quill-files/src/lib.rs`). Linux no cambia. Dato inferido de cada convención, no probado. Quedan el instalador de `quill-update` (`install.rs:45`, usa `LOCALAPPDATA` en Windows y XDG en el resto, **mal en macOS**) y `editors.rs:82` (busca `.desktop` de XDG: en Windows/macOS no encuentra editores por ese camino) |
| Iniciar al abrir sesión (`start_at_login`) | `~/.config/autostart/quill.desktop` | `reg add HKCU\...\Run` | LaunchAgent `~/Library/LaunchAgents/dev.quill.quill.plist` | `engine/src/lib.rs:2816`. Windows y macOS escritos, sin probar. El plist de macOS no se carga con `launchctl`: toma efecto en el próximo login |
| No dejar dormir la máquina (`awake_command`) | `systemd-inhibit` | **falta**: sin FFI no hay `SetThreadExecutionState`; no hace nada | `caffeinate -i -w <pid de quill>` | `engine/src/lib.rs:2793`. Sin probar en macOS |
| Batería (`power_supplies`) | `/sys/class/power_supply` | vacío: nunca "a batería" `lib.rs:201` | vacío | Con la opción "no mantener despierto a batería" no se distingue en ninguno |
| Notificaciones | `notify-send` / `busctl` | PowerShell | `osascript` | Ya estaba (`lib.rs:2894`, `2929`) |
| Leer en voz alta | `spd-say` | **falta** (`lib.rs:3148`): el comando falla en silencio | **falta**: podría ser `say` | Hay evento "leyendo" aunque no suene nada |
| Servidores de `preview_start` (`servers.rs`) | grupo de procesos + `PDEATHSIG` | `taskkill /T /F` (sin grupo; no mueren si quill muere de golpe) | grupo de procesos; sin `PDEATHSIG` (es de Linux) | Antes `libc::prctl` rompía macOS |
| `$BROWSER` de los agentes (`engine/src/bridge.rs`, `main.rs:1507`) | anda (socket Unix) | **falta**: módulo apagado, los agentes abren el navegador del sistema | anda en teoría (compila gateado a `unix`), sin probar | `bridge_env` en `lib.rs:756` |
| Navegador: Chromium por pipe (`quill-browser/chromium.rs`) | anda | **falta**: `launch` da `Unsupported` (`chromium.rs:77`); haría falta `--remote-debugging-io-pipes` con handles | degradado: `pipe` + `FD_CLOEXEC` en vez de `pipe2`, sin `PDEATHSIG` | `find` ahora también mira las rutas habituales de Chrome |
| Navegador nativo CEF + dmabuf (`native.rs`) | anda (X11/Vulkan) | **falta**: dmabuf es de Linux (`native.rs:364`); sin `DISPLAY` cae a headless `native.rs:152` | **falta**: igual (no hay dmabuf; Metal/IOSurface) | Sin compilar. CEF necesita su propio paquete por plataforma |
| Página bajo la ventana (`apps/quill/src/direct.rs`) | anda (Wayland) | no aplica: stub que nunca muestra | no aplica: stub | `wayland-*`, `gbm` y `raw-window-handle` pasaron a dependencias sólo-Linux en `apps/quill/Cargo.toml`. Sin esto la app no podía compilar en Windows |
| Reinicio tras actualizar (`wait_for_restarter`, `main.rs:1486`) | espera por `/proc/<pid>` | degradado: espera 1 s fijo | igual | Sin FFI no hay "¿vive el pid?" |
| Zona horaria / nombre (`utc_offset` `main.rs:1407`, `first_name` `:1425`) | `date`, `getent` | `date` no existe: UTC; el nombre sale de `USERNAME` | `date` anda; sin `getent` usa el usuario | Degradado, cosmético |
| Fuentes del sistema (`fc-match`, `fc-list`) `quill-components/src/lib.rs:22`, `quill-ide/src/disk.rs:142,169` | anda | degradado: sin fontconfig quedan las fuentes incluidas de limn; en quill-ide el selector de familias queda vacío y `faces` da error | igual | Una lectura de registro (Windows) o CoreText (macOS) es el arreglo real |
| Diálogos de carpeta (`rfd`) y portapapeles (`arboard`) | portal XDG | nativo | nativo | Features `xdg-portal`/`wayland-data-control` sólo actúan en Linux; sin probar |
| Mostrar en el gestor de archivos (`show_in_file_manager`) | D-Bus `FileManager1` | `explorer /select,` | `open -R` | Nuevo |
| Actualizador (`quill-update`) | anda | anda (`install.rs` ya tenía rama Windows); **el release de Windows sigue apagado** | falta: no hay artefacto ni rama de instalación | Ver `docs/UPDATES.md` |
| CI | release Linux | check (`check.yml`, `continue-on-error`) + release apagado | check | Decisiones de release en `docs/UPDATES.md` |

## Qué falta, en el orden recomendado

1. **Mirar el primer resultado de `check.yml`.** Es lo único que compila `quill-browser`,
   `quill-engine` y `quill` fuera de Linux; corregir lo que salga antes de seguir. Cuando
   den verde, sacar `continue-on-error`.
2. **Compilar de verdad la app en Windows** (el release ya apunta a `windows-msvc`) y
   correrla: ventana, terminal, `quill-link`, instalación. `native.rs` probablemente
   exija apagar el navegador nativo en Windows con un `cfg` (hoy sólo cae a headless en
   runtime) y, como `chromium::launch` no existe allí, el navegador queda **sin motor**:
   decidir si en Windows la pestaña de navegador se oculta.
3. **Navegador en Windows**: `--remote-debugging-io-pipes` con handles heredados, o
   `--remote-debugging-port` con token. Es la pieza grande.
4. **macOS**: ruta de datos de `quill-update` (`install.rs:45`), artefacto y job de
   release, firma/notarización (no tocado), y decidir el camino del navegador (CEF con
   IOSurface en lugar de dmabuf).
5. **Detalles baratos**: `say` para voz en macOS, lectura de la batería
   (`pmset`/`WMI`), `SetThreadExecutionState` si se acepta una dependencia (`windows-sys`)
   para mantener despierto en Windows, enlaces simbólicos en `copy_entry`, fuentes del
   sistema sin fontconfig, vigilancia de pid sin `/proc`.
6. Todo lo marcado "sin probar" arriba: lo escrito sin ejecutar (registro, LaunchAgent,
   `caffeinate`, `taskkill`, `explorer`, TCP del enlace) hay que correrlo una vez en la
   máquina real.
