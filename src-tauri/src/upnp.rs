use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};

const MAX_DESCRIPTION: usize = 512 * 1024;
const DESCRIPTION_FILE: &str = "network.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub enabled: bool,
    pub external_port: u16,
    pub internal_port: u16,
    pub internal_ip: String,
    pub public_ip: Option<String>,
    pub router: String,
    pub mapped_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkReport {
    pub lan_ip: Option<String>,
    pub public_ip: Option<String>,
    pub router: Option<String>,
    pub igd_found: bool,
    pub mapping: Option<PortMapping>,
    pub address: Option<String>,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    control: String,
    service: String,
    router: String,
}

pub async fn report(manager: &Manager, id: &str) -> Result<NetworkReport> {
    let server = manager.metadata(id)?;
    let lan_ip = crate::network::lan_address();
    let stored = read_mapping(manager, id);
    let gateway = discover().await;
    let mut public_ip = None;
    let mut router = None;
    if let Some(gateway) = &gateway {
        router = Some(gateway.router.clone());
        let client = local_client()?;
        public_ip = external_ip(&client, gateway).await.ok();
    }
    let address = match (&public_ip, &stored) {
        (Some(ip), Some(mapping)) if mapping.enabled => {
            Some(format!("{ip}:{}", mapping.external_port))
        }
        _ => None,
    };
    let mapping = stored.map(|mut m| {
        if let Some(ip) = &public_ip {
            m.public_ip = Some(ip.clone());
        }
        m
    });
    Ok(NetworkReport {
        lan_ip,
        public_ip,
        router,
        igd_found: gateway.is_some(),
        mapping,
        address,
        port: server.port,
    })
}

pub async fn map(manager: &Manager, id: &str, external_port: Option<u16>) -> Result<NetworkReport> {
    let server = manager.metadata(id)?;
    let lan_ip = crate::network::lan_address().ok_or_else(|| {
        AppError::user(
            "Could not detect this computer's local network address. Connect to Wi-Fi or Ethernet and try again.",
        )
    })?;
    let gateway = discover().await.ok_or_else(|| {
        AppError::user(
            "No UPnP router was found. Enable UPnP on your router, or use a tunnel service such as playit.gg.",
        )
    })?;
    let client = local_client()?;
    let external = external_port.unwrap_or(server.port);
    if external < 1024 && external != server.port {
        return Err(AppError::user(
            "Choose an external port between 1024 and 65535.",
        ));
    }
    let previous = read_mapping(manager, id);
    if let Some(old) = &previous {
        if old.enabled && old.external_port != external {
            let _ = remove_mapping(&client, &gateway, old.external_port).await;
        }
    }
    add_mapping(&client, &gateway, external, server.port, &lan_ip).await?;
    let public_ip = external_ip(&client, &gateway).await.ok();
    let mapping = PortMapping {
        enabled: true,
        external_port: external,
        internal_port: server.port,
        internal_ip: lan_ip.clone(),
        public_ip: public_ip.clone(),
        router: gateway.router.clone(),
        mapped_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    write_mapping(manager, id, &mapping)?;
    Ok(NetworkReport {
        lan_ip: Some(lan_ip),
        public_ip: public_ip.clone(),
        router: Some(gateway.router),
        igd_found: true,
        mapping: Some(mapping),
        address: public_ip.map(|ip| format!("{ip}:{external}")),
        port: server.port,
    })
}

pub async fn unmap(manager: &Manager, id: &str) -> Result<NetworkReport> {
    manager.metadata(id)?;
    let previous = read_mapping(manager, id);
    if let Some(old) = &previous {
        if old.enabled {
            if let Some(gateway) = discover().await {
                let client = local_client()?;
                let _ = remove_mapping(&client, &gateway, old.external_port).await;
            }
        }
    }
    if let Some(mut old) = previous {
        old.enabled = false;
        old.public_ip = None;
        old.mapped_at = None;
        write_mapping(manager, id, &old)?;
    }
    report(manager, id).await.map(|r| NetworkReport {
        mapping: read_mapping(manager, id),
        address: None,
        ..r
    })
}

fn local_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| AppError::new("Could not prepare the network request.", e))
}

fn mapping_path(manager: &Manager, id: &str) -> Result<PathBuf> {
    Ok(storage::child(&manager.dir(id)?, DESCRIPTION_FILE)?)
}

