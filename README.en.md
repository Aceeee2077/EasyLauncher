# EasyLauncher

[简体中文](README.md) | [English](README.en.md)

A local Minecraft Java Edition server manager built with **Tauri 2, Rust, React, TypeScript, Vite and Tailwind CSS**. Java runs as a managed child process; the frontend has no filesystem or process permissions.

The interface uses **Shan Shui Glass**: frosted layers, azurite blue, malachite green, silk white and muted gold inspired by the palette of *A Thousand Li of Rivers and Mountains* (《千里江山图》). The background is a public-domain reproduction of the painting bundled locally; there are no Minecraft assets or external image requests. The UI supports Chinese (default), English, and following the system language. Branding is isolated in `src/services/api.ts`, `src-tauri/tauri.conf.json`, and the application icon.

## Current features

- Four-step creation wizard with live upstream release lists, RAM recommendations, port, player limit, game mode and difficulty.
- Vanilla via Mojang's manifest, stable Paper builds via the v3 downloads service, Purpur via its v2 API, and Fabric executable launchers via Fabric Meta. Each software family is an adapter that declares its metadata source, artifact verification and Java requirement. Vanilla and Fabric can also install snapshots, and releases back to 1.7 are supported. The requested Minecraft version is never silently substituted.
- Java discovery from PATH, JAVA_HOME, JDK_HOME and common installation folders; custom executable validation and guided installation. Requirements also use Mojang's per-release Java metadata.
- Managed Java runtimes downloaded from Adoptium (Temurin), verified with SHA-256, extracted safely and pinned per server. The creation wizard offers one-click installation when a compatible runtime is missing.
- Explicit EULA checkbox and persisted acceptance time. Installation never starts Minecraft automatically.
- Persistent server metadata; actual process states, PID, readiness from server output, exit code and crash handling.
- Start, graceful stop, restart, duplicate-start protection and shutdown cleanup. Windows Job Objects terminate managed Java children if EasyLauncher exits abruptly.
- Live stdout/stderr console, bounded 1,500-line history, Minecraft stdin commands, command history, follow-output toggle and clear.
- Common and raw `server.properties` editors, unknown-key preservation, comments, escaped values and Unicode support. Runtime and memory changes are per-server.
- Real OP, de-OP, kick, ban, pardon and whitelist commands with player-name validation. Observed online players come from join/leave output.
- Manual stopped-server world backups, plus consistent live backups while the server runs when RCON is enabled. Backups use 1 MiB chunks, zstd compression and SHA-256 content addressing, with a previewed daily/weekly/monthly retention policy. Listing, confirmed restore/delete, automatic safety snapshots and restore rollback are included. Archives can be uploaded to WebDAV, S3-compatible storage or a remote exposed through local rclone, with optional age passphrase encryption, post-upload size/SHA-256 verification and retries; S3 multipart uploads resume completed parts, and credentials live in the Windows DPAPI vault. The backup page can also read Anvil MCA/NBT data without modifying it: render 2D region maps, show player coordinates and online tracks, count entities and block entities, detect corrupt chunks, and compare regions between two backups.
- Version upgrade wizard for installed, stopped servers: pick a supported stable release, get an automatic world safety backup, then download and verify the new server JAR while keeping the previous one for a one-step rollback. Downgrades require explicit confirmation; mod and plugin compatibility is reported, never changed automatically.
- Content update checks for mods and plugins installed through EasyLauncher: find newer Modrinth versions for this Minecraft release, review the changelog, dependency changes and incompatibility warnings, and pin a version to skip checks. A confirmed update saves the old JAR first, then downloads, verifies and replaces it; a single package can be rolled back without touching worlds or configuration.
- Modpack install and update: parse a Modrinth `.mrpack` manifest, `overrides/` and `server-overrides/`, filter client-only, protected and oversized entries, and record managed files in `modpack.json`. Update checks download and verify the candidate pack, then show a hash-based list of added, changed, removed and unchanged files with its changelog. A confirmed update downloads only changed files, backs up the files it replaces, restores automatically on failure, and keeps a one-step rollback. One modpack is managed per server.
- Local port conflict checks, local/LAN addresses, managed mod/plugin directories and JAR listings.
- Light/dark/system appearance, Chinese/English/system language selection, global Java preference, managed folder shortcuts, friendly errors with technical details, and separate rotating application logs.
- Pre-flight health check on the Ops page: installation, EULA, Java, memory, JVM flags, port, firewall, router mapping, configuration, disk, backups and logs with a 0–100 readiness score. Problems are listed first and passing checks collapse; the check is read-only and never edits firewall or router settings.
- Live overview page: counts running and attention-needed servers, then batch starts, stops or restarts the selected ones. Only servers whose current state accepts the action are touched, and failures are listed per server.
- Pre-flight gate: starting reuses the health check and lists blocking issues such as an unaccepted EULA or missing files; you can still force a start after confirming the risk.
- Scheduled announcements and maintenance windows: tasks can broadcast a one-line `say` message or enter maintenance, which broadcasts first, then stops the server and creates a safety backup.
- Scheduled tasks 2.0: five-field cron expressions with presets, a visual field editor and a checked preview of the next five runs; a seven-day task calendar; serial execution behind one prerequisite task; a missed-run policy (skip, or catch up once after startup); and the latest 200 run-history entries per task. Manual and scheduled runs both record the result, trigger and error detail.
- Configuration templates: server settings ship survival, creative and performance presets. Every change is previewed first, and only whitelisted gameplay and performance keys are written; comments, the port, MOTD and other settings stay untouched.
- Command palette: press `Ctrl/Cmd+K` or click the topbar search field to find servers, pages and actions. Arrow keys move, Enter runs and Esc closes; typing also reveals per-server shortcuts such as console, settings and backups.
- Notification center: the topbar bell collects server-ready, unexpected-exit, backup-complete and scheduled-task-failure events. Unread count, clear and mute live in the dropdown; clicking an entry opens that server's console.
- Online player panel: expand a running server on the overview page to see who is online, then kick, op or message a player directly. Actions reuse the existing player commands and the console `tell` command, with the same server-side name validation.
- Local RCON (optional): explicitly enable it in server settings to generate a strong password, then refresh the authoritative online player list after restarting. EasyLauncher always connects to `127.0.0.1`; enabling RCON makes Minecraft listen on the port on every interface, and the UI says so.
- Disk space and cleanup: the ops page breaks the server folder down by worlds, backups, logs, mods and more, alongside the free space on that disk. You can select old logs (`latest.log` is kept), crash reports and interrupted leftovers, confirm, then see how much was actually freed. Worlds, backups and configuration are never cleaned.
- World preview and save health: the backup page reads Anvil MCA/NBT data without modifying the world. It renders a 2D map per region with player markers and local online tracks, counts chunks, entities and block entities by type, lists region-header, chunk-compression and NBT damage, and compares two backups by region-file SHA-256. Maps support 1/2/4/8-block detail; worlds and backups stay untouched.

