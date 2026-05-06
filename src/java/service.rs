//! Java runtime facade — single entry point for JRE resolution, installation,
//! system detection, and per-instance override persistence.
//!
//! # Precedence order for `resolve_jre_for_launch`
//!
//! 1. `MINELTUI_JAVA` env var — debug escape hatch; logs WARN, skips validation.
//! 2. `instance.java_override` — per-instance override (install-if-missing).
//! 3. Auto-resolve Mojang via `version.java_version.component`.
//! 4. Adoptium fallback (Mojang absent for this platform/variant).
//!
//! Validation of System-override major version is folded into step 2.

use std::path::PathBuf;

use crate::domain::instance::InstanceManifest;
use crate::domain::platform::{Arch, OsName};
use crate::error::AppError;
use crate::java::adoptium::AdoptiumClient;
use crate::java::detect::{scan_system_javas, SystemJava};
use crate::java::mapping::{mojang_platform_key, validate_java_major};
use crate::java::mojang_jre::MojangJreClient;
use crate::java::types::JavaRuntimeId;
use crate::mojang::types::VersionJson;
use crate::persistence::paths::AppPaths;

// ---------------------------------------------------------------------------
// JavaService
// ---------------------------------------------------------------------------

/// Facade over the Mojang JRE client, Adoptium client, and system Java
/// detector. Owns both HTTP clients and the platform identity.
pub struct JavaService {
    mojang: MojangJreClient,
    adoptium: AdoptiumClient,
    arch: Arch,
    os: OsName,
}

impl JavaService {
    /// Construct with production base URLs (reads env-var overrides internally).
    #[tracing::instrument(skip_all)]
    pub fn new() -> Result<Self, AppError> {
        Ok(Self {
            mojang: MojangJreClient::new()?,
            adoptium: AdoptiumClient::new()?,
            arch: Arch::current(),
            os: OsName::current(),
        })
    }

    /// Construct with explicit clients — used in tests to inject httpmock
    /// servers without touching environment variables.
    #[cfg(test)]
    pub fn with_clients(
        mojang: MojangJreClient,
        adoptium: AdoptiumClient,
        arch: Arch,
        os: OsName,
    ) -> Self {
        Self { mojang, adoptium, arch, os }
    }

    // -----------------------------------------------------------------------
    // Primary resolver
    // -----------------------------------------------------------------------

    /// Resolve the java executable path for launching `instance` with `version`.
    ///
    /// Precedence (first match wins):
    ///
    /// 1. `MINELTUI_JAVA` env var — bypass all managed JRE logic (logs WARN).
    /// 2. `instance.java_override` — per-instance override (install-if-missing).
    /// 3. Auto Mojang — uses `version.java_version.component` from the MC manifest.
    /// 4. Adoptium fallback — when Mojang has no entry for the current platform.
    #[tracing::instrument(skip_all, fields(slug = %instance.slug))]
    pub async fn resolve_jre_for_launch(
        &self,
        paths: &AppPaths,
        instance: &InstanceManifest,
        version: &VersionJson,
    ) -> Result<PathBuf, AppError> {
        // Step 1 — MINELTUI_JAVA debug override (wins unconditionally; the
        // early return makes all subsequent steps unreachable when the var
        // is set — semantically equivalent to "checked last as bypass-all").
        if let Ok(p) = std::env::var("MINELTUI_JAVA") {
            tracing::warn!(
                path = %p,
                "MINELTUI_JAVA overrides JRE resolution — validation skipped"
            );
            return Ok(PathBuf::from(p));
        }

        // Derive required major and component hint from the version manifest.
        // Pre-1.17 MC manifests lack the `javaVersion` field → default to
        // "jre-legacy" / Java 8 (research §Fallback inference).
        let (component_hint, required_major) = match &version.java_version {
            Some(jv) => (jv.component.clone(), jv.major_version),
            None => ("jre-legacy".to_string(), 8u32),
        };

        // Step 2 — per-instance override (install-if-missing).
        if let Some(id) = &instance.java_override {
            return self.resolve_override(paths, id, required_major).await;
        }

        // Step 3 — auto Mojang: look up platform key then the component variant.
        if let Some(plat) = mojang_platform_key(self.os, self.arch) {
            let index = self.mojang.fetch_all_json(None).await?;
            if let Some(variant) = MojangJreClient::select_variant(&index, plat, &component_hint) {
                let exe = self
                    .mojang
                    .install_mojang_variant(paths, variant, &component_hint)
                    .await?;
                return Ok(exe);
            }
        }

        // Step 4 — Adoptium fallback (Mojang has no entry for this platform).
        let exe = self
            .adoptium
            .install_adoptium(paths, required_major, self.arch, self.os)
            .await?;
        Ok(exe)
    }

