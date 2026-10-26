// SPDX-License-Identifier: GPL-3.0-or-later
//! Held, mount-bounded roots. Creation names never imply document-read authority.
use std::{
    ffi::{CStr, CString, OsStr},
    fs::{File, Metadata},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    time::Instant,
};

mod context;
pub(crate) mod git;
use context::NamespaceRoot;
pub(crate) use context::{Location,ProjectReadContext,ProjectReadOrigin,filesystem,kernel_cwd};

const DEPTH: usize = 16;
const ENTRIES: usize = 1000;
const NAME_BYTES: usize = 255;
const PATH_BYTES: usize = 4095;
const ENCODED_NAMES_BYTES: usize = 256 * 1024;
const DOCUMENT_BYTES: u64 = 1024 * 1024;
const FILE_BYTES: u64 = 32 * 1024 * 1024;
const ROOT_RESOLVE: u64 = libc::RESOLVE_NO_MAGICLINKS | libc::RESOLVE_NO_SYMLINKS;
const CHILD_RESOLVE: u64 = ROOT_RESOLVE | libc::RESOLVE_BENEATH | libc::RESOLVE_NO_XDEV;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Object {
    device: u64,
    inode: u64,
    mount: u64,
    kind: u32,
}

pub(crate) struct Root {
    file: File,
    object: Object,
    pathname: CString,
}

pub(crate) struct Directory {
    file: File,
    object: Object,
    root: Object,
    components: Vec<String>,
    pathname: PathBuf,
}

pub(crate) struct FileSelection {
    file: File,
    object: Object,
    root: Object,
    components: Vec<String>,
    pathname: PathBuf,
    stamp: (u64, i64, i64, i64, i64),
    maximum: u64,
}

/// Metadata only: a listing never keeps one open descriptor per unselected file.
pub(crate) struct Child {
    pub(crate) name: String,
    object: Object,
    stamp: (u64, i64, i64, i64, i64),
}
impl Child {
    pub(crate) fn is_directory(&self) -> bool { self.object.kind == libc::S_IFDIR }
    pub(crate) fn size(&self) -> Option<u64> { (!self.is_directory()).then_some(self.stamp.0) }
    pub(crate) fn retained_bytes(&self) -> usize { std::mem::size_of::<Self>() + self.name.capacity() }
}

pub(crate) struct Snapshot {
    bytes: Vec<u8>,
    pub(crate) sha256: String,
}
impl Snapshot {
    pub(crate) fn bytes(&self) -> &[u8] { &self.bytes }
}
impl Drop for Snapshot {
    fn drop(&mut self) { self.bytes.fill(0); }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn limit() -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, "project directory bounds exceeded")
}
fn check(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "project directory deadline expired"));
    }
    Ok(())
}
fn component(name: &[u8]) -> io::Result<()> {
    if name.is_empty() || name.len() > NAME_BYTES || name == b"." || name == b".."
        || name.contains(&0) || name.contains(&b'/')
    {
        return Err(invalid("unsafe project directory component"));
    }
    Ok(())
}
fn label(name: &[u8]) -> io::Result<&str> {
    component(name)?;
    let name = std::str::from_utf8(name).map_err(|_| invalid("directory name is not UTF-8"))?;
    if name.chars().any(char::is_control) {
        return Err(invalid("directory name contains a control character"));
    }
    Ok(name)
}
fn relative(components: &[String]) -> io::Result<CString> {
    if components.len() > DEPTH { return Err(limit()); }
    relative_path(components)
}
fn relative_path(components: &[String]) -> io::Result<CString> {
    let mut size = components.len().saturating_sub(1);
    for name in components {
        label(name.as_bytes())?;
        size = size.checked_add(name.len()).ok_or_else(limit)?;
    }
    if size > PATH_BYTES { return Err(limit()); }
    let mut bytes = Vec::with_capacity(size.max(1)+1);
    if components.is_empty() { bytes.push(b'.'); }
    for (index, name) in components.iter().enumerate() {
        if index != 0 { bytes.push(b'/'); }
        bytes.extend_from_slice(name.as_bytes());
    }
    CString::new(bytes).map_err(|_| invalid("unsafe relative project pathname"))
}
fn pin_at(parent: RawFd, pathname: &CStr, resolve: u64, directory: bool, deadline: Instant) -> io::Result<File> {
    check(deadline)?;
    // SAFETY: zeroed Linux open_how, a live parent or AT_FDCWD, and a complete C pathname.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW
        | if directory { libc::O_DIRECTORY } else { 0 }) as u64;
    how.resolve = resolve;
    let fd = unsafe { libc::syscall(libc::SYS_openat2, parent, pathname.as_ptr(), &how, std::mem::size_of::<libc::open_how>()) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: successful openat2 returned this exclusively owned descriptor.
    let file = unsafe { File::from_raw_fd(fd as libc::c_int) };
    check(deadline)?;
    Ok(file)
}
fn object_identity(file: &File, metadata: &Metadata, deadline: Instant) -> io::Result<Object> {
    check(deadline)?;
    let mut sx = std::mem::MaybeUninit::<libc::statx>::zeroed();
    // SAFETY: live held descriptor, empty C pathname, and correctly sized statx storage.
    if unsafe { libc::statx(file.as_raw_fd(), c"".as_ptr(), libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
        libc::STATX_MNT_ID, sx.as_mut_ptr()) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let sx = unsafe { sx.assume_init() };
    if sx.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "directory mount identity unavailable"));
    }
    check(deadline)?;
    Ok(Object { device: metadata.dev(), inode: metadata.ino(), mount: sx.stx_mnt_id,
        kind: metadata.mode() & libc::S_IFMT })
}
fn inspect_directory(file: &File, deadline: Instant) -> io::Result<(Object, Metadata)> {
    check(deadline)?;
    let metadata = file.metadata()?;
    if !metadata.is_dir() { return Err(invalid("selected project object is not a directory")); }
    let proof = object_identity(file, &metadata, deadline)?;
    Ok((proof, metadata))
}
fn inspect_regular_bounded(file: &File, maximum: u64, deadline: Instant) -> io::Result<(Object, Metadata)> {
    check(deadline)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() { return Err(invalid("selected object is not a regular file")); }
    if metadata.len() > maximum { return Err(limit()); }
    Ok((object_identity(file, &metadata, deadline)?, metadata))
}

