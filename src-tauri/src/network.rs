use crate::error::{AppError, Result};
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
    fn occupied_port() {
        let l = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        assert!(super::check_port(l.local_addr().unwrap().port()).is_err());
    }
}
