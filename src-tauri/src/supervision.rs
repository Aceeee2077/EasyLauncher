//! Production service definitions for Linux and macOS.
//!
//! EasyLauncher only supervises a server while the desktop app runs. This
//! module renders the native service definition that lets systemd (Linux) or
//! launchd (macOS) run the same server after the app closes, and writes that
//! one file into the current user's service directory after an explicit
//! confirmation.
//!
//! Nothing in this module starts, stops, enables or otherwise controls a
//! service. The commands in the preview are for the user to run. A
//! service-managed server is also invisible to the app: there is no console,
//! health check, scheduled command or disk coordination until the service is
//! disabled and the server is started in the app again.

use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Appended by both service managers so output survives the app closing.
pub const LOG_FILE: &str = "logs/supervisor.log";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceKind {
    Systemd,
    Launchd,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    /// The Rust target OS name: `linux`, `macos`, `windows` or another value.
    pub platform: String,
    /// Present only when this platform can install a service here.
    pub kind: Option<ServiceKind>,
    pub available: bool,
    /// Absolute directory that will receive the definition, when known.
    pub directory: Option<String>,
    /// Stable reason identifier such as `windows` or `noSystemd`.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicePreview {
    pub platform: String,
    pub kind: ServiceKind,
    pub file_name: String,
    pub path: String,
    pub contents: String,
    /// Human-readable commands the user can run after reviewing the file.
    pub commands: Vec<String>,
    /// Whether the definition already exists on disk.
    pub installed: bool,
    /// Whether the app currently has this server starting, running or stopping.
    pub active: bool,
    pub memory_mb: u32,
    pub log_path: String,
    /// The executable written into the service, shown in warnings.
    pub java_path: String,
    /// False when the Java path is a bare command that relies on PATH.
    pub absolute_java: bool,
}

/// Details needed to render either service format.
struct ServiceSpec {
    id: String,
    description: String,
    java: String,
    args: Vec<String>,
    working_dir: PathBuf,
    log_path: PathBuf,
    memory_mb: u32,
    restart: bool,
    path_env: Option<String>,
}

fn home_dir() -> Option<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn support_for(platform: &str, home: Option<&Path>, xdg: Option<&Path>, systemd: bool) -> Support {
    let directory = match platform {
        "linux" => xdg
            // `has_root` also recognizes Unix-style paths on a non-Unix test
            // host, where `is_absolute` would insist on a drive prefix.
            .filter(|path| path.has_root())
            .map(Path::to_path_buf)
            .or_else(|| home.map(|path| path.join(".config")))
            .map(|path| path.join("systemd").join("user")),
        "macos" => home.map(|path| path.join("Library").join("LaunchAgents")),
        _ => None,
    };
    let kind = match platform {
        "linux" if systemd => Some(ServiceKind::Systemd),
        "macos" => Some(ServiceKind::Launchd),
        _ => None,
    };
    // An unavailable platform still reports its directory so the panel can
    // explain where the file would live once requirements are met.
    let kind = kind.filter(|_| directory.is_some());
    let reason = if platform == "windows" {
        Some("windows")
    } else if platform == "linux" && !systemd {
        Some("noSystemd")
    } else if directory.is_none() {
        Some(if platform == "linux" || platform == "macos" {
            "noHome"
        } else {
            "unsupportedOs"
        })
    } else {
        None
    };
    Support {
        platform: platform.to_string(),
        kind,
        available: kind.is_some() && reason.is_none(),
        directory: directory.map(|path| path.display().to_string()),
        reason: reason.map(str::to_string),
    }
}

/// Detects whether this machine can host a user-level service definition.
pub fn support() -> Support {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let systemd = cfg!(target_os = "linux") && Path::new("/run/systemd/system").is_dir();
    support_for(
        std::env::consts::OS,
        home_dir().as_deref(),
        xdg.as_deref(),
        systemd,
    )
}

fn unsupported(support: &Support) -> AppError {
    let details = support.reason.clone().unwrap_or_default();
    AppError::new(
        "Production services are only available with systemd on Linux or launchd on macOS.",
        details,
    )
    .with_code("supervision.unsupported")
}