    // -----------------------------------------------------------------------
    // Override resolver (step 2 branch)
    // -----------------------------------------------------------------------

    async fn resolve_override(
        &self,
        paths: &AppPaths,
        id: &JavaRuntimeId,
        required_major: u32,
    ) -> Result<PathBuf, AppError> {
        match id {
            JavaRuntimeId::Mojang { variant } => {
                // Requires a Mojang platform entry — fall through error if none.
                let plat = mojang_platform_key(self.os, self.arch)
                    .ok_or(AppError::JavaNotFound)?;
                let index = self.mojang.fetch_all_json(None).await?;
                let entry =
                    MojangJreClient::select_variant(&index, plat, variant)
                        .ok_or(AppError::JavaNotFound)?;
                self.mojang.install_mojang_variant(paths, entry, variant).await
            }

            JavaRuntimeId::Adoptium { major } => {
                self.adoptium
                    .install_adoptium(paths, *major, self.arch, self.os)
                    .await
            }

            JavaRuntimeId::System { path, major_version } => {
                // System path must exist and pass major-version validation.
                if !path.is_file() {
                    return Err(AppError::JavaNotFound);
                }
                validate_java_major(*major_version, required_major, path)?;
                Ok(path.clone())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Installation helpers (TUI "download JRE" actions)
    // -----------------------------------------------------------------------

    /// Install a Mojang JRE variant eagerly.
    ///
    /// Delegates to `MojangJreClient::install_mojang_variant` after fetching
    /// the all.json index and selecting the platform+component entry.
    #[tracing::instrument(skip_all, fields(component))]
    pub async fn install_mojang(
        &self,
        paths: &AppPaths,
        component: &str,
    ) -> Result<PathBuf, AppError> {
        let plat = mojang_platform_key(self.os, self.arch).ok_or(AppError::JavaNotFound)?;
        let index = self.mojang.fetch_all_json(None).await?;
        let entry = MojangJreClient::select_variant(&index, plat, component)
            .ok_or(AppError::JavaNotFound)?;
        self.mojang.install_mojang_variant(paths, entry, component).await
    }

    /// Install an Adoptium JRE for the given major version.
    #[tracing::instrument(skip_all, fields(major))]
    pub async fn install_adoptium(
        &self,
        paths: &AppPaths,
        major: u32,
    ) -> Result<PathBuf, AppError> {
        self.adoptium.install_adoptium(paths, major, self.arch, self.os).await
    }

    // -----------------------------------------------------------------------
    // Listing helpers (TUI picker)
    // -----------------------------------------------------------------------

    /// Scan PATH and common install locations for working system Java binaries.
    ///
    /// Returns a deduplicated list sorted by detection order. Slow candidates
    /// (> 5 s `java -version` timeout) are silently skipped.
    #[tracing::instrument(skip_all)]
    pub async fn list_system_javas(&self) -> Vec<SystemJava> {
        scan_system_javas().await
    }

    // -----------------------------------------------------------------------
    // Override persistence
    // -----------------------------------------------------------------------

    /// Atomically write `override_id` (or clear it if `None`) to the instance
    /// manifest at `paths.instance_manifest(slug)`.
    #[tracing::instrument(skip_all, fields(slug, override_id = ?override_id))]
    pub async fn set_override_for_instance(
        &self,
        paths: &AppPaths,
        slug: &str,
        override_id: Option<JavaRuntimeId>,
    ) -> Result<(), AppError> {
        crate::instance::store::set_java_override(paths, slug, override_id)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Install-time JRE resolver (D-06)
    // -----------------------------------------------------------------------

    /// Resolve a JRE path for installer subprocess use, given only the MC
    /// version string. Loads the vanilla version JSON from
    /// `paths.version_json(mc_version)` and delegates to `resolve_jre_for_launch`
    /// with a synthesized stub `InstanceManifest`.
    ///
    /// Per 07-CONTEXT.md decision D-06: the same JRE that runs Minecraft for
    /// MC version X.Y.Z is sufficient for running the Forge/NeoForge installer
    /// targeting that MC version (researcher cross-checked Forge installer
    /// JDK requirements against per-MC runtime JDKs for every era 1.13+).
    ///
    /// Errors:
    /// - `AppError::VersionNotInstalled { slug }` when the vanilla version JSON
    ///   is not yet on disk (caller must install vanilla MC first).
    /// - `AppError::Io(_)` when the file exists but fails to read.
    /// - `AppError::MojangParse(_)` when the file is malformed JSON.
    /// - Whatever `resolve_jre_for_launch` returns for unresolvable JRE.
    #[tracing::instrument(skip_all, fields(mc_version = %mc_version))]
    pub async fn resolve_jre_for_mc_version_install(
        &self,
        paths: &AppPaths,
        mc_version: &str,
    ) -> Result<PathBuf, AppError> {
        use crate::domain::instance::InstanceManifest;
        use crate::error::AppError;

        let json_path = paths.version_json(mc_version);
        // 1) Vanilla version JSON must be on disk; otherwise surface a
        //    typed "install vanilla first" error using the existing variant.
        if !tokio::fs::try_exists(&json_path).await.unwrap_or(false) {
            return Err(AppError::VersionNotInstalled {
                slug: mc_version.to_string(),
            });
        }
        // 2) Read + parse — `?` auto-wraps via #[from] on AppError::Io and
        //    AppError::MojangParse. No string-based fallback variants.
        let bytes = tokio::fs::read(&json_path).await?;
        let version_json: crate::mojang::types::VersionJson =
            serde_json::from_slice(&bytes)?;

        // 3) Synthesize a stub instance manifest — no overrides, drives the
        //    standard precedence chain inside resolve_jre_for_launch.
        let stub = InstanceManifest::new(
            "loader-install".into(),
            "loader-install".into(),
            mc_version.to_string(),
        );
        self.resolve_jre_for_launch(paths, &stub, &version_json).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::instance::InstanceManifest;
    use crate::instance::store::{read_instance_manifest, write_instance_manifest};
    use crate::java::adoptium::AdoptiumClient;
    use crate::java::mojang_jre::MojangJreClient;
    use crate::mojang::types::{
        AssetIndex, VersionDownloads, VersionJson, JavaVersion,
    };
    use httpmock::MockServer;
    use httpmock::Method::GET;
    use std::sync::OnceLock;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    // Global async-aware mutex serialising all tests that read or write
    // MINELTUI_JAVA. tokio::sync::Mutex is held across await points safely
    // and passes clippy::await_holding_lock because it is the async variant.
    fn java_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_paths(td: &TempDir) -> AppPaths {
        AppPaths::with_roots(
            td.path().to_path_buf(),
            td.path().to_path_buf(),
            td.path().to_path_buf(),
        )
    }

    fn minimal_version(component: &str, major: u32) -> VersionJson {
        VersionJson {
            id: "1.21.4".into(),
            version_type: "release".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index: AssetIndex {
                id: "17".into(),
                sha1: "aaaa".into(),
                size: 0,
                total_size: 0,
                url: "http://example.com/assets.json".into(),
            },
            assets: "17".into(),
            downloads: VersionDownloads::default(),
            libraries: vec![],
            java_version: Some(JavaVersion {
                component: component.into(),
                major_version: major,
            }),
            logging: None,
            compliance_level: None,
            minimum_launcher_version: None,
            release_time: "2024-01-01T00:00:00Z".into(),
            time: "2024-01-01T00:00:00Z".into(),
            arguments: None,
            minecraft_arguments: None,
            inherits_from: None,
        }
    }

    fn version_without_java_version() -> VersionJson {
        VersionJson {
            id: "1.16.5".into(),
            version_type: "release".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index: AssetIndex {
                id: "1.16".into(),
                sha1: "aaaa".into(),
                size: 0,
                total_size: 0,
                url: "http://example.com/assets.json".into(),
            },
            assets: "1.16".into(),
            downloads: VersionDownloads::default(),
            libraries: vec![],
            java_version: None,
            logging: None,
            compliance_level: None,
            minimum_launcher_version: None,
            release_time: "2021-01-15T00:00:00Z".into(),
            time: "2021-01-15T00:00:00Z".into(),
            arguments: None,
            minecraft_arguments: Some("-Xmx1G".into()),
            inherits_from: None,
        }
    }

    fn instance_no_override(slug: &str) -> InstanceManifest {
        InstanceManifest::new(slug.into(), slug.into(), "1.21.4".into())
    }

    fn fixture_bytes() -> &'static [u8] {
        b"fixture-java-bin\n"
    }

    fn make_manifest_body(server_base: &str, _component: &str) -> String {
        let sha1 = crate::mojang::cache::sha1_hex_of_bytes(fixture_bytes());
        let size = fixture_bytes().len();
        // Minimal manifest with one file entry (bin/java)
        format!(
            r#"{{
  "files": {{
    "bin": {{ "type": "directory" }},
    "bin/java": {{
      "type": "file",
      "executable": true,
      "downloads": {{
        "raw": {{ "sha1": "{sha1}", "size": {size}, "url": "{server_base}/bin/java" }}
      }}
    }}
  }}
}}"#,
        )
    }

    fn make_all_json_body(server_base: &str, component: &str, manifest_sha1: &str) -> String {
        format!(
            r#"{{
  "linux": {{
    "{component}": [
      {{
        "manifest": {{ "sha1": "{manifest_sha1}", "size": 100, "url": "{server_base}/manifest-{component}.json" }},
        "version":  {{ "name": "21.0.7", "released": "2025-05-19T00:00:00Z" }},
        "availability": {{ "group": 1, "progress": 100 }}
      }}
    ]
  }}
}}"#,
        )
    }

