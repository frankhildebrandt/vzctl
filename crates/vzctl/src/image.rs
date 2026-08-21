use serde_json::{json, Value};
use sha2::{Digest as _, Sha256, Sha512};
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub const EXIT_IMAGE_NETWORK: u8 = 21;
pub const EXIT_IMAGE_CHECKSUM: u8 = 22;
pub const EXIT_IMAGE_ARCH: u8 = 23;
const EXIT_INVALID_INPUT: u8 = 3;
const EXIT_UNAVAILABLE: u8 = 12;
const EXIT_IMAGE_STATE: u8 = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HashAlgorithm {
    Sha256,
    Sha512,
}

impl HashAlgorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceFormat {
    Qcow2,
    Qcow2Xz,
    Raw,
    RawZst,
    RawBz2,
    Ova,
    ZipQcow2,
}

impl SourceFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Qcow2 => "qcow2",
            Self::Qcow2Xz => "qcow2.xz",
            Self::Raw => "raw",
            Self::RawZst => "raw.zst",
            Self::RawBz2 => "raw.bz2",
            Self::Ova => "ova",
            Self::ZipQcow2 => "zip/qcow2",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Checksum {
    Remote {
        url: &'static str,
        algorithm: HashAlgorithm,
    },
    Inline {
        algorithm: HashAlgorithm,
        digest: &'static str,
    },
}

#[derive(Clone, Copy, Debug)]
enum Resolver {
    Static {
        url: &'static str,
        filename: &'static str,
        checksum: Checksum,
        format: SourceFormat,
    },
    FedoraCoreOs,
    Flatcar,
    GithubLatest {
        repository: &'static str,
        asset: &'static str,
        format: SourceFormat,
    },
    GithubTagged {
        repository: &'static str,
        tag: &'static str,
        asset: &'static str,
        sha256: &'static str,
        format: SourceFormat,
    },
}

