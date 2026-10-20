# V0.1 validation record

Environment: Windows 11, Node 24.18.0, npm 11.16.0, Rust/Cargo 1.97.1. Dependency locks are committed as project files. Validation date: 2026-10-19, Asia/Shanghai.

The user requested that Minecraft not be opened during development. Neither the Minecraft client nor a real Minecraft dedicated server was launched. No EULA was accepted for a real server.

Executed successfully during implementation:

- npm dependency installation; audit reported zero vulnerabilities.
- TypeScript checking and Vite production builds.
- Rust formatting and compile checks.
- The normal `cargo test` command: 17 tests passed and two opt-in checks were ignored. One additional, explicitly built Rust subprocess fixture check passed separately. The fixture contains no Minecraft code and is excluded from normal application builds.
- An additional read-only upstream adapter test resolved Vanilla 1.21.4, Paper 1.21.4 stable build 232, and Fabric 1.21.4 loader 0.19.5 / launcher 1.1.2. It checked release-list filtering and Java requirements without downloading server JARs.
- Tauri optimized Windows executable build and native application startup, with real system memory displayed through IPC.
- NSIS x64 installer creation, including hash-verified installer tooling downloads.
- The final Shan Shui Glass frontend production build and HTTP 200 responses for the local Vite entry point and glass stylesheet. The Vite watcher excludes Rust build output to prevent Windows executable locks.

The two opt-in checks (subprocess lifecycle and live upstream metadata) both passed when explicitly executed, for 19 successful checks across the normal and opt-in suites. The native startup inspection happened before the visual restyle. The user then stopped Computer Use with Escape; no further UI input or visual inspection was attempted. The revised visual style is build-verified, but its final native appearance was not inspected after that interruption.

The initial expanded Windows test harness exposed a missing Common Controls activation manifest. The final build.rs embeds it once via the linker for both the app and test harness, avoiding duplicate manifest resources. Sandboxed tests use a writable project temporary directory. The development URL consistently uses 127.0.0.1; the restricted environment additionally required loopback access for Vite verification.

The README's known limitations describe unverified real Minecraft behavior and platform limits. These tests do not establish successful Minecraft world generation, upstream first-run library downloads or actual player connections.


## Advanced features continuation (2026-10-19)

The latest continuation adds configuration and JVM presets, dependency inspection, an optional log assistant, player insights, PNG cards, indexed backup differences, and user-configured frpc supervision. The normal test run passes 67 library tests plus one storage integration test, with three opt-in checks ignored by default. Both process lifecycle checks were executed explicitly with the test-only fixture; the live upstream metadata check was not rerun. This gives 70 successful tests in this continuation across the normal and explicitly selected suites.

TypeScript and Vite production compilation pass. Browser component checks use an isolated, clearly labelled test-data harness under the ignored `.verification` directory. They verify rendering and interactions; Rust tests separately exercise persistent storage, request serialization, local HTTP transport and controlled process cleanup. This is not evidence of native WebView clipboard access, real player connectivity, real frp registration, or a successful external paid model request. No Minecraft process or external frp client was launched; the copied `frpc.exe` in the lifecycle test is the explicitly built Rust fixture.

The sandbox's system temporary directory caused a first full-suite attempt to fail on filesystem and loopback fixture operations; the final full suite uses `TMP` and `TEMP` under `.verification/tmp` and passes. Cargo formatting normalizes existing Rust formatting as well as the edited modules. See [development notes](DEVELOPMENT.md) for feature entry points and limits.

The final Windows optimized executable and NSIS installer build both passed. The refreshed installer is also copied to the project root as `EasyLauncher_0.1.0_x64-setup.exe`; it was not installed or launched during this continuation.


## Real Minecraft acceptance harness (2026-10-20)

An opt-in harness (`src-tauri/tests/minecraft_e2e.rs`) and a Windows workflow were added. Before the user requested that no further game launches be used for verification, two local Vanilla 1.21.4 runs completed in isolated temporary directories that were deleted afterwards: the harness installed a managed Temurin 21 JRE, downloaded the official server, confirmed the readiness line, an accepting port and the console `list` reply, stopped with exit code 0, created and restored a backup with a safety snapshot, and detected a forced crash and its automatic recovery.

