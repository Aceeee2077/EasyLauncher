# EasyLauncher

[简体中文](README.md) | [English](README.en.md)

基于 **Tauri 2、Rust、React、TypeScript、Vite 与 Tailwind CSS** 构建的 Minecraft Java 版本地开服管理器。Java 作为受控子进程运行，前端不具备文件系统或进程权限。

界面采用 **Shan Shui Glass** 风格：磨砂质感图层、石青、石绿、绢白与沉金，取自《千里江山图》的配色。默认背景为随应用本地打包的公版画作复制品，不包含任何 Minecraft 素材，也不会请求外部图片；你也可以在设置里换成自己的图片，图片会复制到应用数据目录并通过本地资源协议读取。界面支持中文（默认）、英文与跟随系统语言。品牌信息集中在 `src/services/api.ts`、`src-tauri/tauri.conf.json` 与应用图标中。

## 当前功能

- **0.2.3 日志洞察工作台**：服务器详情 → 运维中可浏览普通/压缩历史日志，独立提取玩家加入、离开、常见死亡、进度、聊天、警告与启停事件，按玩家、类型、关键词与小时筛选，保留原始行号，标记并导出当前筛选结果。按 CS2 Insight Agent 的公开功能说明进行适配，未复用参考源码；完整对标范围、读取上限及未实现的录制/剪辑能力见 [功能对标说明](docs/INSIGHT_BENCHMARK.md)。

- 四步建服向导，实时拉取上游版本列表，并提供内存建议、端口、玩家上限、游戏模式与难度设置。
- 原版（Vanilla）通过 Mojang 清单获取，稳定版 Paper 通过 v3 下载服务获取，Purpur 通过其 v2 API 获取，Fabric 可执行启动器通过 Fabric Meta 获取；各软件通过独立的 adapter 声明元数据来源、校验方式与 Java 要求。原版与 Fabric 还可选择快照版，且支持 1.7 起的旧正式版。绝不静默替换用户请求的 Minecraft 版本。
- 支持从 PATH、JAVA_HOME、JDK_HOME 与常见安装目录发现 Java；支持校验自定义可执行文件并引导安装。兼容性要求同时参考 Mojang 各版本的 Java 元数据。
- 托管 Java 运行时：从 Adoptium（Temurin）下载，使用 SHA-256 校验，安全解压，并按服务器固定。缺少兼容运行时时，建服向导提供一键安装。
- 显式勾选 EULA 并持久化接受时间。安装完成后绝不自动启动 Minecraft。
- 持久化服务器元数据；展示真实进程状态、PID、基于服务器输出的就绪判定、退出码与崩溃处理。
- 启动、优雅停止、重启、重复启动保护与关闭清理。若 EasyLauncher 异常退出，Windows Job Object 会终止受控的 Java 子进程。
- 实时 stdout/stderr 控制台，最多保留 1,500 行历史，支持 Minecraft stdin 命令、命令历史、跟随输出开关与清空。
- 常用与原始 `server.properties` 编辑器，保留未知键、注释、转义值与 Unicode。运行时与内存调整按服务器独立保存。
- 真实的 OP、取消 OP、踢出、封禁、解封与白名单命令，并校验玩家名。在线玩家根据加入/离开日志观察。
- 已停止服务器的手动世界备份；启用 RCON 且服务器就绪时，可在不停止服务器的情况下创建在线一致性备份。备份按 1 MiB 分块、zstd 压缩并以 SHA-256 内容寻址去重，可配置每日/每周/每月保留策略，先预览再清理。另有备份列表、确认恢复/删除、自动安全快照与恢复回滚。可把归档上传到 WebDAV、S3 兼容存储或经本机 rclone 转发的远程，支持 age 口令加密、上传后大小/ SHA-256 校验与失败重试，S3 分片可断点续传；凭据保存在 Windows DPAPI。备份页还能只读解析 Anvil MCA/NBT：渲染 2D 区域地图、显示玩家坐标与在线轨迹、统计实体与方块实体、检测损坏区块，并按区域比较两次备份。
- 版本升级向导：为已安装且已停止的服务器选择受支持的稳定版本；升级前自动创建世界安全备份，下载并校验新的服务端 JAR，成功后保留上一份 JAR 以便单步回退。降级需显式确认，模组与插件兼容性只提示、不自动处理。
- 内容更新检查：对由应用安装的模组/插件查询 Modrinth 上适配当前 Minecraft 版本的新版本，展示更新日志、依赖增减与不兼容提示，可钉住版本跳过检查。确认后先保存旧 JAR，再下载校验并替换；支持单包回退，不会改动世界与配置。
- 整合包安装与更新：解析 Modrinth `.mrpack` 的 manifest、`overrides/` 与 `server-overrides/`，过滤客户端专用文件、受保护路径与超限文件，并在安装后记录 `modpack.json`。检查更新会下载并校验候选包，按哈希列出新增、修改、移除与未变文件，展示更新日志；确认后只下载变化的文件，先备份将被替换的文件，失败时自动恢复，并保留一次回退。每台服务器只管理一个整合包。
- 本地端口冲突检查、本地/局域网地址、受控模组/插件目录与 JAR 列表。
- 浅色/深色/跟随系统外观，玻璃透明度与 0–40 px 背景模糊，可自定义背景图片（PNG/JPG/WEBP/BMP，复制到应用数据目录，可一键恢复内置山水画），中文/英文/跟随系统语言，全局 Java 偏好，受控文件夹快捷入口，带技术详情的友好错误提示，以及独立轮转的应用日志。