#[derive(Clone, Copy, Debug)]
struct CatalogEntry {
    canonical: &'static str,
    aliases: &'static [&'static str],
    distribution: &'static str,
    release: &'static str,
    resolver: Resolver,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        canonical: "ubuntu-latest",
        aliases: &["ubuntu-latest"],
        distribution: "Ubuntu",
        release: "26.04 LTS",
        resolver: Resolver::Static {
            url: "https://cloud-images.ubuntu.com/releases/26.04/release/ubuntu-26.04-server-cloudimg-arm64.img",
            filename: "ubuntu-26.04-server-cloudimg-arm64.img",
            checksum: Checksum::Remote {
                url: "https://cloud-images.ubuntu.com/releases/26.04/release/SHA256SUMS",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "ubuntu-26.04",
        aliases: &["ubuntu-26.04"],
        distribution: "Ubuntu",
        release: "26.04 LTS",
        resolver: Resolver::Static {
            url: "https://cloud-images.ubuntu.com/releases/26.04/release/ubuntu-26.04-server-cloudimg-arm64.img",
            filename: "ubuntu-26.04-server-cloudimg-arm64.img",
            checksum: Checksum::Remote {
                url: "https://cloud-images.ubuntu.com/releases/26.04/release/SHA256SUMS",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "ubuntu-24.04",
        aliases: &["ubuntu-24.04"],
        distribution: "Ubuntu",
        release: "24.04 LTS",
        resolver: Resolver::Static {
            url: "https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-arm64.img",
            filename: "ubuntu-24.04-server-cloudimg-arm64.img",
            checksum: Checksum::Remote {
                url: "https://cloud-images.ubuntu.com/releases/24.04/release/SHA256SUMS",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "ubuntu-22.04",
        aliases: &["ubuntu-22.04"],
        distribution: "Ubuntu",
        release: "22.04 LTS",
        resolver: Resolver::Static {
            url: "https://cloud-images.ubuntu.com/releases/22.04/release/ubuntu-22.04-server-cloudimg-arm64.img",
            filename: "ubuntu-22.04-server-cloudimg-arm64.img",
            checksum: Checksum::Remote {
                url: "https://cloud-images.ubuntu.com/releases/22.04/release/SHA256SUMS",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "ubuntu-20.04",
        aliases: &["ubuntu-20.04"],
        distribution: "Ubuntu",
        release: "20.04 LTS",
        resolver: Resolver::Static {
            url: "https://cloud-images.ubuntu.com/releases/20.04/release/ubuntu-20.04-server-cloudimg-arm64.img",
            filename: "ubuntu-20.04-server-cloudimg-arm64.img",
            checksum: Checksum::Remote {
                url: "https://cloud-images.ubuntu.com/releases/20.04/release/SHA256SUMS",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "debian-latest",
        aliases: &["debian-latest"],
        distribution: "Debian",
        release: "13 (stable/Trixie)",
        resolver: Resolver::Static {
            url: "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-arm64.qcow2",
            filename: "debian-13-generic-arm64.qcow2",
            checksum: Checksum::Remote {
                url: "https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS",
                algorithm: HashAlgorithm::Sha512,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "debian-13",
        aliases: &["debian-13"],
        distribution: "Debian",
        release: "13 (Trixie)",
        resolver: Resolver::Static {
            url: "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-generic-arm64.qcow2",
            filename: "debian-13-generic-arm64.qcow2",
            checksum: Checksum::Remote {
                url: "https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS",
                algorithm: HashAlgorithm::Sha512,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "debian-12",
        aliases: &["debian-12"],
        distribution: "Debian",
        release: "12 (Bookworm)",
        resolver: Resolver::Static {
            url: "https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-generic-arm64.qcow2",
            filename: "debian-12-generic-arm64.qcow2",
            checksum: Checksum::Remote {
                url: "https://cloud.debian.org/images/cloud/bookworm/latest/SHA512SUMS",
                algorithm: HashAlgorithm::Sha512,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "debian-11",
        aliases: &["debian-11"],
        distribution: "Debian",
        release: "11 (Bullseye)",
        resolver: Resolver::Static {
            url: "https://cloud.debian.org/images/cloud/bullseye/latest/debian-11-generic-arm64.qcow2",
            filename: "debian-11-generic-arm64.qcow2",
            checksum: Checksum::Remote {
                url: "https://cloud.debian.org/images/cloud/bullseye/latest/SHA512SUMS",
                algorithm: HashAlgorithm::Sha512,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "alpine-latest",
        aliases: &["alpine-latest"],
        distribution: "Alpine Linux",
        release: "3.24.1",
        resolver: Resolver::Static {
            url: "https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/cloud/generic_alpine-3.24.1-aarch64-uefi-cloudinit-r0.qcow2",
            filename: "generic_alpine-3.24.1-aarch64-uefi-cloudinit-r0.qcow2",
            checksum: Checksum::Remote {
                url: "https://dl-cdn.alpinelinux.org/alpine/v3.24/releases/cloud/generic_alpine-3.24.1-aarch64-uefi-cloudinit-r0.qcow2.sha512",
                algorithm: HashAlgorithm::Sha512,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "arch-latest",
        aliases: &["arch-latest"],
        distribution: "Arch Linux ARM",
        release: "rolling (UTM ARM64 VM snapshot)",
        resolver: Resolver::GithubTagged {
            repository: "utmapp/vm-downloads",
            tag: "archlinux-arm64",
            asset: "archlinux-arm64-utm4.zip",
            sha256: "e9891d07b5f1174cc5fc2a37dbb3844de5f9a2d3a5d3ee606891d9470196cfa8",
            format: SourceFormat::ZipQcow2,
        },
    },
    CatalogEntry {
        canonical: "fedora-latest",
        aliases: &["fedora-latest"],
        distribution: "Fedora Cloud",
        release: "44-1.7",
        resolver: Resolver::Static {
            url: "https://download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/aarch64/images/Fedora-Cloud-Base-Generic-44-1.7.aarch64.qcow2",
            filename: "Fedora-Cloud-Base-Generic-44-1.7.aarch64.qcow2",
            checksum: Checksum::Remote {
                url: "https://download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/aarch64/images/Fedora-Cloud-44-1.7-aarch64-CHECKSUM",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "rocky-latest",
        aliases: &["rocky-latest"],
        distribution: "Rocky Linux",
        release: "10.2",
        resolver: Resolver::Static {
            url: "https://download.rockylinux.org/pub/rocky/10/images/aarch64/Rocky-10-GenericCloud-Base.latest.aarch64.qcow2",
            filename: "Rocky-10-GenericCloud-Base.latest.aarch64.qcow2",
            checksum: Checksum::Remote {
                url: "https://download.rockylinux.org/pub/rocky/10/images/aarch64/Rocky-10-GenericCloud-Base.latest.aarch64.qcow2.CHECKSUM",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "alma-latest",
        aliases: &["alma-latest"],
        distribution: "AlmaLinux",
        release: "10 stable",
        resolver: Resolver::Static {
            url: "https://repo.almalinux.org/almalinux/10/cloud/aarch64/images/AlmaLinux-10-GenericCloud-latest.aarch64.qcow2",
            filename: "AlmaLinux-10-GenericCloud-latest.aarch64.qcow2",
            checksum: Checksum::Remote {
                url: "https://repo.almalinux.org/almalinux/10/cloud/aarch64/images/CHECKSUM",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "opensuse-latest",
        aliases: &["opensuse-latest"],
        distribution: "openSUSE Leap",
        release: "16.0",
        resolver: Resolver::Static {
            url: "https://download.opensuse.org/distribution/openSUSE-current/appliances/Leap-16.0-Minimal-VM.aarch64-Cloud.qcow2",
            filename: "Leap-16.0-Minimal-VM.aarch64-Cloud.qcow2",
            checksum: Checksum::Remote {
                url: "https://download.opensuse.org/distribution/openSUSE-current/appliances/Leap-16.0-Minimal-VM.aarch64-Cloud.qcow2.sha256",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "fedora-coreos-latest",
        aliases: &["fedora-coreos-latest", "coreos-latest"],
        distribution: "Fedora CoreOS",
        release: "stable stream",
        resolver: Resolver::FedoraCoreOs,
    },
    CatalogEntry {
        canonical: "flatcar-latest",
        aliases: &["flatcar-latest"],
        distribution: "Flatcar Container Linux",
        release: "stable channel",
        resolver: Resolver::Flatcar,
    },
    CatalogEntry {
        canonical: "photon-latest",
        aliases: &["photon-latest"],
        distribution: "VMware Photon OS",
        release: "5.0 GA",
        resolver: Resolver::Static {
            url: "https://packages.vmware.com/photon/5.0/GA/ova/photon-uefi-hw14-5.0-dde71ec57.aarch64.ova",
            filename: "photon-uefi-hw14-5.0-dde71ec57.aarch64.ova",
            checksum: Checksum::Inline {
                algorithm: HashAlgorithm::Sha512,
                digest: "ffecb532158fc2b0148e3fc2c7d54fcc743d6741fa4c733acf589cd6c493556630c08b4e601e65a5523db404f88ad1e8b4f6df0484a8a7a0de0889a99e6fea5a",
            },
            format: SourceFormat::Ova,
        },
    },
    CatalogEntry {
        canonical: "opensuse-microos-latest",
        aliases: &["opensuse-microos-latest"],
        distribution: "openSUSE MicroOS",
        release: "Tumbleweed current",
        resolver: Resolver::Static {
            url: "https://download.opensuse.org/ports/aarch64/tumbleweed/appliances/openSUSE-MicroOS.aarch64-ContainerHost-OpenStack-Cloud.qcow2",
            filename: "openSUSE-MicroOS.aarch64-ContainerHost-OpenStack-Cloud.qcow2",
            checksum: Checksum::Remote {
                url: "https://download.opensuse.org/ports/aarch64/tumbleweed/appliances/openSUSE-MicroOS.aarch64-ContainerHost-OpenStack-Cloud.qcow2.sha256",
                algorithm: HashAlgorithm::Sha256,
            },
            format: SourceFormat::Qcow2,
        },
    },
    CatalogEntry {
        canonical: "talos-latest",
        aliases: &["talos-latest"],
        distribution: "Talos Linux",
        release: "latest stable",
        resolver: Resolver::GithubLatest {
            repository: "siderolabs/talos",
            asset: "metal-arm64.raw.zst",
            format: SourceFormat::RawZst,
        },
    },
];

#[derive(Debug, Eq, PartialEq)]
pub struct PullFailure {
    pub code: u8,
    pub message: String,
}

impl PullFailure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PullResult {
    pub requested_alias: String,
    pub canonical_alias: String,
    pub distribution: String,
    pub release: String,
    pub source_url: String,
    pub source_format: String,
    pub source_algorithm: String,
    pub source_digest: String,
    pub normalized_digest: String,
    pub image_path: PathBuf,
    pub manifest_path: PathBuf,
    pub unchanged: bool,
    pub sealed: bool,
    pub aliases: Vec<String>,
}

#[derive(Debug)]
struct ResolvedSource {
    url: String,
    filename: String,
    format: SourceFormat,
    algorithm: HashAlgorithm,
    digest: String,
    release: Option<String>,
}

trait Fetcher {
    fn text(&self, url: &str) -> Result<String, PullFailure>;
    fn download(&self, url: &str, destination: &Path) -> Result<(), PullFailure>;
}

struct CurlFetcher {
    progress: bool,
}

impl CurlFetcher {
    fn new() -> Self {
        Self {
            progress: io::stderr().is_terminal() || progress_env_enabled(),
        }
    }
}

impl Fetcher for CurlFetcher {
    fn text(&self, url: &str) -> Result<String, PullFailure> {
        let output = Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error", url])
            .output()
            .map_err(|error| {
                PullFailure::new(
                    EXIT_IMAGE_NETWORK,
                    format!("cannot start curl for {url}: {error}"),
                )
            })?;
        if !output.status.success() {
            return Err(PullFailure::new(
                EXIT_IMAGE_NETWORK,
                format!(
                    "cannot fetch {url}: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        String::from_utf8(output.stdout).map_err(|error| {
            PullFailure::new(
                EXIT_IMAGE_NETWORK,
                format!("non-UTF-8 metadata from {url}: {error}"),
            )
        })
    }

    fn download(&self, url: &str, destination: &Path) -> Result<(), PullFailure> {
        let mut command = Command::new("curl");
        command.arg("--fail").arg("--location").arg("--show-error");
        if self.progress {
            command.arg("--progress-bar");
        } else {
            command.arg("--silent");
        }
        command.arg("--output").arg(destination).arg(url);
        let filename = destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("image");
        let label = format!("Downloading {filename}…");
        if self.progress && !io::stderr().is_terminal() {
            run_with_progress_relay(
                &mut command,
                &label,
                EXIT_IMAGE_NETWORK,
                &format!("download of {url}"),
            )
        } else {
            let status = command.status().map_err(|error| {
                PullFailure::new(
                    EXIT_IMAGE_NETWORK,
                    format!("cannot start curl for {url}: {error}"),
                )
            })?;
            if status.success() {
                Ok(())
            } else {
                Err(PullFailure::new(
                    EXIT_IMAGE_NETWORK,
                    format!("download failed for {url}"),
                ))
            }
        }
    }
}

/// Truthy `VZCTL_PROGRESS` forces phase lines on stderr (supervisor jobs, non-TTY).
pub fn progress_env_enabled() -> bool {
    match std::env::var("VZCTL_PROGRESS") {
        Ok(value) => {
            let v = value.trim();
            !v.is_empty()
                && v != "0"
                && !v.eq_ignore_ascii_case("false")
                && !v.eq_ignore_ascii_case("no")
        }
        Err(_) => false,
    }
}

fn progress_enabled() -> bool {
    io::stderr().is_terminal() || progress_env_enabled()
}

fn progress_status(message: &str) {
    if progress_enabled() {
        eprintln!("{message}");
    }
}

pub fn pull(alias: &str, images_dir: &Path) -> Result<PullResult, PullFailure> {
    pull_with(alias, images_dir, &CurlFetcher::new(), true)
}

fn pull_with(
    alias: &str,
    images_dir: &Path,
    fetcher: &dyn Fetcher,
    enforce_host_arch: bool,
) -> Result<PullResult, PullFailure> {
    if enforce_host_arch && !matches!(std::env::consts::ARCH, "aarch64") {
        return Err(PullFailure::new(
            EXIT_IMAGE_ARCH,
            format!(
                "image pull supports ARM64 hosts only; detected {}",
                std::env::consts::ARCH
            ),
        ));
    }
    let entry = catalog_entry(alias).ok_or_else(|| {
        PullFailure::new(
            EXIT_INVALID_INPUT,
            format!(
                "unknown image alias {alias}; supported aliases: {}",
                aliases().join(", ")
            ),
        )
    })?;
    progress_status(&format!("Resolving {alias}…"));
    let source = resolve_source(entry, fetcher)?;
    validate_digest_text(source.algorithm, &source.digest)?;

    fs::create_dir_all(images_dir.join("objects")).map_err(state_error)?;
    fs::create_dir_all(images_dir.join("aliases")).map_err(state_error)?;
    fs::create_dir_all(images_dir.join(".tmp")).map_err(state_error)?;

    if let Some(result) = unchanged_result(alias, entry, &source, images_dir)? {
        progress_status(&format!("Image {alias} is unchanged"));
        return Ok(result);
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let work_dir = images_dir
        .join(".tmp")
        .join(format!("pull-{}-{nonce}", std::process::id()));
    fs::create_dir(&work_dir).map_err(state_error)?;
    let downloaded = work_dir.join(&source.filename);
    let operation = (|| {
        progress_status(&format!("Downloading {}…", source.filename));
        fetcher.download(&source.url, &downloaded)?;
        progress_status("Verifying checksum…");
        let actual = hash_file(&downloaded, source.algorithm)?;
        if actual != source.digest.to_ascii_lowercase() {
            return Err(PullFailure::new(
                EXIT_IMAGE_CHECKSUM,
                format!(
                    "{} mismatch for {}: expected {}, got {}",
                    source.algorithm.name(),
                    source.filename,
                    source.digest,
                    actual
                ),
            ));
        }

        let normalized = work_dir.join("normalized.raw");
        progress_status("Normalizing image…");
        normalize(&downloaded, source.format, &normalized, &work_dir)?;
        let normalized_digest = hash_file(&normalized, HashAlgorithm::Sha256)?;
        let relative_object = format!("objects/{normalized_digest}.raw");
        let image_path = images_dir.join(&relative_object);
        if image_path.exists() {
            fs::remove_file(&normalized).map_err(state_error)?;
        } else {
            fs::rename(&normalized, &image_path).map_err(state_error)?;
        }

        let release = source
            .release
            .as_deref()
            .unwrap_or(entry.release)
            .to_string();
        let manifest = alias_manifest(
            entry,
            &source,
            &normalized_digest,
            &relative_object,
            &release,
        );
        for registered_alias in entry.aliases {
            write_json_atomic(
                &images_dir
                    .join("aliases")
                    .join(format!("{registered_alias}.json")),
                &manifest,
            )?;
        }
        Ok(PullResult {
            requested_alias: alias.to_string(),
            canonical_alias: entry.canonical.to_string(),
            distribution: entry.distribution.to_string(),
            release,
            source_url: source.url.clone(),
            source_format: source.format.label().to_string(),
            source_algorithm: source.algorithm.name().to_string(),
            source_digest: source.digest.clone(),
            normalized_digest,
            image_path,
            manifest_path: images_dir.join("aliases").join(format!("{alias}.json")),
            unchanged: false,
            sealed: false,
            aliases: entry
                .aliases
                .iter()
                .map(|value| value.to_string())
                .collect(),
        })
    })();
    let _ = fs::remove_dir_all(&work_dir);
    operation
}

fn catalog_entry(alias: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|entry| entry.aliases.contains(&alias))
}

pub fn aliases() -> Vec<&'static str> {
    CATALOG
        .iter()
        .flat_map(|entry| entry.aliases.iter().copied())
        .collect()
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ListImageTag {
    pub tag: String,
    pub path: PathBuf,
    pub sha256: String,
    pub format: String,
    pub baked: bool,
    pub sealed: bool,
    pub agent_version: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ListImage {
    pub alias: String,
    pub canonical_alias: String,
    pub aliases: Vec<String>,
    pub distribution: String,
    pub release: String,
    pub architecture: String,
    pub path: PathBuf,
    pub sha256: String,
    pub format: String,
    pub baked: bool,
    pub sealed: bool,
    pub agent_version: Option<String>,
    pub tags: Vec<ListImageTag>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CatalogAlias {
    pub alias: String,
    pub aliases: Vec<String>,
    pub distribution: String,
    pub release: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ListResult {
    pub images_dir: PathBuf,
    pub images: Vec<ListImage>,
    pub catalog: Vec<CatalogAlias>,
}

pub fn catalog() -> Vec<CatalogAlias> {
    CATALOG
        .iter()
        .map(|entry| CatalogAlias {
            alias: entry.canonical.to_string(),
            aliases: entry
                .aliases
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            distribution: entry.distribution.to_string(),
            release: entry.release.to_string(),
        })
        .collect()
}

pub fn list(images_dir: &Path) -> Result<ListResult, PullFailure> {
    let mut images = Vec::new();
    let aliases_directory = images_dir.join("aliases");
    if aliases_directory.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&aliases_directory)
            .map_err(state_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(state_error)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let alias = path
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| is_safe_alias(value))
                .ok_or_else(|| {
                    PullFailure::new(
                        EXIT_IMAGE_STATE,
                        format!("invalid alias manifest name {}", path.display()),
                    )
                })?
                .to_string();
            images.push(list_image_from_manifest(images_dir, &alias, &path)?);
        }
    }
    Ok(ListResult {
        images_dir: images_dir.to_path_buf(),
        images,
        catalog: catalog(),
    })
}

fn list_image_from_manifest(
    images_dir: &Path,
    alias: &str,
    manifest_path: &Path,
) -> Result<ListImage, PullFailure> {
    let bytes = fs::read(manifest_path).map_err(state_error)?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
        PullFailure::new(
            EXIT_IMAGE_STATE,
            format!(
                "invalid alias manifest {}: {error}",
                manifest_path.display()
            ),
        )
    })?;
    if manifest["apiVersion"] != "vzctl.dev/image-alias/v1" {
        return Err(PullFailure::new(
            EXIT_IMAGE_STATE,
            format!("unsupported alias manifest {}", manifest_path.display()),
        ));
    }
    let tags = list_tags_from_manifest(images_dir, &manifest)?;
    let object_relative = safe_object_path(&manifest)?;
    let object_sha = manifest["image"]["sha256"]
        .as_str()
        .ok_or_else(|| PullFailure::new(EXIT_IMAGE_STATE, "alias manifest lacks image.sha256"))?
        .to_string();
    let object_format = manifest["image"]["format"]
        .as_str()
        .unwrap_or("raw")
        .to_string();
    let (path, sha256, format, baked, sealed, agent_version) = if let Some(tag) = tags
        .iter()
        .find(|tag| tag.sealed)
        .or_else(|| tags.iter().find(|tag| tag.baked))
    {
        (
            tag.path.clone(),
            tag.sha256.clone(),
            tag.format.clone(),
            tag.baked,
            tag.sealed,
            tag.agent_version.clone(),
        )
    } else {
        (
            images_dir.join(&object_relative),
            object_sha,
            object_format,
            false,
            false,
            None,
        )
    };
    let aliases = manifest["aliases"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(ListImage {
        alias: alias.to_string(),
        canonical_alias: manifest["canonical_alias"]
            .as_str()
            .unwrap_or(alias)
            .to_string(),
        aliases,
        distribution: manifest["distribution"].as_str().unwrap_or("").to_string(),
        release: manifest["release"].as_str().unwrap_or("").to_string(),
        architecture: manifest["architecture"]
            .as_str()
            .unwrap_or("arm64")
            .to_string(),
        path,
        sha256,
        format,
        baked,
        sealed,
        agent_version,
        tags,
    })
}

fn list_tags_from_manifest(
    images_dir: &Path,
    manifest: &Value,
) -> Result<Vec<ListImageTag>, PullFailure> {
    let Some(tags_object) = manifest.get("tags").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut tags = Vec::new();
    for (tag, entry) in tags_object {
        if !valid_image_tag(tag) {
            continue;
        }
        let sealed = entry.get("sealed") == Some(&Value::Bool(true));
        let baked = entry.get("baked") == Some(&Value::Bool(true));
        let (relative, sha256, format, agent_version) = if sealed {
            (
                safe_relative_image_path(entry, &["sealed_image", "path"], "sealed")?,
                entry["sealed_image"]["sha256"]
                    .as_str()
                    .ok_or_else(|| {
                        PullFailure::new(
                            EXIT_IMAGE_STATE,
                            "alias manifest tag lacks sealed_image.sha256",
                        )
                    })?
                    .to_string(),
                entry["sealed_image"]["format"]
                    .as_str()
                    .unwrap_or("raw")
                    .to_string(),
                entry
                    .pointer("/baked_image/agent_version")
                    .and_then(Value::as_str)
                    .or_else(|| entry["sealed_image"]["agent_version"].as_str())
                    .map(str::to_string),
            )
        } else if baked {
            (
                safe_relative_image_path(entry, &["baked_image", "path"], "baked")?,
                entry["baked_image"]["sha256"]
                    .as_str()
                    .ok_or_else(|| {
                        PullFailure::new(
                            EXIT_IMAGE_STATE,
                            "alias manifest tag lacks baked_image.sha256",
                        )
                    })?
                    .to_string(),
                entry["baked_image"]["format"]
                    .as_str()
                    .unwrap_or("raw")
                    .to_string(),
                entry["baked_image"]["agent_version"]
                    .as_str()
                    .map(str::to_string),
            )
        } else {
            continue;
        };
        tags.push(ListImageTag {
            tag: tag.clone(),
            path: images_dir.join(relative),
            sha256,
            format,
            baked: baked || sealed,
            sealed,
            agent_version,
        });
    }
    tags.sort_by(|left, right| left.tag.cmp(&right.tag));
    Ok(tags)
}

fn resolve_source(
    entry: &CatalogEntry,
    fetcher: &dyn Fetcher,
) -> Result<ResolvedSource, PullFailure> {
    match entry.resolver {
        Resolver::Static {
            url,
            filename,
            checksum,
            format,
        } => {
            let (algorithm, digest) = match checksum {
                Checksum::Inline { algorithm, digest } => (algorithm, digest.to_string()),
                Checksum::Remote { url, algorithm } => {
                    let text = fetcher.text(url)?;
                    (algorithm, digest_from_checksum(&text, filename, algorithm)?)
                }
            };
            Ok(ResolvedSource {
                url: url.to_string(),
                filename: filename.to_string(),
                format,
                algorithm,
                digest,
                release: None,
            })
        }
        Resolver::FedoraCoreOs => {
            let metadata_url = "https://builds.coreos.fedoraproject.org/streams/stable.json";
            let metadata: Value =
                serde_json::from_str(&fetcher.text(metadata_url)?).map_err(|error| {
                    PullFailure::new(
                        EXIT_IMAGE_NETWORK,
                        format!("invalid Fedora CoreOS stream metadata: {error}"),
                    )
                })?;
            let disk = &metadata["architectures"]["aarch64"]["artifacts"]["qemu"]["formats"]
                ["qcow2.xz"]["disk"];
            let url = required_json_string(disk, "location", "Fedora CoreOS location")?;
            let digest = required_json_string(disk, "sha256", "Fedora CoreOS sha256")?;
            let release = metadata["architectures"]["aarch64"]["artifacts"]["qemu"]["release"]
                .as_str()
                .map(str::to_string);
            Ok(ResolvedSource {
                filename: url
                    .rsplit('/')
                    .next()
                    .unwrap_or("fedora-coreos.qcow2.xz")
                    .to_string(),
                url,
                format: SourceFormat::Qcow2Xz,
                algorithm: HashAlgorithm::Sha256,
                digest,
                release,
            })
        }
        Resolver::Flatcar => {
            let version_url =
                "https://stable.release.flatcar-linux.net/arm64-usr/current/version.txt";
            let version_text = fetcher.text(version_url)?;
            let version = version_text
                .lines()
                .find_map(|line| line.strip_prefix("FLATCAR_VERSION="))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    PullFailure::new(
                        EXIT_IMAGE_NETWORK,
                        "Flatcar version metadata lacks FLATCAR_VERSION",
                    )
                })?;
            let base = format!("https://stable.release.flatcar-linux.net/arm64-usr/{version}");
            let filename = "flatcar_production_qemu_uefi_image.img.bz2";
            let checksum_url = format!("{base}/{filename}.DIGESTS");
            let checksum = fetcher.text(&checksum_url)?;
            Ok(ResolvedSource {
                url: format!("{base}/{filename}"),
                filename: filename.to_string(),
                format: SourceFormat::RawBz2,
                algorithm: HashAlgorithm::Sha512,
                digest: digest_from_checksum(&checksum, filename, HashAlgorithm::Sha512)?,
                release: Some(version.to_string()),
            })
        }
        Resolver::GithubLatest {
            repository,
            asset,
            format,
        } => resolve_github_asset(fetcher, repository, "latest", asset, format),
        Resolver::GithubTagged {
            repository,
            tag,
            asset,
            sha256,
            format,
        } => resolve_github_pinned_asset(fetcher, repository, tag, asset, sha256, format),
    }
}

fn resolve_github_pinned_asset(
    fetcher: &dyn Fetcher,
    repository: &str,
    tag: &str,
    asset_name: &str,
    sha256: &str,
    format: SourceFormat,
) -> Result<ResolvedSource, PullFailure> {
    validate_digest_text(HashAlgorithm::Sha256, sha256)?;
    let api_url = format!("https://api.github.com/repos/{repository}/releases/tags/{tag}");
    let release: Value = serde_json::from_str(&fetcher.text(&api_url)?).map_err(|error| {
        PullFailure::new(
            EXIT_IMAGE_NETWORK,
            format!("invalid GitHub release metadata for {repository}: {error}"),
        )
    })?;
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|candidate| candidate["name"] == asset_name)
        })
        .ok_or_else(|| {
            PullFailure::new(
                EXIT_IMAGE_NETWORK,
                format!("GitHub release for {repository} has no asset {asset_name}"),
            )
        })?;
    Ok(ResolvedSource {
        url: required_json_string(asset, "browser_download_url", "GitHub asset URL")?,
        filename: asset_name.to_string(),
        format,
        algorithm: HashAlgorithm::Sha256,
        digest: sha256.to_string(),
        release: release["tag_name"].as_str().map(str::to_string),
    })
}

fn resolve_github_asset(
    fetcher: &dyn Fetcher,
    repository: &str,
    selector: &str,
    asset_name: &str,
    format: SourceFormat,
) -> Result<ResolvedSource, PullFailure> {
    let api_url = format!("https://api.github.com/repos/{repository}/releases/{selector}");
    let release: Value = serde_json::from_str(&fetcher.text(&api_url)?).map_err(|error| {
        PullFailure::new(
            EXIT_IMAGE_NETWORK,
            format!("invalid GitHub release metadata for {repository}: {error}"),
        )
    })?;
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|asset| asset["name"] == asset_name))
        .ok_or_else(|| {
            PullFailure::new(
                EXIT_IMAGE_NETWORK,
                format!("GitHub release for {repository} has no asset {asset_name}"),
            )
        })?;
    let url = required_json_string(asset, "browser_download_url", "GitHub asset URL")?;
    let digest = required_json_string(asset, "digest", "GitHub asset digest")?;
    let digest = digest.strip_prefix("sha256:").ok_or_else(|| {
        PullFailure::new(
            EXIT_IMAGE_CHECKSUM,
            format!("GitHub asset {asset_name} has no SHA256 digest"),
        )
    })?;
    Ok(ResolvedSource {
        url,
        filename: asset_name.to_string(),
        format,
        algorithm: HashAlgorithm::Sha256,
        digest: digest.to_string(),
        release: release["tag_name"].as_str().map(str::to_string),
    })
}

fn required_json_string(value: &Value, key: &str, label: &str) -> Result<String, PullFailure> {
    value[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            PullFailure::new(
                EXIT_IMAGE_NETWORK,
                format!("upstream metadata lacks {label}"),
            )
        })
}

fn digest_from_checksum(
    text: &str,
    filename: &str,
    algorithm: HashAlgorithm,
) -> Result<String, PullFailure> {
    let candidates = text
        .split(|character: char| character.is_whitespace() || matches!(character, '=' | '(' | ')'))
        .filter(|token| token.len() == algorithm.hex_len())
        .filter(|token| token.chars().all(|character| character.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let digest = text
        .lines()
        .filter(|line| line.contains(filename))
        .find_map(|line| {
            line.split(|character: char| {
                character.is_whitespace() || matches!(character, '=' | '(' | ')')
            })
            .find(|token| {
                token.len() == algorithm.hex_len()
                    && token.chars().all(|character| character.is_ascii_hexdigit())
            })
        })
        .map(str::to_ascii_lowercase)
        .or_else(|| (candidates.len() == 1).then(|| candidates[0].clone()))
        .ok_or_else(|| {
            PullFailure::new(
                EXIT_IMAGE_CHECKSUM,
                format!(
                    "cannot find {} checksum for {filename} in upstream metadata",
                    algorithm.name()
                ),
            )
        })?;
    validate_digest_text(algorithm, &digest)?;
    Ok(digest)
}

fn validate_digest_text(algorithm: HashAlgorithm, digest: &str) -> Result<(), PullFailure> {
    if digest.len() == algorithm.hex_len()
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(PullFailure::new(
            EXIT_IMAGE_CHECKSUM,
            format!("invalid {} digest from upstream", algorithm.name()),
        ))
    }
}

fn hash_file(path: &Path, algorithm: HashAlgorithm) -> Result<String, PullFailure> {
    let mut file = File::open(path).map_err(state_error)?;
    let mut buffer = [0_u8; 1024 * 1024];
    match algorithm {
        HashAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                let read = file.read(&mut buffer).map_err(state_error)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(hex(&hasher.finalize()))
        }
        HashAlgorithm::Sha512 => {
            let mut hasher = Sha512::new();
            loop {
                let read = file.read(&mut buffer).map_err(state_error)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            Ok(hex(&hasher.finalize()))
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    result
}

fn normalize(
    source: &Path,
    format: SourceFormat,
    destination: &Path,
    work_dir: &Path,
) -> Result<(), PullFailure> {
    match format {
        SourceFormat::Raw => fs::copy(source, destination)
            .map(|_| ())
            .map_err(state_error),
        SourceFormat::Qcow2 => qemu_convert(source, destination),
        SourceFormat::RawZst => decode("zstd", source, destination),
        SourceFormat::RawBz2 => decode("bzip2", source, destination),
        SourceFormat::Qcow2Xz => {
            let unpacked = work_dir.join("source.qcow2");
            decode("xz", source, &unpacked)?;
            qemu_convert(&unpacked, destination)
        }
        SourceFormat::Ova => {
            let extracted = work_dir.join("ova");
            fs::create_dir(&extracted).map_err(state_error)?;
            run_checked(
                Command::new("tar")
                    .arg("-xf")
                    .arg(source)
                    .arg("-C")
                    .arg(&extracted),
                "tar",
            )?;
            let disk = find_file_with_extension(&extracted, "vmdk")?.ok_or_else(|| {
                PullFailure::new(EXIT_IMAGE_STATE, "Photon OVA contains no VMDK disk")
            })?;
            qemu_convert(&disk, destination)
        }
        SourceFormat::ZipQcow2 => {
            let extracted = work_dir.join("zip");
            fs::create_dir(&extracted).map_err(state_error)?;
            run_checked(
                Command::new("unzip")
                    .arg("-q")
                    .arg(source)
                    .arg("-d")
                    .arg(&extracted),
                "unzip",
            )?;
            let disk = find_file_with_extension(&extracted, "qcow2")?.ok_or_else(|| {
                PullFailure::new(EXIT_IMAGE_STATE, "Arch VM archive contains no qcow2 disk")
            })?;
            qemu_convert(&disk, destination)
        }
    }
}

fn decode(program: &str, source: &Path, destination: &Path) -> Result<(), PullFailure> {
    let output = File::create(destination).map_err(state_error)?;
    let mut command = Command::new(program);
    command.arg("-dc").arg(source).stdout(Stdio::from(output));
    run_checked(&mut command, program)
}

fn qemu_convert(source: &Path, destination: &Path) -> Result<(), PullFailure> {
    let qemu = crate::hostbin::resolve("qemu-img").ok_or_else(|| {
        PullFailure::new(
            EXIT_UNAVAILABLE,
            "qemu-img is required for image normalization; run make vendor-qemu-img (or set VZCTL_QEMU_IMG)",
        )
    })?;
    let mut command = Command::new(qemu);
    command.arg("convert");
    if progress_enabled() {
        command.arg("-p");
    }
    command.arg("-O").arg("raw").arg(source).arg(destination);
    if progress_enabled() && !io::stderr().is_terminal() {
        run_with_progress_relay(
            &mut command,
            "Normalizing image…",
            EXIT_IMAGE_STATE,
            "qemu-img convert",
        )
    } else {
        run_checked(&mut command, "qemu-img")
    }
}

fn run_checked(command: &mut Command, program: &str) -> Result<(), PullFailure> {
    let status = command.status().map_err(|error| {
        PullFailure::new(
            EXIT_UNAVAILABLE,
            format!("{program} is required for image normalization: {error}"),
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(PullFailure::new(
            EXIT_IMAGE_STATE,
            format!("{program} failed while normalizing the image"),
        ))
    }
}

/// Run a child with piped stderr and emit newline percent lines for job logs.
///
/// curl `--progress-bar` and `qemu-img -p` update a single TTY line with CR;
/// supervisor job logs only see LF-delimited lines.
fn run_with_progress_relay(
    command: &mut Command,
    label: &str,
    fail_code: u8,
    context: &str,
) -> Result<(), PullFailure> {
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        PullFailure::new(
            EXIT_UNAVAILABLE,
            format!("{context} failed to start: {error}"),
        )
    })?;
    let mut stderr = child.stderr.take().ok_or_else(|| {
        PullFailure::new(EXIT_IMAGE_STATE, format!("{context} missing stderr pipe"))
    })?;
    let label_for_thread = label.to_string();
    let label_done = label.to_string();
    let relay = thread::spawn(move || relay_cr_progress(&mut stderr, &label_for_thread));
    let status = child.wait().map_err(|error| {
        PullFailure::new(EXIT_IMAGE_STATE, format!("{context} failed: {error}"))
    })?;
    let last_percent = relay.join().ok().flatten();
    if status.success() {
        if last_percent.is_some_and(|percent| percent < 100) {
            eprintln!("{label_done} 100%");
        }
        Ok(())
    } else {
        Err(PullFailure::new(fail_code, format!("{context} failed")))
    }
}

fn relay_cr_progress(reader: &mut impl Read, label: &str) -> Option<u8> {
    let mut buf = [0_u8; 512];
    let mut current = Vec::new();
    let mut last_percent: Option<u8> = None;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        for &byte in &buf[..n] {
            if byte == b'\r' || byte == b'\n' {
                emit_progress_line(label, &current, &mut last_percent);
                current.clear();
            } else {
                current.push(byte);
            }
        }
    }
    emit_progress_line(label, &current, &mut last_percent);
    last_percent
}

fn emit_progress_line(label: &str, raw: &[u8], last_percent: &mut Option<u8>) {
    let line = String::from_utf8_lossy(raw);
    let Some(percent) = parse_progress_percent(&line) else {
        return;
    };
    if *last_percent == Some(percent) {
        return;
    }
    *last_percent = Some(percent);
    eprintln!("{label} {percent}%");
}

/// Parse curl `--progress-bar` (`12.4%`) and qemu-img `-p` (`(30.00/100%)`) meters.
fn parse_progress_percent(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    let percent_at = trimmed.rfind('%')?;
    let before = trimmed[..percent_at].trim_end();
    let number = if let Some((left, right)) = before.rsplit_once('/') {
        if right.trim() == "100" {
            numeric_token(left.trim())?
        } else {
            numeric_token(before)?
        }
    } else {
        numeric_token(before)?
    };
    let value: f32 = number.parse().ok()?;
    Some(value.clamp(0.0, 100.0) as u8)
}

fn numeric_token(before: &str) -> Option<&str> {
    before
        .rsplit(|character: char| !character.is_ascii_digit() && character != '.')
        .find(|part| !part.is_empty())
}

fn find_file_with_extension(
    directory: &Path,
    extension: &str,
) -> Result<Option<PathBuf>, PullFailure> {
    let mut directories = vec![directory.to_path_buf()];
    while let Some(current) = directories.pop() {
        for entry in fs::read_dir(&current).map_err(state_error)? {
            let path = entry.map_err(state_error)?.path();
            if path.is_dir() {
                directories.push(path);
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn unchanged_result(
    alias: &str,
    entry: &CatalogEntry,
    source: &ResolvedSource,
    images_dir: &Path,
) -> Result<Option<PullResult>, PullFailure> {
    let manifest_path = images_dir.join("aliases").join(format!("{alias}.json"));
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).map_err(state_error)?)
        .map_err(|error| {
        PullFailure::new(
            EXIT_IMAGE_STATE,
            format!(
                "invalid alias manifest {}: {error}",
                manifest_path.display()
            ),
        )
    })?;
    if manifest["apiVersion"] != "vzctl.dev/image-alias/v1"
        || manifest["source"]["digest"] != source.digest
        || manifest["source"]["algorithm"] != source.algorithm.name()
    {
        return Ok(None);
    }
    let relative = safe_object_path(&manifest)?;
    let object_path = images_dir.join(&relative);
    if !object_path.is_file() {
        return Ok(None);
    }
    let object_digest = manifest["image"]["sha256"]
        .as_str()
        .ok_or_else(|| PullFailure::new(EXIT_IMAGE_STATE, "alias manifest lacks image.sha256"))?;
    if hash_file(&object_path, HashAlgorithm::Sha256)? != object_digest {
        return Err(PullFailure::new(
            EXIT_IMAGE_CHECKSUM,
            format!("local image checksum mismatch: {}", object_path.display()),
        ));
    }
    let sealed = manifest
        .get("tags")
        .and_then(Value::as_object)
        .is_some_and(|tags| {
            tags.values()
                .any(|entry| entry.get("sealed") == Some(&Value::Bool(true)))
        })
        || manifest["sealed"].as_bool().unwrap_or(false);
    // Pull idempotency only needs the content-addressed object. Tagged sealed
    // products are independent artifacts and are not re-hashed here.
    Ok(Some(PullResult {
        requested_alias: alias.to_string(),
        canonical_alias: entry.canonical.to_string(),
        distribution: entry.distribution.to_string(),
        release: manifest["release"]
            .as_str()
            .unwrap_or(entry.release)
            .to_string(),
        source_url: source.url.clone(),
        source_format: source.format.label().to_string(),
        source_algorithm: source.algorithm.name().to_string(),
        source_digest: source.digest.clone(),
        normalized_digest: object_digest.to_string(),
        image_path: object_path,
        manifest_path,
        unchanged: true,
        sealed,
        aliases: entry
            .aliases
            .iter()
            .map(|value| value.to_string())
            .collect(),
    }))
}

fn alias_manifest(
    entry: &CatalogEntry,
    source: &ResolvedSource,
    normalized_digest: &str,
    relative_object: &str,
    release: &str,
) -> Value {
    json!({
        "apiVersion": "vzctl.dev/image-alias/v1",
        "canonical_alias": entry.canonical,
        "aliases": entry.aliases,
        "distribution": entry.distribution,
        "release": release,
        "architecture": "arm64",
        "sealed": false,
        "tags": {},
        "source": {
            "url": source.url,
            "filename": source.filename,
            "format": source.format.label(),
            "algorithm": source.algorithm.name(),
            "digest": source.digest,
        },
        "image": {
            "path": relative_object,
            "format": "raw",
            "sha256": normalized_digest,
        },
    })
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<(), PullFailure> {
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        PullFailure::new(
            EXIT_IMAGE_STATE,
            format!("cannot serialize image alias: {error}"),
        )
    })?;
    {
        let mut file = File::create(&temporary).map_err(state_error)?;
        file.write_all(&bytes).map_err(state_error)?;
        file.write_all(b"\n").map_err(state_error)?;
        file.sync_all().map_err(state_error)?;
    }
    fs::rename(&temporary, path).map_err(state_error)
}

pub fn resolve_alias(images_dir: &Path, alias: &str) -> Result<Option<PathBuf>, String> {
    resolve_alias_pulled(images_dir, alias)
}

/// Resolve the content-addressed pull object for an alias (ignores tags).
pub fn resolve_alias_pulled(images_dir: &Path, alias: &str) -> Result<Option<PathBuf>, String> {
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Ok(None);
    };
    let relative = safe_object_path(&manifest).map_err(|error| error.message)?;
    let path = images_dir.join(relative);
    if !path.is_file() {
        return Err(format!(
            "alias {alias} references missing image {}",
            path.display()
        ));
    }
    Ok(Some(path))
}

/// Resolve a tagged sealed (preferred) or baked image path.
pub fn resolve_alias_tag(
    images_dir: &Path,
    alias: &str,
    tag: &str,
) -> Result<Option<PathBuf>, String> {
    if !valid_image_tag(tag) {
        return Err(format!("invalid image tag {tag}"));
    }
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Ok(None);
    };
    let Some(entry) = manifest.pointer(&format!("/tags/{tag}")) else {
        return Ok(None);
    };
    if entry.get("sealed") == Some(&Value::Bool(true)) {
        let relative = safe_relative_image_path(entry, &["sealed_image", "path"], "sealed")
            .map_err(|error| error.message)?;
        let path = images_dir.join(relative);
        if !path.is_file() {
            return Err(format!(
                "alias {alias} tag {tag} references missing sealed image {}",
                path.display()
            ));
        }
        return Ok(Some(path));
    }
    if entry.get("baked") == Some(&Value::Bool(true)) {
        let relative = safe_relative_image_path(entry, &["baked_image", "path"], "baked")
            .map_err(|error| error.message)?;
        let path = images_dir.join(relative);
        if !path.is_file() {
            return Err(format!(
                "alias {alias} tag {tag} references missing baked image {}",
                path.display()
            ));
        }
        return Ok(Some(path));
    }
    Ok(None)
}

/// True when `alias`/`tag` already has a sealed raw + seal marker (apply skip path).
pub fn tagged_seal_ready(images_dir: &Path, alias: &str, tag: &str) -> Result<bool, String> {
    if !valid_image_tag(tag) {
        return Err(format!("invalid image tag {tag}"));
    }
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Ok(false);
    };
    let Some(entry) = manifest.pointer(&format!("/tags/{tag}")) else {
        return Ok(false);
    };
    if entry.get("sealed") != Some(&Value::Bool(true)) {
        return Ok(false);
    }
    let relative = safe_relative_image_path(entry, &["sealed_image", "path"], "sealed")
        .map_err(|error| error.message)?;
    let path = images_dir.join(&relative);
    if !path.is_file() {
        return Ok(false);
    }
    let marker = entry["sealed_image"]["marker"]
        .as_str()
        .map(PathBuf::from)
        .filter(|marker| marker.is_file());
    Ok(marker.is_some())
}

fn read_alias_manifest(images_dir: &Path, alias: &str) -> Result<Option<Value>, String> {
    if !is_safe_alias(alias) {
        return Ok(None);
    }
    let manifest_path = images_dir.join("aliases").join(format!("{alias}.json"));
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid alias manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    if manifest["apiVersion"] != "vzctl.dev/image-alias/v1" {
        return Err(format!(
            "unsupported alias manifest {}",
            manifest_path.display()
        ));
    }
    Ok(Some(manifest))
}

pub fn prepare_alias_for_seal(
    images_dir: &Path,
    alias: &str,
    tag: &str,
) -> Result<Option<PathBuf>, String> {
    if !valid_image_tag(tag) {
        return Err(format!("invalid image tag {tag}"));
    }
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Ok(None);
    };
    if let Some(entry) = manifest.pointer(&format!("/tags/{tag}")) {
        if entry.get("sealed") == Some(&Value::Bool(true)) {
            return resolve_alias_tag(images_dir, alias, tag);
        }
    }

    let source = if let Some(entry) = manifest.pointer(&format!("/tags/{tag}")) {
        if entry.get("baked") == Some(&Value::Bool(true)) {
            images_dir.join(
                safe_relative_image_path(entry, &["baked_image", "path"], "baked")
                    .map_err(|error| error.message)?,
            )
        } else {
            images_dir.join(safe_object_path(&manifest).map_err(|error| error.message)?)
        }
    } else {
        images_dir.join(safe_object_path(&manifest).map_err(|error| error.message)?)
    };
    if !source.is_file() {
        return Err(format!(
            "alias {alias} references missing image {}",
            source.display()
        ));
    }
    let canonical = manifest["canonical_alias"]
        .as_str()
        .filter(|value| is_safe_alias(value))
        .ok_or_else(|| "alias manifest has invalid canonical_alias".to_string())?;
    let sealed_directory = images_dir.join("sealed");
    fs::create_dir_all(&sealed_directory)
        .map_err(|error| format!("cannot create {}: {error}", sealed_directory.display()))?;
    let destination = sealed_directory.join(format!("{canonical}@{tag}.raw"));
    if destination.exists() {
        let mut permissions = fs::metadata(&destination)
            .map_err(|error| format!("cannot inspect {}: {error}", destination.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&destination, permissions)
            .map_err(|error| format!("cannot make {} writable: {error}", destination.display()))?;
    }
    fs::copy(&source, &destination).map_err(|error| {
        format!(
            "cannot materialize alias {alias} tag {tag} from {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    let mut permissions = fs::metadata(&destination)
        .map_err(|error| format!("cannot inspect {}: {error}", destination.display()))?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&destination, permissions)
        .map_err(|error| format!("cannot make {} writable: {error}", destination.display()))?;
    Ok(Some(destination))
}

#[derive(Debug, Eq, PartialEq)]
pub struct BakeResult {
    pub requested_alias: String,
    pub canonical_alias: String,
    pub tag: String,
    pub image_path: PathBuf,
    pub agent_version: String,
    pub unchanged: bool,
}

pub fn prepare_alias_for_bake(
    images_dir: &Path,
    alias: &str,
    tag: &str,
) -> Result<(PathBuf, Value, PathBuf), String> {
    if !is_safe_alias(alias) {
        return Err(format!("invalid image alias {alias}"));
    }
    if !valid_image_tag(tag) {
        return Err(format!("invalid image tag {tag}"));
    }
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Err(format!(
            "alias {alias} not found; run `vzctl image pull {alias}` first"
        ));
    };
    if let Some(entry) = manifest.pointer(&format!("/tags/{tag}")) {
        if entry.get("sealed") == Some(&Value::Bool(true)) {
            return Err(format!(
                "alias {alias} tag {tag} is already sealed; choose another tag, or pull a fresh image"
            ));
        }
    }
    let canonical = manifest["canonical_alias"]
        .as_str()
        .filter(|value| is_safe_alias(value))
        .ok_or_else(|| "alias manifest has invalid canonical_alias".to_string())?
        .to_string();
    let source = images_dir.join(safe_object_path(&manifest).map_err(|error| error.message)?);
    if !source.is_file() {
        return Err(format!(
            "alias {alias} references missing image {}",
            source.display()
        ));
    }
    let baked_directory = images_dir.join("baked");
    fs::create_dir_all(&baked_directory)
        .map_err(|error| format!("cannot create {}: {error}", baked_directory.display()))?;
    let destination = baked_directory.join(format!("{canonical}@{tag}.raw"));
    if destination.exists() {
        let mut permissions = fs::metadata(&destination)
            .map_err(|error| format!("cannot inspect {}: {error}", destination.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&destination, permissions)
            .map_err(|error| format!("cannot make {} writable: {error}", destination.display()))?;
    }
    fs::copy(&source, &destination)
        .map_err(|error| format!("cannot materialize bake target for {alias}@{tag}: {error}"))?;
    let mut permissions = fs::metadata(&destination)
        .map_err(|error| format!("cannot inspect {}: {error}", destination.display()))?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&destination, permissions)
        .map_err(|error| format!("cannot make {} writable: {error}", destination.display()))?;
    let manifest_path = images_dir.join("aliases").join(format!("{alias}.json"));
    Ok((destination, manifest, manifest_path))
}

pub fn mark_aliases_baked(
    images_dir: &Path,
    baked_path: &Path,
    tag: &str,
    agent_version: &str,
) -> Result<(), String> {
    if !valid_image_tag(tag) {
        return Err(format!("invalid image tag {tag}"));
    }
    let aliases_directory = images_dir.join("aliases");
    if !aliases_directory.is_dir() {
        return Ok(());
    }
    let relative_baked = baked_path.strip_prefix(images_dir).map_err(|_| {
        format!(
            "baked image {} is outside {}",
            baked_path.display(),
            images_dir.display()
        )
    })?;
    let expected = relative_baked
        .to_str()
        .ok_or_else(|| "baked path is not UTF-8".to_string())?
        .to_string();
    let digest = hash_file(baked_path, HashAlgorithm::Sha256).map_err(|error| error.message)?;
    for entry in fs::read_dir(&aliases_directory)
        .map_err(|error| format!("cannot read {}: {error}", aliases_directory.display()))?
    {
        let manifest_path = entry
            .map_err(|error| format!("cannot read alias entry: {error}"))?
            .path();
        if manifest_path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&manifest_path)
            .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
        let mut manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "invalid alias manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        if manifest["apiVersion"] != "vzctl.dev/image-alias/v1" {
            continue;
        }
        let Some(canonical) = manifest["canonical_alias"]
            .as_str()
            .filter(|value| is_safe_alias(value))
        else {
            continue;
        };
        if format!("baked/{canonical}@{tag}.raw") != expected {
            continue;
        }
        if !manifest.get("tags").map(Value::is_object).unwrap_or(false) {
            manifest["tags"] = json!({});
        }
        manifest["tags"][tag] = json!({
            "baked": true,
            "baked_image": {
                "path": relative_baked,
                "format": "raw",
                "sha256": digest,
                "agent_version": agent_version,
            },
        });
        // Convenience mirrors for tools that still read flat fields.
        manifest["baked"] = Value::Bool(true);
        manifest["baked_image"] = manifest["tags"][tag]["baked_image"].clone();
        write_json_atomic(&manifest_path, &manifest).map_err(|error| error.message)?;
    }
    Ok(())
}

pub fn already_baked(
    images_dir: &Path,
    alias: &str,
    tag: &str,
    agent_version: &str,
) -> Result<Option<BakeResult>, String> {
    if !is_safe_alias(alias) || !valid_image_tag(tag) {
        return Ok(None);
    }
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Ok(None);
    };
    let Some(entry) = manifest.pointer(&format!("/tags/{tag}")) else {
        return Ok(None);
    };
    if entry.get("baked") != Some(&Value::Bool(true))
        && entry.get("sealed") != Some(&Value::Bool(true))
    {
        return Ok(None);
    }
    if entry.get("sealed") == Some(&Value::Bool(true)) {
        // Sealed tag is already past bake; treat as unchanged bake for apply.
        let path = images_dir.join(
            safe_relative_image_path(entry, &["sealed_image", "path"], "sealed")
                .map_err(|error| error.message)?,
        );
        if !path.is_file() {
            return Ok(None);
        }
        return Ok(Some(BakeResult {
            requested_alias: alias.to_string(),
            canonical_alias: manifest["canonical_alias"]
                .as_str()
                .unwrap_or(alias)
                .to_string(),
            tag: tag.to_string(),
            image_path: path,
            agent_version: agent_version.to_string(),
            unchanged: true,
        }));
    }
    let recorded = entry
        .pointer("/baked_image/agent_version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if recorded != agent_version {
        return Ok(None);
    }
    let path = images_dir.join(
        safe_relative_image_path(entry, &["baked_image", "path"], "baked")
            .map_err(|error| error.message)?,
    );
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(BakeResult {
        requested_alias: alias.to_string(),
        canonical_alias: manifest["canonical_alias"]
            .as_str()
            .unwrap_or(alias)
            .to_string(),
        tag: tag.to_string(),
        image_path: path,
        agent_version: agent_version.to_string(),
        unchanged: true,
    }))
}

/// Update alias manifests for a sealed tagged path.
///
/// When `digest` is `Some`, it is written without hashing the file (idempotent
/// apply path). When `None`, the sealed raw is hashed once.
pub fn mark_aliases_sealed(
    images_dir: &Path,
    sealed_path: &Path,
    marker_path: &Path,
    tag: &str,
    digest: Option<&str>,
) -> Result<(), String> {
    if !valid_image_tag(tag) {
        return Err(format!("invalid image tag {tag}"));
    }
    let aliases_directory = images_dir.join("aliases");
    if !aliases_directory.is_dir() {
        return Ok(());
    }
    let relative_sealed = sealed_path.strip_prefix(images_dir).map_err(|_| {
        format!(
            "sealed image {} is outside {}",
            sealed_path.display(),
            images_dir.display()
        )
    })?;
    let expected = relative_sealed
        .to_str()
        .ok_or_else(|| "sealed path is not UTF-8".to_string())?
        .to_string();
    let digest = match digest {
        Some(value) => value.to_string(),
        None => hash_file(sealed_path, HashAlgorithm::Sha256).map_err(|error| error.message)?,
    };
    for entry in fs::read_dir(&aliases_directory)
        .map_err(|error| format!("cannot read {}: {error}", aliases_directory.display()))?
    {
        let manifest_path = entry
            .map_err(|error| format!("cannot read alias entry: {error}"))?
            .path();
        if manifest_path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&manifest_path)
            .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
        let mut manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "invalid alias manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        if manifest["apiVersion"] != "vzctl.dev/image-alias/v1" {
            continue;
        }
        let Some(canonical) = manifest["canonical_alias"]
            .as_str()
            .filter(|value| is_safe_alias(value))
        else {
            continue;
        };
        if format!("sealed/{canonical}@{tag}.raw") != expected {
            continue;
        }
        if !manifest.get("tags").map(Value::is_object).unwrap_or(false) {
            manifest["tags"] = json!({});
        }
        let mut tag_entry = manifest["tags"]
            .get(tag)
            .cloned()
            .unwrap_or_else(|| json!({}));
        tag_entry["sealed"] = Value::Bool(true);
        tag_entry["baked"] = Value::Bool(true);
        tag_entry["sealed_image"] = json!({
            "path": relative_sealed,
            "format": "raw",
            "sha256": digest,
            "marker": marker_path,
        });
        manifest["tags"][tag] = tag_entry;
        manifest["sealed"] = Value::Bool(true);
        manifest["sealed_image"] = manifest["tags"][tag]["sealed_image"].clone();
        write_json_atomic(&manifest_path, &manifest).map_err(|error| error.message)?;
    }
    Ok(())
}

/// Return existing sealed digest for tag without hashing, when present.
pub fn existing_tag_sealed_digest(
    images_dir: &Path,
    alias: &str,
    tag: &str,
) -> Result<Option<String>, String> {
    let Some(manifest) = read_alias_manifest(images_dir, alias)? else {
        return Ok(None);
    };
    Ok(manifest
        .pointer(&format!("/tags/{tag}/sealed_image/sha256"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

pub fn valid_image_tag(tag: &str) -> bool {
    (1..=64).contains(&tag.len())
        && tag.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'-' | b'_'))
        })
}

fn safe_object_path(manifest: &Value) -> Result<PathBuf, PullFailure> {
    safe_relative_image_path(manifest, &["image", "path"], "objects")
}

fn safe_relative_image_path(
    manifest: &Value,
    keys: &[&str],
    required_prefix: &str,
) -> Result<PathBuf, PullFailure> {
    let mut value = manifest;
    for key in keys {
        value = &value[*key];
    }
    let relative = value.as_str().ok_or_else(|| {
        PullFailure::new(
            EXIT_IMAGE_STATE,
            format!("alias manifest lacks {}", keys.join(".")),
        )
    })?;
    let path = PathBuf::from(relative);
    let safe = !path.is_absolute()
        && path.starts_with(required_prefix)
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if safe {
        Ok(path)
    } else {
        Err(PullFailure::new(
            EXIT_IMAGE_STATE,
            "alias manifest contains an unsafe image path",
        ))
    }
}

fn is_safe_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn state_error(error: io::Error) -> PullFailure {
    PullFailure::new(EXIT_IMAGE_STATE, format!("image store: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct FakeFetcher {
        texts: HashMap<String, String>,
        downloads: HashMap<String, Vec<u8>>,
        download_count: RefCell<usize>,
    }

    impl Fetcher for FakeFetcher {
        fn text(&self, url: &str) -> Result<String, PullFailure> {
            self.texts
                .get(url)
                .cloned()
                .ok_or_else(|| PullFailure::new(EXIT_IMAGE_NETWORK, format!("missing {url}")))
        }

        fn download(&self, url: &str, destination: &Path) -> Result<(), PullFailure> {
            *self.download_count.borrow_mut() += 1;
            fs::write(
                destination,
                self.downloads.get(url).ok_or_else(|| {
                    PullFailure::new(EXIT_IMAGE_NETWORK, format!("missing {url}"))
                })?,
            )
            .map_err(state_error)
        }
    }

    fn temporary_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "vzctl-image-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn catalog_contains_every_documented_alias_and_coreos_is_shared() {
        assert_eq!(
            aliases(),
            vec![
                "ubuntu-latest",
                "ubuntu-26.04",
                "ubuntu-24.04",
                "ubuntu-22.04",
                "ubuntu-20.04",
                "debian-latest",
                "debian-13",
                "debian-12",
                "debian-11",
                "alpine-latest",
                "arch-latest",
                "fedora-latest",
                "rocky-latest",
                "alma-latest",
                "opensuse-latest",
                "fedora-coreos-latest",
                "coreos-latest",
                "flatcar-latest",
                "photon-latest",
                "opensuse-microos-latest",
                "talos-latest",
            ]
        );
        assert_eq!(
            catalog_entry("coreos-latest").unwrap().canonical,
            "fedora-coreos-latest"
        );
        assert!(std::ptr::eq(
            catalog_entry("coreos-latest").unwrap(),
            catalog_entry("fedora-coreos-latest").unwrap()
        ));
    }

    #[test]
    fn versioned_ubuntu_and_debian_aliases_resolve_static_urls() {
        let ubuntu = catalog_entry("ubuntu-24.04").unwrap();
        assert_eq!(ubuntu.distribution, "Ubuntu");
        assert_eq!(ubuntu.release, "24.04 LTS");
        let Resolver::Static { url, filename, .. } = ubuntu.resolver else {
            panic!("expected static resolver")
        };
        assert!(url.contains("/24.04/"));
        assert!(filename.contains("24.04"));

        let debian = catalog_entry("debian-12").unwrap();
        assert_eq!(debian.distribution, "Debian");
        assert_eq!(debian.release, "12 (Bookworm)");
        let Resolver::Static { url, filename, .. } = debian.resolver else {
            panic!("expected static resolver")
        };
        assert!(url.contains("/bookworm/"));
        assert!(filename.contains("debian-12"));
    }

    #[test]
    fn checksum_parser_handles_gnu_bsd_and_signed_fedora_lines() {
        let digest = "a".repeat(64);
        assert_eq!(
            digest_from_checksum(
                &format!("{digest}  image.qcow2"),
                "image.qcow2",
                HashAlgorithm::Sha256
            )
            .unwrap(),
            digest
        );
        assert_eq!(
            digest_from_checksum(
                &format!("SHA256 (image.qcow2) = {digest}"),
                "image.qcow2",
                HashAlgorithm::Sha256
            )
            .unwrap(),
            digest
        );
    }

    #[test]
    fn checksum_mismatch_fails_without_publishing_alias() {
        let entry = catalog_entry("ubuntu-latest").unwrap();
        let Resolver::Static {
            url,
            filename,
            checksum: Checksum::Remote {
                url: checksum_url, ..
            },
            ..
        } = entry.resolver
        else {
            panic!("unexpected resolver")
        };
        let fetcher = FakeFetcher {
            texts: HashMap::from([(
                checksum_url.to_string(),
                format!("{}  {filename}", "0".repeat(64)),
            )]),
            downloads: HashMap::from([(url.to_string(), b"not the expected image".to_vec())]),
            download_count: RefCell::new(0),
        };
        let directory = temporary_directory("mismatch");
        let failure = pull_with("ubuntu-latest", &directory, &fetcher, false).unwrap_err();
        assert_eq!(failure.code, EXIT_IMAGE_CHECKSUM);
        assert!(!directory.join("aliases/ubuntu-latest.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn raw_pull_is_idempotent_and_resolves_alias_without_network_download() {
        static TEST_ENTRY: CatalogEntry = CatalogEntry {
            canonical: "test-latest",
            aliases: &["test-latest"],
            distribution: "Test",
            release: "1",
            resolver: Resolver::Static {
                url: "https://example.invalid/test.raw",
                filename: "test.raw",
                checksum: Checksum::Inline {
                    algorithm: HashAlgorithm::Sha256,
                    digest: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
                },
                format: SourceFormat::Raw,
            },
        };
        let fetcher = FakeFetcher {
            texts: HashMap::new(),
            downloads: HashMap::from([(
                "https://example.invalid/test.raw".to_string(),
                b"hello".to_vec(),
            )]),
            download_count: RefCell::new(0),
        };
        let directory = temporary_directory("idempotent");
        let source = resolve_source(&TEST_ENTRY, &fetcher).unwrap();
        fs::create_dir_all(directory.join("objects")).unwrap();
        fs::create_dir_all(directory.join("aliases")).unwrap();
        fs::create_dir_all(directory.join(".tmp")).unwrap();
        let work = directory.join(".tmp/manual");
        fs::create_dir(&work).unwrap();
        let downloaded = work.join("test.raw");
        fetcher.download(&source.url, &downloaded).unwrap();
        let normalized_digest = hash_file(&downloaded, HashAlgorithm::Sha256).unwrap();
        let relative = format!("objects/{normalized_digest}.raw");
        fs::rename(&downloaded, directory.join(&relative)).unwrap();
        let manifest = alias_manifest(&TEST_ENTRY, &source, &normalized_digest, &relative, "1");
        write_json_atomic(&directory.join("aliases/test-latest.json"), &manifest).unwrap();

        let result = unchanged_result("test-latest", &TEST_ENTRY, &source, &directory)
            .unwrap()
            .unwrap();
        assert!(result.unchanged);
        assert_eq!(*fetcher.download_count.borrow(), 1);
        assert_eq!(
            resolve_alias(&directory, "test-latest").unwrap(),
            Some(result.image_path)
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unsafe_alias_object_path_is_rejected() {
        let manifest = json!({ "image": { "path": "../outside.raw" } });
        assert_eq!(
            safe_object_path(&manifest).unwrap_err().code,
            EXIT_IMAGE_STATE
        );
    }

    #[test]
    fn sealing_alias_keeps_pulled_object_immutable_and_switches_resolution() {
        let directory = temporary_directory("seal-alias");
        fs::create_dir_all(directory.join("objects")).unwrap();
        fs::create_dir_all(directory.join("aliases")).unwrap();
        let object_digest = hash_file_from_bytes(b"pristine");
        let relative_object = format!("objects/{object_digest}.raw");
        let object_path = directory.join(&relative_object);
        fs::write(&object_path, b"pristine").unwrap();
        let source = ResolvedSource {
            url: "https://example.invalid/base.raw".to_string(),
            filename: "base.raw".to_string(),
            format: SourceFormat::Raw,
            algorithm: HashAlgorithm::Sha256,
            digest: object_digest.clone(),
            release: None,
        };
        let entry = CatalogEntry {
            canonical: "test-latest",
            aliases: &["test-latest", "test-short"],
            distribution: "Test",
            release: "1",
            resolver: Resolver::Static {
                url: "https://example.invalid/base.raw",
                filename: "base.raw",
                checksum: Checksum::Inline {
                    algorithm: HashAlgorithm::Sha256,
                    digest: "d4f1f7ac9b3e3a79ef3f9db6e94802c5909364e92b413fbe33f97276f61d2b3c",
                },
                format: SourceFormat::Raw,
            },
        };
        let manifest = alias_manifest(&entry, &source, &object_digest, &relative_object, "1");
        for alias in entry.aliases {
            write_json_atomic(
                &directory.join("aliases").join(format!("{alias}.json")),
                &manifest,
            )
            .unwrap();
        }

        let sealed = prepare_alias_for_seal(&directory, "test-short", "v1")
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&sealed).unwrap(), b"pristine");
        assert_eq!(sealed, directory.join("sealed/test-latest@v1.raw"));
        fs::write(&sealed, b"sealed-and-cleaned").unwrap();
        let marker = directory.join("test.sealed.json");
        fs::write(&marker, b"{}").unwrap();
        mark_aliases_sealed(&directory, &sealed, &marker, "v1", None).unwrap();

        assert_eq!(fs::read(&object_path).unwrap(), b"pristine");
        assert_eq!(
            resolve_alias_pulled(&directory, "test-latest").unwrap(),
            Some(object_path.clone())
        );
        assert_eq!(
            resolve_alias_tag(&directory, "test-latest", "v1").unwrap(),
            Some(sealed.clone())
        );
        assert_eq!(
            resolve_alias_tag(&directory, "test-short", "v1").unwrap(),
            Some(sealed)
        );
        let manifest: Value =
            serde_json::from_slice(&fs::read(directory.join("aliases/test-latest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["sealed"], true);
        assert_eq!(manifest["image"]["sha256"], object_digest);
        assert_eq!(
            manifest["tags"]["v1"]["sealed_image"]["sha256"],
            hash_file_from_bytes(b"sealed-and-cleaned")
        );
        assert!(tagged_seal_ready(&directory, "test-latest", "v1").unwrap());
        let unchanged = unchanged_result("test-latest", &entry, &source, &directory)
            .unwrap()
            .unwrap();
        assert!(unchanged.sealed);
        assert_eq!(unchanged.image_path, object_path);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn tagged_seal_ready_and_already_baked_are_per_tag() {
        let directory = temporary_directory("tag-ready");
        fs::create_dir_all(directory.join("objects")).unwrap();
        fs::create_dir_all(directory.join("aliases")).unwrap();
        fs::create_dir_all(directory.join("baked")).unwrap();
        fs::create_dir_all(directory.join("sealed")).unwrap();
        let object_digest = hash_file_from_bytes(b"object");
        let relative_object = format!("objects/{object_digest}.raw");
        fs::write(directory.join(&relative_object), b"object").unwrap();
        let baked = directory.join("baked/test-latest@v1.raw");
        fs::write(&baked, b"baked").unwrap();
        let sealed = directory.join("sealed/test-latest@v1.raw");
        fs::write(&sealed, b"sealed").unwrap();
        let marker = directory.join("test-v1.sealed.json");
        fs::write(&marker, b"{}").unwrap();
        write_json_atomic(
            &directory.join("aliases/test-latest.json"),
            &json!({
                "apiVersion": "vzctl.dev/image-alias/v1",
                "canonical_alias": "test-latest",
                "aliases": ["test-latest"],
                "distribution": "Test",
                "release": "1",
                "architecture": "arm64",
                "sealed": true,
                "tags": {
                    "v1": {
                        "baked": true,
                        "sealed": true,
                        "baked_image": {
                            "path": "baked/test-latest@v1.raw",
                            "format": "raw",
                            "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                            "agent_version": "9.9.9",
                        },
                        "sealed_image": {
                            "path": "sealed/test-latest@v1.raw",
                            "format": "raw",
                            "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                            "marker": marker,
                        }
                    }
                },
                "source": {
                    "url": "https://example.invalid/base.raw",
                    "filename": "base.raw",
                    "format": "raw",
                    "algorithm": "sha256",
                    "digest": object_digest,
                },
                "image": {
                    "path": relative_object,
                    "format": "raw",
                    "sha256": object_digest,
                },
            }),
        )
        .unwrap();

        assert!(tagged_seal_ready(&directory, "test-latest", "v1").unwrap());
        assert!(!tagged_seal_ready(&directory, "test-latest", "v2").unwrap());
        let baked_hit = already_baked(&directory, "test-latest", "v1", "9.9.9")
            .unwrap()
            .unwrap();
        assert!(baked_hit.unchanged);
        assert_eq!(baked_hit.tag, "v1");
        assert!(already_baked(&directory, "test-latest", "v2", "9.9.9")
            .unwrap()
            .is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    fn hash_file_from_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex(&hasher.finalize())
    }

    #[test]
    fn list_empty_store_returns_catalog() {
        let directory = std::env::temp_dir().join(format!(
            "vzctl-image-list-empty-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let result = list(&directory).unwrap();
        assert!(result.images.is_empty());
        assert!(!result.catalog.is_empty());
        assert_eq!(result.catalog.len(), CATALOG.len());
        assert!(result
            .catalog
            .iter()
            .any(|entry| entry.alias == "ubuntu-latest"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn list_reads_local_alias_manifests() {
        let directory = std::env::temp_dir().join(format!(
            "vzctl-image-list-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let aliases = directory.join("aliases");
        fs::create_dir_all(&aliases).unwrap();
        let object = directory
            .join("objects/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.raw");
        fs::create_dir_all(object.parent().unwrap()).unwrap();
        fs::write(&object, b"raw").unwrap();
        let baked = directory.join("baked/ubuntu-latest@v1.raw");
        fs::create_dir_all(baked.parent().unwrap()).unwrap();
        fs::write(&baked, b"baked").unwrap();
        write_json_atomic(
            &aliases.join("ubuntu-latest.json"),
            &json!({
                "apiVersion": "vzctl.dev/image-alias/v1",
                "canonical_alias": "ubuntu-latest",
                "aliases": ["ubuntu-latest"],
                "distribution": "Ubuntu",
                "release": "26.04 LTS",
                "architecture": "arm64",
                "sealed": false,
                "source": {
                    "url": "https://example.test/ubuntu.img",
                    "filename": "ubuntu.img",
                    "format": "qcow2",
                    "algorithm": "sha256",
                    "digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                },
                "image": {
                    "path": "objects/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.raw",
                    "format": "raw",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                },
                "tags": {
                    "v1": {
                        "baked": true,
                        "baked_image": {
                            "path": "baked/ubuntu-latest@v1.raw",
                            "format": "raw",
                            "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                            "agent_version": "1.2.3",
                        },
                    }
                },
            }),
        )
        .unwrap();

        let result = list(&directory).unwrap();
        assert_eq!(result.images.len(), 1);
        let image = &result.images[0];
        assert_eq!(image.alias, "ubuntu-latest");
        assert!(image.baked);
        assert!(!image.sealed);
        assert_eq!(image.agent_version.as_deref(), Some("1.2.3"));
        assert_eq!(image.path, baked);
        assert_eq!(image.tags.len(), 1);
        assert_eq!(image.tags[0].tag, "v1");
        assert_eq!(
            image.sha256,
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        );
        assert!(!result.catalog.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn progress_env_enabled_accepts_truthy_values() {
        // Isolate from ambient env in the test process.
        let previous = std::env::var_os("VZCTL_PROGRESS");
        std::env::remove_var("VZCTL_PROGRESS");
        assert!(!progress_env_enabled());
        std::env::set_var("VZCTL_PROGRESS", "1");
        assert!(progress_env_enabled());
        std::env::set_var("VZCTL_PROGRESS", "false");
        assert!(!progress_env_enabled());
        std::env::set_var("VZCTL_PROGRESS", "yes");
        assert!(progress_env_enabled());
        match previous {
            Some(value) => std::env::set_var("VZCTL_PROGRESS", value),
            None => std::env::remove_var("VZCTL_PROGRESS"),
        }
    }

    #[test]
    fn parse_progress_percent_reads_curl_and_qemu_meters() {
        assert_eq!(parse_progress_percent("#  12.4%"), Some(12));
        assert_eq!(
            parse_progress_percent(
                "######################################################################## 100.0%"
            ),
            Some(100)
        );
        assert_eq!(parse_progress_percent("    (0.00/100%)"), Some(0));
        assert_eq!(parse_progress_percent("    (30.00/100%)"), Some(30));
        assert_eq!(parse_progress_percent("    (99.91/100%)"), Some(99));
        assert_eq!(parse_progress_percent("no meter"), None);
    }

    #[test]
    #[ignore = "requires upstream network access; no image payloads are downloaded"]
    fn live_catalog_metadata_resolves_every_alias() {
        for entry in CATALOG {
            let source = resolve_source(entry, &CurlFetcher::new())
                .unwrap_or_else(|error| panic!("{}: {}", entry.canonical, error.message));
            validate_digest_text(source.algorithm, &source.digest).unwrap();
            assert!(source.url.starts_with("https://"), "{}", entry.canonical);
        }
    }
}
