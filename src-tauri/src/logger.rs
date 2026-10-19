use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Mutex,
};
pub struct Logger {
    root: PathBuf,
    lock: Mutex<()>,
}
impl Logger {
    pub fn new(root: PathBuf) -> Self {
        let _ = fs::create_dir_all(&root);
        Self {
            root,
            lock: Mutex::new(()),
        }
    }
    pub fn log(&self, level: &str, text: &str) {
        let Ok(_guard) = self.lock.lock() else { return };
        let p = self.root.join("minehost.log");
        if fs::metadata(&p)
            .map(|m| m.len() > 2 * 1024 * 1024)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(self.root.join("minehost.2.log"));
            let _ = fs::rename(
                self.root.join("minehost.1.log"),
                self.root.join("minehost.2.log"),
            );
            let _ = fs::rename(&p, self.root.join("minehost.1.log"));
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(
                f,
                "{} [{level}] {}",
                chrono::Utc::now().to_rfc3339(),
                text.chars().take(8192).collect::<String>()
            );
        }
    }
}