fn read_mapping(manager: &Manager, id: &str) -> Option<PortMapping> {
    let path = mapping_path(manager, id).ok()?;
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The mapping this app last stored for the server, used by the health check.
pub fn mapping(manager: &Manager, id: &str) -> Option<PortMapping> {
    read_mapping(manager, id)
}

fn write_mapping(manager: &Manager, id: &str, mapping: &PortMapping) -> Result<()> {
    storage::write_json(&mapping_path(manager, id)?, mapping)
}

async fn discover() -> Option<Gateway> {
    let locations = tokio::task::spawn_blocking(|| {
        let mut found = search(
            "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
            Duration::from_millis(1500),
        );
        if found.is_empty() {
            found = search("ssdp:all", Duration::from_millis(900));
        }
        found
    })
    .await
    .ok()?;
    let client = local_client().ok()?;
    for location in locations {
        if let Some(gateway) = describe(&client, &location).await {
            return Some(gateway);
        }
    }
    None
}

fn search(service: &str, timeout: Duration) -> Vec<String> {
    let socket = match UdpSocket::bind(("0.0.0.0", 0)) {
        Ok(socket) => socket,
        Err(_) => return Vec::new(),
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = socket.set_write_timeout(Some(Duration::from_millis(500)));
    let target: SocketAddr = match "239.255.255.250:1900".parse() {
        Ok(target) => target,
        Err(_) => return Vec::new(),
    };
    let message = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {service}\r\n\r\n"
    );
    let started = Instant::now();
    let deadline = started + timeout;
    let mut locations: Vec<String> = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut last_probe = started;
    while Instant::now() < deadline {
        if last_probe.elapsed() >= Duration::from_millis(600) {
            last_probe = Instant::now();
            let _ = socket.send_to(message.as_bytes(), target);
        }
        match socket.recv_from(&mut buffer) {
            Ok((read, _)) => {
                let text = String::from_utf8_lossy(&buffer[..read]);
                if let Some(location) = header(&text, "location") {
                    if !locations.iter().any(|item| item == &location) {
                        locations.push(location);
                    }
                    if locations.len() >= 8 {
                        break;
                    }
                }
            }
            Err(_) => continue,
        }
    }
    locations
}

fn header(message: &str, name: &str) -> Option<String> {
    message.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case(name) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

async fn describe(client: &reqwest::Client, location: &str) -> Option<Gateway> {
    if !(location.starts_with("http://") || location.starts_with("https://")) {
        return None;
    }
    let response = client.get(location).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    if text.len() > MAX_DESCRIPTION {
        return None;
    }
    let router = tag(&text, "friendlyName").unwrap_or_else(|| "UPnP router".into());
    for service in blocks(&text, "service") {
        let Some(kind) = tag(&service, "serviceType") else {
            continue;
        };
        if !(kind.contains("WANIPConnection:1") || kind.contains("WANPPPConnection:1")) {
            continue;
        }
        let control = tag(&service, "controlURL")?;
        return Some(Gateway {
            control: resolve(location, &control),
            service: kind,
            router,
        });
    }
    None
}

async fn external_ip(client: &reqwest::Client, gateway: &Gateway) -> Result<String> {
    let text = soap(client, gateway, "GetExternalIPAddress", "").await?;
    tag(&text, "NewExternalIPAddress")
        .filter(|ip| {
            Ipv4Addr::from_str(ip)
                .map(|v| !v.is_unspecified())
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            AppError::user(
                "The router did not report a public IP address. Check the WAN connection.",
            )
        })
}

async fn add_mapping(
    client: &reqwest::Client,
    gateway: &Gateway,
    external: u16,
    internal: u16,
    ip: &str,
) -> Result<()> {
    let body = format!(
        "<NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{external}</NewExternalPort>\
         <NewProtocol>TCP</NewProtocol>\
         <NewInternalPort>{internal}</NewInternalPort>\
         <NewInternalClient>{ip}</NewInternalClient>\
         <NewEnabled>1</NewEnabled>\
         <NewPortMappingDescription>EasyLauncher</NewPortMappingDescription>\
         <NewLeaseDuration>0</NewLeaseDuration>"
    );
    soap(client, gateway, "AddPortMapping", &body).await?;
    Ok(())
}

async fn remove_mapping(client: &reqwest::Client, gateway: &Gateway, external: u16) -> Result<()> {
    let body = format!(
        "<NewRemoteHost></NewRemoteHost>\
         <NewExternalPort>{external}</NewExternalPort>\
         <NewProtocol>TCP</NewProtocol>"
    );
    soap(client, gateway, "DeletePortMapping", &body).await?;
    Ok(())
}

async fn soap(
    client: &reqwest::Client,
    gateway: &Gateway,
    action: &str,
    body: &str,
) -> Result<String> {
    let envelope = format!(
        "<?xml version=\"1.0\"?>\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:{action} xmlns:u=\"{}\">{body}</u:{action}></s:Body></s:Envelope>",
        gateway.service
    );
    let response = client
        .post(&gateway.control)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("SOAPACTION", format!("\"{}#{action}\"", gateway.service))
        .body(envelope)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if status.is_success() {
        return Ok(text);
    }
    let code = tag(&text, "errorCode").unwrap_or_default();
    let message = match code.as_str() {
        "718" => "This external port is already mapped by another device. Choose a different port.",
        "724" => "The router cannot forward to this computer. Check that the local IP still belongs to it.",
        "725" => "The router only accepts mappings created by the computer that requests them.",
        "714" => "The router does not allow UPnP port mappings. Enable UPnP in the router settings.",
        _ => "The router rejected the port mapping request.",
    };
    Err(AppError::new(
        message,
        format!(
            "HTTP {status}; UPnP error {code}; service {}",
            gateway.service
        ),
    ))
}

fn tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(start) = xml[from..].find(&open) {
        let start = from + start;
        let after = start + open.len();
        let boundary = xml[after..].chars().next();
        if !matches!(
            boundary,
            Some('>') | Some(' ') | Some('\t') | Some('\r') | Some('\n') | Some('/')
        ) {
            from = after;
            continue;
        }
        let content_start = xml[after..].find('>')? + after + 1;
        let close = format!("</{name}>");
        let content_end = xml[content_start..].find(&close)? + content_start;
        return Some(unescape(xml[content_start..content_end].trim()));
    }
    None
}

fn blocks(xml: &str, name: &str) -> Vec<String> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut result = Vec::new();
    let mut from = 0;
    while let Some(start) = xml[from..].find(&open) {
        let start = from + start;
        let after = start + open.len();
        let boundary = xml[after..].chars().next();
        if !matches!(
            boundary,
            Some('>') | Some(' ') | Some('\t') | Some('\r') | Some('\n') | Some('/')
        ) {
            from = after;
            continue;
        }
        let Some(content_start) = xml[start..].find('>').map(|i| start + i + 1) else {
            break;
        };
        let Some(end) = xml[content_start..].find(&close).map(|i| content_start + i) else {
            break;
        };
        result.push(xml[content_start..end].to_string());
        from = end + close.len();
    }
    result
}

