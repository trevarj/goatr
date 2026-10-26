// SPDX-License-Identifier: GPL-3.0-or-later
//! All administrative/configuration/object indirections remain beneath explicit grants.
use super::*;
use std::collections::{HashMap, HashSet};
const MAX_OBJECTS: usize = 128;
const MAX_METADATA: usize = 256 * 1024;
const MAX_NATIVE_FILE: u64 = FILE_BYTES;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Repository {
    pub(super) root: PathBuf,
    pub(super) git: PathBuf,
    pub(super) common: PathBuf,
    pub(super) bare: bool,
}
pub(super) struct Candidate {
    pub(super) root: PathBuf,
    pub(super) git: PathBuf,
    pub(super) common: PathBuf,
}
struct FileProof {
    root: usize,
    file: FileSelection,
}
struct DirectoryProof {
    root: usize,
    directory: Directory,
    administrative: bool,
}
pub(super) struct Graph {
    pub(super) context: ProjectReadContext,
    pub(super) roots: Vec<Arc<Root>>,
    views: Vec<Option<NamespaceRoot>>,
    files: Vec<FileProof>,
    directories: Vec<DirectoryProof>,
    file_ids: HashMap<PathBuf, usize>,
    directory_ids: HashMap<PathBuf, usize>,
    absent: Vec<PathBuf>,
    metadata: usize,
    pub(super) control_reads: bool,
}
fn unavailable<T>() -> SecurityResult<T> {
    Err(ErrorCode::NotReady)
}
pub(super) fn normalized(value: &str, base: &Path) -> SecurityResult<PathBuf> {
    if value.is_empty() || value.len() > 4095 || value.chars().any(char::is_control) {
        return unavailable();
    }
    let source = Path::new(value);
    let mut output = if source.is_absolute() {
        PathBuf::from("/")
    } else {
        base.to_owned()
    };
    for part in source.components() {
        match part {
            std::path::Component::RootDir => {}
            std::path::Component::CurDir => {}
            std::path::Component::Normal(name) => output.push(name),
            std::path::Component::ParentDir => {
                if !output.pop() {
                    return unavailable();
                }
            }
            std::path::Component::Prefix(_) => return unavailable(),
        }
    }
    if !output.is_absolute() || output.as_os_str().len() > 4095 {
        return unavailable();
    }
    Ok(output)
}
impl Graph {
    pub(super) fn new(
        context: ProjectReadContext,
        job: &ReadJob,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<Self> {
        job.current()?;
        context.recheck(deadline)?;
        let roots = context.topology()?.approved_read_roots();
        if roots.is_empty() {
            return unavailable();
        }
        let descriptors = runtime
            .roots
            .len()
            .checked_add(roots.len() * 3)
            .and_then(|count| count.checked_add(MAX_OBJECTS + 3 * 64 + 24))
            .ok_or(ErrorCode::ResourceLimit)?;
        job.reserve_fds(descriptors)?;
        let views = std::iter::repeat_with(|| None).take(roots.len()).collect();
        Ok(Self {
            context,
            roots,
            views,
            files: Vec::new(),
            directories: Vec::new(),
            file_ids: HashMap::new(),
            directory_ids: HashMap::new(),
            absent: Vec::new(),
            metadata: 0,
            control_reads: true,
        })
    }
    fn charge(&mut self, bytes: usize) -> SecurityResult<()> {
        self.metadata = self
            .metadata
            .checked_add(bytes)
            .filter(|bytes| *bytes <= MAX_METADATA)
            .ok_or(ErrorCode::ResourceLimit)?;
        Ok(())
    }
    fn root_index(&self, path: &Path) -> SecurityResult<usize> {
        if !path.is_absolute()
            || path
                .to_str()
                .is_none_or(|value| value.len() > 4095 || value.chars().any(char::is_control))
            || path.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
        {
            return unavailable();
        }
        self.roots
            .iter()
            .enumerate()
            .filter(|(_, root)| path.strip_prefix(root.pathname()).is_ok())
            .max_by_key(|(_, root)| root.pathname().components().count())
            .map(|(index, _)| index)
            .ok_or(ErrorCode::NotReady)
    }
    fn view(&mut self, index: usize, deadline: Instant) -> SecurityResult<&NamespaceRoot> {
        if self.views[index].is_none() {
            self.context.recheck(deadline)?;
            self.views[index] = Some(
                NamespaceRoot::capture(
                    &self.roots[index],
                    self.context.location().process(),
                    deadline,
                )
                .map_err(filesystem)?,
            );
        }
        self.views[index].as_ref().ok_or(ErrorCode::NotReady)
    }
    fn pin(&mut self, path: &Path, deadline: Instant) -> SecurityResult<(usize, File)> {
        let index = self.root_index(path)?;
        let root = self.roots[index].clone();
        root.current(deadline).map_err(filesystem)?;
        let relative = path
            .strip_prefix(root.pathname())
            .map_err(|_| ErrorCode::NotReady)?;
        let name = if relative.as_os_str().is_empty() {
            CString::new(".").map_err(|_| ErrorCode::NotReady)?
        } else {
            CString::new(relative.as_os_str().as_bytes()).map_err(|_| ErrorCode::NotReady)?
        };
        let file = pin_at(root.file.as_raw_fd(), &name, CHILD_RESOLVE, false, deadline)
            .map_err(filesystem)?;
        let metadata = file.metadata().map_err(filesystem)?;
        if !metadata.is_file() && !metadata.is_dir() {
            return unavailable();
        }
        let object = object_identity(&file, &metadata, deadline).map_err(filesystem)?;
        if object.mount != root.object.mount {
            return unavailable();
        }
        let parts = relative.to_str().ok_or(ErrorCode::NotReady)?;
        self.view(index, deadline)?
            .resolve(
                parts.split('/').filter(|part| !part.is_empty()),
                object,
                deadline,
            )
            .map_err(filesystem)?;
        Ok((index, file))
    }
    /// Inspect the original traversal, not a canonicalized target. A missing
    /// pathname grants nothing; Landlock and the post-open object join still apply.
    pub(super) fn request(
        &mut self,
        path: &Path,
        deadline: Instant,
    ) -> SecurityResult<Option<File>> {
        let canonical = normalized(path.to_str().ok_or(ErrorCode::NotReady)?, Path::new("/"))?;
        let index = match self.root_index(&canonical) {
            Ok(index) => index,
            Err(_) => {
                return match std::fs::symlink_metadata(path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                    _ => unavailable(),
                };
            }
        };
        let root = self.roots[index].clone();
        root.current(deadline).map_err(filesystem)?;
        let traversal = path
            .strip_prefix(root.pathname())
            .map_err(|_| ErrorCode::NotReady)?;
        let name = if traversal.as_os_str().is_empty() {
            CString::new(".").map_err(|_| ErrorCode::NotReady)?
        } else {
            CString::new(traversal.as_os_str().as_bytes()).map_err(|_| ErrorCode::NotReady)?
        };
        let file = match pin_at(root.file.as_raw_fd(), &name, CHILD_RESOLVE, false, deadline) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(filesystem(error)),
        };
        let metadata = file.metadata().map_err(filesystem)?;
        if !metadata.is_file() && !metadata.is_dir() {
            return unavailable();
        }
        let object = object_identity(&file, &metadata, deadline).map_err(filesystem)?;
        if object.mount != root.object.mount {
            return unavailable();
        }
        let relative = canonical
            .strip_prefix(root.pathname())
            .map_err(|_| ErrorCode::NotReady)?
            .to_str()
            .ok_or(ErrorCode::NotReady)?;
        self.view(index, deadline)?
            .resolve(
                relative.split('/').filter(|part| !part.is_empty()),
                object,
                deadline,
            )
            .map_err(filesystem)?;
        Ok(Some(file))
    }
    fn exists(&mut self, path: &Path, deadline: Instant) -> SecurityResult<bool> {
        let index = self.root_index(path)?;
        let root = self.roots[index].clone();
        root.current(deadline).map_err(filesystem)?;
        let relative = path
            .strip_prefix(root.pathname())
            .map_err(|_| ErrorCode::NotReady)?;
        let name = if relative.as_os_str().is_empty() {
            CString::new(".").map_err(|_| ErrorCode::NotReady)?
        } else {
            CString::new(relative.as_os_str().as_bytes()).map_err(|_| ErrorCode::NotReady)?
        };
        match pin_at(root.file.as_raw_fd(), &name, CHILD_RESOLVE, false, deadline) {
            Ok(file) => {
                let metadata = file.metadata().map_err(filesystem)?;
                if !metadata.is_file() && !metadata.is_dir() {
                    return unavailable();
                }
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(filesystem(error)),
        }
    }
    pub(super) fn directory(&mut self, path: &Path, deadline: Instant) -> SecurityResult<usize> {
        if let Some(index) = self.directory_ids.get(path).copied() {
            let proof = &self.directories[index];
            self.roots[proof.root]
                .revalidate(&proof.directory, deadline)
                .map_err(filesystem)?;
            return Ok(index);
        }
        if self.directories.len() + self.files.len() == MAX_OBJECTS {
            return Err(ErrorCode::ResourceLimit);
        }
        let root = self.root_index(path)?;
        let directory = self.roots[root]
            .directory_at(path, deadline)
            .map_err(filesystem)?;
        self.view(root, deadline)?
            .resolve(
                directory.components.iter().map(String::as_str),
                directory.object,
                deadline,
            )
            .map_err(filesystem)?;
        self.charge(directory.retained_bytes() + path.as_os_str().len() + 128)?;
        let index = self.directories.len();
        self.directories.push(DirectoryProof {
            root,
            directory,
            administrative: false,
        });
        self.directory_ids.insert(path.to_owned(), index);
        Ok(index)
    }
    fn control_file(&mut self, path: &Path, deadline: Instant) -> SecurityResult<usize> {
        if let Some(index) = self.file_ids.get(path).copied() {
            let proof = &self.files[index];
            self.roots[proof.root]
                .revalidate_file(&proof.file, deadline)
                .map_err(filesystem)?;
            return Ok(index);
        }
        if self.directories.len() + self.files.len() == MAX_OBJECTS {
            return Err(ErrorCode::ResourceLimit);
        }
        let root = self.root_index(path)?;
        let file = self.roots[root]
            .pin_review_file(path, MAX_NATIVE_FILE, deadline)
            .map_err(filesystem)?;
        let native = self
            .view(root, deadline)?
            .resolve(
                file.components.iter().map(String::as_str),
                file.object,
                deadline,
            )
            .map_err(filesystem)?;
        if stamp(&native.metadata().map_err(filesystem)?) != file.stamp {
            return unavailable();
        }
        self.charge(file.retained_bytes() + path.as_os_str().len() + 128)?;
        let index = self.files.len();
        self.files.push(FileProof { root, file });
        self.file_ids.insert(path.to_owned(), index);
        Ok(index)
    }
    fn read(&mut self, path: &Path, maximum: u64, deadline: Instant) -> SecurityResult<Snapshot> {
        let index = self.control_file(path, deadline)?;
        let proof = &self.files[index];
        if proof.file.stamp.0 > maximum {
            return Err(ErrorCode::ResourceLimit);
        }
        self.roots[proof.root]
            .snapshot_file(&proof.file, deadline)
            .map_err(filesystem)
    }
    fn optional_file(&mut self, path: &Path, deadline: Instant) -> SecurityResult<bool> {
        if !self.exists(path, deadline)? {
            self.absence(path)?;
            return Ok(false);
        }
        self.control_file(path, deadline)?;
        Ok(true)
    }
    fn absence(&mut self, path: &Path) -> SecurityResult<()> {
        if self.absent.iter().any(|known| known == path) {
            return Ok(());
        }
        if self.absent.len() == MAX_OBJECTS {
            return Err(ErrorCode::ResourceLimit);
        }
        self.charge(path.as_os_str().len() + std::mem::size_of::<PathBuf>())?;
        self.absent.push(path.to_owned());
        Ok(())
    }
    fn external_optional(&mut self, path: &Path, deadline: Instant) -> SecurityResult<()> {
        check(deadline).map_err(filesystem)?;
        if self.root_index(path).is_ok() {
            self.optional_file(path, deadline)?;
            return Ok(());
        }
        // No outside configuration is opened to discover what to authorize. An
        // absent default is only absence, never authority if it appears later.
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => self.absence(path),
            _ => unavailable(),
        }
    }
    pub(super) fn seed_configuration(
        &mut self,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<()> {
        if !environment_flag("GIT_CONFIG_NOSYSTEM")? {
            let path = std::env::var_os("GIT_CONFIG_SYSTEM")
                .map(PathBuf::from)
                .unwrap_or_else(|| runtime.system_config.clone());
            self.external_optional(&path, deadline)?;
        }
        if let Some(path) = std::env::var_os("GIT_CONFIG_GLOBAL") {
            self.external_optional(Path::new(&path), deadline)?;
        } else {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or(ErrorCode::NotReady)?;
            let xdg = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"));
            self.external_optional(&xdg.join("git/config"), deadline)?;
            self.external_optional(&home.join(".gitconfig"), deadline)?;
        }
        Ok(())
    }
    fn gitfile(&mut self, path: &Path, base: &Path, deadline: Instant) -> SecurityResult<PathBuf> {
        let data = self.read(path, 4096, deadline)?;
        let text = std::str::from_utf8(data.bytes()).map_err(|_| ErrorCode::NotReady)?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        let target = text.strip_prefix("gitdir: ").ok_or(ErrorCode::NotReady)?;
        normalized(target, base)
    }
    pub(super) fn candidate(
        &mut self,
        cwd: &Path,
        exact: bool,
        deadline: Instant,
    ) -> SecurityResult<Candidate> {
        self.directory(cwd, deadline)?;
        let mut current = cwd.to_owned();
        loop {
            check(deadline).map_err(filesystem)?;
            let marker = current.join(".git");
            if self.exists(&marker, deadline)? {
                let (_, file) = self.pin(&marker, deadline)?;
                let git = if file.metadata().map_err(filesystem)?.is_dir() {
                    marker.clone()
                } else {
                    self.gitfile(&marker, &current, deadline)?
                };
                return self.administrative_candidate(current, git, Some(marker), deadline);
            }
            if self.exists(&current.join("HEAD"), deadline)?
                && self.exists(&current.join("objects"), deadline)?
            {
                return self.administrative_candidate(current.clone(), current, None, deadline);
            }
            if exact || !current.pop() {
                return unavailable();
            }
            self.root_index(&current)?;
        }
    }
    fn administrative_candidate(
        &mut self,
        root: PathBuf,
        git: PathBuf,
        marker: Option<PathBuf>,
        deadline: Instant,
    ) -> SecurityResult<Candidate> {
        let index = self.directory(&git, deadline)?;
        self.directories[index].administrative = true;
        let common_file = git.join("commondir");
        let common = if self.optional_file(&common_file, deadline)? {
            let data = self.read(&common_file, 4096, deadline)?;
            let text = std::str::from_utf8(data.bytes())
                .map_err(|_| ErrorCode::NotReady)?
                .trim_end_matches(['\r', '\n']);
            normalized(text, &git)?
        } else {
            git.clone()
        };
        let index = self.directory(&common, deadline)?;
        self.directories[index].administrative = true;
        self.optional_file(&common.join("config"), deadline)?;
        self.optional_file(&git.join("config.worktree"), deadline)?;
        self.optional_file(&git.join("HEAD"), deadline)?;
        if common != git {
            let marker = marker.as_ref().ok_or(ErrorCode::NotReady)?;
            let backlink = git.join("gitdir");
            let data = self.read(&backlink, 4096, deadline)?;
            let path = normalized(
                std::str::from_utf8(data.bytes())
                    .map_err(|_| ErrorCode::NotReady)?
                    .trim_end_matches(['\r', '\n']),
                &git,
            )?;
            let (_, actual) = self.pin(&path, deadline)?;
            let (_, original) = self.pin(marker, deadline)?;
            if object_identity(&actual, &actual.metadata().map_err(filesystem)?, deadline)
                .map_err(filesystem)?
                != object_identity(
                    &original,
                    &original.metadata().map_err(filesystem)?,
                    deadline,
                )
                .map_err(filesystem)?
            {
                return unavailable();
            }
        }
        let mut seen = HashSet::new();
        self.alternates(common.join("objects"), &mut seen, deadline)?;
        Ok(Candidate { root, git, common })
    }
    fn alternates(
        &mut self,
        objects: PathBuf,
        seen: &mut HashSet<PathBuf>,
        deadline: Instant,
    ) -> SecurityResult<()> {
        if seen.len() == 64 || !seen.insert(objects.clone()) {
            return Err(ErrorCode::ResourceLimit);
        }
        self.directory(&objects, deadline)?;
        let path = objects.join("info/alternates");
        if !self.optional_file(&path, deadline)? {
            return Ok(());
        }
        let data = self.read(&path, 64 * 1024, deadline)?;
        let text = std::str::from_utf8(data.bytes()).map_err(|_| ErrorCode::NotReady)?;
        for line in text.lines().filter(|line| !line.is_empty()) {
            let value = unquote_path(line)?;
            let target = normalized(&value, &objects)?;
            self.alternates(target, seen, deadline)?;
        }
        Ok(())
    }
    pub(super) fn observe_fd(
        &mut self,
        path: &Path,
        actual: &File,
        deadline: Instant,
    ) -> SecurityResult<()> {
        let (root, pin) = self.pin(path, deadline)?;
        let actual_metadata = actual.metadata().map_err(filesystem)?;
        if object_identity(actual, &actual_metadata, deadline).map_err(filesystem)?
            != object_identity(&pin, &pin.metadata().map_err(filesystem)?, deadline)
                .map_err(filesystem)?
        {
            return unavailable();
        }
        if self.control_reads
            || self.file_ids.contains_key(path)
            || self.directory_ids.contains_key(path)
            || self
                .directories
                .iter()
                .any(|proof| proof.administrative && path.starts_with(&proof.directory.pathname))
        {
            if actual_metadata.is_dir() {
                self.directory(path, deadline)?;
            } else {
                self.control_file(path, deadline)?;
            }
        }
        self.roots[root].current(deadline).map_err(filesystem)
    }
    pub(super) fn recheck(&self, deadline: Instant) -> SecurityResult<()> {
        self.context.recheck(deadline)?;
        self.objects_current(deadline)
    }
    pub(super) fn objects_current(&self, deadline: Instant) -> SecurityResult<()> {
        for proof in &self.directories {
            self.roots[proof.root]
                .revalidate(&proof.directory, deadline)
                .map_err(filesystem)?;
        }
        for proof in &self.files {
            self.roots[proof.root]
                .revalidate_file(&proof.file, deadline)
                .map_err(filesystem)?;
        }
        for (index, view) in self.views.iter().enumerate() {
            if let Some(view) = view {
                view.current(
                    &self.roots[index],
                    self.context.location().process(),
                    deadline,
                )
                .map_err(filesystem)?;
            }
        }
        for path in &self.absent {
            check(deadline).map_err(filesystem)?;
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => return unavailable(),
            }
        }
        Ok(())
    }
    pub(super) fn validate_worktree(
        &mut self,
        path: &Path,
        prunable: bool,
        deadline: Instant,
    ) -> SecurityResult<Option<bool>> {
        self.root_index(path)?;
        if !self.exists(path, deadline)? {
            return if prunable { Ok(None) } else { unavailable() };
        }
        let candidate = self.candidate(path, true, deadline)?;
        if candidate.root != path {
            return unavailable();
        }
        Ok(Some(candidate.git != candidate.common))
    }
    pub(super) fn initialized_submodule(
        &mut self,
        path: &Path,
        deadline: Instant,
    ) -> SecurityResult<bool> {
        self.root_index(path)?;
        if !self.exists(path, deadline)? {
            self.absence(path)?;
            return Ok(false);
        }
        self.directory(path, deadline)?;
        let marker = path.join(".git");
        if !self.exists(&marker, deadline)? {
            self.absence(&marker)?;
            return Ok(false);
        }
        Ok(true)
    }
    pub(super) fn validate_config(
        &mut self,
        records: &[ConfigRecord],
        repo: &Repository,
        runtime: &runtime::Runtime,
        deadline: Instant,
    ) -> SecurityResult<()> {
        let prefix = runtime
            .executable
            .path
            .parent()
            .and_then(Path::parent)
            .ok_or(ErrorCode::NotReady)?;
        for record in records {
            check(deadline).map_err(filesystem)?;
            let origin = if let Some(path) = record.origin.strip_prefix("file:") {
                let path = normalized(path, &repo.root)?;
                self.control_file(&path, deadline)?;
                Some(path)
            } else if record.scope == "command" && record.origin == "command line:" {
                None
            } else {
                return unavailable();
            };
            let key = &record.key;
            if key == "include.path" || key.starts_with("includeif.") && key.ends_with(".path") {
                let origin = origin.as_ref().ok_or(ErrorCode::NotReady)?;
                let path = config_path(
                    record.value.as_deref().ok_or(ErrorCode::NotReady)?,
                    origin.parent().ok_or(ErrorCode::NotReady)?,
                    prefix,
                )?;
                if key == "include.path" || self.file_ids.contains_key(&path) {
                    self.external_optional(&path, deadline)?;
                }
            }
        }
        for key in [
            "core.excludesfile",
            "core.attributesfile",
            "diff.orderfile",
            "core.worktree",
        ] {
            if let Some(record) = records.iter().rev().find(|record| record.key == key) {
                let base = if key == "core.worktree" {
                    &repo.git
                } else {
                    &repo.root
                };
                let path = config_path(
                    record.value.as_deref().ok_or(ErrorCode::NotReady)?,
                    base,
                    prefix,
                )?;
                if key == "core.worktree" {
                    self.directory(&path, deadline)?;
                } else {
                    self.external_optional(&path, deadline)?;
                }
            }
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(ErrorCode::NotReady)?;
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        if !records
            .iter()
            .any(|record| record.key == "core.excludesfile")
        {
            self.external_optional(&xdg.join("git/ignore"), deadline)?;
        }
        if !records
            .iter()
            .any(|record| record.key == "core.attributesfile")
        {
            self.external_optional(&xdg.join("git/attributes"), deadline)?;
        }
        if !environment_flag("GIT_ATTR_NOSYSTEM")? {
            self.external_optional(
                &runtime
                    .system_config
                    .parent()
                    .ok_or(ErrorCode::NotReady)?
                    .join("gitattributes"),
                deadline,
            )?;
        }
        Ok(())
    }
}

pub(super) struct ConfigRecord {
    pub(super) scope: String,
    pub(super) origin: String,
    pub(super) key: String,
    pub(super) value: Option<String>,
}
impl Drop for ConfigRecord {
    fn drop(&mut self) {
        // Exclusive String storage remains valid UTF-8 when overwritten with ASCII NUL.
        for text in [&mut self.scope, &mut self.origin, &mut self.key] {
            unsafe {
                erase(text.as_bytes_mut());
            }
        }
        if let Some(value) = &mut self.value {
            unsafe {
                erase(value.as_bytes_mut());
            }
        }
    }
}
pub(super) fn config_records(names: &[u8], values: &[u8]) -> SecurityResult<Vec<ConfigRecord>> {
    fn fields(bytes: &[u8]) -> SecurityResult<std::str::SplitTerminator<'_, char>> {
        if !bytes.is_empty() && bytes.last() != Some(&0) {
            return unavailable();
        }
        Ok(std::str::from_utf8(bytes)
            .map_err(|_| ErrorCode::NotReady)?
            .split_terminator('\0'))
    }
    let mut names = fields(names)?;
    let mut values = fields(values)?;
    let mut output = Vec::new();
    while let Some(scope) = names.next() {
        if !matches!(
            scope,
            "system" | "global" | "local" | "worktree" | "command"
        ) {
            return unavailable();
        }
        let origin = names.next().ok_or(ErrorCode::NotReady)?;
        let key = names.next().ok_or(ErrorCode::NotReady)?;
        if origin.len() > 4096
            || origin.chars().any(char::is_control)
            || key.is_empty()
            || key.len() > 4096
            || key.chars().any(char::is_control)
        {
            return unavailable();
        }
        if values.next() != Some(scope) || values.next() != Some(origin) {
            return unavailable();
        }
        let entry = values.next().ok_or(ErrorCode::NotReady)?;
        let (actual, value) = entry
            .split_once('\n')
            .map_or((entry, None), |(key, value)| (key, Some(value)));
        if actual != key || output.len() == 1024 {
            return unavailable();
        }
        output.push(ConfigRecord {
            scope: scope.to_owned(),
            origin: origin.to_owned(),
            key: key.to_owned(),
            value: value.map(str::to_owned),
        });
    }
    if values.next().is_some() {
        return unavailable();
    }
    Ok(output)
}
fn config_path(value: &str, base: &Path, prefix: &Path) -> SecurityResult<PathBuf> {
    if let Some(tail) = value.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(ErrorCode::NotReady)?;
        return normalized(tail, &home);
    }
    if let Some(tail) = value.strip_prefix("%(prefix)/") {
        return normalized(tail, prefix);
    }
    if value.starts_with('~') {
        return unavailable();
    }
    normalized(value, base)
}
fn environment_flag(name: &str) -> SecurityResult<bool> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(false);
    };
    let value = value.to_str().ok_or(ErrorCode::NotReady)?;
    if value.is_empty()
        || ["false", "off", "no"]
            .iter()
            .any(|word| value.eq_ignore_ascii_case(word))
    {
        return Ok(false);
    }
    if ["true", "on", "yes"]
        .iter()
        .any(|word| value.eq_ignore_ascii_case(word))
    {
        return Ok(true);
    }
    value
        .parse::<i64>()
        .map(|value| value != 0)
        .map_err(|_| ErrorCode::NotReady)
}
fn unquote_path(value: &str) -> SecurityResult<String> {
    if !value.starts_with('"') {
        return Ok(value.to_owned());
    }
    let value = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or(ErrorCode::NotReady)?;
    let mut output = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            output.push(byte);
            continue;
        }
        let escaped = bytes.next().ok_or(ErrorCode::NotReady)?;
        match escaped {
            b'\\' | b'"' => output.push(escaped),
            b'0'..=b'3' => {
                let second = bytes
                    .next()
                    .filter(|byte| matches!(byte, b'0'..=b'7'))
                    .ok_or(ErrorCode::NotReady)?;
                let third = bytes
                    .next()
                    .filter(|byte| matches!(byte, b'0'..=b'7'))
                    .ok_or(ErrorCode::NotReady)?;
                output.push((escaped - b'0') * 64 + (second - b'0') * 8 + third - b'0');
            }
            _ => return unavailable(),
        }
    }
    String::from_utf8(output).map_err(|_| ErrorCode::NotReady)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_names_and_values_must_be_the_same_complete_native_observation() {
        let names=b"global\0file:/approved/gitconfig\0include.path\0local\0file:/approved/repo/.git/config\0core.bare\0";
        let values=b"global\0file:/approved/gitconfig\0include.path\nrelative/config\0local\0file:/approved/repo/.git/config\0core.bare\0";
        let rows = config_records(names, values).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "include.path");
        assert_eq!(rows[0].value.as_deref(), Some("relative/config"));
        assert!(rows[1].value.is_none());
        assert!(config_records(names, &values[..values.len() - 1]).is_err());
        let changed = String::from_utf8(values.to_vec())
            .unwrap()
            .replace("include.path", "include.other");
        assert!(config_records(names, changed.as_bytes()).is_err());
        let malformed = String::from_utf8(names.to_vec())
            .unwrap()
            .replace("include.path", "include.\npath");
        assert!(config_records(malformed.as_bytes(), values).is_err());
        let foreign = String::from_utf8(values.to_vec())
            .unwrap()
            .replace("file:/approved/gitconfig", "file:/another/config");
        assert!(config_records(names, foreign.as_bytes()).is_err());
    }
    #[test]
    fn administrative_paths_are_component_normalized_and_never_accept_unrepresentable_paths() {
        assert_eq!(
            normalized("../common", Path::new("/approved/repo/admin")).unwrap(),
            Path::new("/approved/repo/common")
        );
        for invalid in ["", "bad\0path", "bad\npath"] {
            assert!(normalized(invalid, Path::new("/approved")).is_err());
        }
        assert!(normalized("../../escape", Path::new("/approved")).is_err());
        assert_eq!(unquote_path("\"../ob\\152ects\"").unwrap(), "../objects");
        assert!(unquote_path("\"bad\\x\"").is_err());
    }
}
