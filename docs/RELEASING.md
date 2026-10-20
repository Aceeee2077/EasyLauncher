# Signed releases and update channels

EasyLauncher checks a signed manifest over HTTPS, downloads the installer into
its app data folder, verifies the signed size and SHA-256, and opens the
installer only after an explicit user action. It never installs silently and
never runs system package managers.

## Trust model

- `src-tauri/updater.pub` is compiled into the app: one 32-byte Ed25519 public
  key as hex. It is the root of trust for updates.
- The channel URL must be a direct HTTPS URL without a query or fragment. The
  app reads `latest.json` from that URL and the detached signature from
  `<url>.sig` (hex, 64 bytes).
- The signature is checked over the exact manifest bytes **before** the JSON is
  parsed. Asset URLs must be HTTPS, the size is bounded, the SHA-256 must be
  well formed, and versions are strict `major.minor.patch` numbers. A manifest
  can never trigger a downgrade.
- The installer itself is not Authenticode-signed by this pipeline. The
  manifest signature protects the URL, size and checksum, but Windows
  SmartScreen can still warn about an unsigned executable. Add a code-signing
  certificate to the build if you distribute publicly.

## One-time setup

The repository currently contains a development key pair generated on the
maintainer's machine:

- public key: `src-tauri/updater.pub` (committed)
- private key: `src-tauri/updater-signing-key.pem` (gitignored, **not** in the
  repository)

Treat the private key as a release secret:

1. Copy the contents of `src-tauri/updater-signing-key.pem` into the
   `UPDATER_SIGNING_KEY` repository secret on GitHub.
2. Store a backup somewhere safe and delete the local copy when you no longer
   build releases locally.
3. Never commit it. `.gitignore` already excludes the file.

To rotate the key, generate a new pair and replace the public key in the same
commit that updates the secret. **Old installations will reject manifests
signed by the new key**, so rotation needs a final release signed with the old
key whose notes tell users to install once manually.

```sh
openssl genpkey -algorithm ED25519 -out src-tauri/updater-signing-key.pem
openssl pkey -in src-tauri/updater-signing-key.pem -pubout -outform DER \
  | tail -c 32 | xxd -p -c 64 > src-tauri/updater.pub
```

## Publishing

1. Bump `version` in `package.json`, `src-tauri/tauri.conf.json` and
   `src-tauri/Cargo.toml`. All three must match.
2. Commit the change.
3. In GitHub Actions, run **Signed release (manual only)** with the version
   (for example `0.2.0`) and release notes.
4. The workflow builds the NSIS installer, writes `latest.json`, signs it with
   `UPDATER_SIGNING_KEY`, verifies the signature against `updater.pub`, creates
   the `v0.2.0` release and uploads the installer, manifest and signature.

The workflow is `workflow_dispatch` only: pushes, pull requests and schedules
never publish anything.

## Channel URL

Use the stable latest-release asset URL:

```
https://github.com/Aceeee2077/EasyLauncher/releases/latest/download/latest.json
```

Paste it into **Settings → Application updates → Manifest URL**. The signature
is fetched from the same URL plus `.sig`. Enabling *Check silently at startup*
shows a banner when a newer version exists; opening Settings offers the
download and the installer button. Clearing the URL disables checks.

## Limitations

- Only the Windows x86_64 installer is built. Linux and macOS builds remain
  unverified, and a manifest without an asset for the current platform is
  shown as unsupported rather than downloaded.
- Download resumption is not implemented; an interrupted download starts over.
- Check and download require network access and are never part of the default
  test suite.
- The installer replaces the app through NSIS. The user must close EasyLauncher
  first; the updater does not orchestrate a restart.
