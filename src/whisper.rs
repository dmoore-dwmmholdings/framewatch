//! Managed whisper.cpp provisioning for the `record` feature.

use crate::error::TranscribeError;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use sha2::{Digest, Sha256};
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::fmt::Write as _;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::fs::{self, File};
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::io::{self, Read};
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::process::Command;

/// Pinned whisper.cpp release used by managed transcription.
pub const WHISPER_VERSION: &str = "1.9.2";
/// Pinned GGML model used by managed transcription.
pub const WHISPER_MODEL: &str = "base.en";

#[cfg(windows)]
const WINDOWS_RUNTIME_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-x64.zip";
#[cfg(windows)]
const WINDOWS_RUNTIME_SHA256: &str =
    "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a";
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
const MODEL_SHA256: &str = "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002";
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
const MODEL_FILE: &str = "ggml-base.en.bin";
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
const INSTALL_MARKER: &str = "framewatch-managed-whisper.txt";

#[cfg(windows)]
fn runtime_url() -> String {
    WINDOWS_RUNTIME_URL.into()
}

#[cfg(target_os = "macos")]
fn runtime_url() -> String {
    format!(
        "https://github.com/dmoore-dwmmholdings/framewatch/releases/download/v{}/{}",
        env!("CARGO_PKG_VERSION"),
        runtime_archive_name()
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const LINUX_RUNTIME_SHA256: &str =
    "46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const LINUX_RUNTIME_SHA256: &str =
    "7e26fa6a36d9174d5c0bf033ccbc026c3b5e569e2ee787058241346ef5392719";

#[cfg(target_os = "linux")]
fn runtime_url() -> String {
    format!(
        "https://github.com/ggml-org/whisper.cpp/releases/download/v{WHISPER_VERSION}/{}",
        runtime_archive_name()
    )
}

#[cfg(windows)]
fn runtime_archive_name() -> &'static str {
    "whisper-bin-x64.zip"
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn runtime_archive_name() -> &'static str {
    "framewatch-whisper-aarch64-apple-darwin.zip"
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn runtime_archive_name() -> &'static str {
    "framewatch-whisper-x86_64-apple-darwin.zip"
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn runtime_archive_name() -> &'static str {
    "whisper-bin-ubuntu-x64.tar.gz"
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn runtime_archive_name() -> &'static str {
    "whisper-bin-ubuntu-arm64.tar.gz"
}

#[cfg(windows)]
fn runtime_executable(root: &Path) -> PathBuf {
    root.join("Release").join("whisper-cli.exe")
}

#[cfg(target_os = "macos")]
fn runtime_executable(root: &Path) -> PathBuf {
    root.join("whisper-cli")
}

#[cfg(target_os = "linux")]
fn runtime_executable(root: &Path) -> PathBuf {
    root.join("whisper-cli")
}

#[cfg(windows)]
fn runtime_sha256() -> Result<String, TranscribeError> {
    Ok(WINDOWS_RUNTIME_SHA256.into())
}

#[cfg(target_os = "macos")]
fn runtime_sha256() -> Result<String, TranscribeError> {
    let checksum_url = format!("{}.sha256", runtime_url());
    let mut response = ureq::get(&checksum_url).call().map_err(|error| {
        TranscribeError::Setup(format!("download failed for {checksum_url}: {error}"))
    })?;
    let mut checksum = String::new();
    response
        .body_mut()
        .as_reader()
        .read_to_string(&mut checksum)?;
    checksum
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| {
            TranscribeError::Setup(format!("invalid runtime checksum file: {checksum_url}"))
        })
}

#[cfg(target_os = "linux")]
fn runtime_sha256() -> Result<String, TranscribeError> {
    Ok(LINUX_RUNTIME_SHA256.into())
}

#[cfg(windows)]
fn runtime_marker() -> &'static str {
    WINDOWS_RUNTIME_SHA256
}

