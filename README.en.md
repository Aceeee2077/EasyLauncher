# EasyLauncher

[简体中文](README.md) | [English](README.en.md)

A local Minecraft Java Edition server manager built with **Tauri 2, Rust, React, TypeScript, Vite and Tailwind CSS**. Java runs as a managed child process; the frontend has no filesystem or process permissions.

The interface uses **Shan Shui Glass**: frosted layers, azurite blue, malachite green, silk white and muted gold inspired by the palette of *A Thousand Li of Rivers and Mountains* (《千里江山图》). The background is a public-domain reproduction of the painting bundled locally; there are no Minecraft assets or external image requests. The UI supports Chinese (default), English, and following the system language. Branding is isolated in `src/services/api.ts`, `src-tauri/tauri.conf.json`, and the application icon.

## Current features

- Four-step creation wizard with live upstream release lists, RAM recommendations, port, player limit, game mode and difficulty.
- Vanilla via Mojang's manifest, stable Paper builds via the v3 downloads service, and Fabric executable launchers via Fabric Meta. The requested Minecraft release is never silently substituted.
- Java discovery from PATH, JAVA_HOME, JDK_HOME and common installation folders; custom executable validation and guided installation. Requirements also use Mojang's per-release Java metadata.
- Managed Java runtimes downloaded from Adoptium (Temurin), verified with SHA-256, extracted safely and pinned per server. The creation wizard offers one-click installation when a compatible runtime is missing.
- Explicit EULA checkbox and persisted acceptance time. Installation never starts Minecraft automatically.
- Persistent server metadata; actual process states, PID, readiness from server output, exit code and crash handling.
- Start, graceful stop, restart, duplicate-start protection and shutdown cleanup. Windows Job Objects terminate managed Java children if EasyLauncher exits abruptly.
- Live stdout/stderr console, bounded 1,500-line history, Minecraft stdin commands, command history, follow-output toggle and clear.
- Common and raw `server.properties` editors, unknown-key preservation, comments, escaped values and Unicode support. Runtime and memory changes are per-server.
- Real OP, de-OP, kick, ban, pardon and whitelist commands with player-name validation. Observed online players come from join/leave output.
- Manual stopped-server world backups, listing, confirmed restore/delete, automatic safety snapshots and restore rollback.
- Local port conflict checks, local/LAN addresses, managed mod/plugin directories and JAR listings.
- Light/dark/system appearance, Chinese/English/system language selection, global Java preference, managed folder shortcuts, friendly errors with technical details, and separate rotating application logs.
- Pre-flight health check on the Ops page: installation, EULA, Java, memory, JVM flags, port, firewall, router mapping, configuration, disk, backups and logs with a 0–100 readiness score. Problems are listed first and passing checks collapse; the check is read-only and never edits firewall or router settings.
- Live overview page: counts running and attention-needed servers, then batch starts, stops or restarts the selected ones. Only servers whose current state accepts the action are touched, and failures are listed per server.
- Pre-flight gate: starting reuses the health check and lists blocking issues such as an unaccepted EULA or missing files; you can still force a start after confirming the risk.
- Scheduled announcements and maintenance windows: tasks can broadcast a one-line `say` message or enter maintenance, which broadcasts first, then stops the server and creates a safety backup.
- Configuration templates: server settings ship survival, creative and performance presets. Every change is previewed first, and only whitelisted gameplay and performance keys are written; comments, the port, MOTD and other settings stay untouched.
- Command palette: press `Ctrl/Cmd+K` or click the topbar search field to find servers, pages and actions. Arrow keys move, Enter runs and Esc closes; typing also reveals per-server shortcuts such as console, settings and backups.
- Notification center: the topbar bell collects server-ready, unexpected-exit, backup-complete and scheduled-task-failure events. Unread count, clear and mute live in the dropdown; clicking an entry opens that server's console.
- Online player panel: expand a running server on the overview page to see who is online, then kick, op or message a player directly. Actions reuse the existing player commands and the console `tell` command, with the same server-side name validation.
- Local RCON (optional): explicitly enable it in server settings to generate a strong password, then refresh the authoritative online player list after restarting. EasyLauncher always connects to `127.0.0.1`; enabling RCON makes Minecraft listen on the port on every interface, and the UI says so.
- Disk space and cleanup: the ops page breaks the server folder down by worlds, backups, logs, mods and more, alongside the free space on that disk. You can select old logs (`latest.log` is kept), crash reports and interrupted leftovers, confirm, then see how much was actually freed. Worlds, backups and configuration are never cleaned.

