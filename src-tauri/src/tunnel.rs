//! User-configured frpc process supervision. No automatic downloads or sign-ups.
use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub executable: String,
    pub config_path: String,
    pub address: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub config: Config,
    pub running: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
}
pub struct Runtime {
    child: Option<Child>,
    config: Config,
    exit_code: Option<i32>,
}
fn config_file(m: &Manager, id: &str) -> Result<PathBuf> {
    m.metadata(id)?;
    storage::child(&m.dir(id)?, "tunnel.json")
}
fn saved(m: &Manager, id: &str) -> Result<Config> {
    let path = config_file(m, id)?;
    if !path.exists() {
        return Ok(Config::default());
    }
    if fs::metadata(&path)?.len() > 16384 {
        return Err(AppError::user("Invalid tunnel configuration size."));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn validate(config: &Config) -> Result<(PathBuf, PathBuf)> {
    let exe = Path::new(&config.executable);
    let file = Path::new(&config.config_path);
    if !exe.is_absolute() || !file.is_absolute() || !exe.is_file() || !file.is_file() {
        return Err(AppError::user("Choose absolute paths to an installed frpc executable and an existing configuration file."));
    }
    let expected = if cfg!(windows) { "frpc.exe" } else { "frpc" };
    if !exe
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(expected))
    {
        return Err(AppError::user(
            "Select the frpc executable from a trusted official installation.",
        ));
    }
    let extension = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "toml" | "yaml" | "yml" | "json" | "ini")
        || fs::metadata(file)?.len() > 1024 * 1024
    {
        return Err(AppError::user(
            "Use a frpc TOML, YAML, JSON or INI configuration smaller than 1 MB.",
        ));
    }
    if config.address.is_empty()
        || config.address.len() > 253
        || !config
            .address
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_:[]".contains(&b))
    {
        return Err(AppError::user("Enter the public Minecraft address from your tunnel provider, such as play.example.com:25565."));
    }
    Ok((exe.canonicalize()?, file.canonicalize()?))
}
fn inspect(runtime: &mut Runtime) -> Result<Info> {
    if let Some(child) = runtime.child.as_mut() {
        if let Some(exit) = child.try_wait()? {
            runtime.exit_code = exit.code();
            runtime.child = None;
        }
    }
    Ok(Info {
        config: runtime.config.clone(),
        running: runtime.child.is_some(),
        pid: runtime.child.as_ref().and_then(Child::id),
        exit_code: runtime.exit_code,
    })
}
pub async fn info(m: &Manager, id: &str) -> Result<Info> {
    m.metadata(id)?;
    let mut tunnels = m.tunnels.lock().await;
    if let Some(runtime) = tunnels.get_mut(id) {
        return inspect(runtime);
    }
    Ok(Info {
        config: saved(m, id)?,
        running: false,
        pid: None,
        exit_code: None,
    })
}
/// Persist the user's frpc choice without starting a process. Public access
/// uses this so a provider can be configured before the server is running.
pub fn save(m: &Manager, id: &str, config: &Config) -> Result<()> {
    m.metadata(id)?;
    validate(config)?;
    storage::write_json(&config_file(m, id)?, config)
}
/// Start the saved frpc configuration, used by the automatic lifecycle.
pub async fn start_saved(m: &Manager, id: &str) -> Result<Info> {
    start(m, id, saved(m, id)?).await
}
pub async fn start(m: &Manager, id: &str, config: Config) -> Result<Info> {
    m.ready()?;
    let runtime = m.runtime(id)?;
    let _gate = runtime.gate.lock().await;
    let (executable, file) = validate(&config)?;
    let mut tunnels = m.tunnels.lock().await;
    m.ready()?;
    if let Some(runtime) = tunnels.get_mut(id) {
        if inspect(runtime)?.running {
            return Err(AppError::user("This server already has a managed tunnel."));
        }
    }
    // Save user choices before spawning; credentials stay in the user's own frpc config.
    storage::write_json(&config_file(m, id)?, &config)?;
    let mut command = Command::new(executable);
    command
        .args(["-c"])
        .arg(&file)
        .current_dir(file.parent().unwrap())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = command.spawn().map_err(|e| {
        AppError::new(
            "Could not start frpc. Check its installation and configuration.",
            e,
        )
    })?;
    m.process_group.attach(&child)?;
    let mut state = Runtime {
        child: Some(child),
        config,
        exit_code: None,
    };
    let info = inspect(&mut state)?;
    tunnels.insert(id.into(), state);
    Ok(info)
}
pub async fn stop(m: &Manager, id: &str) -> Result<Info> {
    m.metadata(id)?;
    let mut tunnels = m.tunnels.lock().await;
    if let Some(runtime) = tunnels.get_mut(id) {
        inspect(runtime)?;
        if let Some(mut child) = runtime.child.take() {
            // Keep a failed kill tracked so the user can retry instead of losing the handle.
            if let Err(e) = child.kill().await {
                runtime.child = Some(child);
                return Err(AppError::new("Could not stop frpc.", e));
            }
            let _ = child.wait().await;
            runtime.exit_code = None;
        }
        return inspect(runtime);
    }
    Ok(Info {
        config: saved(m, id)?,
        running: false,
        pid: None,
        exit_code: None,
    })
}
pub async fn stop_all(m: &Manager) -> Result<()> {
    let ids: Vec<_> = m.tunnels.lock().await.keys().cloned().collect();
    let mut error = None;
    for id in ids {
        if let Err(e) = stop(m, &id).await {
            error = Some(e);
        }
    }
    error.map_or(Ok(()), Err)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_user_selected_paths_without_executing() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp
            .path()
            .join(if cfg!(windows) { "frpc.exe" } else { "frpc" });
        let file = temp.path().join("frpc.toml");
        fs::write(&exe, b"not executed").unwrap();
        fs::write(&file, b"serverAddr='example.invalid'").unwrap();
        let mut config = Config {
            executable: exe.to_string_lossy().into(),
            config_path: file.to_string_lossy().into(),
            address: "play.example.com:25565".into(),
        };
        assert!(validate(&config).is_ok());
        config.address = "bad address\n".into();
        assert!(validate(&config).is_err());
    }
}
