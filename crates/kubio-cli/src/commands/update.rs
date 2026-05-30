use crate::args::UpdateArgs;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::debug;

const DEFAULT_REPO: &str = "song-younghoon/kubio";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const UPDATE_CACHE_SCHEMA: u64 = 1;
const INSTALL_MANIFEST_SCHEMA: u64 = 1;
const AMBIENT_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

pub(crate) async fn update(args: UpdateArgs) -> Result<()> {
    let config = UpdateConfig::from_args(&args)?;
    if args.check {
        print_update_check(&config).await
    } else {
        self_update(&config).await
    }
}

pub(crate) fn spawn_ambient_update_check(disabled_by_flag: bool) {
    if ambient_update_check_disabled(disabled_by_flag) || !ambient_check_due() {
        return;
    }
    tokio::spawn(async move {
        if let Err(err) = run_ambient_update_check(false).await {
            debug!(error = %err, "ambient update check failed");
        }
    });
}

pub(crate) async fn run_ambient_update_check(disabled_by_flag: bool) -> Result<()> {
    if ambient_update_check_disabled(disabled_by_flag) || !ambient_check_due() {
        return Ok(());
    }
    let config = UpdateConfig::ambient()?;
    let current = ReleaseVersion::parse(CURRENT_VERSION)
        .ok_or_else(|| anyhow!("current version is not a stable release: {CURRENT_VERSION}"))?;
    let latest = fetch_latest_release(&config, true).await?;
    if latest.version > current {
        eprintln!(
            "kubio {} is available; current is {}. Run `kubio update`.",
            latest.version, current
        );
    }
    Ok(())
}

async fn print_update_check(config: &UpdateConfig) -> Result<()> {
    let current = ReleaseVersion::parse(CURRENT_VERSION)
        .ok_or_else(|| anyhow!("current version is not a stable release: {CURRENT_VERSION}"))?;
    let target = resolve_target_release(config).await?;
    if target.version > current {
        println!(
            "kubio {} is available. Run `kubio update` to install it.",
            target.version
        );
    } else if target.version == current {
        println!("kubio {current} is current.");
    } else {
        println!(
            "kubio {current} is newer than {}. Use `kubio update --version {}` only if you intend to downgrade.",
            target.version, target.tag
        );
    }
    Ok(())
}

async fn self_update(config: &UpdateConfig) -> Result<()> {
    let current = ReleaseVersion::parse(CURRENT_VERSION)
        .ok_or_else(|| anyhow!("current version is not a stable release: {CURRENT_VERSION}"))?;
    let target = resolve_target_release(config).await?;

    if target.version == current && !config.force {
        println!("kubio {current} is already current.");
        return Ok(());
    }
    if target.version < current && !config.force {
        bail!(
            "target release {} is older than current kubio {}; use --force to install it",
            target.version,
            current
        );
    }

    let install_path = resolve_install_path(config)?;
    if is_development_path(&install_path) && !config.force {
        bail!(
            "refusing to update development binary {}; pass --force to override",
            install_path.display()
        );
    }

    let release_target = ReleaseTarget::current()?;
    let manifest = read_install_manifest();
    validate_manifest_target(release_target, manifest.as_ref())?;
    let flavor = resolve_flavor(config, manifest.as_ref())?;
    let artifact = flavor.artifact_name(release_target);
    let base_url = config.download_base_url.as_deref().map_or_else(
        || {
            format!(
                "https://github.com/{}/releases/download/{}",
                config.repo, target.tag
            )
        },
        |base| base.trim_end_matches('/').to_string(),
    );
    let artifact_url = format!("{base_url}/{artifact}");
    let sums_url = format!("{base_url}/SHA256SUMS");
    let client = http_client()?;
    let artifact_response = fetch_bytes(&client, &artifact_url, None)
        .await
        .with_context(|| format!("failed to download {artifact_url}"))?;
    if artifact_response.status != StatusCode::OK {
        bail!(
            "artifact download failed: {} {}",
            artifact_response.status,
            artifact_url
        );
    }
    let artifact_bytes = artifact_response.bytes;
    let sums_response = fetch_bytes(&client, &sums_url, None)
        .await
        .with_context(|| format!("failed to download {sums_url}"))?;
    if sums_response.status != StatusCode::OK {
        bail!(
            "checksum download failed: {} {}",
            sums_response.status,
            sums_url
        );
    }
    let sums = sums_response.text()?;
    verify_sha256(&artifact, &artifact_bytes, &sums)?;
    replace_binary(&install_path, &artifact_bytes)?;
    write_install_manifest(&InstallManifest {
        schema_version: INSTALL_MANIFEST_SCHEMA,
        repo: config.repo.clone(),
        installed_path: install_path.clone(),
        target: release_target.triple().to_string(),
        flavor,
        installed_version: target.version.to_string(),
    });
    println!(
        "Updated kubio from {} to {} at {}.",
        current,
        target.version,
        install_path.display()
    );
    Ok(())
}

