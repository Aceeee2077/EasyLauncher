# EasyLauncher

[简体中文](README.md) | [English](README.en.md)

基于 **Tauri 2、Rust、React、TypeScript、Vite 与 Tailwind CSS** 构建的 Minecraft Java 版本地开服管理器。Java 作为受控子进程运行，前端不具备文件系统或进程权限。

界面采用 **Shan Shui Glass** 风格：磨砂质感图层、石青、石绿、绢白与沉金，取自《千里江山图》的配色。背景为随应用本地打包的公版画作复制品，不包含任何 Minecraft 素材，也不会请求外部图片。界面支持中文（默认）、英文与跟随系统语言。品牌信息集中在 `src/services/api.ts`、`src-tauri/tauri.conf.json` 与应用图标中。

## 当前功能

- 四步建服向导，实时拉取上游版本列表，并提供内存建议、端口、玩家上限、游戏模式与难度设置。
- 原版（Vanilla）通过 Mojang 清单获取，稳定版 Paper 通过 v3 下载服务获取，Fabric 可执行启动器通过 Fabric Meta 获取。绝不静默替换用户请求的 Minecraft 版本。
- 支持从 PATH、JAVA_HOME、JDK_HOME 与常见安装目录发现 Java；支持校验自定义可执行文件并引导安装。兼容性要求同时参考 Mojang 各版本的 Java 元数据。
- 托管 Java 运行时：从 Adoptium（Temurin）下载，使用 SHA-256 校验，安全解压，并按服务器固定。缺少兼容运行时时，建服向导提供一键安装。
- 显式勾选 EULA 并持久化接受时间。安装完成后绝不自动启动 Minecraft。
- 持久化服务器元数据；展示真实进程状态、PID、基于服务器输出的就绪判定、退出码与崩溃处理。
- 启动、优雅停止、重启、重复启动保护与关闭清理。若 EasyLauncher 异常退出，Windows Job Object 会终止受控的 Java 子进程。
- 实时 stdout/stderr 控制台，最多保留 1,500 行历史，支持 Minecraft stdin 命令、命令历史、跟随输出开关与清空。
- 常用与原始 `server.properties` 编辑器，保留未知键、注释、转义值与 Unicode。运行时与内存调整按服务器独立保存。
- 真实的 OP、取消 OP、踢出、封禁、解封与白名单命令，并校验玩家名。在线玩家根据加入/离开日志观察。
- 已停止服务器的手动世界备份、列表、确认恢复/删除、自动安全快照与恢复回滚。
- 本地端口冲突检查、本地/局域网地址、受控模组/插件目录与 JAR 列表。
- 浅色/深色/跟随系统外观，中文/英文/跟随系统语言，全局 Java 偏好，受控文件夹快捷入口，带技术详情的友好错误提示，以及独立轮转的应用日志。

## 进阶功能（2026-10-19）

- 定时任务、实时资源告警、世界回滚与崩溃重启；手机 Web 控制台、UPnP 端口映射、端口与 Windows 防火墙检查，以及现有服务器目录导入。
- **配置与 JVM 预设**：小型、均衡、流畅配置建议；均衡、Aikar、Paper 调优变体和受校验的自定义 `-XX:` 参数；可填入当前系统的内存建议，保存后下次启动生效。
- **依赖检查**：在内容页读取 Fabric 顶层和嵌套 JAR 的 `fabric.mod.json`，显示重复 ID、缺失依赖、版本问题、客户端模组和显式冲突。手动点击后查询 Modrinth 托管项目的必需、可选、内嵌和不兼容声明。
- **日志助手**：本地规则包含内存、Java、JVM 参数、端口、磁盘、存档和模组错误。可选连接用户自备的 Chat Completions 兼容接口：先预览和编辑片段，再显式发送；Key 不保存，请求结束后清空。较新的进程输出不会被旧崩溃报告覆盖。
- **玩家洞察**：玩家页展示累计玩家、登录次数、在线时长、14 天登录图和会话记录。仅记录应用观察到的加入与离开事件；统计持久化保存在服务器目录，进程退出时结算。
- **分享卡片**：网络页提供本地 Canvas PNG 卡片，支持编辑邀请语和连接地址、保存、复制图片与打开保存目录。卡片不包含 Web 控制台令牌。
- **备份差异**：世界快照保存 SHA-256 文件索引；比较两个时间点，汇总新增、删除、修改和未变文件，并按目录展开差异。旧备份首次比较时补建索引。
- **frp 托管**：网络页填写已安装的官方 `frpc` 路径、独立配置文件路径和公网地址，即可启动或停止受控进程。关闭应用时清理进程；不自动下载客户端、开通服务、填写凭据或验证公网连通。