fn spec_for(m: &Manager, id: &str) -> Result<ServiceSpec> {
    let server = m.metadata(id)?;
    if !server.installed {
        return Err(
            AppError::user("Install this server before creating a production service.")
                .with_code("supervision.notInstalled"),
        );
    }
    let info = m.launch_info(id)?;
    if !info.jar_present {
        return Err(AppError::user(
            "The server JAR is missing. Install or repair the server first.",
        )
        .with_code("supervision.notInstalled"));
    }
    let working_dir = PathBuf::from(&info.working_directory);
    Ok(ServiceSpec {
        id: server.id.clone(),
        description: describe(&server.name),
        java: info.java_path,
        args: info.args,
        log_path: working_dir.join(LOG_FILE),
        working_dir,
        memory_mb: server.memory_max_mb,
        restart: server.auto_restart,
        path_env: std::env::var("PATH")
            .ok()
            .filter(|path| !path.trim().is_empty() && !path.contains(['\n', '\r', '\0'])),
    })
}

/// One-line, length-limited text for service metadata.
fn describe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text = trimmed.chars().take(120).collect::<String>();
    if text.is_empty() {
        text = "Minecraft server".into();
    }
    format!("EasyLauncher server: {text}")
}

/// Quotes one systemd value. `%` is always a specifier, so it is doubled;
/// quotes, backslashes and newlines are neutralized for every directive.
fn systemd_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '%' => out.push_str("%%"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `$` expands in command lines, so a literal dollar must be doubled there.
/// It is left alone in paths and descriptions, where no expansion occurs.
fn systemd_command_quote(value: &str) -> String {
    systemd_quote(&value.replace('$', "$$"))
}

fn systemd_unit(spec: &ServiceSpec) -> String {
    let exec = std::iter::once(spec.java.as_str())
        .chain(spec.args.iter().map(String::as_str))
        .map(systemd_command_quote)
        .collect::<Vec<_>>()
        .join(" ");
    let restart = if spec.restart { "on-failure" } else { "no" };
    let mut unit = format!(
        "# Generated by EasyLauncher. Reinstall from the app after changing the server.\n\
         [Unit]\n\
         Description={}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={}\n\
         ExecStart={exec}\n\
         Restart={restart}\n\
         RestartSec=10\n\
         SuccessExitStatus=0 143\n\
         KillSignal=SIGTERM\n\
         TimeoutStopSec=90\n\
         LimitNOFILE=65536\n\
         MemoryMax={}M\n\
         StandardInput=null\n\
         StandardOutput={}\n\
         StandardError={}\n",
        systemd_quote(&spec.description),
        systemd_quote(&spec.working_dir.display().to_string()),
        spec.memory_mb,
        systemd_quote(&format!("append:{}", spec.log_path.display())),
        systemd_quote(&format!("append:{}", spec.log_path.display())),
    );
    if let Some(path) = &spec.path_env {
        unit.push_str(&format!(
            "Environment={}\n",
            systemd_quote(&format!("PATH={path}"))
        ));
    }
    unit.push_str("\n[Install]\nWantedBy=default.target\n");
    unit
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn launchd_label(spec: &ServiceSpec) -> String {
    format!("com.easylauncher.server.{}", spec.id)
}

fn launchd_plist(spec: &ServiceSpec) -> String {
    let mut arguments = String::new();
    for argument in std::iter::once(&spec.java).chain(spec.args.iter()) {
        arguments.push_str(&format!("    <string>{}</string>\n", xml_escape(argument)));
    }
    let keep_alive = if spec.restart {
        "  <key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n"
            .to_string()
    } else {
        "  <key>KeepAlive</key>\n  <false/>\n".to_string()
    };
    let mut environment = String::new();
    if let Some(path) = &spec.path_env {
        environment.push_str(&format!(
            "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>PATH</key>\n    <string>{}</string>\n  </dict>\n",
            xml_escape(path)
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n\
         \x20 <string>{label}</string>\n\
         \x20 <key>ProgramArguments</key>\n\
         \x20 <array>\n{arguments}\x20 </array>\n\
         \x20 <key>WorkingDirectory</key>\n\
         \x20 <string>{working}</string>\n\
         \x20 <key>StandardOutPath</key>\n\
         \x20 <string>{log}</string>\n\
         \x20 <key>StandardErrorPath</key>\n\
         \x20 <string>{log}</string>\n\
         \x20 <key>RunAtLoad</key>\n\
         \x20 <true/>\n\
         {keep_alive}\
         \x20 <key>ProcessType</key>\n\
         \x20 <string>Interactive</string>\n\
         \x20 <key>ThrottleInterval</key>\n\
         \x20 <integer>10</integer>\n\
         {environment}\
         </dict>\n\
         </plist>\n",
        label = xml_escape(&launchd_label(spec)),
        working = xml_escape(&spec.working_dir.display().to_string()),
        log = xml_escape(&spec.log_path.display().to_string()),
    )
}

fn render(kind: ServiceKind, spec: &ServiceSpec, dir: &Path) -> (String, String, Vec<String>) {
    match kind {
        ServiceKind::Systemd => {
            let file = format!("easylauncher-{}.service", spec.id);
            let commands = vec![
                "systemctl --user daemon-reload".into(),
                format!("systemctl --user enable --now {file}"),
                format!(
                    "journalctl --user -u {file} -f  # or: tail -f {}",
                    spec.log_path.display()
                ),
            ];
            (file, systemd_unit(spec), commands)
        }
        ServiceKind::Launchd => {
            let label = launchd_label(spec);
            let file = format!("{label}.plist");
            let commands = vec![
                format!("launchctl enable gui/$(id -u)/{label}"),
                format!(
                    "launchctl bootstrap gui/$(id -u) {}",
                    dir.join(&file).display()
                ),
                format!("launchctl kickstart -k gui/$(id -u)/{label}"),
                format!("tail -f {}", spec.log_path.display()),
            ];
            (file, launchd_plist(spec), commands)
        }
    }
}

fn preview_with(support: &Support, spec: &ServiceSpec) -> Result<ServicePreview> {
    let kind = support.kind.ok_or_else(|| unsupported(support))?;
    let directory = support
        .directory
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| unsupported(support))?;
    let (file_name, contents, commands) = render(kind, spec, &directory);
    Ok(ServicePreview {
        platform: support.platform.clone(),
        kind,
        path: directory.join(&file_name).display().to_string(),
        file_name,
        contents,
        commands,
        installed: false,
        active: false,
        memory_mb: spec.memory_mb,
        log_path: spec.log_path.display().to_string(),
        java_path: spec.java.clone(),
        // A bare `java` command relies on PATH and is worth warning about.
        // `has_root` answers that question even in the cross-platform tests.
        absolute_java: Path::new(&spec.java).has_root(),
    })
}

/// Builds the definition without writing it, for review in the app.
pub fn preview(m: &Manager, id: &str) -> Result<ServicePreview> {
    let support = support();
    if !support.available {
        return Err(unsupported(&support));
    }
    let spec = spec_for(m, id)?;
    let mut preview = preview_with(&support, &spec)?;
    preview.installed = Path::new(&preview.path).is_file();
    preview.active = m
        .runtime(id)
        .map(|runtime| runtime.snapshot().status.active())
        .unwrap_or(false);
    Ok(preview)
}

/// Writes only the user service file. It never invokes systemctl or
/// launchctl, so enabling the service stays an explicit user action.
pub fn install(m: &Manager, id: &str, confirmed: bool) -> Result<ServicePreview> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm that the server is stopped before installing the service file.",
        )
        .with_code("supervision.notConfirmed"));
    }
    let mut preview = preview(m, id)?;
    m.runtime(id)?.idle()?;
    let target = PathBuf::from(&preview.path);
    let directory = target
        .parent()
        .ok_or_else(|| AppError::user("The service directory is invalid."))?;
    storage::no_links(directory)?;
    fs::create_dir_all(directory)?;
    if let Some(logs) = Path::new(&preview.log_path).parent() {
        fs::create_dir_all(logs)?;
    }
    storage::atomic_write(&target, preview.contents.as_bytes())?;
    preview.installed = true;
    Ok(preview)
}

