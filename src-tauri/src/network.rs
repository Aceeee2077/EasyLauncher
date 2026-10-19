use crate::error::{AppError, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkCheck {
    pub port: u16,
    pub lan_ip: Option<String>,
    pub port_free: bool,
    pub firewall_rule: bool,
    pub rule_name: String,
}

pub fn rule_name(port: u16) -> String {
    format!("EasyLauncher-Minecraft-{port}")
}

pub fn port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("0.0.0.0", port)).is_ok()
}

#[cfg(windows)]
pub fn firewall_rule(port: u16) -> bool {
    let name = rule_name(port);
    std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &format!("name={name}"),
            "dir=in",
        ])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&name))
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn firewall_rule(port: u16) -> bool {
    let _ = port;
    false
}

#[cfg(windows)]
pub fn allow_firewall(port: u16) -> Result<()> {
    let name = rule_name(port);
    let script = format!(
        "Start-Process -FilePath netsh -Verb RunAs -ArgumentList 'advfirewall firewall add rule name={name} dir=in action=allow protocol=TCP localport={port}'"
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .spawn()
        .map_err(|e| {
            AppError::new(
                "Could not request administrator permission to add the firewall rule.",
                e,
            )
        })?;
    Ok(())
}

#[cfg(not(windows))]
pub fn allow_firewall(port: u16) -> Result<()> {
    let _ = port;
    Err(AppError::user(
        "Firewall rules can only be managed on Windows.",
    ))
}

pub fn check_port(port: u16) -> Result<()> {
    let _listener=std::net::TcpListener::bind(("0.0.0.0",port)).map_err(|e|AppError::new(format!("Port {port} is already being used or unavailable. Choose another port or stop the application using it."),e))?;
    Ok(())
}
pub fn lan_address() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}
#[cfg(test)]
mod tests {
    #[test]
    fn firewall_rule_name_is_stable() {
        assert_eq!(super::rule_name(25565), "EasyLauncher-Minecraft-25565");
        let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!super::port_free(port));
        drop(listener);
        assert!(super::port_free(port));
    }

    #[test]
    fn occupied_port() {
        let l = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        assert!(super::check_port(l.local_addr().unwrap().port()).is_err());
    }
}