async fn resolve_target_release(config: &UpdateConfig) -> Result<ResolvedRelease> {
    if let Some(version) = config.requested_version {
        return Ok(ResolvedRelease {
            version,
            tag: version.tag(),
            url: Some(format!(
                "https://github.com/{}/releases/tag/{}",
                config.repo,
                version.tag()
            )),
        });
    }
    fetch_latest_release(config, true).await
}

async fn fetch_latest_release(
    config: &UpdateConfig,
    use_cache_etag: bool,
) -> Result<ResolvedRelease> {
    let cache = read_update_cache();
    let url = config.release_api_url.clone().unwrap_or_else(|| {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            config.repo
        )
    });
    let client = http_client()?;
    let etag = if use_cache_etag {
        cache.as_ref().and_then(|cache| cache.etag.as_deref())
    } else {
        None
    };
    let fetched = fetch_bytes(&client, &url, etag).await?;
    if fetched.status == StatusCode::NOT_MODIFIED {
        let mut cache =
            cache.ok_or_else(|| anyhow!("GitHub returned 304 without a local cache"))?;
        cache.checked_at_unix = now_unix();
        if fetched.etag.is_some() {
            cache.etag = fetched.etag;
        }
        let version = ReleaseVersion::parse(&cache.latest_version).ok_or_else(|| {
            anyhow!(
                "cached release version is invalid: {}",
                cache.latest_version
            )
        })?;
        write_update_cache(&cache);
        return Ok(ResolvedRelease {
            version,
            tag: version.tag(),
            url: Some(cache.latest_url),
        });
    }
    if fetched.status != StatusCode::OK {
        bail!(
            "release metadata request failed: {} {}",
            fetched.status,
            url
        );
    }
    let api: ReleaseApiResponse = serde_json::from_slice(&fetched.bytes)
        .with_context(|| format!("failed to parse release metadata from {url}"))?;
    if api.prerelease {
        bail!("latest release is marked prerelease: {}", api.tag_name);
    }
    let version = ReleaseVersion::parse(&api.tag_name)
        .ok_or_else(|| anyhow!("latest release tag is not supported: {}", api.tag_name))?;
    let release = ResolvedRelease {
        version,
        tag: version.tag(),
        url: api.html_url,
    };
    write_update_cache(&UpdateCheckCache {
        schema_version: UPDATE_CACHE_SCHEMA,
        checked_at_unix: now_unix(),
        latest_version: version.to_string(),
        latest_url: release.url.clone().unwrap_or_else(|| {
            format!(
                "https://github.com/{}/releases/tag/{}",
                config.repo,
                version.tag()
            )
        }),
        etag: fetched.etag,
    });
    Ok(release)
}

fn http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_millis(1500))
        .user_agent(format!("kubio/{CURRENT_VERSION}"))
        .build()
        .context("failed to build HTTP client")
}

