//! Collecting package files and building a signed publication request.

use std::path::{Component, Path};

use seal_api::ApiSnapshot;
use seal_project::Project;
use seal_registry::publish::{
    PackageFile, PublishRequest, build_archive, document_digest, encode_archive, publisher,
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ui;

/// A built publication, ready to send or print.
pub(crate) struct Bundle {
    /// The request body.
    pub(crate) request: PublishRequest,
    /// File names and sizes in the archive.
    pub(crate) files: Vec<(String, u64)>,
    /// The compressed archive size.
    pub(crate) archive_size: usize,
    /// The archive digest.
    pub(crate) checksum: String,
    /// The API snapshot digest.
    pub(crate) api_digest: String,
    /// The SBOM digest.
    pub(crate) sbom_digest: String,
    /// The provenance envelope digest.
    pub(crate) provenance_digest: String,
}

/// Builds the archive, SBOM, API snapshot, and provenance envelope.
pub(crate) fn build(
    project: &Project,
    index: usize,
    snapshot: &ApiSnapshot,
    seed: &[u8; 32],
    identity: Option<&str>,
) -> Result<Bundle, ui::Failure> {
    let package = project.package(index);
    let manifest = package.manifest.package.as_ref().ok_or_else(|| {
        ui::failure(
            "E8093",
            format!("the package `{}` has no `[package]` table", package.name),
        )
    })?;
    if manifest
        .license
        .as_deref()
        .is_none_or(|license| license.trim().is_empty())
    {
        return Err(ui::failure(
            "E8093",
            format!(
                "the package `{}` must declare `license` before publishing",
                package.name
            ),
        ));
    }
    if let Some(readme) = &manifest.readme
        && !safe_join(&package.root, readme).is_some_and(|path| path.is_file())
    {
        return Err(ui::failure(
            "E8093",
            format!(
                "the package `{}` declares the readme `{readme}`, but it does not exist",
                package.name
            ),
        ));
    }
    let sbom = super::sbom::generate(project, index)?;
    let sbom_bytes = serde_json::to_vec_pretty(&sbom)
        .map_err(|error| ui::failure("E8031", format!("cannot encode the SBOM: {error}")))?;
    let api_bytes = snapshot
        .canonical_bytes()
        .map_err(|error| ui::failure(error.code(), error.message()))?;
    let documentation = seal_doc::Documentation::from_snapshot(snapshot);
    let docs_bytes = seal_doc::render_json(&documentation)
        .map_err(|error| ui::failure(error.code(), error.to_string()))?;

    let mut files: Vec<PackageFile> = Vec::new();
    let mut listed = Vec::new();
    add_file(&mut files, &package.root, Path::new("seal.toml"), true)?;
    if let Some(readme) = &manifest.readme {
        add_file(&mut files, &package.root, Path::new(readme), false)?;
    }
    for file in &package.files {
        let relative = file.strip_prefix(&package.root).map_err(|_| {
            ui::failure(
                "E8093",
                format!("`{}` is outside the package root", file.display()),
            )
        })?;
        add_file(&mut files, &package.root, relative, false)?;
    }
    files.push(PackageFile {
        path: "seal.api.json".to_owned(),
        contents: api_bytes.clone(),
        executable: false,
    });
    files.push(PackageFile {
        path: "seal.docs.json".to_owned(),
        contents: docs_bytes.into_bytes(),
        executable: false,
    });
    files.push(PackageFile {
        path: "seal.sbom.json".to_owned(),
        contents: sbom_bytes.clone(),
        executable: false,
    });
    if !files
        .iter()
        .any(|file| file.path.ends_with(".sl") && file.path != "seal.toml")
    {
        return Err(ui::failure(
            "E8093",
            format!("the package `{}` has no Seal source files", package.name),
        ));
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in &files {
        listed.push((file.path.clone(), file.contents.len() as u64));
    }
    let archive =
        build_archive(&files).map_err(|error| ui::failure(error.code(), error.message()))?;
    let mut hasher = Sha256::new();
    hasher.update(&archive);
    let archive_sha256 = hex::encode(hasher.finalize());
    let api_digest = snapshot
        .hash()
        .map_err(|error| ui::failure(error.code(), error.message()))?;
    let sbom_digest =
        document_digest(&sbom).map_err(|error| ui::failure(error.code(), error.message()))?;
    let started = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| ui::failure("E8096", format!("cannot format the time: {error}")))?;
    let invocation = {
        let mut hasher = Sha256::new();
        hasher.update(package.name.as_bytes());
        hasher.update(package.version.as_bytes());
        hasher.update(archive_sha256.as_bytes());
        hasher.update(std::process::id().to_be_bytes());
        hex::encode(hasher.finalize())
    };
    let provenance = seal_registry::provenance::sign(
        &seal_registry::provenance::SignOptions {
            name: &package.name,
            version: &package.version,
            archive_sha256: &archive_sha256,
            sbom: &sbom_digest,
            api: &api_digest,
            repository: manifest.repository.as_deref(),
            identity,
            invocation_id: &invocation,
            started_on: &started,
            finished_on: &started,
        },
        seed,
    )
    .map_err(|error| ui::failure(error.code(), error.message()))?;
    let provenance_digest = seal_registry::provenance::envelope_digest(&provenance)
        .map_err(|error| ui::failure(error.code(), error.message()))?;
    let request = PublishRequest {
        name: package.name.clone(),
        version: package.version.clone(),
        archive: encode_archive(&archive),
        api: snapshot.clone(),
        sbom,
        provenance,
        publisher: publisher(seed, identity),
    };
    Ok(Bundle {
        request,
        files: listed,
        archive_size: archive.len(),
        checksum: format!("sha256:{archive_sha256}"),
        api_digest,
        sbom_digest,
        provenance_digest,
    })
}

/// Returns whether the package directory has uncommitted Git changes.
pub(crate) fn dirty(root: &Path) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain", "--", "."])
        .output();
    match output {
        Ok(output) if output.status.success() => !output.stdout.is_empty(),
        _ => false,
    }
}

fn add_file(
    files: &mut Vec<PackageFile>,
    root: &Path,
    relative: &Path,
    required: bool,
) -> Result<(), ui::Failure> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ui::failure(
            "E8093",
            format!("`{}` is not a safe package path", relative.display()),
        ));
    }
    let name = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if matches!(
        name.as_str(),
        "seal.api.json" | "seal.docs.json" | "seal.sbom.json"
    ) {
        return Err(ui::failure(
            "E8093",
            format!("`{name}` is reserved for generated publication metadata"),
        ));
    }
    let path = root.join(relative);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => {
            return Err(ui::failure(
                "E8093",
                format!("cannot read `{}`: {error}", path.display()),
            ));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(ui::failure(
            "E8093",
            format!("`{}` is not a regular file", path.display()),
        ));
    }
    if metadata.len() > 64 * 1024 * 1024 {
        return Err(ui::failure(
            "E8093",
            format!("`{}` is larger than 64 MiB", path.display()),
        ));
    }
    let contents = std::fs::read(&path).map_err(|error| {
        ui::failure(
            "E8093",
            format!("cannot read `{}`: {error}", path.display()),
        )
    })?;
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    let relative = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    files.push(PackageFile {
        path: relative,
        contents,
        executable,
    });
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(path))
}