## 进阶功能（2026-10-19）

- 定时任务、实时资源告警、世界回滚与崩溃重启；手机 Web 控制台、UPnP 端口映射、端口与 Windows 防火墙检查，以及现有服务器目录导入。
- **配置与 JVM 预设**：小型、均衡、流畅配置建议；均衡、Aikar、Paper 调优变体和受校验的自定义 `-XX:` 参数；可填入当前系统的内存建议，保存后下次启动生效。
- **依赖检查**：在内容页读取 Fabric 顶层和嵌套 JAR 的 `fabric.mod.json`，以及 Paper/Bukkit 插件的 `paper-plugin.yml`、`plugin.yml`，显示重复 ID、缺失依赖、版本问题、客户端模组、插件 API 版本和显式冲突。手动点击后查询 Modrinth 托管项目的必需、可选、内嵌和不兼容声明，并按 SHA-1 把手动安装的 JAR 映射回 Modrinth 项目。
- **日志助手**：本地规则包含内存、Java、JVM 参数、端口、磁盘、存档和模组错误。可选连接用户自备的 Chat Completions 兼容接口：先预览和编辑片段，再显式发送；默认不保存 Key，勾选“记住配置”后才用 Windows DPAPI 加密保存在本机密钥库，并可在设置中查看与删除。较新的进程输出不会被旧崩溃报告覆盖。
- **安全与审计**：应用设置新增只读模式，后端会拒绝创建、修改、删除与启动等操作，仍允许停止服务器和隧道；破坏性或改变状态的操作（删除、恢复、强制终止、RCON 命令、密钥写入等）记录到本机审计日志，超过约 1 MB 自动轮转两份。审计只保存动作、时间、服务器名与简短说明，不包含密钥内容。
- **玩家洞察**：玩家页展示累计玩家、登录次数、在线时长、14 天登录图和会话记录。仅记录应用观察到的加入与离开事件；统计持久化保存在服务器目录，进程退出时结算。
- **分享卡片**：网络页提供本地 Canvas PNG 卡片，支持编辑邀请语和连接地址、保存、复制图片与打开保存目录。卡片不包含 Web 控制台令牌。
- **备份差异**：世界快照保存每个文件与数据块的 SHA-256 清单；比较两个时间点，汇总新增、删除、修改和未变文件，并按目录展开差异。旧目录快照首次比较时补建索引，分块备份可从 manifest 重建。
- **frp 托管**：网络页填写已安装的官方 `frpc` 路径、独立配置文件路径和公网地址，即可启动或停止受控进程。关闭应用时清理进程；不自动下载客户端、开通服务、填写凭据或验证公网连通。
- **樱花 FRP 自动穿透**：在“联机开服”中粘贴一次访问密钥并选择线路，之后随开服自动创建或更新 TCP 隧道、验证公网 Minecraft 响应，关服自动断开。密钥用 Windows DPAPI 加密保存，frpc 隐藏运行，隧道地址自动复用；修改服务器端口后会同步更新。公开宣传的游戏服务器必须使用“多线”专用节点，界面会明确提示。详见 [docs/SAKURA.md](docs/SAKURA.md)。
- **公网联机统一入口**：网络页为每台服务器选择一次联机方式（仅局域网 / 樱花 FRP / playit.gg / 自定义 FRP）与协议（Java TCP / 基岩 UDP），设置保存在 `public-access.json`。隧道会等待本地端口就绪后随开服建立、关服自动断开；服务器主页显示当前阶段、公网地址、复制按钮和错误信息。详见 [docs/PUBLIC_ACCESS.md](docs/PUBLIC_ACCESS.md)。
- **开服体检**：运维页汇总安装、EULA、Java、内存、JVM 参数、端口、防火墙、路由器映射、配置、磁盘、备份与日志检查，给出 0–100 就绪评分；问题优先展示，通过项可折叠。只读检查，不自动修改防火墙或路由器。
- **多服务器总览**：总览页统计全部服务器的运行与待处理数量，勾选后可批量启动、停止或重启；只对当前状态允许的服务器执行，逐台列出失败原因。
- **启动前门禁**：启动前复用开服体检，未接受 EULA、缺少文件等严重问题会在弹窗中列出；确认风险后仍可强制启动。
- **定时公告与维护窗口**：定时任务新增游戏内公告与维护窗口。公告通过 `say` 单行广播；维护窗口先广播自定义内容，再停服并创建安全备份。
- **计划任务 2.0**：定时任务支持五段 cron 表达式，提供常用模板、五个字段的可视化编辑和接下来 5 次执行的校准预览；另有未来 7 天任务日历、单前置任务的串行执行、错过执行策略（跳过或启动后补跑一次），以及每个任务最近 200 条执行历史。手动触发与计划触发都会记录结果、触发方式和错误详情。
- **配置模板**：服务器设置内置生存、创造与性能三套模板。应用前先展示逐项差异；只写入玩法与性能白名单键，保留注释、端口、MOTD 和其他配置。
- **命令面板**：按 `Ctrl/Cmd+K` 或点击顶栏搜索框，搜索服务器、页面与动作。支持方向键选择、Enter 执行、Esc 关闭；输入关键字后可直接跳到某台服务器的控制台、设置、备份等分页。
- **通知中心**：顶栏铃铛汇总服务器就绪、异常退出、备份完成与定时任务失败。未读数量、清空与静音开关都在下拉面板内，点击条目直接打开对应服务器的控制台。
- **在线玩家面板**：总览页展开运行中服务器的在线列表，可直接踢出、设为 OP 或发送私聊。动作复用现有玩家命令与控制台 `tell`，玩家名沿用后端校验。
- **本地 RCON（可选）**：在服务器设置中显式启用并生成强密码，重启后用 RCON 拉取权威在线玩家列表；连接固定发往 `127.0.0.1`。启用会让 Minecraft 在所有网络接口监听 RCON 端口，界面会给出提示。
- **磁盘空间与清理**：运维页按世界、备份、日志、模组等分类统计服务器目录占用，并显示所在磁盘可用空间。可勾选清理历史日志（保留 `latest.log`）、崩溃报告和中断残留，确认后显示实际释放量；世界、备份与配置不会被清理。
- **世界预览与存档健康**：备份页只读解析 Anvil MCA/NBT，按区域渲染 2D 地图并叠加玩家位置与本地在线轨迹；统计区块、实体、方块实体及类型，列出区域头、区块压缩与 NBT 损坏，并按区域文件 SHA-256 比较两次备份。地图可选 1/2/4/8 格精度；存档与备份不会被修改。
- **崩溃归因 2.0**：运维页通过 `CrashAnalyzer` 契约与确定性签名库识别内存不足、模组冲突或缺依赖、区块损坏、Java 版本、JVM 参数、端口占用、磁盘与文件权限等启动失败。按日志首次出现顺序定位“第一个根因”，后续错误与看门狗转储作为关联症状列出；展示命中行号、次数、端口、所需 Java、相关模组或插件等细节，并可一键生成不落盘的脱敏 Markdown 报告。报告会遮挡路径、URL、IP 地址与含凭据的行。
- **性能剖析**：监控页可选用本地 RCON 采样 TPS 与 MSPT（`tick query`、Paper `tps`，装有 Spark 时回退到 `spark tps`），按 5–60 秒间隔把最多 720 个样本保存在服务器文件夹中，并绘制 TPS、MSPT 与进程内存趋势。JFR 页面通过 Java 同目录的 `jcmd` 启动本地录制，文件只写入服务器的 `profiles/`，可手动停止或删除；还可读取一次 `GC.heap_info` 堆快照。托管下载的是 JRE，不含 `jcmd`，界面会明确提示改选完整 JDK。所有采样与录制都需要显式开启，不会启动游戏，也没有网络上传。
- **多服务器资源调度**：总览页按内存上限统计运行中、已停止与预计总占用，以系统内存的 90% 作为启动预算，并提前检查端口重复或被其他程序占用。勾选服务器后可预览启动顺序、跳过原因与错峰时间，再按队列依次启动：每台之间默认间隔 10 秒，启动前重新检查可用内存，等待就绪超时只记录提示而不终止进程。取消队列只阻止尚未启动的服务器；端口问题可一键改用建议端口，写入对应服务器的 `server.properties`。
- **Linux / macOS 生产服务**：运维页可为已安装且已停止的服务器预览 systemd 用户服务或 launchd LaunchAgent 的定义、日志路径与启用命令，确认后只把服务文件写入当前用户的服务目录。应用不会启用、启动或停止系统服务；服务托管后，控制台、体检、计划任务与备份协调不再可见，需要先在终端停用服务，再回到应用内启动。Windows 继续依靠应用的任务对象在退出时清理子进程，不提供此功能。
- **导出与跨机器迁移**：服务器设置的“导出与迁移”可把世界、配置、模组、插件与受管内容打包为 ZIP，默认不含日志、计划任务、统计、隧道、回滚记录和备份（备份可显式包含）。导出要求服务器已停止，写入归档时移除 RCON 密码并关闭远程控制、清空绑定地址。导入页新增“EasyLauncher 归档”标签，读取清单后可改名称、版本和端口；解压前校验每个路径、拒绝链接与重复项并限制条目总数与总大小，导入后再应用同样的安全修复。归档只有 ZIP 校验，没有签名或加密。
- **签名更新与发布**：设置页可填写直接 HTTPS 更新清单（`latest.json`，签名从同地址加 `.sig` 读取），并选择启动时静默检查。清单使用编译进应用的 Ed25519 公钥验签后才解析，下载安装包按签名中的大小与 SHA-256 校验，只有点击后才会由系统打开安装程序，不会静默安装。仓库提供仅手动触发的签名发布工作流，普通推送不会创建发布。发布步骤与密钥管理见 [docs/RELEASING.md](docs/RELEASING.md)。
- **手机 PWA 与状态提醒**：手机控制台提供 Web 应用清单、192/512 图标与只缓存静态外壳的 Service Worker，可添加到主屏幕；API 与控制台输出从不进入缓存。令牌失效后可直接在页面粘贴从电脑复制的访问令牌。服务器崩溃、安装失败、意外停止或启动时会产生本地提醒；HTTP 局域网连接下浏览器只允许页面内横幅，系统通知需要安全上下文，且不经过任何第三方推送服务。

