# Public access (tunnels)

Public access lets friends outside the local network join a Minecraft server.
It is configured once per server and then follows the server lifecycle:
starting the server starts the tunnel, stopping it closes the tunnel.

## Providers

| Provider | Transport | Notes |
| --- | --- | --- |
| LAN only | none | Default. Friends must be on the same network. |
| SakuraFrp | TCP / UDP | Reuses the installed SakuraFrp `frpc.exe`, the access key in the DPAPI vault, and one tunnel per server. |
| playit.gg | TCP / UDP | Downloads and supervises the playit agent; the first run needs one browser authorization. |
| Custom FRP | TCP / UDP | Supervises a user-provided `frpc` executable and config. Credentials stay inside that config. |

`src-tauri/src/public_access.rs` contains the unified layer:

- `TunnelProvider` is the contract every provider implements (`status`,
  `activate`, `deactivate`).
- `PROVIDERS` is the registry consulted at runtime, so a new provider only
  needs an implementation plus a new `Provider` variant.
- `Service` holds the configuration gate, TCP verification cache, error cache,
  and the custom-frp supervisor task.
- The per-server choice is stored in `public-access.json` at the app data
  root. It contains only provider, protocol, and an optional local port;
  secrets never go there.

Servers created before this feature have no entry. Their provider is inferred
from the existing playit/Sakura enabled flags, otherwise LAN is used. Saving
the Public Access form writes an explicit entry.

## Protocol and port

The local port is never hardcoded:

- **TCP** is used for Java Edition. Leaving the local port blank follows the
  server's `server.properties` port, including future changes.
- **UDP** is used for Bedrock. Enter the Bedrock port (normally `19132`); it is
  not the Java server port.
- A tunnel is not started until the local endpoint is ready. TCP readiness is a
  real loopback connection; UDP uses the live server status because UDP has no
  handshake.

## Lifecycle and status

The playit and Sakura workers already run in the background. Once selected,
they start their agent/tunnel only while the server is `Starting` or `Running`
and disconnect when it stops. The public-access supervisor starts and stops a
custom `frpc` for the same conditions.

Phases shown in the UI:

| Phase | Meaning |
| --- | --- |
| `needs_setup` | A provider step is missing (key, authorization, or custom config). |
| `preparing` | Loading the provider. |
| `authorizing` | First-time provider authorization. |
| `downloading` | Fetching a managed component. |
| `ready` | Configured; the server is not running. |
| `waiting` | The server is live but is not listening locally yet. |
| `connecting` | The tunnel is up and the public endpoint is being verified. |
| `online` | The public Minecraft response was verified (or the tunnel is allocated for UDP). |
| `error` | A connection or provider error; retries continue automatically. |

TCP addresses are only offered as "copyable" after a Minecraft status probe
succeeds. Closing the app stops all tunnels.

## Security

- Sakura access keys and the playit agent key are stored in the Windows DPAPI
  vault (`secrets.json`), never in `public-access.json`, logs, command lines,
  or the frontend.
- The playit claim URL contains a one-time claim code. It is shown only while
  authorization is pending.
- Custom FRP configuration files remain on disk; the app stores their paths
  and the public address only.

## Troubleshooting

- **Sakura `needs_setup`**: enter the access key in the Public Access panel;
  install the SakuraFrp launcher so `frpc.exe` is available.
- **Sakura protocol switch fails**: a tunnel created for the other protocol
  cannot be converted. Delete it in the SakuraFrp panel, then save again.
- **playit keeps waiting**: start the server once while authorized; the claim
  link is generated only for a live server.
- **Custom frpc exits**: check the executable and config paths and the
  provider-side tunnel; the supervisor retries while the server is live.
- **Address not verified**: some providers take a few seconds. The panel shows
  `connecting` and keeps probing; copy is enabled only after verification.