    /// Build a synthetic tar.gz with a single `bin/java` file for Adoptium tests.
    #[cfg(unix)]
    fn make_adoptium_tar_gz(content: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut enc);
            // Top-level prefix dir (stripped by extract_tar_gz_blocking)
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "jdk-21.0.7-jre/", std::io::empty())
                .unwrap();
            // bin/ dir
            let mut header2 = tar::Header::new_gnu();
            header2.set_size(0);
            header2.set_entry_type(tar::EntryType::Directory);
            header2.set_mode(0o755);
            header2.set_cksum();
            builder
                .append_data(&mut header2, "jdk-21.0.7-jre/bin/", std::io::empty())
                .unwrap();
            // bin/java file
            let mut header3 = tar::Header::new_gnu();
            header3.set_size(content.len() as u64);
            header3.set_mode(0o755);
            header3.set_cksum();
            builder
                .append_data(&mut header3, "jdk-21.0.7-jre/bin/java", content)
                .unwrap();
            builder.finish().unwrap();
        }
        enc.finish().unwrap()
    }

    // -----------------------------------------------------------------------
    // Step 1: MINELTUI_JAVA env var wins
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_mineltui_java_env_wins() {
        let _guard = java_env_lock().lock().await;
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);

        // Create a fake java file so the path could theoretically be validated
        let fake_java = td.path().join("myjava");
        std::fs::write(&fake_java, b"#!/bin/sh\n").unwrap();

        let prior = std::env::var("MINELTUI_JAVA").ok();
        std::env::set_var("MINELTUI_JAVA", fake_java.to_str().unwrap());

        // Service with real (production) clients — they must NOT be called.
        let svc = JavaService::new().expect("service build");
        let instance = instance_no_override("env-wins");
        let version = minimal_version("java-runtime-delta", 21);

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;

        match prior {
            Some(v) => std::env::set_var("MINELTUI_JAVA", v),
            None => std::env::remove_var("MINELTUI_JAVA"),
        }

        let path = result.expect("should return Ok");
        assert_eq!(path, fake_java, "must return the MINELTUI_JAVA path verbatim");
    }

    // -----------------------------------------------------------------------
    // Step 2 – System override (validates major)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_system_override_accepts_sufficient_major() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);

        // Create a fake java file
        let fake_java = td.path().join("bin").join("java");
        std::fs::create_dir_all(fake_java.parent().unwrap()).unwrap();
        std::fs::write(&fake_java, b"#!/bin/sh\n").unwrap();

        let svc = JavaService::new().expect("service build");
        let mut instance = instance_no_override("sys-ok");
        instance.java_override = Some(JavaRuntimeId::System {
            path: fake_java.clone(),
            major_version: 21,
        });
        let version = minimal_version("java-runtime-delta", 21); // required = 21

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;
        assert!(result.is_ok(), "major == required must succeed: {result:?}");
        assert_eq!(result.unwrap(), fake_java);
    }

    #[tokio::test]
    async fn test_resolve_system_override_validates_major_mismatch() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);

        let fake_java = td.path().join("java8");
        std::fs::write(&fake_java, b"#!/bin/sh\n").unwrap();

        let svc = JavaService::new().expect("service build");
        let mut instance = instance_no_override("sys-mismatch");
        instance.java_override = Some(JavaRuntimeId::System {
            path: fake_java.clone(),
            major_version: 8,
        });
        let version = minimal_version("java-runtime-delta", 21); // required = 21

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;
        assert!(
            matches!(result, Err(AppError::JavaMismatch { required: 21, found: 8, .. })),
            "major mismatch must return JavaMismatch; got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_resolve_system_override_missing_path() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);

        let svc = JavaService::new().expect("service build");
        let mut instance = instance_no_override("sys-missing");
        instance.java_override = Some(JavaRuntimeId::System {
            path: td.path().join("nonexistent-java"),
            major_version: 21,
        });
        let version = minimal_version("java-runtime-delta", 21);

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;
        assert!(
            matches!(result, Err(AppError::JavaNotFound)),
            "missing path must return JavaNotFound; got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Step 2 – Adoptium override
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn test_resolve_adoptium_override() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let archive_content = b"adoptium-java\n";
        let archive = make_adoptium_tar_gz(archive_content);
        let sha256 = {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&archive);
            digest.iter().fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write;
                write!(s, "{b:02x}").unwrap();
                s
            })
        };

        let release_body = format!(
            r#"[{{"binary":{{"package":{{"link":"{base}/adoptium-21.tar.gz","checksum":"{sha256}","size":{size}}}}},"version":{{"major":21}},"release_name":"jdk-21.0.7+7"}}]"#,
            base = server.base_url(),
            sha256 = sha256,
            size = archive.len(),
        );

        let _m_api = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/21/hotspot");
            then.status(200)
                .header("content-type", "application/json")
                .body(release_body.clone());
        });
        let _m_archive = server.mock(|when, then| {
            when.method(GET).path("/adoptium-21.tar.gz");
            then.status(200).body(archive.clone());
        });

        let adoptium = AdoptiumClient::new_with_base_url(server.base_url())
            .expect("adoptium client");
        let mojang = MojangJreClient::new().expect("mojang client");
        let svc = JavaService::with_clients(mojang, adoptium, Arch::X86_64, OsName::Linux);

        let mut instance = instance_no_override("adoptium-override");
        instance.java_override = Some(JavaRuntimeId::Adoptium { major: 21 });
        let version = minimal_version("java-runtime-delta", 21);

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;
        assert!(result.is_ok(), "Adoptium override must succeed: {result:?}");

        let exe = result.unwrap();
        assert!(exe.exists(), "executable must exist at {exe:?}");
        let content = std::fs::read(&exe).unwrap();
        assert_eq!(content, archive_content);
    }

    // -----------------------------------------------------------------------
    // Step 3 – Auto Mojang path
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_auto_mojang_path() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let component = "java-runtime-delta";
        let manifest_body = make_manifest_body(&server.base_url(), component);
        let manifest_sha1 =
            crate::mojang::cache::sha1_hex_of_bytes(manifest_body.as_bytes());
        let all_json_body =
            make_all_json_body(&server.base_url(), component, &manifest_sha1);

        let _m_all = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body(all_json_body.clone());
        });
        let _m_manifest = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/manifest-{component}.json"));
            then.status(200).body(manifest_body.clone());
        });
        let _m_file = server.mock(|when, then| {
            when.method(GET).path("/bin/java");
            then.status(200).body(fixture_bytes());
        });

        // Override the Mojang JRE all.json URL via env var so the service uses our mock.
        let all_url = format!("{}/all.json", server.base_url());
        let prior = std::env::var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV).ok();
        std::env::set_var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV, &all_url);

        let mojang = MojangJreClient::new().expect("mojang client");
        let adoptium = AdoptiumClient::new().expect("adoptium client");
        let svc = JavaService::with_clients(mojang, adoptium, Arch::X86_64, OsName::Linux);

        let instance = instance_no_override("auto-mojang");
        let version = minimal_version(component, 21);

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;

        match prior {
            Some(v) => std::env::set_var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV, v),
            None => std::env::remove_var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV),
        }

        let exe = result.expect("auto Mojang path must succeed");
        assert!(exe.exists(), "java executable must exist at {exe:?}");
        let content = std::fs::read(&exe).unwrap();
        assert_eq!(content, fixture_bytes());
    }

    // -----------------------------------------------------------------------
    // Step 4 – Aarch64 Linux falls through to Adoptium
    // -----------------------------------------------------------------------

    #[cfg(unix)]
    #[tokio::test]
    async fn test_resolve_auto_falls_through_to_adoptium_for_aarch64_linux() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let archive_content = b"adoptium-java-aarch64\n";
        let archive = make_adoptium_tar_gz(archive_content);
        let sha256 = {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&archive);
            digest.iter().fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write;
                write!(s, "{b:02x}").unwrap();
                s
            })
        };

        let release_body = format!(
            r#"[{{"binary":{{"package":{{"link":"{base}/adoptium-21.tar.gz","checksum":"{sha256}","size":{size}}}}},"version":{{"major":21}},"release_name":"jdk-21.0.7+7"}}]"#,
            base = server.base_url(),
            sha256 = sha256,
            size = archive.len(),
        );

        let _m_api = server.mock(|when, then| {
            when.method(GET).path("/v3/assets/latest/21/hotspot");
            then.status(200)
                .header("content-type", "application/json")
                .body(release_body.clone());
        });
        let _m_archive = server.mock(|when, then| {
            when.method(GET).path("/adoptium-21.tar.gz");
            then.status(200).body(archive.clone());
        });

        let adoptium =
            AdoptiumClient::new_with_base_url(server.base_url()).expect("adoptium client");
        let mojang = MojangJreClient::new().expect("mojang client");
        // Force Aarch64 Linux — mojang_platform_key returns None → Adoptium.
        let svc = JavaService::with_clients(mojang, adoptium, Arch::Aarch64, OsName::Linux);

        let instance = instance_no_override("aarch64-fallback");
        let version = minimal_version("java-runtime-delta", 21);

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;
        assert!(result.is_ok(), "aarch64 must fall through to Adoptium: {result:?}");

        let exe = result.unwrap();
        assert!(exe.exists(), "executable must exist: {exe:?}");
        let content = std::fs::read(&exe).unwrap();
        assert_eq!(content, archive_content);
    }

    // -----------------------------------------------------------------------
    // Missing javaVersion in VersionJson → defaults to jre-legacy (Java 8)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_version_without_java_version_defaults_to_legacy() {
        let _guard = java_env_lock().lock().await;
        std::env::remove_var("MINELTUI_JAVA");

        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let server = MockServer::start();

        let component = "jre-legacy";
        let manifest_body = make_manifest_body(&server.base_url(), component);
        let manifest_sha1 =
            crate::mojang::cache::sha1_hex_of_bytes(manifest_body.as_bytes());
        let all_json_body =
            make_all_json_body(&server.base_url(), component, &manifest_sha1);

        let _m_all = server.mock(|when, then| {
            when.method(GET).path("/all.json");
            then.status(200).body(all_json_body.clone());
        });
        let _m_manifest = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/manifest-{component}.json"));
            then.status(200).body(manifest_body.clone());
        });
        let _m_file = server.mock(|when, then| {
            when.method(GET).path("/bin/java");
            then.status(200).body(fixture_bytes());
        });

        let all_url = format!("{}/all.json", server.base_url());
        let prior = std::env::var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV).ok();
        std::env::set_var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV, &all_url);

        let mojang = MojangJreClient::new().expect("mojang client");
        let adoptium = AdoptiumClient::new().expect("adoptium client");
        let svc = JavaService::with_clients(mojang, adoptium, Arch::X86_64, OsName::Linux);

        let instance = instance_no_override("legacy-mc");
        let version = version_without_java_version();

        let result = svc.resolve_jre_for_launch(&paths, &instance, &version).await;

        match prior {
            Some(v) => std::env::set_var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV, v),
            None => std::env::remove_var(crate::java::mojang_jre::MOJANG_JRE_URL_ENV),
        }

        let exe = result.expect("legacy MC must resolve jre-legacy");
        assert!(exe.exists(), "executable must exist: {exe:?}");
    }

    // -----------------------------------------------------------------------
    // set_override_for_instance persists
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_override_for_instance_persists() {
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);

        // Write initial instance.json
        let m = InstanceManifest::new("alpha".into(), "alpha".into(), "1.21.4".into());
        write_instance_manifest(&paths, &m).await.unwrap();

        let svc = JavaService::new().expect("service build");

        // Set Mojang override
        svc.set_override_for_instance(
            &paths,
            "alpha",
            Some(JavaRuntimeId::Mojang { variant: "java-runtime-delta".into() }),
        )
        .await
        .expect("set override must succeed");

        let loaded = read_instance_manifest(&paths, "alpha").await.unwrap();
        assert_eq!(
            loaded.java_override,
            Some(JavaRuntimeId::Mojang { variant: "java-runtime-delta".into() }),
            "override must be persisted"
        );

        // Clear the override
        svc.set_override_for_instance(&paths, "alpha", None)
            .await
            .expect("clear override must succeed");

        let loaded2 = read_instance_manifest(&paths, "alpha").await.unwrap();
        assert!(
            loaded2.java_override.is_none(),
            "override must be cleared"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_jre_for_mc_version_install
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_resolve_jre_for_mc_version_install_missing_vanilla_returns_error() {
        use crate::error::AppError;
        let td = TempDir::new().unwrap();
        let paths = make_paths(&td);
        let svc = JavaService::new().unwrap();
        let r = svc.resolve_jre_for_mc_version_install(&paths, "1.21.4").await;
        match r {
            Err(AppError::VersionNotInstalled { slug }) => {
                assert_eq!(slug, "1.21.4", "slug should carry the missing MC version id");
            }
            other => panic!("expected VersionNotInstalled, got {other:?}"),
        }
    }
}