各项入口、边界与验证记录见 [开发进度](docs/DEVELOPMENT.md)。

## 截图

截图占位：`docs/screenshots/overview.png` —— 原生 Shan Shui Glass 主界面。

截图占位：`docs/screenshots/console.png` —— 用户创建的服务器启动后的实时控制台。

开发过程中刻意不生成包含真实 Minecraft 活动的截图。真实服务器只会在显式运行的 E2E 验收中于临时目录启动，不用于截图，也不会在默认测试或日常使用中后台启动。

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

使用 EasyLauncher 的流程：创建服务器，选择版本与软件，分配内存，选择兼容的 Java 运行时（或直接在向导中安装），阅读并接受 EULA，等待安装完成，然后点击 **启动服务器**。只有在 Minecraft 输出 `Done (...)! For help, type ...` 之后才会判定为就绪。诸如 `say Hello` 的命令会写入该 Java 进程的 stdin。编辑属性前请先停止服务器。世界备份可在启用 RCON 后于运行中创建，否则需要先停止服务器；恢复与删除备份仍要求服务器已停止。

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
npm run check:ipc
npm run typecheck
npm test
npm run check:errors
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

以上离线检查同样由 `.github/workflows/ci.yml` 在推送到 `main`、拉取请求和手动触发时执行：前端安装、IPC 契约漂移检查、类型检查、Vitest 单元测试、错误码覆盖检查、生产构建与包体积预算，以及 Rust 格式、Clippy 正确性规则、库测试、存储安全测试和全目标编译。该工作流不会下载或启动 Minecraft 服务器。

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