各项入口、边界与验证记录见 [开发进度](docs/DEVELOPMENT.md)。

## 截图

截图占位：`docs/screenshots/overview.png` —— 原生 Shan Shui Glass 主界面。

截图占位：`docs/screenshots/console.png` —— 用户创建的服务器启动后的实时控制台。

开发过程中刻意不生成包含真实 Minecraft 活动的截图。验证期间未启动任何 Minecraft 客户端或专用服务器。

## 环境要求

- Node.js **22.12+**（验证使用 Node 24）、npm 与稳定版 Rust。锁定的依赖版本请使用较新的 Rust 工具链。
- Windows：安装 Microsoft C++ Build Tools（含“使用 C++ 的桌面开发”）、Windows SDK 与 Microsoft Edge WebView2 Runtime。
- Linux/macOS：参考对应平台的 [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/)。Rust 代码包含平台分支处理，但仅验证过 Windows 构建。
- **准备创建的服务器**需要匹配的 Java 运行时。仅构建应用本身不需要 Java。
- 解析版本、安装与首次启动加载库时需要访问 Mojang、PaperMC 与 Fabric 的网络。

## 安装与运行

在项目目录中执行：

```powershell
npm install
npm run tauri dev
```

如果 PowerShell 执行策略阻止 `npm.ps1`，请改用 `npm.cmd`。

单独运行 `npm run dev` 会在 `http://127.0.0.1:1420` 启动前端。普通浏览器会明确提示桌面后端不可用，绝不会用模拟服务器或伪造的成功状态替代。

使用 EasyLauncher 的流程：创建服务器，选择版本与软件，分配内存，选择兼容的 Java 运行时（或直接在向导中安装），阅读并接受 EULA，等待安装完成，然后点击 **启动服务器**。只有在 Minecraft 输出 `Done (...)! For help, type ...` 之后才会判定为就绪。诸如 `say Hello` 的命令会写入该 Java 进程的 stdin。编辑属性或创建/恢复世界备份前，请先停止服务器。

## 构建

```powershell
npm run build
npm run tauri build -- --no-bundle
# Windows NSIS 安装包（首次运行可能下载 NSIS 工具）：
npm run tauri build -- --bundles nsis
```

Windows 可执行文件位于 `src-tauri/target/release/easylauncher.exe`；NSIS 安装包位于 `src-tauri/target/release/bundle/nsis/`。当前配置仅面向 Windows 安装包；在其他系统上请覆盖该平台的打包目标。安装包未签名。

## 验证

```powershell
npm run typecheck
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

额外的进程集成测试使用一个**仅用于测试、不含任何 Minecraft 代码的小型 Rust 子进程**：

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --features test-fixtures --bin easylauncher-process-fixture
$env:EASYLAUNCHER_TEST_PROCESS = (Resolve-Path src-tauri/target/debug/easylauncher-process-fixture.exe).Path
cargo test --manifest-path src-tauri/Cargo.toml --lib subprocess_lifecycle_without_minecraft -- --ignored
```

对三个官方元数据适配器的只读验证，不下载也不运行服务器 JAR：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib upstream_adapter_resolution -- --ignored --nocapture
```

测试覆盖 Properties 的解析/修改/序列化/未知键/Unicode、Java 输出与版本边界、UUID 路径穿越防护、元数据默认值与往返、状态迁移、端口冲突、校验和拒绝与残留文件清理、备份恢复与安全副本，以及重新打开已持久化的服务器。子进程测试覆盖重复启动、stdout/stderr、stdin、重启、崩溃退出码与优雅关闭。

如果沙箱限制了系统临时目录，请在运行测试前将 `TMP` 与 `TEMP` 指向项目内可写目录。这属于环境设置，不是应用存储路径的覆盖。

已执行的检查以及“夹具验证”与“真实 Minecraft 测试”的区别见 [docs/VALIDATION.md](docs/VALIDATION.md)。

## 架构

```text
src/
  components/          共享控件、对话框、状态与错误展示
  pages/               向导、控制台、属性、玩家、备份与应用设置
  services/            类型化 Tauri 命令契约、品牌与外观
  hooks/               任务去重、挂起状态与错误处理
  types/               前端数据模型
  assets/              本地打包的公版山水背景
  styles.css           布局与可复用组件结构
  glass.css            Shan Shui Glass 配色与光学表面