/// Removes the definition file. A loaded service must be disabled by the user
/// first; this function deliberately does not guess at service state.
pub fn remove(m: &Manager, id: &str, confirmed: bool) -> Result<ServicePreview> {
    if !confirmed {
        return Err(AppError::user(
            "Confirm that the production service is disabled before removing its file.",
        )
        .with_code("supervision.notConfirmed"));
    }
    let mut preview = preview(m, id)?;
    if !preview.installed {
        return Err(
            AppError::user("No service file is installed for this server.")
                .with_code("supervision.noFile"),
        );
    }
    let target = PathBuf::from(&preview.path);
    storage::no_links(&target)?;
    fs::remove_file(&target)?;
    preview.installed = false;
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ServiceSpec {
        ServiceSpec {
            id: "018f2a4c-0000-7000-8000-000000000001".into(),
            description: describe("Survival \"main\" 100%"),
            java: "/usr/bin/java".into(),
            args: vec![
                "-Xms1024M".into(),
                "-Xmx2048M".into(),
                "-Dserver.name=100%$".into(),
                "-jar".into(),
                "server.jar".into(),
                "nogui".into(),
            ],
            working_dir: PathBuf::from("/opt/Easy Servers/main world"),
            log_path: PathBuf::from("/opt/Easy Servers/main world/logs/supervisor.log"),
            memory_mb: 2048,
            restart: true,
            path_env: Some("/usr/bin:/bin".into()),
        }
    }

    #[test]
    fn systemd_unit_quotes_paths_arguments_and_specifiers() {
        let unit = systemd_unit(&sample());
        assert!(unit.contains("Description=\"EasyLauncher server: Survival \\\"main\\\" 100%%\""));
        assert!(unit.contains("WorkingDirectory=\"/opt/Easy Servers/main world\""));
        assert!(unit.contains(
            "ExecStart=\"/usr/bin/java\" \"-Xms1024M\" \"-Xmx2048M\" \"-Dserver.name=100%%$$\""
        ));
        assert!(unit.contains("MemoryMax=2048M"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("StandardInput=null"));
        assert!(unit.contains(
            "StandardOutput=\"append:/opt/Easy Servers/main world/logs/supervisor.log\""
        ));
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/bin\""));
        assert!(unit.contains("[Install]\nWantedBy=default.target"));
        assert!(!unit.contains("Restart=no"));
    }

    #[test]
    fn systemd_unit_only_doubles_dollars_in_commands() {
        let mut spec = sample();
        spec.working_dir = PathBuf::from("/srv/$data/server");
        spec.args.push("-Dmotd=$HOME".into());
        let unit = systemd_unit(&spec);
        assert!(unit.contains("WorkingDirectory=\"/srv/$data/server\""));
        assert!(unit.contains("\"-Dmotd=$$HOME\""));
    }

    #[test]
    fn systemd_unit_can_disable_restart() {
        let mut spec = sample();
        spec.restart = false;
        spec.path_env = None;
        let unit = systemd_unit(&spec);
        assert!(unit.contains("Restart=no\nRestartSec=10\n"));
        assert!(!unit.contains("Environment="));
    }

    #[test]
    fn launchd_plist_escapes_xml_and_sets_restart_policy() {
        let mut spec = sample();
        spec.args.push("-Dmotd=A&B<C>\"D\"".into());
        spec.working_dir = PathBuf::from("/opt/A&B");
        let plist = launchd_plist(&spec);
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(plist.contains(
            "<string>com.easylauncher.server.018f2a4c-0000-7000-8000-000000000001</string>"
        ));
        assert!(plist.contains("<string>-Dmotd=A&amp;B&lt;C&gt;&quot;D&quot;</string>"));
        assert!(plist.contains("<string>/opt/A&amp;B</string>"));
        assert!(plist.contains(
            "<key>KeepAlive</key>\n  <dict>\n    <key>SuccessfulExit</key>\n    <false/>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>ThrottleInterval</key>\n  <integer>10</integer>"));
        assert!(plist.contains("<key>PATH</key>\n    <string>/usr/bin:/bin</string>"));
        assert!(plist.ends_with("</plist>\n"));
    }

    #[test]
    fn launchd_plist_without_restart_has_no_keep_alive_dict() {
        let mut spec = sample();
        spec.restart = false;
        let plist = launchd_plist(&spec);
        assert!(plist.contains("<key>KeepAlive</key>\n  <false/>"));
        assert!(!plist.contains("SuccessfulExit"));
    }

    #[test]
    fn render_names_files_and_bootstrap_commands_per_platform() {
        let spec = sample();
        let dir = Path::new("/home/steve/.config/systemd/user");
        let (file, contents, commands) = render(ServiceKind::Systemd, &spec, dir);
        assert_eq!(
            file,
            "easylauncher-018f2a4c-0000-7000-8000-000000000001.service"
        );
        assert!(contents.contains("[Service]"));
        assert_eq!(commands[0], "systemctl --user daemon-reload");
        assert_eq!(
            commands[1],
            "systemctl --user enable --now easylauncher-018f2a4c-0000-7000-8000-000000000001.service"
        );

        let dir = Path::new("/Users/steve/Library/LaunchAgents");
        let (file, contents, commands) = render(ServiceKind::Launchd, &spec, dir);
        assert_eq!(
            file,
            "com.easylauncher.server.018f2a4c-0000-7000-8000-000000000001.plist"
        );
        assert!(contents.contains("<plist version=\"1.0\">"));
        assert_eq!(
            commands[0],
            "launchctl enable gui/$(id -u)/com.easylauncher.server.018f2a4c-0000-7000-8000-000000000001"
        );
        let expected = dir
            .join("com.easylauncher.server.018f2a4c-0000-7000-8000-000000000001.plist")
            .display()
            .to_string();
        assert!(commands[1].contains(&expected));
    }

    #[test]
    fn support_reports_windows_and_unknown_systems_as_unsupported() {
        let windows = support_for("windows", Some(Path::new("C:\\Users\\steve")), None, false);
        assert_eq!(windows.platform, "windows");
        assert!(!windows.available);
        assert!(windows.kind.is_none());
        assert_eq!(windows.reason.as_deref(), Some("windows"));

        let other = support_for("freebsd", Some(Path::new("/home/steve")), None, false);
        assert!(!other.available);
        assert_eq!(other.reason.as_deref(), Some("unsupportedOs"));
    }

    #[test]
    fn support_uses_xdg_then_home_and_requires_systemd() {
        let xdg = support_for(
            "linux",
            Some(Path::new("/home/steve")),
            Some(Path::new("/srv/config")),
            true,
        );
        assert!(xdg.available);
        assert_eq!(xdg.kind, Some(ServiceKind::Systemd));
        assert_eq!(
            xdg.directory.as_deref().map(Path::new),
            Some(
                Path::new("/srv/config")
                    .join("systemd")
                    .join("user")
                    .as_path()
            )
        );

        let relative = support_for(
            "linux",
            Some(Path::new("/home/steve")),
            Some(Path::new("relative")),
            true,
        );
        assert_eq!(
            relative.directory.as_deref().map(Path::new),
            Some(
                Path::new("/home/steve")
                    .join(".config")
                    .join("systemd")
                    .join("user")
                    .as_path()
            )
        );

        let no_systemd = support_for("linux", Some(Path::new("/home/steve")), None, false);
        assert!(!no_systemd.available);
        assert_eq!(no_systemd.reason.as_deref(), Some("noSystemd"));
        assert!(no_systemd.kind.is_none());
        assert!(no_systemd.directory.is_some());

        let no_home = support_for("linux", None, None, true);
        assert!(!no_home.available);
        assert_eq!(no_home.reason.as_deref(), Some("noHome"));
    }

    #[test]
    fn support_places_launch_agents_in_the_home_directory() {
        let support = support_for("macos", Some(Path::new("/Users/steve")), None, false);
        assert!(support.available);
        assert_eq!(support.kind, Some(ServiceKind::Launchd));
        assert_eq!(
            support.directory.as_deref().map(Path::new),
            Some(
                Path::new("/Users/steve")
                    .join("Library")
                    .join("LaunchAgents")
                    .as_path()
            )
        );
        assert!(support.reason.is_none());

        let no_home = support_for("macos", None, None, false);
        assert!(!no_home.available);
        assert_eq!(no_home.reason.as_deref(), Some("noHome"));
    }

    #[test]
    fn descriptions_stay_single_line_and_bounded() {
        let long = format!("{}\n{}", "name\twith control", "x".repeat(300));
        let text = describe(&long);
        assert!(text.starts_with("EasyLauncher server: name with control"));
        assert!(!text.contains('\n'));
        assert!(!text.contains('\t'));
        assert!(text.chars().count() <= "EasyLauncher server: ".len() + 120);
        assert_eq!(describe("   "), "EasyLauncher server: Minecraft server");
    }

    #[test]
    fn preview_reports_the_file_and_absolute_java_state() {
        let spec = sample();
        let support = support_for("linux", Some(Path::new("/home/steve")), None, true);
        let preview = preview_with(&support, &spec).unwrap();
        assert_eq!(preview.kind, ServiceKind::Systemd);
        assert_eq!(
            Path::new(&preview.path).to_path_buf(),
            Path::new("/home/steve")
                .join(".config")
                .join("systemd")
                .join("user")
                .join("easylauncher-018f2a4c-0000-7000-8000-000000000001.service")
        );
        assert!(preview.absolute_java);
        assert!(!preview.installed);
        assert!(!preview.active);
        assert_eq!(preview.log_path, spec.log_path.display().to_string());
        assert_eq!(preview.java_path, "/usr/bin/java");

        let mut bare = sample();
        bare.java = "java".into();
        assert!(!preview_with(&support, &bare).unwrap().absolute_java);
    }
}