真实 Minecraft 端到端验收默认忽略，**只会由这条显式命令手动触发**，不会在普通测试、push、PR 或定时任务中启动游戏；它需要网络与磁盘空间，全部操作在临时目录内完成并在结束后删除：

```powershell
cd src-tauri
$env:E2E_SOFTWARE = "vanilla"   # 或 paper / fabric
$env:E2E_MC_VERSION = "1.21.4"
cargo test --test minecraft_e2e -- --ignored --nocapture
```

它会下载真实服务端、仅接受临时目录内的 EULA、等待日志就绪行、验证端口与控制台命令、干净停止、备份恢复，以及强制崩溃后的自动重启。缺少匹配 Java 主版本时会先安装受管 Temurin。`.github/workflows/minecraft-e2e.yml` 仅提供手动触发入口。

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
  server/software.rs   各服务端软件的版本来源、产物解析与 Java 要求
  server/installer.rs  共享下载、重试、哈希、进度与取消
  server/process.rs    Java 生命周期、管道读取、stdin 与就绪/崩溃观察
  server/crash.rs      崩溃签名库、首个根因选择与修复建议上下文
  server/diagnostics.rs 开服体检、崩溃归因报告与证据脱敏
  server/job.rs        Windows 关闭即终止的子进程托管
  backup.rs            一致性快照、恢复暂存与回滚
  backup_store.rs      zstd 分块存储、内容寻址去重、校验恢复与垃圾回收
  remote_backup.rs     归档打包、age 加密、WebDAV / S3 / rclone 上传与校验
  world_map.rs         Anvil MCA/NBT 只读解析、地图渲染、存档健康与区域差异
  retention.rs         每日/每周/每月保留策略、清理预览与执行
  upgrade.rs           暂存式服务端 JAR 升级与单步回退
  content_update.rs    内容更新检查、旧包快照与单包回退
  backup_diff.rs       文件哈希索引与快照差异
  dependencies.rs      Fabric / Paper / Purpur JAR 与 Modrinth 依赖检查
  modrinth.rs          Modrinth 内容安装、整合包安装/更新/回退与更新检查
  player_stats.rs      持久化玩家时长、会话与日统计
  log_assistant.rs     可选自带 Key 的日志片段分析
  tunnel.rs            用户配置的 frpc 子进程托管
  rcon.rs              可选本地 RCON 客户端、密码生成与玩家列表解析
  perf.rs              本地 TPS/MSPT 采样与历史、JFR 录制、堆快照
  resources.rs         内存预算与端口规划、错峰启动队列
  supervision.rs       systemd 单元 / launchd plist 生成、预览与确认安装
  update.rs            Ed25519 签名更新清单、校验下载与安装程序交接
  migration.rs         服务器 ZIP 归档导出、校验解压与跨机器导入
  network.rs           本地绑定检查与局域网地址探测
  logger.rs            独立的时间戳日志；2 MB 轮转并保留两个文件
  extensions.rs        内容提供方契约与 CrashAnalyzer 根因接口