## Screenshots

Screenshot slot: `docs/screenshots/overview.png` — the native Shan Shui Glass dashboard.

Screenshot slot: `docs/screenshots/console.png` — live console after a user-created server is started.

Screenshots containing real Minecraft activity are intentionally not generated during development. No Minecraft game client or dedicated server was launched for validation.

## Prerequisites

- Node.js **22.12+** (Node 24 was used for validation), npm, and stable Rust. Use a current Rust toolchain for the locked dependency versions.
- Windows: Microsoft C++ Build Tools with Desktop development with C++, Windows SDK, and Microsoft Edge WebView2 Runtime.
- Linux/macOS: the platform's [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Rust code includes platform handling, but only Windows builds have been validated.
- A matching Java runtime **for servers you intend to create**. Java is not required to build the application.
- Network access to Mojang, PaperMC and Fabric for release resolution, installation and first-run libraries.

## Install and run

From the project directory:

```powershell
npm install
npm run tauri dev
```

Use `npm.cmd` instead of `npm` if PowerShell's execution policy blocks `npm.ps1`.

`npm run dev` alone starts the frontend at `http://127.0.0.1:1420`. A regular browser explicitly reports that the desktop backend is unavailable; it never substitutes mock servers or simulated success.

To use EasyLauncher: create a server, select a release and software, allocate RAM, choose a compatible Java runtime (or install one in the wizard), read and accept the EULA, wait for installation, then click **Start server**. Readiness is announced only after Minecraft's `Done (...)! For help, type ...` output. Commands such as `say Hello` go to that Java process's stdin. Stop the server before editing properties or creating/restoring world backups.

## Build

```powershell
npm run build
npm run tauri build -- --no-bundle
# Windows NSIS installer (may download NSIS tooling on the first run):
npm run tauri build -- --bundles nsis
```

The Windows executable is `src-tauri/target/release/easylauncher.exe`; NSIS installers are under `src-tauri/target/release/bundle/nsis/`. The configuration currently targets the Windows installer. On another OS, override bundle targets for that platform. Installers are unsigned.

## Verification

```powershell
npm run typecheck
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

Additional process integration test, using a tiny **test-only Rust subprocess with no Minecraft code**:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --features test-fixtures --bin easylauncher-process-fixture
$env:EASYLAUNCHER_TEST_PROCESS = (Resolve-Path src-tauri/target/debug/easylauncher-process-fixture.exe).Path
cargo test --manifest-path src-tauri/Cargo.toml --lib subprocess_lifecycle_without_minecraft -- --ignored
```

Read-only verification of all three official metadata adapters, without downloading or running server JARs:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib upstream_adapter_resolution -- --ignored --nocapture
```

Tests cover Properties parsing/modification/serialization/unknown keys/Unicode, Java outputs and version boundaries, UUID traversal prevention, metadata defaults and round trips, state transitions, port conflicts, checksum rejection/partial-file cleanup, backup restoration/safety copies, and reopening persisted servers. The subprocess test exercises duplicate starts, stdout/stderr, stdin, restart, crash codes and graceful application shutdown.

If a sandbox restricts the system temporary directory, point `TMP` and `TEMP` to a writable project subdirectory before running tests. This is an environment setting, not an application storage override.

See [docs/VALIDATION.md](docs/VALIDATION.md) for executed checks and the distinction between fixture verification and actual Minecraft testing.

## Architecture

```text
src/
  components/          Shared controls, dialogs, status and error presentation
  pages/               Wizard, console, properties, players, backups and app settings
  services/            Typed Tauri command contract, branding and appearance
  hooks/               Task deduplication, pending state and error handling
  types/               Frontend data models
  assets/              Bundled public-domain landscape background
  styles.css           Layout and reusable component structure
  glass.css            Shan Shui Glass palette and optical surfaces
