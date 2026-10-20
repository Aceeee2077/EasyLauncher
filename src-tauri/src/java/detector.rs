use crate::{
    error::{AppError, Result},
    java::version::{compatible, parse_java},
};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaRuntime {
    pub path: String,
    pub major: u32,
    pub version: String,
    pub managed: bool,
}
pub async fn inspect(path: &str) -> Result<JavaRuntime> {
    let p = PathBuf::from(path);
    if !p.is_absolute() || !p.is_file() {
        return Err(AppError::user(
            "Choose the full path to the Java executable (java.exe on Windows).",
        )
        .with_code("server.javaPathInvalid"));
    }
    let mut command = Command::new(&p);
    command
        .arg("-version")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let out = tokio::time::timeout(Duration::from_secs(8), command.output())
        .await
        .map_err(|_| {
            AppError::user("Java did not respond within 8 seconds. Choose another runtime.")
                .with_code("server.javaTimeout")
        })?
        .map_err(|e| {
            AppError::new(
                "Could not run Java. Check that the selected executable is installed correctly.",
                e,
            )
            .with_code("server.javaUnusable")
        })?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let major = parse_java(&text)
        .filter(|_| out.status.success())
        .ok_or_else(|| {
            AppError::new(
                "The selected executable did not report a valid Java runtime.",
                &text,
            )
            .with_code("server.javaInvalid")
        })?;
    Ok(JavaRuntime {
        path: p.canonicalize()?.to_string_lossy().into(),
        major,
        version: text.lines().next().unwrap_or("").to_string(),
        managed: false,
    })
}
pub async fn validate(path: &str, required: u32) -> Result<JavaRuntime> {
    let j = inspect(path).await?;
    if !compatible(j.major, required) {
        return Err(AppError::new(
            "This server requires a different Java version.",
            format!(
                "Required Java {required}, selected runtime is Java {}. Install a matching runtime or choose another path.",
                j.major
            ),
        )
        .with_code("server.javaMismatch"));
    }
    Ok(j)
}
pub async fn detect(custom: Option<String>, managed_root: Option<&Path>) -> Vec<JavaRuntime> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let mut paths = BTreeSet::new();
    let mut managed_paths = BTreeSet::new();
    if let Some(c) = custom {
        paths.insert(PathBuf::from(c));
    }
    if let Some(root) = managed_root {
        if let Ok(entries) = std::fs::read_dir(root) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name.starts_with("deleted-") {
                    continue;
                }
                if let Some(found) = crate::java::managed::executable(&e.path().join("runtime")) {
                    managed_paths.insert(found.clone());
                    paths.insert(found);
                }
            }
        }
    }
    for name in ["JAVA_HOME", "JDK_HOME"] {
        if let Some(home) = std::env::var_os(name) {
            paths.insert(PathBuf::from(home).join("bin").join(exe));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            paths.insert(dir.join(exe));
        }
    }
    let mut roots = vec![
        PathBuf::from("/usr/lib/jvm"),
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
    ];
    if let Some(p) = std::env::var_os("ProgramFiles") {
        for vendor in ["Java", "Eclipse Adoptium", "Microsoft", "Amazon Corretto"] {
            roots.push(PathBuf::from(&p).join(vendor));
        }
    }
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(root) {
            for e in entries.flatten() {
                paths.insert(e.path().join("bin").join(exe));
                paths.insert(e.path().join("Contents/Home/bin").join(exe));
            }
        }
    }
    let mut jobs = Vec::new();
    for path in paths.into_iter().filter(|p| p.is_file()) {
        jobs.push(tokio::spawn(async move {
            inspect(&path.to_string_lossy()).await.ok()
        }));
    }
    let mut result = Vec::new();
    for job in jobs {
        if let Ok(Some(j)) = job.await {
            if !result.iter().any(|v: &JavaRuntime| v.path == j.path) {
                result.push(j);
            }
        }
    }
    let managed: BTreeSet<PathBuf> = managed_paths
        .iter()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
        .collect();
    for j in &mut result {
        j.managed = managed.contains(Path::new(&j.path));
    }
    result.sort_by_key(|j| std::cmp::Reverse(j.major));
    result
}
