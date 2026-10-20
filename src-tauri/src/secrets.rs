//! Small opt-in secret vault. Values are encrypted per Windows user with
//! DPAPI before they touch disk, so copying `secrets.json` to another account
//! or machine does not reveal them. The frontend never sees other entries.
use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Mutex};

const FILE: &str = "secrets.json";
const MAX_VALUE: usize = 16 * 1024;
#[cfg(windows)]
const ENTROPY: &[u8] = b"EasyLauncher.secrets.v1";
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
struct Vault {
    version: u32,
    entries: BTreeMap<String, String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub available: bool,
    pub names: Vec<String>,
}
pub fn status(m: &Manager) -> Result<Status> {
    Ok(Status {
        available: available(),
        names: if available() { names(m)? } else { Vec::new() },
    })
}
pub fn available() -> bool {
    cfg!(windows)
}
fn path(m: &Manager) -> Result<PathBuf> {
    storage::child(&m.root, FILE)
}
fn read(m: &Manager) -> Result<Vault> {
    let path = path(m)?;
    if !path.exists() {
        return Ok(Vault {
            version: 1,
            entries: BTreeMap::new(),
        });
    }
    storage::no_links(&path)?;
    let bytes = fs::read(&path)?;
    if bytes.len() > 1024 * 1024 {
        return Err(AppError::user("The secret vault exceeds the size limit.")
            .with_code("secrets.unreadable"));
    }
    let vault: Vault = serde_json::from_slice(&bytes).map_err(|e| {
        AppError::new("The secret vault cannot be read.", e).with_code("secrets.unreadable")
    })?;
    if vault.version != 1 {
        return Err(
            AppError::user("The secret vault was written by a newer version of the app.")
                .with_code("secrets.unreadable"),
        );
    }
    Ok(vault)
}
fn write(m: &Manager, vault: &Vault) -> Result<()> {
    storage::write_json(&path(m)?, vault)
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        || name.starts_with('.')
    {
        return Err(AppError::user(
            "Secret names use lowercase letters, digits, dots, dashes or underscores.",
        )
        .with_code("secrets.name"));
    }
    Ok(())
}
fn validate_value(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_VALUE || value.contains('\0') {
        return Err(
            AppError::user("The secret value is empty or too long.").with_code("secrets.value")
        );
    }
    Ok(())
}
pub fn names(m: &Manager) -> Result<Vec<String>> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Ok(read(m)?.entries.into_keys().collect())
}
pub fn save(m: &Manager, name: &str, value: &str) -> Result<()> {
    validate_name(name)?;
    validate_value(value)?;
    let encrypted = hex(&protect(value.as_bytes())?);
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vault = read(m)?;
    vault.entries.insert(name.into(), encrypted);
    write(m, &vault)
}
pub fn load(m: &Manager, name: &str) -> Result<Option<String>> {
    validate_name(name)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(value) = read(m)?.entries.get(name).cloned() else {
        return Ok(None);
    };
    let bytes = unhex(&value).ok_or_else(|| {
        AppError::user("The stored secret is damaged.").with_code("secrets.unreadable")
    })?;
    let plain = unprotect(&bytes)?;
    String::from_utf8(plain).map(Some).map_err(|_| {
        AppError::user("The stored secret is not valid text.").with_code("secrets.unreadable")
    })
}
pub fn delete(m: &Manager, name: &str) -> Result<()> {
    validate_name(name)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut vault = read(m)?;
    if vault.entries.remove(name).is_some() {
        write(m, &vault)?;
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
fn unhex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

#[cfg(windows)]
fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            0x1, // CRYPTPROTECT_UI_FORBIDDEN
            &mut output,
        )
    };
    if ok == 0 {
        return Err(dpapi_error());
    }
    let encrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut core::ffi::c_void);
    }
    Ok(encrypted)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            0x1,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(dpapi_error());
    }
    let plain =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut core::ffi::c_void);
    }
    Ok(plain)
}
#[cfg(windows)]
fn dpapi_error() -> AppError {
    AppError::new(
        "Windows could not decrypt or protect the secret for this account.",
        std::io::Error::last_os_error(),
    )
    .with_code("secrets.dpapi")
}
#[cfg(not(windows))]
fn protect(_: &[u8]) -> Result<Vec<u8>> {
    Err(
        AppError::user("Secure secret storage requires Windows DPAPI on this build.")
            .with_code("secrets.unsupported"),
    )
}
#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Result<Vec<u8>> {
    Err(
        AppError::user("Secure secret storage requires Windows DPAPI on this build.")
            .with_code("secrets.unsupported"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_values_are_validated() {
        for name in ["assistant.key", "a", "a-b_c.1"] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
        for name in ["", "Upper", "space name", ".hidden", "a/b", &"a".repeat(65)] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        assert!(validate_value("").is_err());
        assert!(validate_value("key\nvalue").is_ok());
        assert!(validate_value(&"a".repeat(MAX_VALUE + 1)).is_err());
    }
    #[test]
    fn hex_roundtrips() {
        let bytes = vec![0, 1, 15, 16, 254, 255];
        assert_eq!(unhex(&hex(&bytes)), Some(bytes));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrips_and_is_not_plaintext() {
        let secret = "sk-test-value-1234";
        let encrypted = protect(secret.as_bytes()).unwrap();
        assert_ne!(encrypted, secret.as_bytes());
        assert_eq!(unprotect(&encrypted).unwrap(), secret.as_bytes());
    }
}