## Screenshots

Screenshot slot: `docs/screenshots/overview.png` — the native Shan Shui Glass dashboard.

Screenshot slot: `docs/screenshots/console.png` — live console after a user-created server is started.

Screenshots containing real Minecraft activity are intentionally not generated during development. Real servers are started only by the explicitly invoked E2E harness, inside a temporary directory, never for screenshots and never in the background during normal use.

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

To use EasyLauncher: create a server, select a release and software, allocate RAM, choose a compatible Java runtime (or install one in the wizard), read and accept the EULA, wait for installation, then click **Start server**. Readiness is announced only after Minecraft's `Done (...)! For help, type ...` output. Commands such as `say Hello` go to that Java process's stdin. Stop the server before editing properties. World backups can be created while it runs once RCON is enabled; otherwise stop it first. Restoring and deleting backups still require a stopped server.

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

The real Minecraft acceptance harness is ignored by default and runs **only when this command is invoked explicitly**. Normal tests, pushes, pull requests and scheduled jobs never start a game server. It needs network access and disk space and works entirely inside a temporary directory that is deleted afterwards:

```powershell
cd src-tauri
$env:E2E_SOFTWARE = "vanilla"   # or paper / fabric
$env:E2E_MC_VERSION = "1.21.4"
cargo test --test minecraft_e2e -- --ignored --nocapture
```

