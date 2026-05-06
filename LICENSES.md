# Third-Party Attributions

This project bundles or links the following third-party software at
distribution time. Each entry lists the upstream project, the version
pinned, the license, and (where applicable) the SHA-256 of the bundled
binary asset.

## ForgeWrapper

- Upstream: https://github.com/ZekerZhayard/ForgeWrapper
- Tag: `1.6.0`
- Bundled binary: `assets/forge_wrapper/ForgeWrapper-mmc4.jar`
- SHA-256: `1dabf6d0fdb376fbae0f8db61de17ab73fb0d5b19b104d14d4eb29906a1c2cd6`
  (matches `FORGE_WRAPPER_SHA256` in `src/loader/forgewrapper.rs` and the
  row in `assets/forge_wrapper/README.md`)
- Verified main class: `io.github.zekerzhayard.forgewrapper.installer.Main`
- License: Microsoft Public License (MS-PL),
  https://github.com/ZekerZhayard/ForgeWrapper/blob/master/LICENSE
- Used by: `src/loader/forgewrapper.rs` to bridge the Forge / NeoForge
  installer's MultiMC headless flow.

MS-PL §3(D) requires us to include this attribution and a copy of the
license terms with our distribution. The full license text is at the
upstream URL above; a copy is also linked from
`assets/forge_wrapper/README.md`.