After that request, the workflow was reduced to `workflow_dispatch` only (no schedule or push trigger), local default tests keep the harness ignored, no further server was launched, and both the temporary directory and all Java processes were confirmed gone. The Paper and Fabric branches are implemented but were not executed locally; they require an explicit manual invocation of the harness.


## Consistent live backups (2026-10-20)

Live backups (`create_live_backup`) pause autosave, flush, copy and resume autosave over the existing local RCON channel. The change was validated without starting Minecraft: 88 library tests and one storage integration test pass, including new checks that a stopped server and a running server with RCON disabled are rejected with the stable codes `backup.notRunning` and `backup.rconRequired`. TypeScript checking, the Vite production build and Rust formatting pass. No Java process, RCON connection or real world copy was exercised here, so the successful running-server path and the `save-on` recovery behaviour still need an explicitly requested acceptance run.


## Version upgrade wizard (2026-10-20)

The upgrade flow stages a verified `server.upgrade.jar`, swaps it with `server.jar`, keeps `server.jar.previous` plus an `upgrade.json` marker, and can roll the JAR back once. Validation stayed offline: 92 library tests and one storage integration test pass. New tests cover numeric release ordering (`1.21.10` after `1.21.9`), the JAR swap/rollback file transitions in a temporary directory, marker serialization, and rejection of unconfirmed, same-version and unconfirmed-downgrade requests before any upstream request. TypeScript checking, the Vite production build and Rust formatting pass. No server JAR was downloaded, no upstream metadata was requested by the new tests, and no Java process or Minecraft server was started. The successful download/verify/apply path and the wizard UI still need an explicitly requested acceptance run.


## Content update checks (2026-10-20)

Installed Modrinth packages can be checked for newer versions that match the server's Minecraft release and loader, with changelog, dependency impact (added, changed, removed, missing required, incompatible), pre-release and older-than-installed warnings, and per-project version pinning. A confirmed update copies the current JAR into `content-backups/`, records it in `content-rollback.json`, downloads and verifies the new file, swaps it in, and updates `content.json`; the UI offers a one-step, per-package rollback.

Validation stayed offline: 99 library tests and one storage integration test pass. New tests cover newest-release selection over newer betas, dependency-impact classification and warnings, changelog bounding, the renamed-file swap in a temporary directory, pin normalisation, and rollback-store filtering. TypeScript checking, the Vite production build and Rust formatting pass. No Modrinth request was made by these tests, no content was downloaded, and no Java process or server was started. The live check, download and rollback paths plus the new UI still need an explicitly requested acceptance run.


## Scheduled tasks 2.0 (2026-10-20)

Scheduled tasks gained five-field cron expressions with a preset editor and a backend-computed preview of the next five runs, a seven-day calendar, a single prerequisite for serial execution, a missed-run policy (skip or catch up once), and per-task run history with manual and scheduled trigger labels.

Validation stayed offline: 106 library tests and one storage integration test pass. New tests cover cron field parsing and bounds, Sunday as 0 or 7, impossible dates, preview ordering and spacing for cron and daily schedules, the two-minute missed-run threshold, prerequisite loop rejection, and dependency ordering. TypeScript checking, the Vite production build and Rust formatting pass. No scheduled task action was executed and no Java process or server was started. The calendar and history UI, DST transitions and real overnight catch-up behaviour still need an explicitly requested acceptance run.


## Server software adapters (2026-10-20)

Server installation now dispatches through a `SoftwareAdapter` trait. Vanilla, Paper, Purpur and Fabric each declare their version source, artifact verification and Java requirement, while the installer keeps one shared download, retry, length and checksum path. Vanilla and Fabric can list snapshots behind an explicit wizard toggle, and every adapter accepts releases back to 1.7 from Mojang's manifest. The wizard adds Purpur cards and hides the snapshot toggle for Paper/Purpur.

Validation stayed offline: 108 library tests and one storage integration test pass. New tests cover the 1.7 lower bound and snapshot handling in `supported_release`, and the content folder/loader declarations used by plugin, modpack, dependency and content-listing code. TypeScript checking, the Vite production build and Rust formatting pass. No upstream version API was called by the new tests, no server JAR was downloaded, and no Java process or server was started. Purpur and snapshot resolution remain in the default-ignored read-only upstream adapter test and were not executed here; the Purpur download endpoint, its MD5-only publication, Fabric snapshots and actual old-release launches still need an explicitly requested acceptance run.

