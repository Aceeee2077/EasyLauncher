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
