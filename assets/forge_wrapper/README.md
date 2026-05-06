# ForgeWrapper (vendored)

This directory contains a vendored copy of ForgeWrapper, an MS-PL licensed
Java helper used by mineltui to drive the Forge / NeoForge installer JAR
in headless mode (the official Forge installer has no `--installClient`
flag — see `.planning/phases/07-forge-and-neoforge-modloaders/07-RESEARCH.md`
§Critical Finding for the full rationale).

## Vendor metadata

| Field | Value |
|-------|-------|
| Source | https://github.com/ZekerZhayard/ForgeWrapper |
| Release tag | `1.6.0` |
| Release URL | https://github.com/ZekerZhayard/ForgeWrapper/releases/tag/1.6.0 |
| Upstream README (verified) | https://github.com/ZekerZhayard/ForgeWrapper/blob/master/README.md |
| Filename | `ForgeWrapper-mmc4.jar` |
| SHA-256 | `1dabf6d0fdb376fbae0f8db61de17ab73fb0d5b19b104d14d4eb29906a1c2cd6` |
| verified class name | `io.github.zekerzhayard.forgewrapper.installer.Main` |
| License | MS-PL (Microsoft Public License) |
| Pin rationale | Version 1.6.0 is the latest stable release (2024-03-01); widely used by MultiMC, Prism Launcher, and PolyMC. Vendored as `ForgeWrapper-mmc4.jar` to match the plan's intent (mmc4 was the planned tag name; this is the actual latest equivalent). |

**Note:** The original plan referenced a `mmc4` tag, but the upstream repo
uses semantic versions. `ForgeWrapper-1.6.0.jar` is the actual latest stable
release corresponding to the MultiMC integration version; the filename is
kept as `ForgeWrapper-mmc4.jar` per the plan artifact contract.

**The `verified class name` row above contains the exact fully-qualified
class string verified from the upstream JAR contents (confirmed via
`unzip -l ForgeWrapper-1.6.0.jar | grep installer/Main.class`). The
`FORGE_WRAPPER_MAIN_CLASS` constant in `src/loader/forgewrapper.rs` is
initialised to this same string.**

## License notice (MS-PL summary)

ForgeWrapper is distributed under the Microsoft Public License (MS-PL).
Section 3(D) of the MS-PL requires that we include a copy of the license
with our distribution. The full license text is available at the upstream
repository: https://github.com/ZekerZhayard/ForgeWrapper/blob/master/LICENSE

Attribution: ZekerZhayard, https://github.com/ZekerZhayard

## Refresh procedure

1. Visit https://github.com/ZekerZhayard/ForgeWrapper/releases
2. Pick the latest release tag (currently `1.6.0`)
3. Download `ForgeWrapper-<tag>.jar` to this directory (rename to `ForgeWrapper-mmc4.jar`)
4. Update `SHA-256` above and `FORGE_WRAPPER_SHA256` in `src/loader/forgewrapper.rs`
5. Update the `verified class name` row if the main class changed
6. Update `FORGE_WRAPPER_MAIN_CLASS` in `src/loader/forgewrapper.rs`
7. Re-run `cargo test --lib -- loader::forgewrapper::tests` to confirm magic bytes + non-empty