src-tauri/src/
  commands.rs          类型化 IPC 命令；校验命令与文件夹意图
  models.rs            元数据、显式状态枚举与事件载荷
  storage.rs           UUID/路径/链接检查、原子 JSON 与世界复制
  java/                有边界的运行时检测与集中的兼容性规则
  minecraft/           上游版本元数据与 Java Properties 编解码
  server/manager.rs    持久化、按服务器的操作锁与事件/历史状态
  server/installer.rs  软件适配器、重试、哈希、进度与取消
  server/process.rs    Java 生命周期、管道读取、stdin 与就绪/崩溃观察
  server/job.rs        Windows 关闭即终止的子进程托管
  backup.rs            一致性快照、恢复暂存与回滚
  backup_diff.rs       文件哈希索引与快照差异
  dependencies.rs      Fabric JAR 与 Modrinth 依赖检查
  player_stats.rs      持久化玩家时长、会话与日统计
  log_assistant.rs     可选自带 Key 的日志片段分析
  tunnel.rs            用户配置的 frpc 子进程托管
  network.rs           本地绑定检查与局域网地址探测
  logger.rs            独立的时间戳日志；2 MB 轮转并保留两个文件
  extensions.rs        未来内容提供方与崩溃分析的契约
```

React 调用 Rust 命令，并订阅 `server-status`、`server-console`、`download-progress`、`backup-progress`、`app-closing` 与 `app-error` 事件。以 Rust 为权威来源，不使用生命周期布尔值或前端计时器伪造服务器状态。备份、生命周期变更与安装共享同一把按服务器的操作互斥锁。进程监控在等待退出或读取流时不持有子进程锁。

## 存储

根目录来自 Tauri 应用数据目录（`app_data_dir`），没有硬编码的用户名或开发路径。设置页会显示实际路径；Windows 上通常解析为 `%APPDATA%/dev.easylauncher.desktop`。

```text
<应用数据>/
  settings.json
  java/<uuid>/
    runtime/             # 托管的 Adoptium 运行时（java.exe / bin/java）
    minehost-java.json   # 内部清单；文件名出于兼容性保留
  logs/minehost.log      # 与 Minecraft 日志分离；2 MB 轮转
  servers/<uuid>/
    minehost.json        # 带版本的元数据；运行状态永不持久化
    server.jar
    server.properties
    eula.txt
    world/               # Vanilla / Fabric 的维度存档
    world_nether/        # 生成后的 Paper 独立维度
    world_the_end/
    mods/
    plugins/
    logs/latest.log
    backups/<uuid>/
      backup.json
      world/
      ...