Spigot, Forge, NeoForge and Quilt are intentionally not offered. Spigot has no official artifact API, and the other loaders require running official installers and a non-`-jar` launch model that this application does not yet support; the wizard and docs state this rather than shipping an untested path.


## Modpack install and update (2026-10-20)

Installed Modrinth packs now leave a `modpack.json` record of managed files and server overrides. A preview downloads and verifies the newest compatible `.mrpack` into `modpack-pending.mrpack`, then computes a hash-based added/changed/removed/unchanged diff and a count of changed overrides; nothing else in the server folder changes. A confirmed update backs up every replaced or removed file under `modpack-backups/`, downloads only changed managed files, reuses local files whose hashes match, restores the backup automatically if a write fails, and writes a one-step `modpack-rollback.json`. The installed-content page shows the pack card, changelog, file diff and rollback action. Installing a different pack or version is refused while a record exists: one modpack per server.

Validation stayed offline: 113 library tests and one storage integration test pass. New tests build synthetic `.mrpack` archives to cover server-side env filtering, protected paths, managed/override collisions, `server-overrides` precedence, loader-tag candidate filtering, hash-based diff classification, strict local hash matching, and file/record restoration from a backup. TypeScript checking, the Vite production build and Rust formatting pass. No Modrinth request was made by these tests, no pack was downloaded, and no Java process or server was started. The live check/download/apply path, rollback after a real failure, and the new UI still need an explicitly requested acceptance run. CurseForge packs are not implemented because their download API requires a key; this is stated in the UI documentation rather than shipping an untested path.


## Compressed, deduplicated backups and retention (2026-10-20)

New backups are no longer directory copies. Each world file is read in 1 MiB chunks; every chunk is hashed after decompression, compressed with zstd and stored once under `backups/chunks/<prefix>/<sha256>.zst`. `manifest.json` records paths, file hashes and chunk references, while `files.json` remains the file-level index used by the existing comparison view. Restore verifies each chunk's length and SHA-256 before staging world folders, so a damaged chunk aborts the restore with `backup.corrupt` instead of installing partial data. Deleting a backup and retention cleanup both garbage-collect unreferenced chunks; legacy directory snapshots remain readable, restorable and comparable and are not converted automatically.

Retention is opt-in and defaults to 7 daily, 4 weekly and 3 monthly groups. The preview lists every candidate and an estimated reclaimable size; confirmation is required to delete. Safety snapshots, labelled snapshots, snapshots with unreadable dates and the newest backup are always kept. When enabled, cleanup also runs after a successful scheduled backup.

Validation stayed offline: 119 library tests and one storage integration test pass. New tests cover chunked round-trip, cross-backup chunk sharing, incremental growth after one changed file, rejection of a tampered chunk, garbage collection of unreferenced chunks, manifest path traversal, and daily/weekly/monthly selection including protected backups. The manager integration test now verifies a safety snapshot by restoring it from its manifest. TypeScript checking, the Vite production build and Rust formatting pass. No server was started, no Minecraft process or RCON connection was opened, and no network request was made. Real-world compression ratios, large-world restore time, interrupted-write recovery and the new retention UI still need an explicitly requested acceptance run.


## Remote backup targets (2026-10-20)

The backup page can now upload a completed snapshot to WebDAV, S3-compatible storage or a destination exposed by a local rclone process. A chunked backup is packaged into one ZIP containing `backup.json`, `manifest.json`, `files.json` when present and every referenced chunk; legacy directory snapshots are packaged from their tree. The archive can optionally be encrypted with an age passphrase before upload. Passwords, secret keys and passphrases go through the Windows DPAPI vault, while the JSON configuration stores only non-secret fields (endpoint, bucket, region, username or access key ID) and the audit log records no credentials.

S3 uploads use SigV4 signing, path-style or virtual-host URLs, a single PUT below 64 MiB and multipart uploads above it. Completed part numbers and ETags are persisted so a retry can list remote parts and continue. WebDAV creates the target collection with MKCOL, streams to a `.part` object, moves it into place and removes the temporary object on failure; because WebDAV has no standard resume protocol, an interrupted transfer restarts. rclone is invoked as a local process with argument arrays, checksum copying, remote size checks and optional re-download hashing. All backends retry transient failures and, after upload, verify the remote size; when content verification is enabled the archive is downloaded again and its SHA-256 compared.