#[cfg(target_os = "macos")]
fn runtime_marker() -> &'static str {
    "release-sidecar-sha256"
}

#[cfg(target_os = "linux")]
fn runtime_marker() -> &'static str {
    LINUX_RUNTIME_SHA256
}

/// Paths to a ready managed whisper.cpp installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedWhisper {
    /// Absolute path to `whisper-cli.exe`.
    pub executable: PathBuf,
    /// Absolute path to the pinned GGML model.
    pub model: PathBuf,
}

/// Ensure the pinned whisper.cpp runtime and model exist in the user cache.
///
/// The first call downloads about 150 MiB over HTTPS and verifies SHA-256
/// digests before making the installation visible. Set
/// `FRAMEWATCH_WHISPER_DIR` to override the cache parent directory.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub fn ensure_managed_whisper() -> Result<ManagedWhisper, TranscribeError> {
    if !cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        return Err(TranscribeError::Setup(
            "managed whisper.cpp requires Windows x64, macOS Apple Silicon/Intel, or Linux x86_64/aarch64".into(),
        ));
    }
    let parent = match std::env::var_os("FRAMEWATCH_WHISPER_DIR") {
        Some(path) => PathBuf::from(path),
        None => dirs::cache_dir()
            .ok_or_else(|| TranscribeError::Setup("could not resolve user cache directory".into()))?
            .join("framewatch")
            .join("whisper.cpp"),
    };
    ensure_managed_whisper_at(&parent.join(format!("v{WHISPER_VERSION}")))
}