async fn fetch_bytes(client: &Client, url: &str, etag: Option<&str>) -> Result<FetchedBytes> {
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(FetchedBytes {
            status: StatusCode::OK,
            bytes: fs::read(path).with_context(|| format!("failed to read {url}"))?,
            etag: None,
        });
    }

    let mut request = client.get(url);
    if url.contains("api.github.com") {
        request = request.header("Accept", "application/vnd.github+json");
    }
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    let response = request.send().await?;
    let status = response.status();
    let response_etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if status == StatusCode::NOT_MODIFIED {
        return Ok(FetchedBytes {
            status,
            bytes: Vec::new(),
            etag: response_etag,
        });
    }
    let bytes = response.bytes().await?.to_vec();
    Ok(FetchedBytes {
        status,
        bytes,
        etag: response_etag,
    })
}

fn verify_sha256(artifact: &str, bytes: &[u8], sums: &str) -> Result<()> {
    let expected = expected_sha256(artifact, sums)?;
    let actual = sha256_hex(bytes);
    if expected != actual {
        bail!("checksum verification failed for {artifact}");
    }
    Ok(())
}

fn expected_sha256(artifact: &str, sums: &str) -> Result<String> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        if name.trim_start_matches('*') == artifact {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    bail!("SHA256SUMS does not contain {artifact}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("nibble is always <= 15"),
    }
}

fn resolve_install_path(config: &UpdateConfig) -> Result<PathBuf> {
    if let Some(dir) = config.install_dir.as_ref() {
        return Ok(dir.join("kubio"));
    }
    if let Some(manifest) = read_install_manifest() {
        return Ok(manifest.installed_path);
    }
    env::current_exe().context("failed to determine current executable path")
}