Validation stayed offline: 128 library tests and one storage integration test pass. The nine new remote tests cover archive construction and chunk references, object naming, configuration validation, SigV4 against the published AWS example, S3 URL styles, multipart completion XML, age encryption round-trip, WebDAV URL joining and staging-name restrictions. Rust formatting, all-target compilation, TypeScript checking and the Vite production build pass. No WebDAV/S3/rclone server was contacted, no file was uploaded or downloaded, no credential was stored, no Minecraft process or RCON connection was opened, and no game launch was used for verification.

Remote restore is not implemented: the page uploads and verifies archives, but cannot download and install one back into the local backup store yet. WebDAV failures restart rather than resume, rclone's behaviour depends on its own configuration and backend, and an HTTP endpoint (rather than HTTPS) sends credentials and backup data in clear text. Real WebDAV/S3 servers, MinIO, S3 multipart resume after a real interruption, rclone with an actual remote, large-archive transfer time and the new UI still need an explicitly requested acceptance run.


## World preview and save health (2026-10-20)

The backup page now has a read-only world inspector. It parses Anvil region headers, decompresses gzip/zlib/uncompressed chunk payloads (LZ4 is reported as unsupported), reads the NBT subset needed for coordinates, sections, block entities and entities, and renders a 2D top-block map per region at 1/2/4/8-block detail. It also lists playerdata positions, samples online positions over local RCON into `world-tracks.json`, counts entity and block-entity types, reports header/compression/NBT damage, and compares two snapshots by region-file SHA-256. Chunked backups are reconstructed through the existing verified `backup_store::read_file`; legacy directory snapshots are read directly. No world or backup file is written, and the new commands are read-only apart from the explicitly requested track sample.

Validation stayed offline: 136 library tests and one storage integration test pass. Eight new tests build synthetic NBT, zlib-compressed region files, gzip playerdata and a chunked backup to cover compound/list/array parsing, section palette rendering, entity and block-entity counts, corruption detection, playerdata coordinates, region-path dimension mapping, RCON position/dimension parsing, region-level diff classification and verified reconstruction from shared chunks. TypeScript checking, the Vite production build, `cargo fmt --check` and all-target compilation pass. No Minecraft process was started, no RCON connection was opened, no game was launched and no network request was made.

The tests use synthetic region files rather than worlds produced by real Minecraft versions. Custom dimensions, modded blocks, old pre-1.16 section packing, LZ4 chunks, very large worlds and the map canvas in a real webview still need an explicitly requested acceptance run. Map colours are approximations, and player markers reflect backup-time playerdata rather than live positions.


## Crash attribution 2.0 (2026-10-20)

`CrashAnalyzer` now has a deterministic signature library in `server/crash.rs`. It recognizes JVM-option, Java-version, out-of-memory, port, disk, file-permission, missing-JAR, EULA, chunk/region corruption, mod-conflict, missing-dependency, plugin-load and watchdog signatures. Findings are ordered with the first actionable root cause first: critical causes are taken in log order, warnings are considered next, and watchdog or generic errors are marked as symptoms and never replace a specific cause. Each finding records its first line, occurrence count and extracted context such as the conflicting port, class-file/required-Java version, mod names, plugin name, JVM option, chunk position and configured heap. The ops page shows a root-cause badge, symptoms, these values and a one-click sanitized Markdown report that is generated locally in memory only.

Validation stayed offline: 141 library tests and one storage integration test pass. Eight new tests cover port conflicts with later errors, watchdog suppression, earliest-critical selection, class-file/Java mapping, extracted mod/plugin/chunk values, exit-code-only fallback, trait delegation, English and Chinese report headings, and redaction of user paths, drive paths, IP addresses and credential lines. `cargo fmt --check`, all-target compilation, TypeScript checking and the Vite production build pass. No Minecraft process was started, no RCON connection was opened and no game launch was used.

The signature tests use synthetic log lines and do not establish coverage of every mod loader, crash handler or localized server message. Reports redact common addresses, URLs, user paths and credential-looking lines, but they are not a guarantee of anonymity; the UI still asks for a manual review before sharing. AI log analysis remains a separate, explicitly triggered feature and was not contacted.