src-tauri/src/
  commands.rs          Typed IPC commands; validates command and folder intent
  models.rs            Metadata, explicit state enum and event payloads
  storage.rs           UUID/path/link checks, atomic JSON and world copying
  java/                Bounded runtime detection and centralized compatibility rules
  minecraft/           Upstream release metadata and Java Properties codec
  server/manager.rs    Persistence, per-server operation locks and event/history state
  server/installer.rs  Software adapters, retries, hashes, progress and cancellation
  server/process.rs    Java lifecycle, pipe reading, stdin and readiness/crash observation
  server/job.rs        Windows kill-on-close child process containment
  backup.rs            Consistent snapshots, restore staging and rollback
  rcon.rs              Optional local RCON client, password generation and player-list parsing
  network.rs           Local bind checks and LAN address detection
  logger.rs            Separate timestamped logs; 2 MB rotation with two retained files
  extensions.rs        Contracts for future content providers and crash analysis
```

React invokes Rust commands and subscribes to `server-status`, `server-console-batch`, `download-progress`, `backup-progress`, `app-closing` and `app-error`. Rust is authoritative. No lifecycle booleans or frontend timers fabricate server status. Backups, lifecycle changes and installation share a per-server operation mutex. Process monitoring holds no child lock while awaiting exit or reading streams.

## Storage

The Tauri application data directory (`app_data_dir`) supplies the root. There are no hardcoded usernames or development paths. Settings displays the actual path; Windows commonly resolves it under `%APPDATA%/dev.easylauncher.desktop`.

```text
<application data>/
  settings.json
  java/<uuid>/
    runtime/             # Managed Adoptium runtime (java.exe / bin/java)
    minehost-java.json   # Internal manifest; name retained for compatibility
  logs/minehost.log      # Separate from Minecraft logs; rotates at 2 MB
  servers/<uuid>/
    minehost.json        # Versioned metadata; runtime state is never persisted
    server.jar
    server.properties
    eula.txt
    world/               # Contains dimensions for Vanilla / Fabric
    world_nether/        # Separate Paper dimension, when generated
    world_the_end/
    mods/
    plugins/
    logs/latest.log
    backups/<uuid>/
      backup.json
      world/
      ...
