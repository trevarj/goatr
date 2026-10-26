// SPDX-License-Identifier: GPL-3.0-or-later
//! Original namespace/cwd authority, independent of creation, input and uploads.
use super::*;
use crate::{
    backends::PublicationGuard,
    herdr::{self, ProcessEvidence, Source, Topology},
    pairing::SecurityResult,
    protocol::*,
};
use std::sync::{Arc, Weak};
use std::time::Duration;

pub(crate) fn filesystem(error: io::Error) -> ErrorCode {
    match error.kind() {
        io::ErrorKind::OutOfMemory => ErrorCode::ResourceLimit,
        io::ErrorKind::TimedOut => ErrorCode::Timeout,
        _ => ErrorCode::NotReady,
    }
}
fn process_current(process: &ProcessEvidence, deadline: Instant) -> io::Result<()> {
    check(deadline)?;
    if herdr::process_evidence(process.pid)? != *process {
        return Err(invalid("original process changed"));
    }
    check(deadline)
}
pub(super) fn process_directory(process: &ProcessEvidence, deadline: Instant) -> io::Result<File> {
    process_current(process, deadline)?;
    let proc = pin_at(libc::AT_FDCWD, c"/proc", ROOT_RESOLVE, true, deadline)?;
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    if unsafe { libc::fstatfs(proc.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { filesystem.assume_init() }.f_type != libc::PROC_SUPER_MAGIC {
        return Err(invalid("verified procfs unavailable"));
    }
    let name =
        CString::new(process.pid.to_string()).map_err(|_| invalid("invalid process identity"))?;
    let directory = pin_at(proc.as_raw_fd(), &name, CHILD_RESOLVE, true, deadline)?;
    process_current(process, deadline)?;
    Ok(directory)
}
fn process_link(directory: &File, name: &CStr, deadline: Instant) -> io::Result<File> {
    check(deadline)?;
    // Only root/cwd of the exact original PID in verified procfs may be followed.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    inspect_directory(&file, deadline)?;
    Ok(file)
}
pub(crate) fn kernel_cwd(
    process: &ProcessEvidence,
    deadline: Instant,
) -> io::Result<(File, PathBuf)> {
    let directory = process_directory(process, deadline)?;
    let cwd = process_link(&directory, c"cwd", deadline)?;
    let mut buffer = [0u8; PATH_BYTES + 1];
    let size = unsafe {
        libc::readlinkat(
            directory.as_raw_fd(),
            c"cwd".as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if size < 0 {
        return Err(io::Error::last_os_error());
    }
    let size = usize::try_from(size).map_err(|_| limit())?;
    if size == 0 || size == buffer.len() {
        return Err(limit());
    }
    let text =
        std::str::from_utf8(&buffer[..size]).map_err(|_| invalid("kernel cwd is not UTF-8"))?;
    if !text.starts_with('/') || text.chars().any(char::is_control) {
        return Err(invalid("kernel cwd is unavailable"));
    }
    process_current(process, deadline)?;
    Ok((cwd, PathBuf::from(text)))
}
fn same_object(left: Object, right: Object) -> bool {
    left.device == right.device && left.inode == right.inode && left.kind == right.kind
}
pub(super) struct NamespaceRoot {
    namespace: File,
    namespace_object: Object,
    file: File,
    object: Object,
}
impl NamespaceRoot {
    pub(super) fn capture(
        root: &Root,
        process: &ProcessEvidence,
        deadline: Instant,
    ) -> io::Result<Self> {
        root.current(deadline)?;
        let directory = process_directory(process, deadline)?;
        let namespace = process_link(&directory, c"root", deadline)?;
        let namespace_object = inspect_directory(&namespace, deadline)?.0;
        let file = pin_at(
            namespace.as_raw_fd(),
            &root.pathname,
            ROOT_RESOLVE | libc::RESOLVE_IN_ROOT,
            true,
            deadline,
        )?;
        let object = inspect_directory(&file, deadline)?.0;
        if !same_object(object, root.object) {
            return Err(invalid("project namespace mapping is unavailable"));
        }
        process_current(process, deadline)?;
        Ok(Self {
            namespace,
            namespace_object,
            file,
            object,
        })
    }
    pub(super) fn current(
        &self,
        root: &Root,
        process: &ProcessEvidence,
        deadline: Instant,
    ) -> io::Result<()> {
        root.current(deadline)?;
        if inspect_directory(&self.namespace, deadline)?.0 != self.namespace_object
            || inspect_directory(&self.file, deadline)?.0 != self.object
        {
            return Err(invalid("held namespace mapping changed"));
        }
        let current = Self::capture(root, process, deadline)?;
        if current.namespace_object != self.namespace_object || current.object != self.object {
            return Err(invalid("original namespace mapping changed"));
        }
        Ok(())
    }
    pub(super) fn resolve<'a>(
        &self,
        parts: impl IntoIterator<Item = &'a str>,
        expected: Object,
        deadline: Instant,
    ) -> io::Result<File> {
        let mut buffer = [0u8; PATH_BYTES + 1];
        let mut length = 0usize;
        for name in parts {
            label(name.as_bytes())?;
            let separator = usize::from(length != 0);
            let end = length
                .checked_add(separator)
                .and_then(|value| value.checked_add(name.len()))
                .filter(|value| *value <= PATH_BYTES)
                .ok_or_else(limit)?;
            if separator != 0 {
                buffer[length] = b'/';
                length += 1;
            }
            buffer[length..end].copy_from_slice(name.as_bytes());
            length = end;
        }
        if length == 0 {
            buffer[0] = b'.';
            length = 1;
        }
        let name = CStr::from_bytes_with_nul(&buffer[..length + 1])
            .map_err(|_| invalid("invalid namespace pathname"))?;
        let file = pin_at(self.file.as_raw_fd(), name, CHILD_RESOLVE, false, deadline)?;
        let metadata = file.metadata()?;
        let object = object_identity(&file, &metadata, deadline)?;
        if object.mount != self.object.mount || !same_object(object, expected) {
            return Err(invalid(
                "project object is not the original provider-visible object",
            ));
        }
        Ok(file)
    }
}

pub(crate) struct Location {
    pub(crate) root: Arc<Root>,
    pub(crate) cwd: Directory,
    process: ProcessEvidence,
    namespace: NamespaceRoot,
    native_cwd: Object,
    kernel: bool,
}
impl Location {
    pub(crate) fn capture(
        root: Arc<Root>,
        process: ProcessEvidence,
        cwd: &Path,
        kernel: bool,
        deadline: Instant,
    ) -> io::Result<Arc<Self>> {
        let directory = root.directory_at(cwd, deadline)?;
        let namespace = NamespaceRoot::capture(&root, &process, deadline)?;
        let native = namespace.resolve(
            directory.components.iter().map(String::as_str),
            directory.object,
            deadline,
        )?;
        let native_cwd = inspect_directory(&native, deadline)?.0;
        let location = Arc::new(Self {
            root,
            cwd: directory,
            process,
            namespace,
            native_cwd,
            kernel,
        });
        location.recheck(deadline)?;
        Ok(location)
    }
    pub(crate) fn process(&self) -> &ProcessEvidence {
        &self.process
    }
    pub(crate) fn cwd_path(&self) -> &Path {
        &self.cwd.pathname
    }
    pub(crate) fn recheck(&self, deadline: Instant) -> io::Result<()> {
        process_current(&self.process, deadline)?;
        self.root.revalidate(&self.cwd, deadline)?;
        self.namespace
            .current(&self.root, &self.process, deadline)?;
        let native = self.namespace.resolve(
            self.cwd.components.iter().map(String::as_str),
            self.cwd.object,
            deadline,
        )?;
        if inspect_directory(&native, deadline)?.0 != self.native_cwd {
            return Err(invalid("original cwd namespace object changed"));
        }
        if self.kernel {
            let (actual, path) = kernel_cwd(&self.process, deadline)?;
            if path != self.cwd.pathname
                || inspect_directory(&actual, deadline)?.0 != self.native_cwd
            {
                return Err(invalid("original shell cwd changed"));
            }
        }
        process_current(&self.process, deadline)
    }
    pub(crate) fn visible_directory(
        &self,
        directory: &Directory,
        deadline: Instant,
    ) -> io::Result<()> {
        self.recheck(deadline)?;
        self.root.revalidate(directory, deadline)?;
        self.namespace.resolve(
            directory.components.iter().map(String::as_str),
            directory.object,
            deadline,
        )?;
        self.recheck(deadline)
    }
    pub(crate) fn visible_children(
        &self,
        directory: &Directory,
        children: &[Child],
        deadline: Instant,
    ) -> io::Result<()> {
        self.visible_directory(directory, deadline)?;
        for child in children {
            self.namespace.resolve(
                directory
                    .components
                    .iter()
                    .map(String::as_str)
                    .chain(std::iter::once(child.name.as_str())),
                child.object,
                deadline,
            )?;
        }
        self.recheck(deadline)
    }
    pub(crate) fn visible_file(
        &self,
        selected: &FileSelection,
        deadline: Instant,
    ) -> io::Result<()> {
        self.recheck(deadline)?;
        self.root.revalidate_file(selected, deadline)?;
        let native = self.namespace.resolve(
            selected.components.iter().map(String::as_str),
            selected.object,
            deadline,
        )?;
        if stamp(&native.metadata()?) != selected.stamp {
            return Err(invalid("original provider-visible file changed"));
        }
        self.recheck(deadline)
    }
    fn same_identity(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
            && self.cwd.object == other.cwd.object
            && self.cwd.pathname == other.cwd.pathname
            && self.process == other.process
            && self.namespace.namespace_object == other.namespace.namespace_object
            && self.namespace.object == other.namespace.object
            && self.native_cwd == other.native_cwd
            && self.kernel == other.kernel
    }
}

pub(crate) struct ProjectReadOrigin {
    pub(crate) source: ArtifactSource,
    pub(crate) epoch: Uuid,
    pub(crate) physical: Source,
    pub(crate) validate: Arc<dyn Fn() -> SecurityResult<()> + Send + Sync>,
    pub(crate) publish: PublicationGuard,
    pub(crate) captured: Instant,
}
struct ProjectAuthority {
    source: ArtifactSource,
    epoch: Uuid,
    physical: Source,
    project: Uuid,
    location: Arc<Location>,
    topology: Weak<Topology>,
    validate: Arc<dyn Fn() -> SecurityResult<()> + Send + Sync>,
    publish: PublicationGuard,
    captured: Instant,
}
#[derive(Clone)]
pub(crate) struct ProjectReadContext {
    inner: Arc<ProjectAuthority>,
}
impl ProjectReadContext {
    pub(crate) fn capture(
        topology: &Arc<Topology>,
        origin: ProjectReadOrigin,
        project: Uuid,
        location: Arc<Location>,
    ) -> SecurityResult<Self> {
        let ProjectReadOrigin {
            source,
            epoch,
            physical,
            validate,
            publish,
            captured,
        } = origin;
        let context = Self {
            inner: Arc::new(ProjectAuthority {
                source,
                epoch,
                physical,
                project,
                location,
                topology: Arc::downgrade(topology),
                validate,
                publish,
                captured,
            }),
        };
        context.recheck(captured + Duration::from_secs(5))?;
        Ok(context)
    }
    pub(crate) fn source(&self) -> &ArtifactSource {
        &self.inner.source
    }
    pub(crate) fn epoch(&self) -> &Uuid {
        &self.inner.epoch
    }
    pub(crate) fn physical(&self) -> &Source {
        &self.inner.physical
    }
    pub(crate) fn project(&self) -> &Uuid {
        &self.inner.project
    }
    pub(crate) fn location(&self) -> &Arc<Location> {
        &self.inner.location
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        16 * 1024 + self.inner.location.cwd.retained_bytes()
    }
    pub(crate) fn topology(&self) -> SecurityResult<Arc<Topology>> {
        self.inner.topology.upgrade().ok_or(ErrorCode::StaleSession)
    }
    pub(crate) fn observation_deadline(&self) -> Instant {
        self.inner.captured + Duration::from_secs(5)
    }
    pub(crate) fn recheck(&self, deadline: Instant) -> SecurityResult<()> {
        (self.inner.validate)()?;
        self.inner.location.recheck(deadline).map_err(filesystem)?;
        self.topology()?.with_project_context(self, &mut || Ok(()))
    }
    pub(crate) fn with_current(
        &self,
        deadline: Instant,
        mut commit: impl FnMut() -> SecurityResult<()>,
    ) -> SecurityResult<()> {
        (self.inner.publish)(&mut || {
            self.inner.location.recheck(deadline).map_err(filesystem)?;
            self.topology()?.with_project_context(self, &mut commit)
        })
    }
    pub(crate) fn same_authority(&self, other: &Self) -> bool {
        self.source() == other.source()
            && self.epoch() == other.epoch()
            && self.project() == other.project()
            && self.physical().same_physical_identity(other.physical())
            && self.location().same_identity(other.location())
    }
}
