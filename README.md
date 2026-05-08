# mineltui

Terminal-UI Minecraft Java Edition launcher (Linux + Windows).

A user can create an instance, install a modloader and mods, and launch a
working modded Minecraft — entirely from the TUI.

## Install

### From source (recommended for v1)

```bash
cargo install --git https://github.com/Lucasldab/mineltui
```

Requires the Rust toolchain (>= 1.88; pinned in `rust-toolchain.toml`).

### From a release binary

Download the appropriate archive from
<https://github.com/Lucasldab/mineltui/releases/latest>:

- Linux x86_64: `mineltui-x86_64-unknown-linux-gnu.tar.gz`
- Windows x86_64: `mineltui-x86_64-pc-windows-msvc.zip`

Or use the install scripts:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Lucasldab/mineltui/releases/latest/download/mineltui-installer.sh | sh
```

```powershell
powershell -c "irm https://github.com/Lucasldab/mineltui/releases/latest/download/mineltui-installer.ps1 | iex"
```

## Windows long-path support

mineltui's binary declares `longPathAware` in its Windows manifest. For the
JVM child process to also benefit, the system must have long paths enabled:

```reg
[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\FileSystem]
"LongPathsEnabled"=dword:00000001
```

Check with:

```powershell
reg query "HKLM\SYSTEM\CurrentControlSet\Control\FileSystem" /v LongPathsEnabled
```

Setting this key requires admin elevation and a reboot. mineltui will warn
in `mineltui.log` if the key is absent or 0.

## License

Dual-licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)

at your option.

Third-party attributions (bundled assets such as ForgeWrapper) are listed in
[LICENSES.md](LICENSES.md).