```

备份仅包含世界数据，不包含服务器配置、模组或插件。`files.json` 保存创建时的文件哈希；请勿在受控目录中手动修改快照内容，否则索引不再代表实际文件。备份为未压缩的目录快照。如需防范磁盘损坏或误删服务器，请将副本保存到受控存储之外。删除受控服务器的对话框会明确说明同时删除其本地备份。

## 安全与运行行为

- 接受 EULA 是用户操作；EasyLauncher 绝不在首次启动时于后台下载并启动服务器。
- 命令通过进程参数传递，绝不拼接操作系统 shell 字符串。控制台输入限制为单行，且只写入 Minecraft 的 stdin。
- 启动与重启会重新校验 Java、内存、EULA、受控路径与本地端口可用性。内存分配至少为系统保留 1 GB 且不低于总内存的 20%，并额外检查当前可用内存。
- 停止会发送 `stop`，最多等待 45 秒，随后在强制终止前于控制台给出警告。正常关闭应用会等待关闭完成；重复的关闭请求不会绕过该等待。
- 会校验受控 UUID、世界名称、符号链接、Windows 重解析点、保留名称、链接文件夹与不安全的原始属性。V0.1 中远程 RCON/query/管理端点处于禁用状态。
- 恢复要求服务器已停止并显式确认。恢复会先保存现有世界、暂存新数据，再重命名文件夹。回滚失败会将恢复数据保留在 `restore-*` 文件夹中；在问题解决前，启动会拒绝该服务器；请先使用其安全备份与保留文件，再移除该文件夹。
- 无效元数据会被跳过并记录日志，而不是导致目录崩溃。外观配置损坏可通过重新保存偏好修复。关闭再打开时绝不盲目信任持久化的 Running 标记。

## 已知限制

- **按用户要求，验证期间未启动任何 Minecraft 客户端或专用服务器。** 已验证编译、桌面启动、真实上游元数据、文件操作与仅用于测试的子进程。在用户显式接受 EULA 后，完整的真实服务器启动与库引导路径仍需用户自行验收。
- 仅列出稳定的 Minecraft **1.16 及更高版本**；快照与旧版启动格式不在当前范围内。目录导入支持可识别的 Vanilla、Paper 与 Fabric 服务器。Paper 需要所选版本存在上游 `STABLE` 构建；不支持的选择会给出明确错误。
- Java 选择要求主版本兼容。托管的 Adoptium 安装按服务器固定，可在设置中移除。Minecraft 26.x 的要求遵循上游元数据，并以 Java 25 作为回退；版本规则集中在一个模块中。
- Fabric 提供的启动器没有上游产物哈希。EasyLauncher 会校验 HTTPS 传输、JAR 签名与上报长度；Vanilla 使用 SHA-1，Paper 使用 SHA-256。Fabric 与 Paper 可能在服务器首次启动时下载库文件；这些上游 Java 引导下载不会向 EasyLauncher 报告逐字节进度。
- 备份要求服务器已停止。可使用定时任务备份，但实时保存同步、压缩与远程副本暂未实现。若 `level-name` 与原值不同，恢复会被拒绝，直到配置回原名称。
- 在线玩家观察依赖上游英文的加入/离开日志，插件可能改变这些消息。玩家洞察只累计应用实际观察到的事件，不回填历史日志，不等同于 RCON 的权威玩家目录。
- 正常关闭会在所有支持的平台上优雅停止子进程。Windows 另外使用 Job Object 处理异常退出清理。Linux/macOS 上父进程被强制终止时的清理尚未保证；在这些平台重新启动前请检查残留的 Java 进程。
- 局域网地址探测仅作为路由提示；VPN 与多网卡会影响结果。本地或局域网可用不代表公网可访问。用户可显式触发 UPnP 映射或 Windows 防火墙放行；frp 则需要用户自己的服务与配置。
- 模组/插件支持 Modrinth 内容管理与依赖声明检查；本地 JAR 深入检查目前仅支持 Fabric。Minecraft 自身日志仍遵循服务器的上游日志策略；EasyLauncher 应用日志与控制台缓冲区各有独立上限。
- API 可用性与吞吐取决于上游服务。Paper 请求会携带标识性 User-Agent；若分发更名后的公开分支，请配置真实项目联系地址。
- Windows 是已验证的构建目标。不包含代码签名、macOS 公证、自动更新以及 NSIS 之外的其他平台安装包。

## 路线图

当前计划中的 A—D 组已具备可使用的基础实现，具体范围见开发进度。后续仍可扩展历史日志回填、Paper 本地插件元数据、完整 Fabric 版本谓词、playit.gg 账号与客户端集成，以及真实 Minecraft 与公网环境的验收。

## 上游参考

- [Tauri 命令与事件](https://v2.tauri.app/develop/calling-rust/)
- [Paper 下载服务](https://docs.papermc.io/misc/downloads-service/)
- [Fabric 服务器启动器](https://fabricmc.net/use/server/)
- [Minecraft 26.1 Java 要求](https://www.minecraft.net/en-us/article/minecraft-java-edition-26-1)
- [Minecraft EULA](https://www.minecraft.net/eula)

EasyLauncher 为独立软件，**与 Mojang Studios 或 Microsoft 无任何隶属、背书或关联关系**。Minecraft 商标归其各自所有者所有。