```

Backups contain world data only, not server configuration, mods or plugins. They are uncompressed directory snapshots. Save copies outside managed storage if you need protection from disk loss or server deletion. The managed server deletion dialog explicitly includes its local backups.

## Safety and operational behavior

- EULA acceptance is a user action; EasyLauncher never downloads and boots a server in the background on first launch.
- Commands use process arguments, never an OS shell string. Console input is limited to one line and goes only to Minecraft stdin.
- Start and restart revalidate Java, memory, EULA, managed paths and local port availability. RAM leaves at least 1 GB and 20% of total memory for the system, plus a check against currently available memory.
- Stop sends `stop`, waits up to 45 seconds, then warns in the console before force termination. Normal application close waits for shutdown; repeated close requests do not bypass it.
- Managed UUIDs, world names, symbolic links, Windows reparse points, reserved names, linked folders and unsafe raw properties are validated. RCON can only be enabled explicitly in server settings; it requires a password of at least 12 characters and a port that differs from the game port, and the app always connects to `127.0.0.1`. Query and the management endpoint remain disabled.
- Restore requires a stopped server and explicit confirmation. It first saves existing worlds, stages new data, then renames folders. Failed rollback preserves recovery data in a `restore-*` folder. Startup refuses such a server until recovery is resolved; use its safety backup and preserved files before removing that folder.
- Invalid metadata is skipped and logged instead of crashing the catalogue. Application appearance corruption can be repaired by saving preferences again. Closing and reopening never blindly trusts a persisted Running flag.

## Known limitations

- **No Minecraft client or dedicated server was launched during validation**, as requested. Compilation, desktop startup, actual upstream metadata, file operations and a test-only subprocess were verified. The complete real-server startup/library bootstrap path still needs a user-run acceptance check after explicit EULA acceptance.
- Stable Minecraft releases **1.16 and newer** are listed; snapshots and older launch formats remain outside the supported list. Folder imports support recognized Vanilla, Paper and Fabric servers. Paper needs an upstream `STABLE` build for the selected release; an unsupported selection produces a clear error.
- Java selection requires a compatible major version. Managed Adoptium installations are pinned per server and can be removed from Settings. Minecraft 26.x requirements follow upstream metadata, with a Java 25 fallback; version rules live in one module.
- Fabric supplies a launcher without an upstream artifact hash. EasyLauncher validates HTTPS transport, JAR signature and reported length; Vanilla uses SHA-1 and Paper uses SHA-256. Fabric and Paper may download libraries on first server start; those upstream Java bootstrap downloads do not report per-byte progress to EasyLauncher.
- Backups require stopped servers. Scheduled backups are available; live save synchronization, compression and remote copies are deferred. Restore rejects a different `level-name` until the original name is configured.
- Connected-player observation relies on English upstream join/leave log messages by default; plugins can change these. With RCON enabled, the overview page can refresh the authoritative list, but player insights still persist only observed events and never backfill older logs.
- Minecraft itself binds RCON on every network interface; `server.properties` has no bind-address option. Enable it only on trusted networks and keep the generated strong password. EasyLauncher does not create router port mappings automatically.
- Normal close gracefully stops children on all supported platforms. Windows additionally uses Job Objects for abrupt exit cleanup. Abrupt parent termination cleanup on Linux/macOS is not yet guaranteed; inspect remaining Java processes before relaunching there.
- LAN address detection is a routing hint; VPNs/multiple adapters can affect it. Local or LAN availability does not imply public access. Users can explicitly request UPnP mappings or Windows firewall rules. frp requires their own provider and configuration.
- Mod/plugin management includes Modrinth content and dependency declarations. Local inspection covers Fabric mods plus Paper/Bukkit `paper-plugin.yml` and `plugin.yml` declarations and common semantic-version predicates, but it is not a complete loader resolver. Manually installed JARs that cannot be mapped back to Modrinth may still be reported as a missing managed dependency; check the local files. Minecraft's own logs remain under the server's upstream logging policy; EasyLauncher application logs and console buffers have independent limits.
- API availability and throughput depend on upstream providers. Paper requests use an identifying User-Agent; configure a real project contact URL before distributing a renamed/public fork.
- Windows is the tested build target. Code signing, macOS notarization, automatic updates and platform installers beyond NSIS are not included.

## Roadmap

The A–D roadmap has usable baseline implementations. Remaining extensions include historical player-log backfill, loader-specific edge-case version predicates, playit.gg account/client integration, and acceptance testing with real Minecraft and public networks. See the development notes for scope.

## Upstream references

- [Tauri commands and events](https://v2.tauri.app/develop/calling-rust/)
- [Paper downloads service](https://docs.papermc.io/misc/downloads-service/)
- [Fabric server launcher](https://fabricmc.net/use/server/)
- [Minecraft 26.1 Java requirements](https://www.minecraft.net/en-us/article/minecraft-java-edition-26-1)
- [Minecraft EULA](https://www.minecraft.net/eula)

EasyLauncher is independent software and **is not affiliated with, endorsed by, or associated with Mojang Studios or Microsoft**. Minecraft is a trademark of its respective owner.


## Advanced feature update (2026-10-19)

The scheduler, monitoring, rollback, crash recovery, mobile console, network checks and folder import are followed by configuration/JVM presets, Fabric and Modrinth dependency inspection, an optional bring-your-own-key log assistant, persistent player insights, PNG share cards, SHA-256 backup comparisons, managed frpc processes, a read-only mode, and a local audit log.

The log assistant sends only the editable excerpt displayed to the user, and only after an explicit click. API keys are never written to settings or application logs; checking “Remember configuration” stores the key encrypted with Windows DPAPI for the current account and it can be deleted again from app settings. Read-only mode is enforced by the backend and still allows stopping servers and tunnels. The audit log records action, time, server name and a short detail, never secret values, and rotates at about 1 MB. The frpc integration requires a trusted installed client and the user's own provider configuration; process status does not establish public connectivity. No real Minecraft process, public tunnel or paid AI service was used during this implementation.