```

React 调用 Rust 命令，并订阅 `server-status`、`server-console-batch`、`download-progress`、`backup-progress`、`app-closing` 与 `app-error` 事件。以 Rust 为权威来源，不使用生命周期布尔值或前端计时器伪造服务器状态。备份、生命周期变更与安装共享同一把按服务器的操作互斥锁。进程监控在等待退出或读取流时不持有子进程锁。

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
    backups/
      chunks/<前缀>/<sha256>.zst   # 所有备份共享的压缩数据块
      retention.json               # 可选保留策略
      <uuid>/
        backup.json
        manifest.json              # 文件路径、文件哈希与数据块引用
        files.json                 # 差异比较用文件索引
    backups/<旧版 uuid>/world/      # 旧目录快照，仍可恢复与比较
```

备份仅包含世界数据，不包含服务器配置、模组或插件。新备份按 1 MiB 分块读取，每块独立 zstd 压缩，并以解压后内容的 SHA-256 寻址；哈希相同的块只存一份，因此重复备份只增加变化的数据。`manifest.json` 记录文件与数据块引用，`files.json` 保留兼容的文件级索引。恢复时会逐块校验长度与 SHA-256，任何缺失或损坏都会拒绝替换世界。旧版目录快照保留原结构，可继续恢复和比较，但不会自动转换。`backups/chunks` 中未被任何 manifest 引用的数据块会在删除备份或执行清理时回收。如需防范磁盘损坏或误删服务器，请将副本保存到受控存储之外。删除受控服务器的对话框会明确说明同时删除其本地备份。

