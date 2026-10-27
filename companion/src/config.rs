// SPDX-License-Identifier: GPL-3.0-or-later
//! Local-only version1 configuration. Validation never executes provider wrappers.
use crate::{
    pairing::{private_dir, private_file, uid},
    protocol::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

pub const CONFIG_BYTES: usize = 1024 * 1024;

/// Omission chooses the documented public origin; an explicit empty list still denies all.
pub(crate) fn default_push_allowed_origins() -> Vec<String> {
    vec!["https://ntfy.sh".into()]
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_name: Option<Label>,
    pub runtime_dir: PathBuf,
    pub runtime_dir_agent: PathBuf,
    #[serde(default)]
    pub allow_scoped_shell_probe: bool,
    #[serde(default)]pub allow_attachments:bool,
    #[serde(default)]pub allow_terminal_control:bool,
    #[serde(default)]pub allow_omp_controls:bool,
    #[serde(default)]
    pub herdr_sockets: Vec<PathBuf>,
    #[serde(default = "default_push_allowed_origins")]
    pub push_allowed_origins: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omp_registry_dir: Option<PathBuf>,
    #[serde(default)]
    pub omp_session_roots: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omp_blob_root: Option<PathBuf>,
    /// Explicit source identity for the pinned per-process OMP Skills extension.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub omp_skills_package_root:Option<PathBuf>,
    /// Explicit document-read roots, independent of creation, attachments and controls.
    #[serde(default)]pub skill_document_roots:Vec<PathBuf>,
    pub executables: Executables,
    #[serde(default)]
    pub projects: Vec<ConfiguredProject>,
    #[serde(default)]
    pub codex_owners: Vec<CodexOwner>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_codex_owner_id: Option<Uuid>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Executables {
    pub herdr: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omp: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<PathBuf>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredProject {
    pub id: Uuid,
    pub label: Label,
    pub path: PathBuf,
    /// Explicit subtree read/export and Git review authority; never implied by creation.
    #[serde(default)]
    pub allow_read_tools: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexOwner {
    pub id: Uuid,
    pub socket_host: PathBuf,
    pub socket_agent: PathBuf,
    pub bindings_dir: PathBuf,
    pub unit: String,
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
pub fn default_path() -> io::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(std::env::var_os("HOME").ok_or_else(|| invalid("HOME unavailable"))?)
            .join(".config"),
    };
    absolute(&base)?;
    Ok(base.join("goatr/config.toml"))
}
pub fn absolute(path: &Path) -> io::Result<()> {
    let text = path.to_str().ok_or_else(|| invalid("path is not UTF-8"))?;
    if !path.is_absolute()
        || text.chars().any(char::is_control)
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid(format!(
            "absolute control-free path required: {text}"
        )));
    }
    Ok(())
}
pub fn socket_path(path: &Path) -> io::Result<()> {
    absolute(path)?;
    if path.as_os_str().len() > 107 {
        return Err(invalid(format!(
            "Unix socket path exceeds 107 UTF-8 bytes: {}",
            path.display()
        )));
    }
    Ok(())
}
/// The provider view is not presumed to be accessible in the companion namespace.
pub fn socket_parent(path: &Path) -> io::Result<()> {
    socket_path(path)?;
    private_dir(
        path.parent()
            .ok_or_else(|| invalid("missing socket parent"))?,
        false,
    )
}
fn configured_socket(path: &Path) -> io::Result<()> {
    socket_parent(path)?;
    match fs::symlink_metadata(path) {
        Ok(m)
            if m.file_type().is_socket()
                && m.uid() == uid()
                && m.mode() & 0o7777 == 0o600
                && fs::canonicalize(path)? == path =>
        {
            Ok(())
        }
        Ok(_) => Err(invalid(format!(
            "unsafe existing configured socket: {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
/// Stock0.159.1 hashes the canonical advertised provider pathname into its private /tmp root.
/// Runtime resolution must separately prove that the provider parent has no symlink components.
pub fn codex_socket_target(agent: &Path) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    socket_path(agent)?;
    let hash = crate::pairing::hex(&Sha256::digest(agent.as_os_str().as_encoded_bytes()));
    let path = PathBuf::from(format!("/tmp/codex-daemon-{}", uid())).join(hash);
    socket_path(&path)?;
    Ok(path)
}
/// This is configured-path trust/attempt eligibility, never owner or native readiness.
pub fn configured_codex_socket(host: &Path, agent: &Path) -> io::Result<()> {
    socket_parent(host)?;
    socket_path(agent)?;
    let metadata = match fs::symlink_metadata(host) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.uid() == uid()
        && ((metadata.file_type().is_socket()
            && metadata.mode() & 0o7777 == 0o600
            && fs::canonicalize(host)? == host)
            || (metadata.file_type().is_symlink()
                && metadata.mode() & 0o7777 == 0o777
                && metadata.nlink() == 1
                && fs::read_link(host)? == codex_socket_target(agent)?))
    {
        return Ok(());
    }
    Err(invalid(
        "unsafe configured Codex socket or native rendezvous alias",
    ))
}
pub fn project_directory(path: &Path) -> io::Result<()> {
    absolute(path)?;
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != uid() || m.mode() & 0o022 != 0 || fs::canonicalize(path)? != path {
        return Err(invalid(format!(
            "canonical same-user project directory required: {}",
            path.display()
        )));
    }
    Ok(())
}
fn installed_trust(path: &Path, metadata: &fs::Metadata) -> io::Result<bool> {
    if [0, uid()].contains(&metadata.uid()) && metadata.mode() & 0o022 == 0 {
        return Ok(true);
    }
    let canonical = fs::canonicalize(path)?;
    if !canonical.starts_with("/nix/store") {
        return Ok(false);
    }
    let file = File::open(&canonical)?;
    let opened = file.metadata()?;
    if (opened.dev(), opened.ino()) != (metadata.dev(), metadata.ino()) {
        return Err(invalid("installed object changed during trust proof"));
    }
    // SAFETY: statvfs is zero-initialized and fstatvfs receives a live owned descriptor.
    let mut mount: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatvfs(file.as_raw_fd(), &mut mount) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(mount.f_flag & libc::ST_RDONLY != 0)
}

fn optional_prerequisite(
    path: &Path,
    check: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<bool> {
    match check(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A missing leaf is unavailable; an existing dangling/unsafe entry is not absent.
            match fs::symlink_metadata(path) {
                Ok(_) => return Err(invalid("unsafe existing unavailable backend prerequisite")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            for parent in path.ancestors().skip(1) {
                match fs::symlink_metadata(parent) {
                    Ok(metadata) => {
                        if !metadata.is_dir()
                            || fs::canonicalize(parent)? != parent
                            || !installed_trust(parent, &metadata)?
                        {
                            return Err(invalid("unsafe missing backend prerequisite parent"));
                        }
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Preserve the configured outer symlink/wrapper. Canonicalization checks provenance only.
pub fn executable(path: &Path) -> io::Result<()> {
    absolute(path)?;
    let resolved = fs::canonicalize(path)?;
    let m = fs::metadata(&resolved)?;
    if !m.is_file() || m.mode() & 0o111 == 0 || !installed_trust(&resolved, &m)? {
        return Err(invalid(format!(
            "unsafe installed executable: {}",
            path.display()
        )));
    }
    for parent in path.ancestors().skip(1).chain(resolved.ancestors().skip(1)) {
        let m = fs::metadata(parent)?;
        if !m.is_dir() || !installed_trust(parent, &m)? {
            return Err(invalid(format!(
                "unsafe executable parent: {}",
                parent.display()
            )));
        }
    }
    Ok(())
}
impl Config {
    /// Local setup serialization uses the retained TOML implementation.
    pub fn serialized(&self) -> io::Result<String> {
        self.validate_syntax()?;
        let text = toml_edit::ser::to_string_pretty(self).map_err(io::Error::other)?;
        if text.len() > CONFIG_BYTES {
            return Err(invalid("configuration exceeds 1 MiB"));
        }
        Ok(text)
    }
    /// Explicit project approval never changes an existing canonical project's identity.
    pub fn approve_project(&mut self, input: &str) -> io::Result<bool> {
        let path = setup_project(input)?;
        let mut found = None;
        for (index, project) in self.projects.iter().enumerate() {
            if (project.path == path
                || project.path.to_str().is_some_and(|text| {
                    setup_project(text).is_ok_and(|canonical| canonical == path)
                }))
                && found.replace(index).is_some()
            {
                return Err(invalid(
                    "existing canonical project IDs conflict; no identity is discarded",
                ));
            }
        }
        if let Some(index) = found {
            let changed = self.projects[index].path != path;
            self.projects[index].path = path;
            return Ok(changed);
        }
        if self.projects.len() == 128 {
            return Err(invalid("configuration project limit"));
        }
        let label = Label(
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Project")
                .into(),
        );
        label
            .validate()
            .map_err(|_| invalid("project basename is not a valid label"))?;
        self.projects.push(ConfiguredProject {
            id: Uuid::random(),
            label,
            path,
            allow_read_tools: false,
        });
        Ok(true)
    }
    pub fn set_project_read_tools(&mut self,input:&Path,allow:bool)->io::Result<bool> {
        let input=input.to_str().ok_or_else(||invalid("project permission path is not UTF-8"))?;
        let path=setup_project(input)?;
        let project=self.projects.iter_mut().find(|project|project.path==path)
            .ok_or_else(||invalid("read permission requires an explicitly configured project; approve it with --project first"))?;
        let changed=project.allow_read_tools!=allow;
        project.allow_read_tools=allow;
        Ok(changed)
    }
    pub fn parse(text: &str) -> io::Result<Self> {
        if text.len() > CONFIG_BYTES {
            return Err(invalid("configuration exceeds 1 MiB"));
        }
        let config: Self = toml_edit::de::from_str(text)
            .map_err(|_| invalid("invalid version1 TOML configuration"))?;
        config.validate_syntax()?;
        Ok(config)
    }
    pub fn load(path: &Path) -> io::Result<Self> {
        absolute(path)?;
        private_dir(
            path.parent()
                .ok_or_else(|| invalid("missing config parent"))?,
            false,
        )?;
        private_file(path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let before = file.metadata()?;
        let mut text = String::new();
        (&mut file)
            .take((CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut text)?;
        private_file(path)?;
        let after = fs::symlink_metadata(path)?;
        if (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
        ) {
            return Err(invalid("configuration changed while reading"));
        }
        let config = Self::parse(&text)?;
        config.preflight()?;
        Ok(config)
    }
    fn validate_syntax(&self) -> io::Result<()> {
        if self.version != 1 {
            return Err(invalid("unsupported configuration version"));
        }
        if let Some(name) = &self.pairing_name {
            name.validate().map_err(|_| invalid("invalid pairing name"))?;
        }
        if self.projects.len() > 128
            || self.herdr_sockets.len() > 64
            || self.codex_owners.len() > 512
            || self.omp_session_roots.len() > 512
            || self.push_allowed_origins.len() > 512
            || self.skill_document_roots.len() > 128
        {
            return Err(invalid("configuration resource limit"));
        }
        absolute(&self.runtime_dir)?;
        absolute(&self.runtime_dir_agent)?;
        absolute(&self.executables.herdr)?;
        for path in [
            &self.executables.omp,
            &self.executables.codex,
            &self.omp_registry_dir,
            &self.omp_blob_root,
            &self.omp_skills_package_root,
        ]
        .into_iter()
        .flatten()
        .chain(self.omp_session_roots.iter())
        .chain(self.skill_document_roots.iter())
        {
            absolute(path)?;
        }
        let mut document_roots=HashSet::new();
        if self.skill_document_roots.iter().any(|root|!document_roots.insert(root)) {
            return Err(invalid("duplicate Skill document root"));
        }
        let mut ids = HashSet::new();
        let mut project_paths=HashSet::new();
        for project in &self.projects {
            project
                .id
                .validate()
                .map_err(|_| invalid("invalid project UUID"))?;
            project
                .label
                .validate()
                .map_err(|_| invalid("invalid project label"))?;
            absolute(&project.path)?;
            if !project_paths.insert(&project.path) {
                return Err(invalid("duplicate canonical project paths; no project identity is discarded"));
            }
            if !ids.insert(&project.id) {
                return Err(invalid("duplicate project/owner UUID"));
            }
        }
        for owner in &self.codex_owners {
            owner
                .id
                .validate()
                .map_err(|_| invalid("invalid owner UUID"))?;
            if !ids.insert(&owner.id) {
                return Err(invalid("duplicate project/owner UUID"));
            }
            socket_path(&owner.socket_host)?;
            socket_path(&owner.socket_agent)?;
            absolute(&owner.bindings_dir)?;
            if owner.unit.is_empty()
                || owner.unit.len() > 255
                || !owner.unit.ends_with(".service")
                || !owner
                    .unit
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
            {
                return Err(invalid("invalid independent Codex unit name"));
            }
        }
        if let Some(id) = &self.default_codex_owner_id {
            id.validate()
                .map_err(|_| invalid("invalid default Codex owner UUID"))?;
            if self
                .codex_owners
                .iter()
                .filter(|owner| &owner.id == id)
                .count()
                != 1
            {
                return Err(invalid(
                    "default_codex_owner_id must name one configured owner",
                ));
            }
        }
        let mut sockets = HashSet::new();
        for path in &self.herdr_sockets {
            socket_path(path)?;
            if !sockets.insert(path) {
                return Err(invalid("duplicate Herdr socket"));
            }
        }
        let mut origins = HashSet::new();
        for origin in &self.push_allowed_origins {
            let url =
                url::Url::parse(origin).map_err(|_| invalid("invalid HTTPS allowed origin"))?;
            if origin.chars().any(char::is_control)
                || url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.path() != "/"
                || url.origin().ascii_serialization() != *origin
                || !origins.insert(origin)
            {
                return Err(invalid(
                    "allowed origin must be a unique exact HTTPS origin",
                ));
            }
        }
        Ok(())
    }
    /// Read-only trust preflight. Project disappearance is reflected per project, not by erasing inventory.
    pub fn preflight(&self) -> io::Result<()> {
        self.validate_syntax()?;
        let mut canonical_projects=HashSet::new();
        for project in &self.projects {
            if let Ok(path)=fs::canonicalize(&project.path) {
                if !canonical_projects.insert(path) {
                    return Err(invalid("existing canonical project IDs conflict; no identity is discarded"));
                }
            }
        }
        private_dir(&self.runtime_dir, false)?;
        executable(&self.executables.herdr)?;
        self.backend_reason(Provider::Omp)?;
        self.backend_reason(Provider::Codex)?;
        for socket in &self.herdr_sockets {
            configured_socket(socket)?;
        }
        for root in &self.skill_document_roots {
            // Absence disables that preview root only; existing unsafe paths still fail trust.
            let _=optional_prerequisite(root,project_directory)?;
        }
        Ok(())
    }
    /// A missing optional prerequisite disables only this backend; unsafe existing paths fail.
    pub fn backend_reason(&self, provider: Provider) -> io::Result<Option<&'static str>> {
        let mut reason = match self.wrapper(provider) {
            Some(path) if optional_prerequisite(path, executable)? => None,
            Some(_) => Some("Configured outer provider wrapper is unavailable"),
            None => Some("Installed outer provider wrapper is not configured"),
        };
        match provider {
            Provider::Omp => {
                for (path, unavailable) in [
                    (
                        self.omp_registry_dir.as_deref(),
                        "Configured OMP registry directory is unavailable",
                    ),
                    (
                        self.omp_blob_root.as_deref(),
                        "Configured OMP blob root is unavailable",
                    ),
                ] {
                    if let Some(path) = path
                        && !optional_prerequisite(path, project_directory)?
                    {
                        reason.get_or_insert(unavailable);
                    }
                }
                for root in &self.omp_session_roots {
                    if !optional_prerequisite(root, project_directory)? {
                        reason.get_or_insert("Configured OMP session root is unavailable");
                    }
                }
            }
            Provider::Codex => {
                // Trust-check all entries, but only selected-owner absence gates creation.
                let selected = self.creation_codex_owner().ok();
                if selected.is_none() {
                    reason.get_or_insert(if self.codex_owners.is_empty() {
                        "No shared Codex owner configured; explicit local setup required"
                    } else {
                        "Codex creation is ambiguous: set local default_codex_owner_id"
                    });
                }
                for owner in &self.codex_owners {
                    let socket_available = optional_prerequisite(&owner.socket_host, |path| {
                        configured_codex_socket(path, &owner.socket_agent)
                    })?;
                    let bindings_available = optional_prerequisite(&owner.bindings_dir, |path| {
                        private_dir(path, false)
                    })?;
                    if selected.is_some_and(|selected| selected.id == owner.id) {
                        if !socket_available {
                            reason.get_or_insert(
                                "Configured Codex owner socket parent is unavailable",
                            );
                        }
                        if !bindings_available {
                            reason.get_or_insert(
                                "Configured Codex owner bindings directory is unavailable",
                            );
                        }
                    }
                }
            }
        }
        Ok(reason)
    }
    pub fn creation_codex_owner(&self) -> Result<&CodexOwner, ErrorCode> {
        match &self.default_codex_owner_id {
            Some(id) => self
                .codex_owners
                .iter()
                .find(|owner| &owner.id == id)
                .ok_or(ErrorCode::NotReady),
            None if self.codex_owners.len() == 1 => Ok(&self.codex_owners[0]),
            None => Err(ErrorCode::NotReady),
        }
    }
    pub fn projects(&self) -> Vec<Project> {
        self.projects
            .iter()
            .map(|p| {
                let error = project_directory(&p.path).err();
                Project {
                    project_id: p.id.clone(),
                    label: p.label.clone(),
                    available: error.is_none(),
                    reason: error
                        .map(|_| "Configured project directory is unavailable or unsafe".into()),
                }
            })
            .collect()
    }
    pub fn project(&self, id: &Uuid) -> Result<&ConfiguredProject, ErrorCode> {
        let project = self
            .projects
            .iter()
            .find(|p| &p.id == id)
            .ok_or(ErrorCode::NotFound)?;
        project_directory(&project.path).map_err(|_| ErrorCode::NotReady)?;
        Ok(project)
    }
    pub fn wrapper(&self, provider: Provider) -> Option<&Path> {
        match provider {
            Provider::Omp => self.executables.omp.as_deref(),
            Provider::Codex => self.executables.codex.as_deref(),
        }
    }
}
/// Expansion is confined to explicit local setup arguments, never TOML or remote data.
pub fn setup_project(text: &str) -> io::Result<PathBuf> {
    let path = if text == "~" || text.starts_with("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or_else(|| invalid("HOME unavailable"))?)
            .join(text.strip_prefix("~/").unwrap_or(""))
    } else {
        PathBuf::from(text)
    };
    let path = fs::canonicalize(path)?;
    project_directory(&path)?;
    Ok(path)
}