It downloads a real server, accepts the EULA only inside the temporary directory, waits for the authoritative readiness line, checks the port and a console command, stops cleanly, verifies backup and restore, then forces a crash and waits for automatic recovery. A matching Java major is installed as a managed Temurin runtime when none exists. `.github/workflows/minecraft-e2e.yml` exposes the same matrix for manual runs only.

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
  backup_store.rs      zstd chunk storage, content-addressed dedupe, verified restore and GC
  remote_backup.rs     Archive packaging, age encryption, WebDAV / S3 / rclone upload and verification
  world_map.rs         Read-only Anvil MCA/NBT parsing, map rendering, save health and region diff
  retention.rs         Daily/weekly/monthly retention, preview and cleanup
  upgrade.rs           Staged server JAR upgrades and single-step rollback
  content_update.rs    Content update checks, per-package snapshots and rollback
  modrinth.rs          Modrinth content install, modpack install/update/rollback and checks
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
    backups/
      chunks/<prefix>/<sha256>.zst   # Compressed chunks shared by all backups
      retention.json                 # Optional retention policy
      <uuid>/
        backup.json
        manifest.json                # Paths, file hashes and chunk references
        files.json                   # File index used by comparisons
    backups/<legacy uuid>/world/      # Old directory snapshots still restore/compare