## 安全与运行行为

- 接受 EULA 是用户操作；EasyLauncher 绝不在首次启动时于后台下载并启动服务器。
- 命令通过进程参数传递，绝不拼接操作系统 shell 字符串。控制台输入限制为单行，且只写入 Minecraft 的 stdin。
- 启动与重启会重新校验 Java、内存、EULA、受控路径与本地端口可用性。内存分配至少为系统保留 1 GB 且不低于总内存的 20%，并额外检查当前可用内存。
- 停止会发送 `stop`，最多等待 45 秒，随后在强制终止前于控制台给出警告。正常关闭应用会等待关闭完成；重复的关闭请求不会绕过该等待。
- 会校验受控 UUID、世界名称、符号链接、Windows 重解析点、保留名称、链接文件夹与不安全的原始属性。RCON 只能由用户在设置中显式启用，要求至少 12 位密码且端口不与游戏端口冲突；应用自身始终连接 `127.0.0.1`。query 与管理端点仍处于禁用状态。
- 恢复要求服务器已停止并显式确认。恢复会先保存现有世界、暂存新数据，再重命名文件夹。回滚失败会将恢复数据保留在 `restore-*` 文件夹中；在问题解决前，启动会拒绝该服务器；请先使用其安全备份与保留文件，再移除该文件夹。
- 在线备份持有该服务器的操作锁，要求状态为“运行”且 RCON 已启用；无论复制是否成功都会尝试恢复 `save-on`，连续失败时返回 `backup.saveOnFailed` 并保留已完成的备份。只读模式与普通备份一样会拒绝该操作，成功或失败都会写入审计日志。
- 无效元数据会被跳过并记录日志，而不是导致目录崩溃。外观配置损坏可通过重新保存偏好修复。关闭再打开时绝不盲目信任持久化的 Running 标记。

## 已知限制

