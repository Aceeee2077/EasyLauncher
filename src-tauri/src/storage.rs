use crate::{
    error::{AppError, Result},
    models::Server,
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub fn validate_id(id: &str) -> Result<()> {
    if Uuid::parse_str(id)
        .map(|u| u.to_string() == id)
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(AppError::user(
            "Invalid managed server or backup identifier.",
        ))
    }
}
pub fn no_links(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(m) = metadata {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(AppError::user(
                    "Linked folders and junctions are not allowed in managed storage.",
                ));
            }
        }
        if m.file_type().is_symlink() {
            return Err(AppError::user(
                "Symbolic links are not allowed in managed storage.",
            ));
        }
    }
    Ok(())
}
pub fn child(root: &Path, name: &str) -> Result<PathBuf> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&stem.as_str())
    {
        return Err(AppError::user(
            "This name is reserved by the operating system.",
        ));
    }
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_- .".contains(&c))
        || name.ends_with(['.', ' '])
        || name.contains(':')
    {
        return Err(AppError::user("This file or world name is unsafe. Use letters, numbers, spaces, hyphens or underscores."));
    }
    no_links(root)?;
    let p = root.join(name);
    no_links(&p)?;
    Ok(p)
}
pub fn managed(root: &Path, id: &str) -> Result<PathBuf> {
    validate_id(id)?;
    child(root, id)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    no_links(path)?;
    let temp = path.with_extension(format!("{}.part", Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut f = fs::File::create(&temp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}
pub fn read_server(path: &Path) -> Result<Server> {
    let file = child(path, "minehost.json")?;
    let s: Server = serde_json::from_slice(&fs::read(file)?)?;
    validate_id(&s.id)?;
    if s.schema_version > 1 {
        return Err(AppError::user(
            "This server was created by a newer EasyLauncher version.",
        ));
    }
    if path.file_name().and_then(|s| s.to_str()) != Some(&s.id) {
        return Err(AppError::user(
            "Server metadata does not match its managed directory.",
        ));
    }
    Ok(s)
}
pub fn save_server(root: &Path, server: &Server) -> Result<()> {
    write_json(
        &child(&managed(root, &server.id)?, "minehost.json")?,
        server,
    )
}
pub fn validate_tree(path: &Path) -> Result<()> {
    no_links(path)?;
    if path.is_dir() {
        for e in fs::read_dir(path)? {
            validate_tree(&e?.path())?;
        }
    }
    Ok(())
}
pub fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    no_links(from)?;
    no_links(to)?;
    fs::create_dir_all(to)?;
    let mut size = 0;
    for e in fs::read_dir(from)? {
        let e = e?;
        no_links(&e.path())?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            size += copy_tree(&e.path(), &target)?;
        } else if e.file_type()?.is_file() {
            size += fs::copy(e.path(), target)?;
        } else {
            return Err(AppError::user(
                "A world contains an unsupported special file.",
            ));
        }
    }
    Ok(size)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traversal_rejected() {
        let r = Path::new("servers");
        for id in [
            "../escape",
            "..\\escape",
            "C:\\x",
            "",
            "A",
            "00000000-0000-0000-0000-000000000000/../x",
        ] {
            assert!(managed(r, id).is_err());
        }
        assert!(managed(r, &Uuid::new_v4().to_string()).is_ok());
    }
    #[test]
    fn atomic_metadata() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("test.json");
        write_json(&p, &vec![1, 2]).unwrap();
        write_json(&p, &vec![3]).unwrap();
        assert_eq!(fs::read_to_string(p).unwrap(), "[\n  3\n]");
    }
    #[test]
    fn world_names() {
        for n in [
            "../world",
            "world/other",
            "world\\other",
            ".",
            "..",
            "C:world",
            "world.",
        ] {
            assert!(child(Path::new("servers"), n).is_err());
        }
    }
}