fn resolve(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    let origin = base
        .split_once("://")
        .and_then(|(scheme, rest)| {
            let host = rest.split('/').next().unwrap_or("");
            if host.is_empty() {
                None
            } else {
                Some(format!("{scheme}://{host}"))
            }
        })
        .unwrap_or_default();
    if relative.starts_with('/') {
        format!("{origin}{relative}")
    } else {
        format!("{origin}/{relative}")
    }
}

fn unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESCRIPTION: &str = r#"<?xml version="1.0"?>
<root>
  <device>
    <friendlyName>Home Router &amp; Gateway</friendlyName>
    <deviceList>
      <device>
        <serviceList>
          <service>
            <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
            <controlURL>/upnp/control/WANIPConn1</controlURL>
          </service>
          <service>
            <serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>
            <controlURL>/upnp/control/L3F</controlURL>
          </service>
        </serviceList>
      </device>
    </deviceList>
  </device>
</root>"#;

    #[test]
    fn gateway_service_is_detected() {
        let services = blocks(DESCRIPTION, "service");
        assert_eq!(services.len(), 2);
        let control = tag(&services[0], "controlURL").unwrap();
        assert_eq!(
            resolve("http://192.168.1.1:5000/root.xml", &control),
            "http://192.168.1.1:5000/upnp/control/WANIPConn1"
        );
        assert_eq!(
            tag(DESCRIPTION, "friendlyName").unwrap(),
            "Home Router & Gateway"
        );
    }

    #[test]
    fn relative_control_urls_are_resolved() {
        assert_eq!(
            resolve("http://10.0.0.1:49152/desc.xml", "ctl/IPConn"),
            "http://10.0.0.1:49152/ctl/IPConn"
        );
        assert_eq!(
            resolve("http://10.0.0.1:49152/desc.xml", "http://10.0.0.1/other"),
            "http://10.0.0.1/other"
        );
    }

    #[test]
    fn headers_and_soap_errors_parse() {
        let response = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.0.1:1900/rootDesc.xml\r\nST: upnp:rootdevice\r\n\r\n";
        assert_eq!(
            header(response, "location").as_deref(),
            Some("http://192.168.0.1:1900/rootDesc.xml")
        );
        let fault = r#"<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>718</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#;
        assert_eq!(tag(fault, "errorCode").as_deref(), Some("718"));
    }
}