- 真实服务器验收是可选的：默认测试与日常使用不会在后台下载或启动服务器；显式运行 `cargo test --test minecraft_e2e -- --ignored` 时，会在临时目录下载并启动真实的 Vanilla、Paper 或 Fabric，接受该临时目录内的 EULA，结束后删除。Vanilla 1.21.4 已在本机通过；Paper 与 Fabric 可通过 `E2E_SOFTWARE` 或 CI 矩阵验收。真实玩家连接、公网隧道与外部付费模型仍需实际环境验收。
- 版本列表覆盖 Mojang 清单中 **1.7 起的正式版与快照**：原版与 Fabric 可在向导中勾选快照，Paper 与 Purpur 只列上游存在稳定构建的正式版；旧版本仍使用标准 `-jar server.jar nogui` 启动格式。快照可能不稳定，升级向导仍只提供正式版。目录导入支持可识别的 Vanilla、Paper、Purpur 与 Fabric 服务器。Spigot 没有官方产物 API；Forge、NeoForge 与 Quilt 需要运行官方安装器并使用非 `-jar` 的启动参数，目前尚未适配，创建向导不会提供这些选项。
- Java 选择要求主版本兼容。托管的 Adoptium 安装按服务器固定，可在设置中移除。Minecraft 26.x 的要求遵循上游元数据，并以 Java 25 作为回退；版本规则集中在一个模块中。
- Fabric 提供的启动器没有上游产物哈希，Purpur 的 API 只公布 MD5（当前未使用该值）。EasyLauncher 会校验 HTTPS 传输、ZIP/JAR 头与上报长度；Vanilla 使用 SHA-1，Paper 使用 SHA-256。Fabric、Paper 与 Purpur 可能在服务器首次启动时下载库文件；这些上游 Java 引导下载不会向 EasyLauncher 报告逐字节进度。
- 普通备份与恢复要求服务器已停止。启用 RCON 且服务器就绪时，可创建在线一致性备份：先 `save-off`，再 `save-all flush`，复制世界目录后恢复 `save-on`；`save-on` 失败会明确报错并要求重启或手动执行。Minecraft 没有文件系统快照接口，暂停自动保存与强制刷盘只能显著降低写入风险，复制期间区块卸载等写入仍可能发生，因此这不是原子快照。若 RCON 未启用或不可达，不会尝试在线备份。本地备份现已使用 zstd 分块压缩与内容寻址去重。远程目标支持 WebDAV、S3 兼容存储与本机 rclone：分块备份打包为 ZIP，可选 age 口令加密；S3 分片上传记录已完成分片并可续传，WebDAV 没有标准续传、失败后从头重试，rclone 使用自身续传能力。上传后可校验远程大小，启用时再下载校验 SHA-256；上传始终手动触发，当前版本不提供从远程归档直接恢复。密码、Secret Key 与归档口令只保存在 Windows DPAPI 密钥库，JSON 配置不写明文；HTTP 端点会明文传输凭据与备份内容，仅应在本机或可信网络使用。保留策略只按时间分组清理非安全、无备注的备份，最新一份与有备注的备份始终保留；自动清理只在定时备份成功后执行，手动清理需要先预览并确认。若 `level-name` 与原值不同，恢复会被拒绝，直到配置回原名称。
- 世界预览只读取备份，不修改世界或备份。它支持 gzip、zlib 与未压缩区块；1.20.5 起可选的 LZ4 区块压缩（类型 4）目前报告为不支持，不会尝试猜测内容。地图颜色按方块名称与高度近似生成，并非游戏内材质；单次扫描受 128 MiB 文件、64 MiB NBT、256 个区域和 50,000 个区块上限约束，超出部分会标记为截断。实体与方块实体只统计存档列表中的记录，不解析实体完整数据。玩家坐标来自备份时的 `playerdata`，不代表当前在线位置；在线轨迹只有在用户点击采样时通过本机 RCON 读取，保存在服务器目录 7 天，不发送到网络。
- 版本升级只替换 `server.jar`，并保留一份 `server.jar.previous` 用于单步回退；不会迁移模组、插件、配置或世界。升级前的安全备份只包含世界数据。服务端必须停止且已安装，所需 Java 主版本缺失时会拒绝升级并提供安装入口。降级会额外要求确认；即使确认，旧版本也可能无法读取新版本生成的世界。回退只恢复 JAR，不自动回退世界；界面会提示升级前安全备份的 ID。
- 内容更新检查只覆盖由应用安装并记录在 `content.json` 中的 Modrinth 项目；手动放入的 JAR 不做更新。检查只读取 Modrinth 元数据，可能受上游可用性影响。更新前保存的旧包快照位于服务器目录的 `content-backups/`，只支持单包单步回退，不包含依赖联动升级；页面会提示新增、变化或缺失的必需前置，但不会自动安装。更新只替换该 JAR，不迁移配置；服务器必须停止，世界不受影响。
- 整合包目前支持 Modrinth `.mrpack`。CurseForge 整合包的下载地址需要 CurseForge API Key，`manifest.json`/`overrides` 结构也不同，因此尚未支持，应用不会把未验证的 CurseForge 路径当作可用功能。每台服务器只允许记录一个整合包：安装另一个整合包或同一整合包的其他版本会被拒绝，请使用“检查整合包更新”或一步回退，或新建服务器。更新检查会把候选 `.mrpack` 暂存为 `modpack-pending.mrpack`，应用或重新检查时清理；`modpack-backups/` 只保留最近一次更新的备份，回退后删除。更新只处理整合包记录中的文件，不迁移世界，也不会检查整合包新增的 Java 运行时要求。
- 计划任务的 cron 只接受五段数字语法（分钟 0–59、小时 0–23、日 1–31、月 1–12、星期 0–7，其中 0 与 7 都表示周日），支持 `*`、列表、范围和步长，不识别 `JAN` 等英文名称。日与星期同时被限定时要求两者都匹配（取交集），因此常规“每周”或“每月”应把另一个字段留为 `*`。调度器每 20 秒轮询一次；超过 2 分钟才执行会被视为应用离线期间错过，按任务策略跳过或补跑一次，不会补齐所有错过次数。前置任务只在同一次调度中生效：前置失败时后继本次不执行；手动触发不强制前置。执行历史保存在服务器目录的 `task-history.json`，最多 200 条，不计入备份内容。
- 在线玩家观察默认依赖上游英文的加入/离开日志，插件可能改变这些消息。启用 RCON 后可在总览页拉取权威列表，但玩家洞察仍只累计应用实际观察到的事件，不回填历史日志。
- RCON 由 Minecraft 自身在所有网络接口上监听，无法在 `server.properties` 中限制绑定地址。仅应在可信网络中启用，并保持自动生成的强密码；应用不会自动开启路由器端口映射。
- 正常关闭会在所有支持的平台上优雅停止子进程。Windows 另外使用 Job Object 处理异常退出清理。Linux/macOS 上父进程被强制终止时的清理尚未保证；在这些平台重新启动前请检查残留的 Java 进程。
- 局域网地址探测仅作为路由提示；VPN 与多网卡会影响结果。本地或局域网可用不代表公网可访问。用户可显式触发 UPnP 映射或 Windows 防火墙放行；frp 则需要用户自己的服务与配置。
- 模组/插件支持 Modrinth 内容管理与依赖声明检查；本地检查覆盖 Fabric 模组、Paper/Bukkit 插件声明与常见语义化版本谓词，但不是完整的加载器解析器。无法映射回 Modrinth 的手动 JAR 仍可能显示“未找到托管前置”，需结合本地文件核对。Minecraft 自身日志仍遵循服务器的上游日志策略；EasyLauncher 应用日志与控制台缓冲区各有独立上限。
- API 可用性与吞吐取决于上游服务。Paper、Purpur 与 Fabric 请求会携带标识性 User-Agent；若分发更名后的公开分支，请配置真实项目联系地址。
- Windows 是已验证的构建目标。不包含代码签名、macOS 公证、自动更新以及 NSIS 之外的其他平台安装包。

## 路线图

当前计划中的 A—D 组已具备可使用的基础实现，具体范围见开发进度。后续仍可扩展历史日志回填、加载器特有的边界版本谓词，以及真实玩家连接与公网环境的验收（真实服务器生命周期已由可选 E2E 覆盖）。

## 上游参考

- [Tauri 命令与事件](https://v2.tauri.app/develop/calling-rust/)
- [Paper 下载服务](https://docs.papermc.io/misc/downloads-service/)
- [Purpur API](https://purpurmc.org/docs/API)
- [Fabric 服务器启动器](https://fabricmc.net/use/server/)
- [Minecraft 26.1 Java 要求](https://www.minecraft.net/en-us/article/minecraft-java-edition-26-1)
- [Minecraft EULA](https://www.minecraft.net/eula)

EasyLauncher 为独立软件，**与 Mojang Studios 或 Microsoft 无任何隶属、背书或关联关系**。Minecraft 商标归其各自所有者所有。