fn replace_binary(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("install path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create install directory {}", parent.display()))?;
    let temp = parent.join(format!(
        ".kubio-update-{}-{}",
        std::process::id(),
        now_unix()
    ));
    let backup = parent.join(format!(
        ".kubio-previous-{}-{}",
        std::process::id(),
        now_unix()
    ));
    fs::write(&temp, bytes).with_context(|| format!("failed to write {}", temp.display()))?;
    make_executable(&temp)?;

    let had_existing = path.exists();
    if had_existing {
        fs::rename(path, &backup).with_context(|| {
            format!(
                "failed to move current binary from {} to {}",
                path.display(),
                backup.display()
            )
        })?;
    }

    if let Err(err) = fs::rename(&temp, path) {
        if had_existing {
            let _ = fs::rename(&backup, path);
        }
        bail!("failed to move updated binary into place: {err}");
    }

    if let Err(err) = verify_installed_binary(path) {
        let _ = fs::remove_file(path);
        if had_existing {
            let _ = fs::rename(&backup, path);
        }
        bail!("updated binary did not run successfully: {err}");
    }

    if had_existing {
        let _ = fs::remove_file(&backup);
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn verify_installed_binary(path: &Path) -> Result<()> {
    let status = Command::new(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run {}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{} --version exited with {status}", path.display())
    }
}

fn validate_manifest_target(
    current: ReleaseTarget,
    manifest: Option<&InstallManifest>,
) -> Result<()> {
    let Some(manifest) = manifest else {
        return Ok(());
    };
    let target = manifest.target.trim();
    if target.is_empty() {
        return Ok(());
    }
    let Some(manifest_target) = ReleaseTarget::from_triple(target) else {
        return Ok(());
    };
    if manifest_target != current {
        bail!(
            "install manifest target {} does not match current host target {}; reinstall with install.sh",
            manifest_target.triple(),
            current.triple()
        );
    }
    Ok(())
}

fn resolve_flavor(config: &UpdateConfig, manifest: Option<&InstallManifest>) -> Result<Flavor> {
    if let Some(flavor) = config.flavor {
        return Ok(flavor);
    }
    if let Some(manifest) = manifest {
        return Ok(manifest.flavor);
    }
    if cfg!(feature = "experimental-http3") {
        Ok(Flavor::Http3Experimental)
    } else {
        Ok(Flavor::Standard)
    }
}

fn ambient_update_check_disabled(disabled_by_flag: bool) -> bool {
    disabled_by_flag
        || env::var("KUBIO_UPDATE_CHECK")
            .map(|value| matches!(value.as_str(), "off" | "0" | "false"))
            .unwrap_or(false)
        || env::var("KUBIO_NO_UPDATE_CHECK")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

fn ambient_check_due() -> bool {
    let Some(cache) = read_update_cache() else {
        return true;
    };
    now_unix().saturating_sub(cache.checked_at_unix) >= AMBIENT_CHECK_INTERVAL_SECS
}

fn is_development_path(path: &Path) -> bool {
    let value = path.to_string_lossy();
    value.contains("/target/debug/") || value.contains("/target/release/")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_update_cache() -> Option<UpdateCheckCache> {
    let path = update_cache_path()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_update_cache(cache: &UpdateCheckCache) {
    let Some(path) = update_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(text) = serde_json::to_string_pretty(cache) {
        let _ = fs::write(path, text);
    }
}

fn read_install_manifest() -> Option<InstallManifest> {
    let path = install_manifest_path()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_install_manifest(manifest: &InstallManifest) {
    let Some(path) = install_manifest_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(text) = serde_json::to_string_pretty(manifest) {
        let _ = fs::write(path, text);
    }
}

fn update_cache_path() -> Option<PathBuf> {
    Some(base_cache_dir()?.join("kubio").join("update-check.json"))
}

fn install_manifest_path() -> Option<PathBuf> {
    Some(base_config_dir()?.join("kubio").join("install.json"))
}

fn base_cache_dir() -> Option<PathBuf> {
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

fn base_config_dir() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

#[derive(Debug, Clone)]
struct UpdateConfig {
    repo: String,
    requested_version: Option<ReleaseVersion>,
    flavor: Option<Flavor>,
    install_dir: Option<PathBuf>,
    force: bool,
    release_api_url: Option<String>,
    download_base_url: Option<String>,
}

impl UpdateConfig {
    fn from_args(args: &UpdateArgs) -> Result<Self> {
        let requested_version = args
            .version
            .as_deref()
            .map(|version| {
                ReleaseVersion::parse(version)
                    .ok_or_else(|| anyhow!("unsupported release version: {version}"))
            })
            .transpose()?;
        let flavor = args.flavor.as_deref().map(Flavor::parse).transpose()?;
        Ok(Self {
            repo: args
                .repo
                .clone()
                .or_else(|| env::var("KUBIO_REPO").ok())
                .unwrap_or_else(|| DEFAULT_REPO.to_string()),
            requested_version,
            flavor,
            install_dir: args.install_dir.clone(),
            force: args.force,
            release_api_url: args
                .release_api_url
                .clone()
                .or_else(|| env::var("KUBIO_RELEASE_API_URL").ok()),
            download_base_url: args
                .download_base_url
                .clone()
                .or_else(|| env::var("KUBIO_DOWNLOAD_BASE_URL").ok()),
        })
    }

    fn ambient() -> Result<Self> {
        Ok(Self {
            repo: env::var("KUBIO_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string()),
            requested_version: None,
            flavor: None,
            install_dir: None,
            force: false,
            release_api_url: env::var("KUBIO_RELEASE_API_URL").ok(),
            download_base_url: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseTarget {
    X86_64UnknownLinuxGnu,
    Aarch64UnknownLinuxGnu,
    Aarch64AppleDarwin,
}

impl ReleaseTarget {
    fn current() -> Result<Self> {
        Self::from_os_arch(env::consts::OS, env::consts::ARCH).ok_or_else(|| {
            anyhow!(
                "unsupported update platform: os={} arch={}; supported targets: {}",
                env::consts::OS,
                env::consts::ARCH,
                Self::supported_targets()
            )
        })
    }

    fn from_os_arch(os: &str, arch: &str) -> Option<Self> {
        match (os, arch) {
            ("linux", "x86_64") | ("Linux", "x86_64") | ("Linux", "amd64") => {
                Some(Self::X86_64UnknownLinuxGnu)
            }
            ("linux", "aarch64")
            | ("linux", "arm64")
            | ("Linux", "aarch64")
            | ("Linux", "arm64") => Some(Self::Aarch64UnknownLinuxGnu),
            ("macos", "aarch64")
            | ("macos", "arm64")
            | ("Darwin", "arm64")
            | ("Darwin", "aarch64") => Some(Self::Aarch64AppleDarwin),
            _ => None,
        }
    }

    fn from_triple(value: &str) -> Option<Self> {
        match value {
            "x86_64-unknown-linux-gnu" => Some(Self::X86_64UnknownLinuxGnu),
            "aarch64-unknown-linux-gnu" => Some(Self::Aarch64UnknownLinuxGnu),
            "aarch64-apple-darwin" => Some(Self::Aarch64AppleDarwin),
            _ => None,
        }
    }

    fn triple(self) -> &'static str {
        match self {
            Self::X86_64UnknownLinuxGnu => "x86_64-unknown-linux-gnu",
            Self::Aarch64UnknownLinuxGnu => "aarch64-unknown-linux-gnu",
            Self::Aarch64AppleDarwin => "aarch64-apple-darwin",
        }
    }

    fn supported_targets() -> &'static str {
        "x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, aarch64-apple-darwin"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Flavor {
    Standard,
    Http3Experimental,
}

impl Flavor {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "standard" => Ok(Self::Standard),
            "http3-experimental" => Ok(Self::Http3Experimental),
            _ => bail!("unsupported flavor: {value}"),
        }
    }

    fn artifact_name(self, target: ReleaseTarget) -> String {
        match self {
            Self::Standard => format!("kubio-{}", target.triple()),
            Self::Http3Experimental => {
                format!("kubio-{}-http3-experimental", target.triple())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl ReleaseVersion {
    fn parse(value: &str) -> Option<Self> {
        let value = value.strip_prefix('v').unwrap_or(value);
        let mut parts = value.split('.');
        let major = parse_numeric_part(parts.next()?)?;
        let minor = parse_numeric_part(parts.next()?)?;
        let patch = parse_numeric_part(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    fn tag(self) -> String {
        format!("v{self}")
    }
}

impl std::fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_numeric_part(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[derive(Debug, Clone)]
struct ResolvedRelease {
    version: ReleaseVersion,
    tag: String,
    url: Option<String>,
}

#[derive(Debug)]
struct FetchedBytes {
    status: StatusCode,
    bytes: Vec<u8>,
    etag: Option<String>,
}

impl FetchedBytes {
    fn text(self) -> Result<String> {
        String::from_utf8(self.bytes).context("downloaded response was not UTF-8")
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseApiResponse {
    tag_name: String,
    html_url: Option<String>,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpdateCheckCache {
    schema_version: u64,
    checked_at_unix: u64,
    latest_version: String,
    latest_url: String,
    #[serde(default)]
    etag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallManifest {
    schema_version: u64,
    repo: String,
    installed_path: PathBuf,
    #[serde(default)]
    target: String,
    flavor: Flavor,
    installed_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stable_release_tags() {
        assert_eq!(
            ReleaseVersion::parse("v0.4.1"),
            Some(ReleaseVersion {
                major: 0,
                minor: 4,
                patch: 1,
            })
        );
        assert_eq!(ReleaseVersion::parse("0.4.1").unwrap().tag(), "v0.4.1");
        assert!(ReleaseVersion::parse("v0.4").is_none());
        assert!(ReleaseVersion::parse("v0.4.1-beta.1").is_none());
        assert!(ReleaseVersion::parse("v0.x.1").is_none());
    }

    #[test]
    fn orders_release_versions_numerically() {
        assert!(ReleaseVersion::parse("v0.10.0") > ReleaseVersion::parse("v0.9.9"));
        assert!(ReleaseVersion::parse("v1.0.0") > ReleaseVersion::parse("v0.99.99"));
    }

    #[test]
    fn chooses_artifact_for_flavor() {
        assert_eq!(
            Flavor::Standard.artifact_name(ReleaseTarget::X86_64UnknownLinuxGnu),
            "kubio-x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            Flavor::Http3Experimental.artifact_name(ReleaseTarget::X86_64UnknownLinuxGnu),
            "kubio-x86_64-unknown-linux-gnu-http3-experimental"
        );
        assert_eq!(
            Flavor::Standard.artifact_name(ReleaseTarget::Aarch64UnknownLinuxGnu),
            "kubio-aarch64-unknown-linux-gnu"
        );
        assert_eq!(
            Flavor::Http3Experimental.artifact_name(ReleaseTarget::Aarch64AppleDarwin),
            "kubio-aarch64-apple-darwin-http3-experimental"
        );
    }

    #[test]
    fn maps_supported_release_targets() {
        assert_eq!(
            ReleaseTarget::from_os_arch("linux", "x86_64"),
            Some(ReleaseTarget::X86_64UnknownLinuxGnu)
        );
        assert_eq!(
            ReleaseTarget::from_os_arch("Linux", "amd64"),
            Some(ReleaseTarget::X86_64UnknownLinuxGnu)
        );
        assert_eq!(
            ReleaseTarget::from_os_arch("linux", "aarch64"),
            Some(ReleaseTarget::Aarch64UnknownLinuxGnu)
        );
        assert_eq!(
            ReleaseTarget::from_os_arch("Linux", "arm64"),
            Some(ReleaseTarget::Aarch64UnknownLinuxGnu)
        );
        assert_eq!(
            ReleaseTarget::from_os_arch("macos", "aarch64"),
            Some(ReleaseTarget::Aarch64AppleDarwin)
        );
        assert_eq!(
            ReleaseTarget::from_os_arch("Darwin", "arm64"),
            Some(ReleaseTarget::Aarch64AppleDarwin)
        );
        assert_eq!(ReleaseTarget::from_os_arch("macos", "x86_64"), None);
        assert_eq!(ReleaseTarget::from_os_arch("linux", "armv7l"), None);
    }

    #[test]
    fn validates_manifest_target_mismatch() {
        let mut manifest = InstallManifest {
            schema_version: INSTALL_MANIFEST_SCHEMA,
            repo: DEFAULT_REPO.to_string(),
            installed_path: PathBuf::from("/tmp/kubio"),
            target: "x86_64-unknown-linux-gnu".to_string(),
            flavor: Flavor::Standard,
            installed_version: "0.4.0".to_string(),
        };
        assert!(
            validate_manifest_target(ReleaseTarget::X86_64UnknownLinuxGnu, Some(&manifest)).is_ok()
        );
        assert!(
            validate_manifest_target(ReleaseTarget::Aarch64AppleDarwin, Some(&manifest)).is_err()
        );

        manifest.target = "unknown-target".to_string();
        assert!(
            validate_manifest_target(ReleaseTarget::Aarch64AppleDarwin, Some(&manifest)).is_ok()
        );
    }

    #[test]
    fn verifies_sha256_from_sums() {
        let bytes = b"kubio";
        let sum = sha256_hex(bytes);
        let sums = format!("{sum}  kubio-x86_64-unknown-linux-gnu\n");
        assert!(verify_sha256("kubio-x86_64-unknown-linux-gnu", bytes, &sums).is_ok());
        assert!(verify_sha256("missing", bytes, &sums).is_err());
    }

    #[test]
    fn detects_development_paths() {
        assert!(is_development_path(Path::new(
            "/home/user/kubio/target/debug/kubio"
        )));
        assert!(is_development_path(Path::new(
            "/home/user/kubio/target/release/kubio"
        )));
        assert!(!is_development_path(Path::new(
            "/home/user/.local/bin/kubio"
        )));
    }

    #[tokio::test]
    async fn fetches_file_url_fixture() {
        let dir = env::temp_dir().join(format!(
            "kubio-update-test-{}-{}",
            std::process::id(),
            now_unix()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latest.json");
        fs::write(
            &path,
            r#"{"tag_name":"v9.8.7","html_url":"https://example.invalid/v9.8.7","prerelease":false}"#,
        )
        .unwrap();
        let client = http_client().unwrap();
        let fetched = fetch_bytes(&client, &format!("file://{}", path.display()), None)
            .await
            .unwrap();
        let api: ReleaseApiResponse = serde_json::from_slice(&fetched.bytes).unwrap();
        assert_eq!(api.tag_name, "v9.8.7");
        let _ = fs::remove_dir_all(dir);
    }
}