```

Backups contain world data only, not server configuration, mods or plugins. Each world is read in 1 MiB chunks, every chunk is compressed with zstd and addressed by the SHA-256 of its uncompressed content, so identical chunks are stored once. `manifest.json` records file and chunk references, while `files.json` keeps the compatible file-level index. Restore verifies every chunk's length and hash and refuses to replace worlds on any mismatch. Legacy directory snapshots keep their original layout and remain restorable and comparable, but are not converted automatically. Chunks under `backups/chunks` that no manifest references are reclaimed when backups are deleted or cleanup runs. Save copies outside managed storage if you need protection from disk loss or server deletion. The managed server deletion dialog explicitly includes its local backups.

## Safety and operational behavior

- EULA acceptance is a user action; EasyLauncher never downloads and boots a server in the background on first launch.
- Commands use process arguments, never an OS shell string. Console input is limited to one line and goes only to Minecraft stdin.
- Start and restart revalidate Java, memory, EULA, managed paths and local port availability. RAM leaves at least 1 GB and 20% of total memory for the system, plus a check against currently available memory.
- Stop sends `stop`, waits up to 45 seconds, then warns in the console before force termination. Normal application close waits for shutdown; repeated close requests do not bypass it.
- Managed UUIDs, world names, symbolic links, Windows reparse points, reserved names, linked folders and unsafe raw properties are validated. RCON can only be enabled explicitly in server settings; it requires a password of at least 12 characters and a port that differs from the game port, and the app always connects to `127.0.0.1`. Query and the management endpoint remain disabled.
- Restore requires a stopped server and explicit confirmation. It first saves existing worlds, stages new data, then renames folders. Failed rollback preserves recovery data in a `restore-*` folder. Startup refuses such a server until recovery is resolved; use its safety backup and preserved files before removing that folder.
- Live backups hold the same per-server operation lock, require a running server with RCON enabled, and always try to resume autosave even when the copy fails. Repeated `save-on` failures return `backup.saveOnFailed` while keeping the completed backup. Read-only mode blocks the action like any other backup, and every attempt is written to the audit log.
- Invalid metadata is skipped and logged instead of crashing the catalogue. Application appearance corruption can be repaired by saving preferences again. Closing and reopening never blindly trusts a persisted Running flag.

## Known limitations

- **Real-server acceptance is opt-in.** Normal tests and daily use never download or start a server in the background. Running `cargo test --test minecraft_e2e -- --ignored` downloads and starts real Vanilla, Paper or Fabric inside a temporary directory, accepts the EULA only there, and deletes it afterwards. Vanilla 1.21.4 passes locally; Paper and Fabric can be exercised with `E2E_SOFTWARE` or the CI matrix. Real player connections, public tunnels and paid external models still need an environment-specific acceptance check.
- Version lists cover **releases and snapshots back to 1.7** in Mojang's manifest: Vanilla and Fabric can opt into snapshots, while Paper and Purpur list only releases with an upstream stable build. Older releases still use the standard `-jar server.jar nogui` launch format. Snapshots may be unstable, and the upgrade wizard remains release-only. Folder imports support recognized Vanilla, Paper, Purpur and Fabric servers. Spigot has no official artifact API; Forge, NeoForge and Quilt require running official installers and a non-`-jar` launch model and are not supported yet, so the wizard does not offer them.
- Java selection requires a compatible major version. Managed Adoptium installations are pinned per server and can be removed from Settings. Minecraft 26.x requirements follow upstream metadata, with a Java 25 fallback; version rules live in one module.
- Fabric supplies a launcher without an upstream artifact hash, and Purpur's API publishes only MD5 (currently unused). EasyLauncher validates HTTPS transport, the ZIP/JAR header and reported length; Vanilla uses SHA-1 and Paper uses SHA-256. Fabric, Paper and Purpur may download libraries on first server start; those upstream Java bootstrap downloads do not report per-byte progress to EasyLauncher.
- Normal backups and restores require stopped servers. With RCON enabled and the server ready, a live backup pauses autosave (`save-off`), flushes (`save-all flush`), copies the world directories, then resumes autosave (`save-on`); if autosave cannot be re-enabled, the action reports a stable error and asks you to restart or run `save-on`. Minecraft has no filesystem snapshot API, so this lowers the risk of writes during the copy but is not an atomic snapshot; chunk unloads can still write. If RCON is unavailable the app never pretends a live backup is consistent. Local backups now use zstd chunk compression and content-addressed deduplication. Remote targets support WebDAV, S3-compatible storage and local rclone: a chunked backup is packaged as a ZIP with optional age passphrase encryption; S3 multipart uploads record completed parts and resume after failure, WebDAV has no standard resume and restarts, and rclone uses its own resumability. After upload the remote size is checked and, when enabled, the archive is downloaded again to verify SHA-256. Uploads are always manual, and restoring directly from a remote archive is not available yet. Passwords, secret keys and passphrases are stored only in the Windows DPAPI vault, never in plain JSON; HTTP endpoints send credentials and backup contents in clear text and should be used only on this machine or a trusted network. The retention policy only considers non-safety backups without a label in its time buckets, always keeps the newest backup and labelled backups, runs automatically only after successful scheduled backups, and requires a preview plus confirmation for manual cleanup. Restore rejects a different `level-name` until the original name is configured.
- World preview is read-only. It supports gzip, zlib and uncompressed chunks; the optional LZ4 chunk compression introduced in 1.20.5 (type 4) is reported as unsupported rather than guessed. Map colours are approximations based on block names and height, not in-game textures. A scan is bounded to 128 MiB per file, 64 MiB of decompressed NBT, 256 regions and 50,000 chunks; work beyond that is marked as truncated. Entity statistics count records in the saved lists without parsing full entity data. Player coordinates come from the backup's `playerdata`, not from current online positions; online tracks are sampled only when the user asks, through local RCON, kept in the server folder for seven days and never uploaded.
- Version upgrades replace `server.jar` only and keep one `server.jar.previous` for a single-step rollback; mods, plugins, configuration and worlds are not migrated. The pre-upgrade safety backup contains world data only. The server must be installed and stopped, and a missing required Java major blocks the upgrade while offering to install it. Downgrades need an extra confirmation, and even then an older version may not read a world generated by the newer one. Rolling back replaces the JAR only, never the world; the UI shows the pre-upgrade safety backup ID.
- Content update checks cover only Modrinth projects installed and recorded in `content.json`; manually copied JARs are not updated. Checks read upstream metadata, so availability depends on Modrinth. The replaced JAR is saved under `content-backups/` in the server folder for a one-step, per-package rollback; dependent packages are not upgraded automatically, and missing, changed or newly required dependencies are reported without being installed. An update replaces that JAR only, leaves configuration and worlds alone, and requires a stopped server.
- Modpacks currently support Modrinth `.mrpack`. CurseForge pack downloads require a CurseForge API key and use a different `manifest.json`/`overrides` layout, so they are not supported yet; the application does not present an unverified CurseForge path as working. Each server records one modpack: installing another pack or another version of the same pack is refused; use "Check modpack updates", the one-step rollback, or create a new server. An update check stages the candidate archive as `modpack-pending.mrpack`, cleaned up when it is applied or checked again; `modpack-backups/` keeps only the latest update backup and removes it after a rollback. Updates touch only files recorded for the pack, never migrate worlds, and do not check for a newly required Java runtime.
- Scheduled-task cron accepts five numeric fields (minute 0–59, hour 0–23, day 1–31, month 1–12, weekday 0–7 where both 0 and 7 mean Sunday) with `*`, lists, ranges and steps; English names such as `JAN` are not recognized. When both day and weekday are restricted, both must match, so leave one as `*` for ordinary weekly or monthly schedules. The scheduler polls every 20 seconds; a run more than two minutes late is treated as missed while the application was closed and is skipped or caught up once according to the task policy, never once per missed slot. A prerequisite only coordinates runs in the same scheduling pass: if it fails, the dependent task is skipped for that pass, and manual triggers do not enforce prerequisites. Run history lives in `task-history.json` in the server folder, keeps the latest 200 entries, and is not part of backups.
- Connected-player observation relies on English upstream join/leave log messages by default; plugins can change these. With RCON enabled, the overview page can refresh the authoritative list, but player insights still persist only observed events and never backfill older logs.
- Minecraft itself binds RCON on every network interface; `server.properties` has no bind-address option. Enable it only on trusted networks and keep the generated strong password. EasyLauncher does not create router port mappings automatically.
- Normal close gracefully stops children on all supported platforms. Windows additionally uses Job Objects for abrupt exit cleanup. Abrupt parent termination cleanup on Linux/macOS is not yet guaranteed; inspect remaining Java processes before relaunching there.
- LAN address detection is a routing hint; VPNs/multiple adapters can affect it. Local or LAN availability does not imply public access. Users can explicitly request UPnP mappings or Windows firewall rules. frp requires their own provider and configuration.
- Mod/plugin management includes Modrinth content and dependency declarations. Local inspection covers Fabric mods plus Paper/Bukkit `paper-plugin.yml` and `plugin.yml` declarations and common semantic-version predicates, but it is not a complete loader resolver. Manually installed JARs that cannot be mapped back to Modrinth may still be reported as a missing managed dependency; check the local files. Minecraft's own logs remain under the server's upstream logging policy; EasyLauncher application logs and console buffers have independent limits.
- API availability and throughput depend on upstream providers. Paper, Purpur and Fabric requests use an identifying User-Agent; configure a real project contact URL before distributing a renamed/public fork.
- Windows is the tested build target. Code signing, macOS notarization, automatic updates and platform installers beyond NSIS are not included.

## Roadmap

The A–D roadmap has usable baseline implementations. Remaining extensions include historical player-log backfill, loader-specific edge-case version predicates, playit.gg account/client integration, and acceptance testing with real Minecraft and public networks. See the development notes for scope.

## Upstream references

- [Tauri commands and events](https://v2.tauri.app/develop/calling-rust/)
- [Paper downloads service](https://docs.papermc.io/misc/downloads-service/)
- [Purpur API](https://purpurmc.org/docs/API)
- [Fabric server launcher](https://fabricmc.net/use/server/)
- [Minecraft 26.1 Java requirements](https://www.minecraft.net/en-us/article/minecraft-java-edition-26-1)
- [Minecraft EULA](https://www.minecraft.net/eula)

EasyLauncher is independent software and **is not affiliated with, endorsed by, or associated with Mojang Studios or Microsoft**. Minecraft is a trademark of its respective owner.


## Advanced feature update (2026-10-19)

The scheduler, monitoring, rollback, crash recovery, mobile console, network checks and folder import are followed by configuration/JVM presets, Fabric and Modrinth dependency inspection, an optional bring-your-own-key log assistant, persistent player insights, PNG share cards, SHA-256 backup comparisons, managed frpc processes, a read-only mode, and a local audit log.

The log assistant sends only the editable excerpt displayed to the user, and only after an explicit click. API keys are never written to settings or application logs; checking “Remember configuration” stores the key encrypted with Windows DPAPI for the current account and it can be deleted again from app settings. Read-only mode is enforced by the backend and still allows stopping servers and tunnels. The audit log records action, time, server name and a short detail, never secret values, and rotates at about 1 MB. The frpc integration requires a trusted installed client and the user's own provider configuration; process status does not establish public connectivity. The opt-in E2E harness is the only path that starts a real Minecraft server; no public tunnel or paid AI service was used during this implementation.