fn trusted_root(metadata: &Metadata) -> io::Result<()> {
    if metadata.uid() != crate::pairing::uid() || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "unsafe configured project root"));
    }
    Ok(())
}
fn stamp(metadata: &Metadata) -> (u64, i64, i64, i64, i64) {
    (metadata.size(), metadata.mtime(), metadata.mtime_nsec(), metadata.ctime(), metadata.ctime_nsec())
}
fn readable_object(pin: &File, expected: Object, deadline: Instant) -> io::Result<File> {
    if !matches!(expected.kind, libc::S_IFDIR | libc::S_IFREG)
        || object_identity(pin, &pin.metadata()?, deadline)? != expected {
        return Err(invalid("held object changed"));
    }
    // Only this verified procfs seam follows a magic link: our still-held numeric FD.
    let fd = unsafe { libc::open(c"/proc/self/fd".as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let proc = unsafe { File::from_raw_fd(fd) };
    check(deadline)?;
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::zeroed();
    // SAFETY: live proc-directory descriptor and correctly sized statfs storage.
    if unsafe { libc::fstatfs(proc.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { filesystem.assume_init() }.f_type != libc::PROC_SUPER_MAGIC {
        return Err(invalid("verified procfs directory unavailable"));
    }
    check(deadline)?;
    let name = CString::new(pin.as_raw_fd().to_string()).map_err(|_| invalid("invalid held descriptor"))?;
    // Only a proved regular object or directory is reopened; directory callers retain O_DIRECTORY.
    let fd = unsafe { libc::openat(proc.as_raw_fd(), name.as_ptr(),
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NONBLOCK
            | if expected.kind == libc::S_IFDIR { libc::O_DIRECTORY } else { 0 }) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let readable = unsafe { File::from_raw_fd(fd) };
    if object_identity(&readable, &readable.metadata()?, deadline)? != expected
        || object_identity(pin, &pin.metadata()?, deadline)? != expected {
        return Err(invalid("readable-open identity changed"));
    }
    Ok(readable)
}

impl Root {
    pub(crate) fn open(pathname: &Path, deadline: Instant) -> io::Result<Self> {
        check(deadline)?;
        if pathname.as_os_str().len() > PATH_BYTES { return Err(limit()); }
        let text = pathname.to_str().ok_or_else(|| invalid("project root is not UTF-8"))?;
        if !pathname.is_absolute() || text.chars().any(char::is_control) {
            return Err(invalid("absolute control-free project root required"));
        }
        if text != "/" {
            for name in text.strip_prefix('/').ok_or_else(|| invalid("absolute project root required"))?.split('/') {
                label(name.as_bytes())?;
            }
        }
        let pathname = CString::new(text).map_err(|_| invalid("unsafe project root pathname"))?;
        // Mount crossings to the configured root are allowed; descendant crossings are not.
        let file = pin_at(libc::AT_FDCWD, &pathname, ROOT_RESOLVE, true, deadline)?;
        let (object, metadata) = inspect_directory(&file, deadline)?;
        trusted_root(&metadata)?;
        let root = Self { file, object, pathname };
        root.current(deadline)?;
        Ok(root)
    }
    pub(crate) fn pathname(&self) -> &Path {
        Path::new(OsStr::from_bytes(self.pathname.as_bytes()))
    }
    pub(crate) fn current(&self, deadline: Instant) -> io::Result<()> {
        let (held, metadata) = inspect_directory(&self.file, deadline)?;
        trusted_root(&metadata)?;
        let current = pin_at(libc::AT_FDCWD, &self.pathname, ROOT_RESOLVE, true, deadline)?;
        let (current, metadata) = inspect_directory(&current, deadline)?;
        trusted_root(&metadata)?;
        if held != self.object || current != self.object {
            return Err(invalid("configured project root object changed"));
        }
        check(deadline)
    }
    fn selected_path(&self, components: &[String]) -> io::Result<PathBuf> {
        let mut bytes = self.pathname.as_bytes().len();
        for (index, name) in components.iter().enumerate() {
            let separator = usize::from(index != 0 || self.pathname.as_bytes() != b"/");
            bytes = bytes.checked_add(separator).and_then(|bytes| bytes.checked_add(name.len())).ok_or_else(limit)?;
        }
        if bytes > PATH_BYTES { return Err(limit()); }
        let mut pathname = PathBuf::with_capacity(bytes);
        pathname.push(self.pathname());
        for name in components { pathname.push(name); }
        Ok(pathname)
    }
    pub(crate) fn select(&self, components: &[String], deadline: Instant) -> io::Result<Directory> {
        let relative = relative(components)?;
        let pathname = self.selected_path(components)?;
        self.current(deadline)?;
        let file = pin_at(self.file.as_raw_fd(), &relative, CHILD_RESOLVE, false, deadline)?;
        let (object, _) = inspect_directory(&file, deadline)?;
        if object.mount != self.object.mount { return Err(invalid("project directory crossed a mount")); }
        self.current(deadline)?;
        let selected = Directory { file, object, root: self.object, components: components.to_vec(), pathname };
        check(deadline)?;
        Ok(selected)
    }
    /// Returns the stable authorized pathname, never a proc-FD alias. Native chdir is still racy.
    pub(crate) fn revalidate<'a>(&self, selected: &'a Directory, deadline: Instant) -> io::Result<&'a Path> {
        self.current(deadline)?;
        if selected.root != self.object
            || selected.pathname.as_os_str() != self.selected_path(&selected.components)?.as_os_str()
            || inspect_directory(&selected.file, deadline)?.0 != selected.object {
            return Err(invalid("selected directory does not belong to the held root"));
        }
        let relative = relative_path(&selected.components)?;
        let current = pin_at(self.file.as_raw_fd(), &relative, CHILD_RESOLVE, false, deadline)?;
        if inspect_directory(&current, deadline)?.0 != selected.object {
            return Err(invalid("selected project directory object changed"));
        }
        self.current(deadline)?;
        Ok(&selected.pathname)
    }
    pub(crate) fn matches_directory(&self, selected: &Directory, actual: &File, deadline: Instant) -> io::Result<bool> {
        self.revalidate(selected, deadline)?;
        let matches = inspect_directory(actual, deadline)?.0 == selected.object;
        self.revalidate(selected, deadline)?;
        Ok(matches)
    }
    fn components_for(&self, pathname: &Path) -> io::Result<Vec<String>> {
        let child = pathname.strip_prefix(self.pathname()).map_err(|_| invalid("object is outside the held root"))?;
        let components = child.components().map(|part| match part {
            std::path::Component::Normal(name) => label(name.as_bytes()).map(str::to_owned),
            _ => Err(invalid("unsafe project component")),
        }).collect::<io::Result<Vec<_>>>()?;
        if self.selected_path(&components)? != pathname { return Err(invalid("exact object pathname required")); }
        Ok(components)
    }
    pub(crate) fn directory_at(&self, pathname: &Path, deadline: Instant) -> io::Result<Directory> {
        let components=self.components_for(pathname)?;
        let relative=relative_path(&components)?;
        self.current(deadline)?;
        let file=pin_at(self.file.as_raw_fd(),&relative,CHILD_RESOLVE,true,deadline)?;
        let (object,_)=inspect_directory(&file,deadline)?;
        if object.mount!=self.object.mount {return Err(invalid("native directory crossed a mount"));}
        let selected=Directory {file,object,root:self.object,components,pathname:pathname.to_owned()};
        self.revalidate(&selected,deadline)?;Ok(selected)
    }
    /// Exact enabled Skill documents retain their independent one-MiB ceiling.
    pub(crate) fn pin_file(&self, pathname: &Path, deadline: Instant) -> io::Result<FileSelection> {
        self.pin_regular(pathname, DOCUMENT_BYTES, deadline, true)
    }
    fn pin_review_file(&self,pathname:&Path,maximum:u64,deadline:Instant)->io::Result<FileSelection> {
        self.pin_regular(pathname,maximum,deadline,false)
    }
    fn pin_regular(&self, pathname: &Path, maximum: u64, deadline: Instant, navigation:bool) -> io::Result<FileSelection> {
        if maximum == 0 || maximum > FILE_BYTES { return Err(limit()); }
        let components = self.components_for(pathname)?;
        if components.is_empty() { return Err(invalid("regular child pathname required")); }
        let relative = if navigation {relative(&components)?}else{relative_path(&components)?};
        self.current(deadline)?;
        let file = pin_at(self.file.as_raw_fd(), &relative, CHILD_RESOLVE, false, deadline)?;
        let (object, metadata) = inspect_regular_bounded(&file, maximum, deadline)?;
        if object.mount != self.object.mount { return Err(invalid("file crossed a mount")); }
        self.current(deadline)?;
        Ok(FileSelection { file, object, root:self.object, components, pathname:pathname.to_owned(), stamp:stamp(&metadata), maximum })
    }
    pub(crate) fn child_directory(&self, parent: &Directory, child: &Child, deadline: Instant) -> io::Result<Directory> {
        self.revalidate(parent, deadline)?;
        if !child.is_directory() { return Err(invalid("directory reference required")); }
        let mut components = parent.components.clone(); components.push(child.name.clone());
        let selected = self.select(&components, deadline)?;
        if selected.object != child.object { return Err(invalid("listed directory was replaced")); }
        Ok(selected)
    }
    pub(crate) fn child_file(&self, parent: &Directory, child: &Child, deadline: Instant) -> io::Result<FileSelection> {
        self.revalidate(parent, deadline)?;
        if child.is_directory() { return Err(invalid("file reference required")); }
        let mut components = parent.components.clone(); components.push(child.name.clone());
        let selected = self.pin_regular(&self.selected_path(&components)?, FILE_BYTES, deadline, true)?;
        if selected.object != child.object || selected.stamp != child.stamp { return Err(invalid("listed file changed")); }
        Ok(selected)
    }
    pub(crate) fn revalidate_file(&self, selected: &FileSelection, deadline: Instant) -> io::Result<()> {
        self.current(deadline)?;
        let (held, metadata) = inspect_regular_bounded(&selected.file, selected.maximum, deadline)?;
        if selected.root != self.object || held != selected.object || stamp(&metadata) != selected.stamp
            || self.selected_path(&selected.components)? != selected.pathname {
            return Err(invalid("selected document object changed"));
        }
        let relative = relative_path(&selected.components)?;
        let current = pin_at(self.file.as_raw_fd(), &relative, CHILD_RESOLVE, false, deadline)?;
        let (object, metadata) = inspect_regular_bounded(&current, selected.maximum, deadline)?;
        if object != selected.object || stamp(&metadata) != selected.stamp {
            return Err(invalid("document pathname changed"));
        }
        self.current(deadline)
    }
    /// Reserve the real worker/snapshot owner before calling; cancellation cannot release it early.
    pub(crate) fn snapshot_file(&self, selected: &FileSelection, deadline: Instant) -> io::Result<Snapshot> {
        use sha2::Digest;
        self.revalidate_file(selected, deadline)?;
        let mut file = readable_object(&selected.file, selected.object, deadline)?;
        let mut snapshot = Snapshot { bytes:vec![0u8; usize::try_from(selected.stamp.0).map_err(|_| limit())?], sha256:String::new() };
        let mut offset = 0;
        while offset < snapshot.bytes.len() {
            check(deadline)?;
            let end = snapshot.bytes.len().min(offset + 65536);
            let count = file.read(&mut snapshot.bytes[offset..end])?;
            if count == 0 { return Err(invalid("file ended before its proved size")); }
            offset += count;
            check(deadline)?;
        }
        let mut extra = [0u8; 1];
        if file.read(&mut extra)? != 0 { return Err(invalid("file exceeded its proved size")); }
        let (object, metadata) = inspect_regular_bounded(&file, selected.maximum, deadline)?;
        if object != selected.object || stamp(&metadata) != selected.stamp { return Err(invalid("file changed during capture")); }
        self.revalidate_file(selected, deadline)?;
        snapshot.sha256 = crate::pairing::hex(&sha2::Sha256::digest(&snapshot.bytes));
        Ok(snapshot)
    }
    fn visit_directory(&self, selected: &Directory, deadline: Instant, mut visit: impl FnMut(&[u8], File, Metadata) -> io::Result<()>) -> io::Result<()> {
        self.revalidate(selected, deadline)?;
        let (object, metadata) = inspect_directory(&selected.file, deadline)?;
        if object != selected.object { return Err(invalid("selected listing directory changed")); }
        let before = stamp(&metadata);
        let readable = readable_object(&selected.file, selected.object, deadline)?;
        let fd = readable.into_raw_fd();
        // SAFETY: fdopendir takes ownership only on success; failure closes our descriptor below.
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            unsafe { libc::close(fd); }
            return Err(error);
        }
        struct Entries(*mut libc::DIR);
        impl Drop for Entries { fn drop(&mut self) { unsafe { libc::closedir(self.0); } } }
        let stream = Entries(stream);
        let mut examined = 0usize;
        let mut encoded_bytes = 0usize;
        loop {
            check(deadline)?;
            // SAFETY: the owned stream stays live; errno disambiguates EOF from readdir failure.
            unsafe { *libc::__errno_location() = 0; }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if unsafe { *libc::__errno_location() } != 0 { return Err(io::Error::last_os_error()); }
                break;
            }
            let offset = std::mem::offset_of!(libc::dirent, d_name);
            let record_bytes = unsafe { (*entry).d_reclen as usize };
            if record_bytes <= offset { return Err(invalid("invalid directory entry")); }
            // Bound the native name scan by this record and Linux's maximum component size.
            let name = unsafe { std::slice::from_raw_parts(entry.cast::<u8>().add(offset), (record_bytes - offset).min(NAME_BYTES + 1)) };
            let name = CStr::from_bytes_until_nul(name).map_err(|_| invalid("unterminated directory entry"))?.to_bytes();
            if name == b"." || name == b".." { continue; }
            if examined == ENTRIES { return Err(limit()); }
            examined += 1;
            component(name)?;
            let encoded = name.iter().try_fold(2usize, |bytes, &byte| bytes.checked_add(match byte {
                b'"' | b'\\' => 2, 0..=0x1f => 6, _ => 1,
            })).ok_or_else(limit)?;
            encoded_bytes = encoded_bytes.checked_add(encoded).ok_or_else(limit)?;
            if encoded_bytes > ENCODED_NAMES_BYTES { return Err(limit()); }
            let name_c = CString::new(name).map_err(|_| invalid("unsafe directory entry"))?;
            // This parent was proved beneath the held root; each child remains beneath it on that mount.
            let file = pin_at(selected.file.as_raw_fd(), &name_c, CHILD_RESOLVE, false, deadline)?;
            let metadata = file.metadata()?;
            check(deadline)?;
            visit(name, file, metadata)?;
        }
        let (object, metadata) = inspect_directory(&selected.file, deadline)?;
        if object != selected.object || stamp(&metadata) != before {
            return Err(invalid("directory changed during listing"));
        }
        self.revalidate(selected, deadline)?;
        check(deadline)
    }
    pub(crate) fn list(&self, selected: &Directory, deadline: Instant) -> io::Result<Vec<Directory>> {
        let mut output = Vec::new();
        self.visit_directory(selected, deadline, |name, file, metadata| {
            if !metadata.is_dir() { return Ok(()); }
            let object = object_identity(&file, &metadata, deadline)?;
            if object.mount != self.object.mount { return Err(invalid("project directory crossed a mount")); }
            let name = label(name)?;
            if selected.components.len() == DEPTH { return Err(limit()); }
            let mut components = selected.components.clone(); components.push(name.to_owned());
            let pathname = self.selected_path(&components)?;
            output.push(Directory { file, object, root:self.object, components, pathname });
            Ok(())
        })?;
        output.sort_unstable_by(|left,right|left.components.last().cmp(&right.components.last()));
        Ok(output)
    }
    pub(crate) fn list_entries(&self, selected: &Directory, deadline: Instant) -> io::Result<Vec<Child>> {
        let mut output = Vec::new();
        let mut bytes = 0usize;
        self.visit_directory(selected, deadline, |name, file, metadata| {
            if !metadata.is_dir() && !metadata.is_file() { return Err(invalid("unsupported project entry type")); }
            if selected.components.len() == DEPTH { return Err(limit()); }
            let object = object_identity(&file, &metadata, deadline)?;
            if object.mount != self.object.mount { return Err(invalid("project entry crossed a mount")); }
            let name = label(name)?;
            bytes = bytes.checked_add(std::mem::size_of::<Child>() + name.len()).ok_or_else(limit)?;
            if bytes > ENCODED_NAMES_BYTES { return Err(limit()); }
            output.push(Child { name:name.to_owned(), object, stamp:stamp(&metadata) });
            Ok(())
        })?;
        output.sort_unstable_by(|left,right|left.name.cmp(&right.name));
        Ok(output)
    }
}
impl Directory {
    pub(crate) fn name(&self) -> Option<&str> {
        self.components.last().map(String::as_str)
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()+self.pathname.capacity()+self.components.capacity()*std::mem::size_of::<String>()
            +self.components.iter().map(String::capacity).sum::<usize>()
    }
}
impl FileSelection {
    pub(crate) fn pathname(&self) -> &Path { &self.pathname }
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()+self.pathname.capacity()+self.components.capacity()*std::mem::size_of::<String>()
            +self.components.iter().map(String::capacity).sum::<usize>()
    }
    pub(crate) fn provider_object(&self) -> (u64,u64) { (self.object.device,self.object.inode) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::unix::{ffi::OsStringExt, fs::DirBuilderExt}, time::Duration};

    #[test]
    fn held_creation_directories_refuse_escape_and_replacement_without_retiring_for_contents() {
        fn deadline() -> Instant { Instant::now() + Duration::from_secs(30) }
        struct Fixture(PathBuf);
        impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let fixture = Fixture(parent.join(format!("goatr-project-{}", crate::protocol::Uuid::random().0)));
        std::fs::DirBuilder::new().mode(0o700).create(&fixture.0).unwrap();
        let project = fixture.0.join("project");
        let outside = fixture.0.join("outside");
        for path in [&project, &outside] {
            std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        std::fs::create_dir(project.join("empty")).unwrap();
        std::fs::create_dir(outside.join("leaf")).unwrap();
        std::fs::write(project.join("ordinary-file"), b"not read").unwrap();
        std::os::unix::fs::symlink(&outside, project.join("escape")).unwrap();
        std::os::unix::fs::symlink(&project, fixture.0.join("root-alias")).unwrap();
        let root = Root::open(&project, deadline()).unwrap();
        let top = root.select(&[], deadline()).unwrap();
        let empty = root.select(&["empty".into()], deadline()).unwrap();
        assert!(root.list(&empty, deadline()).unwrap().is_empty());
        assert_eq!(root.revalidate(&empty, deadline()).unwrap(), project.join("empty"));
        let names = root.list(&top, deadline()).unwrap().into_iter().map(|entry| entry.name().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(names, ["empty"]);
        assert!(root.select(&["..".into()], deadline()).is_err());
        assert!(root.select(&["/outside".into()], deadline()).is_err());
        assert!(root.select(&["escape".into()], deadline()).is_err());
        assert!(root.select(&["escape".into(), "leaf".into()], deadline()).is_err());
        assert!(Root::open(&fixture.0.join("root-alias"), deadline()).is_err());
        let magic = PathBuf::from(format!("/proc/self/fd/{}", root.file.as_raw_fd()));
        assert!(Root::open(&magic, deadline()).is_err());
        std::fs::write(project.join("empty/new-file"), b"not read").unwrap();
        root.revalidate(&empty, deadline()).unwrap();
        let same = pin_at(root.file.as_raw_fd(), c"empty", CHILD_RESOLVE, false, deadline()).unwrap();
        assert!(root.matches_directory(&empty, &same, deadline()).unwrap());
        let invalid = project.join(OsString::from_vec(vec![b'd', 0xff]));
        std::fs::create_dir(&invalid).unwrap();
        assert!(root.list(&top, deadline()).is_err());
        std::fs::remove_dir(&invalid).unwrap();
        let control = project.join("bad\nname");
        std::fs::create_dir(&control).unwrap();
        assert!(root.list(&top, deadline()).is_err());
        std::fs::remove_dir(&control).unwrap();
        let mut components = Vec::new();
        let mut nested = project.clone();
        for _ in 0..DEPTH {
            components.push("d".to_owned());
            nested.push("d");
            std::fs::create_dir(&nested).unwrap();
        }
        root.select(&components, deadline()).unwrap();
        components.push("d".into());
        assert!(root.select(&components, deadline()).is_err());
        std::fs::create_dir(project.join("many")).unwrap();
        for index in 0..=ENTRIES { File::create(project.join("many").join(format!("file-{index}"))).unwrap(); }
        let many = root.select(&["many".into()], deadline()).unwrap();
        assert_eq!(root.list(&many, deadline()).err().unwrap().kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(root.list_entries(&many, deadline()).err().unwrap().kind(), io::ErrorKind::OutOfMemory);
        assert!(root.select(&[], Instant::now()).is_err());
        std::fs::rename(project.join("empty"), project.join("moved-empty")).unwrap();
        std::fs::create_dir(project.join("empty")).unwrap();
        assert!(root.revalidate(&empty, deadline()).is_err());
        let replacement = root.select(&["empty".into()], deadline()).unwrap();
        assert!(!root.matches_directory(&replacement, &same, deadline()).unwrap());
        std::fs::rename(&project, fixture.0.join("retired-project")).unwrap();
        std::fs::DirBuilder::new().mode(0o700).create(&project).unwrap();
        assert!(root.revalidate(&top, deadline()).is_err());
        assert!(root.list(&top, deadline()).is_err());
        let renamed = Root::open(&fixture.0.join("retired-project"), deadline()).unwrap();
        assert!(renamed.revalidate(&top, deadline()).is_err());
    }
    #[test]
    fn held_document_snapshots_are_complete_and_refuse_alias_type_or_object_changes() {
        fn deadline() -> Instant { Instant::now() + Duration::from_secs(30) }
        struct Fixture(PathBuf);
        impl Drop for Fixture { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
        let parent=std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let fixture=Fixture(parent.join(format!("goatr-document-{}",crate::protocol::Uuid::random().0)));
        std::fs::DirBuilder::new().mode(0o700).create(&fixture.0).unwrap();
        let granted=fixture.0.join("approved");
        std::fs::DirBuilder::new().mode(0o700).create(&granted).unwrap();
        let root=Root::open(&granted,deadline()).unwrap();
        let path=granted.join("SKILL.md");
        let content="Complete skill\n界\n".as_bytes();
        std::fs::write(&path,content).unwrap();
        let selected=root.pin_file(&path,deadline()).unwrap();
        let snapshot=root.snapshot_file(&selected,deadline()).unwrap();
        assert_eq!(snapshot.bytes(),content);
        assert!(root.pin_file(&fixture.0.join("SKILL.md"),deadline()).is_err());
        assert!(root.pin_file(&granted,deadline()).is_err());
        std::os::unix::fs::symlink(&path,granted.join("alias")).unwrap();
        assert!(root.pin_file(&granted.join("alias"),deadline()).is_err());
        let fifo=granted.join("fifo");let fifo_name=CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe {libc::mkfifo(fifo_name.as_ptr(),0o600)},0);
        assert!(root.pin_file(&fifo,deadline()).is_err());
        let oversized=granted.join("oversized");
        File::create(&oversized).unwrap().set_len(DOCUMENT_BYTES+1).unwrap();
        assert!(root.pin_file(&oversized,deadline()).is_err());
        std::fs::write(&path,b"changed original").unwrap();
        assert!(root.snapshot_file(&selected,deadline()).is_err());
        assert_eq!(snapshot.bytes(),content);
        let changed=root.pin_file(&path,deadline()).unwrap();
        std::fs::rename(&path,granted.join("moved")).unwrap();
        std::fs::write(&path,b"changed original").unwrap();
        assert!(root.snapshot_file(&changed,deadline()).is_err());
        let replacement=root.pin_file(&path,deadline()).unwrap();
        assert!(root.snapshot_file(&replacement,Instant::now()).is_err());
        std::fs::rename(&granted,fixture.0.join("retired")).unwrap();
        std::fs::DirBuilder::new().mode(0o700).create(&granted).unwrap();
        assert!(root.snapshot_file(&replacement,deadline()).is_err());
    }
    #[test]
    fn artifact_rows_pin_complete_files_and_refuse_special_replaced_or_cross_mount_objects() {
        fn deadline()->Instant {Instant::now()+Duration::from_secs(30)}
        struct Fixture(PathBuf);impl Drop for Fixture {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let parent=std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let fixture=Fixture(parent.join(format!("goatr-artifact-root-{}",crate::protocol::Uuid::random().0)));
        std::fs::DirBuilder::new().mode(0o700).create(&fixture.0).unwrap();
        let approved=fixture.0.join("approved");std::fs::create_dir(&approved).unwrap();
        let path=approved.join("complete");let bytes=vec![b'x';65536+7];std::fs::write(&path,&bytes).unwrap();
        std::fs::create_dir(approved.join("empty")).unwrap();
        let root=Root::open(&approved,deadline()).unwrap();let top=root.select(&[],deadline()).unwrap();
        let rows=root.list_entries(&top,deadline()).unwrap();
        let file=rows.iter().find(|row|row.name=="complete").unwrap();assert_eq!(file.size(),Some(bytes.len() as u64));
        let selected=root.child_file(&top,file,deadline()).unwrap();let snapshot=root.snapshot_file(&selected,deadline()).unwrap();
        assert_eq!(snapshot.bytes(),bytes);use sha2::Digest;assert_eq!(snapshot.sha256,crate::pairing::hex(&sha2::Sha256::digest(&bytes)));
        let empty=rows.iter().find(|row|row.name=="empty").unwrap();assert!(empty.is_directory());
        let directory=root.child_directory(&top,empty,deadline()).unwrap();assert!(root.list_entries(&directory,deadline()).unwrap().is_empty());
        std::fs::rename(&path,approved.join("old-file")).unwrap();std::fs::write(&path,&bytes).unwrap();
        assert!(root.child_file(&top,file,deadline()).is_err());assert!(root.snapshot_file(&selected,deadline()).is_err());assert_eq!(snapshot.bytes(),bytes);
        std::fs::rename(approved.join("empty"),approved.join("old-directory")).unwrap();std::fs::create_dir(approved.join("empty")).unwrap();
        assert!(root.child_directory(&top,empty,deadline()).is_err());
        let large=approved.join("large-artifact");File::create(&large).unwrap().set_len(DOCUMENT_BYTES+1).unwrap();
        assert!(root.pin_file(&large,deadline()).is_err());
        let rows=root.list_entries(&top,deadline()).unwrap();let large=rows.iter().find(|row|row.name=="large-artifact").unwrap();
        let selected=root.child_file(&top,large,deadline()).unwrap();assert_eq!(root.snapshot_file(&selected,deadline()).unwrap().bytes().len(),DOCUMENT_BYTES as usize+1);
        let oversized=approved.join("too-large");File::create(&oversized).unwrap().set_len(FILE_BYTES+1).unwrap();
        let rows=root.list_entries(&top,deadline()).unwrap();let oversized=rows.iter().find(|row|row.name=="too-large").unwrap();assert!(root.child_file(&top,oversized,deadline()).is_err());
        for special in ["alias","fifo","socket"] {
            let path=approved.join(special);
            let mut socket=None;
            match special {
                "alias"=>std::os::unix::fs::symlink(approved.join("complete"),&path).unwrap(),
                "fifo"=>{let name=CString::new(path.as_os_str().as_bytes()).unwrap();assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);},
                _=>socket=Some(std::os::unix::net::UnixListener::bind(&path).unwrap()),
            }
            assert!(root.list_entries(&top,deadline()).is_err());drop(socket);std::fs::remove_file(path).unwrap();
        }
        let host=pin_at(libc::AT_FDCWD,c"/",ROOT_RESOLVE,true,deadline()).unwrap();
        assert!(pin_at(host.as_raw_fd(),c"proc",CHILD_RESOLVE,true,deadline()).is_err());
        std::fs::rename(&approved,fixture.0.join("retired-root")).unwrap();std::fs::create_dir(&approved).unwrap();
        assert!(root.list_entries(&top,deadline()).is_err());assert!(root.snapshot_file(&selected,deadline()).is_err());
    }

}
