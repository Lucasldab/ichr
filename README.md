# ichr

Terminal-UI Minecraft Java Edition launcher for Linux and Windows.

Create instances, install modloaders, browse and install mods from
Modrinth, import modpacks, manage resource and shader packs, launch
Minecraft -- without leaving the terminal.

```
┌─ ichr ────────────────────────────────────────────────┐
│ Instances                                             │
│   ► fabric-1.20.4         vanilla → Fabric 0.16       │
│     forge-1.20.1          Forge 47.4.10               │
│     vanilla-1.21.4        latest release              │
│                                                       │
│ N new   Enter open   L launch   E edit   D delete     │
│ Q quit                                                │
└───────────────────────────────────────────────────────┘
```

## Features

- **Instances**: per-instance `.minecraft/`, isolated game directories,
  per-instance Java override
- **Modloaders**: Fabric, Quilt, Forge, NeoForge -- installed via the official
  installer JARs
- **Modrinth integration**: search, filter by MC version + loader, install with
  automatic dependency resolution
- **Modpack import**: `.mrpack` (Modrinth)
- **Resource and shader packs**: drop-in install + Modrinth browse-and-install
- **Java runtime management**: auto-resolves Mojang JRE per version, Adoptium
  fallback for unsupported architectures, system-Java override
- **Microsoft Account auth**: device-code OAuth -- no embedded credentials, no
  password handling. Tokens stored in OS keychain (libsecret on Linux,
  DPAPI on Windows) or AES-256-GCM encrypted file fallback.
- **Single binary**: no installer, no runtime dependencies. `cargo install` or
  download a release archive.

## Status

**v0.1.0** -- first public release. The core path (create instance, install
modloader, install mods from Modrinth, launch) works end-to-end on Linux.

**Microsoft Account sign-in currently requires a one-time setup** to register
your own Azure AD app and supply its client ID via environment variable.
This is a transitional requirement: Microsoft retired the legacy shared
client ID that older third-party launchers used, and ichr's own client ID
is pending review on Microsoft's AppID approval list. Once approved, the
client ID will be embedded in the binary and no setup will be required.

See [`docs/msa-setup.md`](docs/msa-setup.md) for the 5-minute walkthrough.

CurseForge integration is in the codebase but disabled by default in v1 --
it requires a CurseForge API key that must be obtained from
console.curseforge.com.

## Install

### From source

```bash
cargo install --git https://github.com/Lucasldab/ichr
```

Requires the Rust toolchain (>= 1.88; pinned in `rust-toolchain.toml`).

### From a release archive

Download from <https://github.com/Lucasldab/ichr/releases/latest>:

- Linux x86_64: `ichr-x86_64-unknown-linux-gnu.tar.xz`
- Windows x86_64: `ichr-x86_64-pc-windows-msvc.zip`

Or use the install scripts:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Lucasldab/ichr/releases/latest/download/ichr-installer.sh | sh
```

```powershell
powershell -c "irm https://github.com/Lucasldab/ichr/releases/latest/download/ichr-installer.ps1 | iex"
```

## First-run setup

Set the Microsoft Account client ID per
[`docs/msa-setup.md`](docs/msa-setup.md):

```bash
export ICHR_MSA_CLIENT_ID=<your-azure-app-client-id>
ichr
```

This is a one-time setup. The launcher writes its data to platform-standard
directories:

| Platform | Data | Config | Cache |
|----------|------|--------|-------|
| Linux | `~/.local/share/ichr` | `~/.config/ichr` | `~/.cache/ichr` |
| Windows | `%APPDATA%\ichr\data` | `%APPDATA%\ichr\config` | `%LOCALAPPDATA%\ichr\cache` |

## Windows long-path support

ichr's binary declares `longPathAware` in its Windows manifest. For the
JVM child process to also benefit, the system must have long paths enabled:

```reg
[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\FileSystem]
"LongPathsEnabled"=dword:00000001
```

Check with:

```powershell
reg query "HKLM\SYSTEM\CurrentControlSet\Control\FileSystem" /v LongPathsEnabled
```

Setting this key requires admin elevation and a reboot. ichr will warn
in `ichr.log` if the key is absent or 0.

## Build from source (for development)

```bash
git clone https://github.com/Lucasldab/ichr
cd ichr
cargo build --release
./target/release/ichr
```

Run the test suite:

```bash
cargo nextest run --profile ci
# or, without nextest:
cargo test
```

## Roadmap

- AppID approval (pending Microsoft review) -> embedded client ID, zero setup
- Configurable keybinds and color theme via `~/.config/ichr/config.toml`
- CurseForge integration enabled (post-API-key bureaucracy)
- macOS support
- ARM64 binaries

## License

Dual-licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)

at your option.

Third-party attributions (bundled assets such as ForgeWrapper) are listed in
[LICENSES.md](LICENSES.md).