/// Managed whisper.cpp is provisioned on the platforms supported by live recording.
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn ensure_managed_whisper() -> Result<ManagedWhisper, TranscribeError> {
    Err(TranscribeError::Setup(
        "managed whisper.cpp is available only on Windows x64, macOS, and Linux x86_64/aarch64"
            .into(),
    ))
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn ensure_managed_whisper_at(root: &Path) -> Result<ManagedWhisper, TranscribeError> {
    let expected = installation_paths(root);
    if installation_ready(root, &expected) {
        validate_runtime(&expected)?;
        return Ok(expected);
    }
    if root.exists() {
        // This is a version-specific directory wholly owned by the managed
        // installer (even with FRAMEWATCH_WHISPER_DIR, the version is appended).
        // Repair an interrupted/corrupt first-use installation automatically.
        fs::remove_dir_all(root)?;
    }

    let parent = root.parent().ok_or_else(|| {
        TranscribeError::Setup(format!("invalid managed cache path: {}", root.display()))
    })?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".v{WHISPER_VERSION}-install-{}",
        std::process::id()
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;

    let install_result = install_into(&staging);
    if let Err(error) = install_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    match fs::rename(&staging, root) {
        Ok(()) => {}
        Err(_) if installation_ready(root, &expected) => {
            let _ = fs::remove_dir_all(&staging);
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    }

    validate_runtime(&expected)?;
    Ok(expected)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn install_into(staging: &Path) -> Result<(), TranscribeError> {
    let archive_path = staging.join(runtime_archive_name());
    download(&runtime_url(), &archive_path)?;
    verify_sha256(&archive_path, &runtime_sha256()?, "whisper.cpp runtime")?;
    extract_runtime_archive(&archive_path, staging)?;
    fs::remove_file(&archive_path)?;

    let model_path = staging.join(MODEL_FILE);
    download(MODEL_URL, &model_path)?;
    verify_sha256(&model_path, MODEL_SHA256, "Whisper model")?;

    fs::write(staging.join(INSTALL_MARKER), marker_contents())?;
    validate_runtime(&installation_paths(staging))
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn download(url: &str, destination: &Path) -> Result<(), TranscribeError> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|error| TranscribeError::Setup(format!("download failed for {url}: {error}")))?;
    let mut reader = response.body_mut().as_reader();
    let mut output = File::create(destination)?;
    io::copy(&mut reader, &mut output)?;
    output.sync_all()?;
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
fn extract_runtime_archive(archive_path: &Path, destination: &Path) -> Result<(), TranscribeError> {
    let archive_file = File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| TranscribeError::Setup(format!("invalid runtime archive: {error}")))?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            TranscribeError::Setup(format!("could not read runtime archive: {error}"))
        })?;
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            TranscribeError::Setup(format!("unsafe path in runtime archive: {}", entry.name()))
        })?;
        let output_path = destination.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&output_path)?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&output_path)?;
        io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn extract_runtime_archive(archive_path: &Path, destination: &Path) -> Result<(), TranscribeError> {
    let archive_file = File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|error| TranscribeError::Setup(format!("invalid runtime archive: {error}")))?
    {
        let mut entry = entry.map_err(|error| {
            TranscribeError::Setup(format!("could not read runtime archive: {error}"))
        })?;
        let path = entry.path().map_err(|error| {
            TranscribeError::Setup(format!("could not read runtime archive path: {error}"))
        })?;
        let mut relative = PathBuf::new();
        for component in path.components().skip(1) {
            relative.push(component.as_os_str());
        }
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(TranscribeError::Setup(format!(
                "unsafe path in runtime archive: {}",
                path.display()
            )));
        }
        entry.unpack(destination.join(relative)).map_err(|error| {
            TranscribeError::Setup(format!("could not extract runtime archive: {error}"))
        })?;
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn installation_paths(root: &Path) -> ManagedWhisper {
    ManagedWhisper {
        executable: runtime_executable(root),
        model: root.join(MODEL_FILE),
    }
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn installation_ready(root: &Path, install: &ManagedWhisper) -> bool {
    fs::read_to_string(root.join(INSTALL_MARKER)).is_ok_and(|value| value == marker_contents())
        && install.executable.is_file()
        && install.model.is_file()
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn marker_contents() -> String {
    format!(
        "whisper.cpp={WHISPER_VERSION}\nmodel={WHISPER_MODEL}\nruntime_sha256={}\nmodel_sha256={MODEL_SHA256}\n",
        runtime_marker()
    )
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn validate_runtime(install: &ManagedWhisper) -> Result<(), TranscribeError> {
    let output = Command::new(&install.executable)
        .arg("--version")
        .output()
        .map_err(|error| {
            TranscribeError::Setup(format!(
                "could not execute {}: {error}",
                install.executable.display()
            ))
        })?;
    if !output.status.success() {
        return Err(TranscribeError::Setup(format!(
            "{} --version exited with {}: {}",
            install.executable.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn verify_sha256(path: &Path, expected: &str, description: &str) -> Result<(), TranscribeError> {
    let actual = sha256(path)?;
    if actual != expected {
        return Err(TranscribeError::Setup(format!(
            "{description} checksum mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn sha256(path: &Path) -> Result<String, TranscribeError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(hex)
}

#[cfg(all(test, any(windows, target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn managed_paths_match_the_release_layout() {
        let paths = installation_paths(Path::new("cache/v1.9.2"));
        #[cfg(windows)]
        assert_eq!(
            paths.executable,
            Path::new("cache/v1.9.2/Release/whisper-cli.exe")
        );
        #[cfg(target_os = "macos")]
        assert_eq!(paths.executable, Path::new("cache/v1.9.2/whisper-cli"));
        #[cfg(target_os = "linux")]
        assert_eq!(paths.executable, Path::new("cache/v1.9.2/whisper-cli"));
        assert_eq!(paths.model, Path::new("cache/v1.9.2/ggml-base.en.bin"));
    }

    #[test]
    fn sha256_matches_a_known_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vector");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn install_marker_pins_runtime_and_model_hashes() {
        let marker = marker_contents();
        assert!(marker.contains("whisper.cpp=1.9.2"));
        assert!(marker.contains("model=base.en"));
        assert!(marker.contains(runtime_marker()));
        assert!(marker.contains(MODEL_SHA256));
    }
}
