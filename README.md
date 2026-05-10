# ichr

Terminal-UI Minecraft Java Edition launcher for Linux and Windows.

Create instances, install modloaders, browse and install mods from
Modrinth, import modpacks, manage resource and shader packs, launch
Minecraft -- without leaving the terminal.

```
┌─ ichr ────────────────────────────────────────────────┐
│ Instances                                             │
│   ► fabric-1.20.4         vanilla -> Fabric 0.16      │
│     forge-1.20.1          Forge 47.4.10               │
│     vanilla-1.21.4        latest release              │
│                                                       │
│ c new   Enter launch   L loader   M mods   d delete   │
│ A accounts   J java   R packs   q quit                │
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
- **Customizable**: rebind keys and re-skin the color palette via
  `~/.config/ichr/config.toml`. See [`docs/config.md`](docs/config.md).

## Status

**v0.1.0** -- first public release. The core path (create instance, install
modloader, install mods from Modrinth, launch) works end-to-end on Linux.

**Microsoft Account sign-in works out of the box.** ichr's Azure AD app
was approved by Mojang Enforcement and the production AppID is embedded
in the binary, matching the convention used by every major OSS launcher
(PrismLauncher, HMCL, ATLauncher). Press `A` to add an account and
follow the device-code prompt -- no environment variables, no Azure
portal trips. The AppID is a public-client identifier (no secret) and
authentication still requires explicit per-user consent at
microsoft.com/link.

**Forks and downstream redistributions** must register their own Azure
AD app and override via `ICHR_MSA_CLIENT_ID` -- per the Minecraft Usage
Guidelines each launcher needs its own approved AppID. Reusing the
upstream ichr AppID for a fork attributes that fork's traffic to ichr
and risks the AppID being revoked project-wide. See
[`docs/msa-setup.md`](docs/msa-setup.md) for the registration
walkthrough.

CurseForge integration is in the codebase but disabled by default in v1 --
it requires a CurseForge API key that must be obtained from
console.curseforge.com.

## Install

### From source

```bash
cargo install --git https://github.com/Lucasldab/ichr
```

Requires the Rust toolchain (>= 1.90; pinned in `rust-toolchain.toml`).

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

None for end users -- launch and press `A` to sign in. The launcher
writes its data to platform-standard directories:

| Platform | Data | Config | Cache |
|----------|------|--------|-------|
| Linux | `~/.local/share/ichr` | `~/.config/ichr` | `~/.cache/ichr` |
| Windows | `%APPDATA%\ichr\data` | `%APPDATA%\ichr\config` | `%LOCALAPPDATA%\ichr\cache` |

## Customization

Drop a `config.toml` into the platform's config directory above to
rebind keys or re-skin the color palette. The file is optional;
missing or malformed files fall back to defaults with a warning in
`ichr.log`.

```toml
[colors]
accent      = "lightcyan"      # focused borders, active filter chips
dim         = "#444444"        # placeholders, inactive borders
selected_bg = "#003366"

[keybinds]
quit                 = "Q"     # uppercase letter implies Shift
open_loader_picker   = "Ctrl+L"
browser_begin_search = "?"
```

Schema, accepted color names + hex literals, the keybind wire format
(modifier syntax, named keys, the "uppercase implies Shift" rule),
and an example dark-theme config live in
[`docs/config.md`](docs/config.md). Coverage is incremental: the
most-visible surfaces (browser search bars, instance-list keys, key
hint strings) read from the config today; modal chrome and per-view
keybinds still use defaults and are migrated as the project evolves.

There is no live reload -- restart `ichr` to pick up edits.

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

- Full keybind + color customization coverage (currently the most-visible
  surfaces; deeper modal chrome still uses defaults)
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
