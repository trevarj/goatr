// SPDX-License-Identifier: GPL-3.0-or-later
//! The only executable authority is the manifest compiled by the locked flake.
use super::*;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::sync::OnceLock;

const MANIFEST_BYTES: u64 = 1024 * 1024;
const EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CLOSURE: usize = 4096;
static SELECTED: OnceLock<SecurityResult<Runtime>> = OnceLock::new();
pub(super) fn selected(deadline: Instant) -> SecurityResult<&'static Runtime> {
    SELECTED
        .get_or_init(|| {
            let path = option_env!("GOATR_READ_GIT_MANIFEST").ok_or(ErrorCode::NotReady)?;
            Runtime::load(Path::new(path), deadline)
        })
        .as_ref()
        .map_err(|code| *code)
}
fn fail<T>() -> SecurityResult<T> {
    Err(ErrorCode::NotReady)
}
fn field(fields: &mut Map<String, Value>, name: &str) -> SecurityResult<String> {
    match fields.remove(name) {
        Some(Value::String(value)) => Ok(value),
        _ => fail(),
    }
}
fn digest_text(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn store_prefix(path: &Path) -> SecurityResult<&Path> {
    let text = path.to_str().ok_or(ErrorCode::NotReady)?;
    if !path.is_absolute() || text.chars().any(char::is_control) || text.len() > 4095 {
        return fail();
    }
    if path.components().any(|part| {
        !matches!(
            part,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        )
    }) {
        return fail();
    }
    let suffix = text
        .strip_prefix("/nix/store/")
        .ok_or(ErrorCode::NotReady)?;
    let component = suffix.split('/').next().ok_or(ErrorCode::NotReady)?;
    let bytes = component.as_bytes();
    if bytes.len() < 34
        || bytes[32] != b'-'
        || !bytes[..32]
            .iter()
            .all(|byte| b"0123456789abcdfghijklmnpqrsvwxyz".contains(byte))
    {
        return fail();
    }
    Ok(Path::new(&text[..11 + component.len()]))
}
fn immutable(metadata: &Metadata) -> SecurityResult<()> {
    if ![0, crate::pairing::uid()].contains(&metadata.uid())
        || metadata.mode() & 0o222 != 0
        || (!metadata.is_dir() && !metadata.is_file())
    {
        return fail();
    }
    Ok(())
}
fn pin(path: &Path, deadline: Instant) -> SecurityResult<File> {
    store_prefix(path)?;
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| ErrorCode::NotReady)?;
    let file = pin_at(libc::AT_FDCWD, &name, ROOT_RESOLVE, false, deadline).map_err(filesystem)?;
    immutable(&file.metadata().map_err(filesystem)?)?;
    Ok(file)
}
fn bytes(
    path: &Path,
    maximum: u64,
    expected: Option<&str>,
    deadline: Instant,
) -> SecurityResult<Vec<u8>> {
    let file = pin(path, deadline)?;
    let metadata = file.metadata().map_err(filesystem)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return fail();
    }
    let object = object_identity(&file, &metadata, deadline).map_err(filesystem)?;
    let before = stamp(&metadata);
    let reader = readable_object(&file, object, deadline).map_err(filesystem)?;
    let mut data = Vec::with_capacity(metadata.len() as usize);
    reader
        .take(maximum + 1)
        .read_to_end(&mut data)
        .map_err(filesystem)?;
    if data.len() as u64 != metadata.len()
        || file
            .metadata()
            .map_err(filesystem)
            .map(|metadata| stamp(&metadata))?
            != before
    {
        return fail();
    }
    if let Some(expected) = expected
        && (!digest_text(expected) || crate::pairing::hex(&Sha256::digest(&data)) != expected)
    {
        return fail();
    }
    check(deadline).map_err(filesystem)?;
    Ok(data)
}

fn file_digest(file: &File, maximum: u64, deadline: Instant) -> SecurityResult<String> {
    let metadata = file.metadata().map_err(filesystem)?;
    if !metadata.is_file() || metadata.len() > maximum {
        return fail();
    }
    let object = object_identity(file, &metadata, deadline).map_err(filesystem)?;
    let before = stamp(&metadata);
    let mut reader = readable_object(file, object, deadline).map_err(filesystem)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut count = 0u64;
    loop {
        check(deadline).map_err(filesystem)?;
        let size = reader.read(&mut buffer).map_err(filesystem)?;
        if size == 0 {
            break;
        }
        count = count
            .checked_add(size as u64)
            .filter(|count| *count <= metadata.len())
            .ok_or(ErrorCode::NotReady)?;
        hash.update(&buffer[..size]);
    }
    if count != metadata.len() || stamp(&file.metadata().map_err(filesystem)?) != before {
        return fail();
    }
    Ok(crate::pairing::hex(&hash.finalize()))
}

pub(super) struct StoreRoot {
    pub(super) path: PathBuf,
    object: Object,
    stamp: (u64, i64, i64, i64, i64),
}
impl StoreRoot {
    fn capture(path: PathBuf, deadline: Instant) -> SecurityResult<Self> {
        if store_prefix(&path)? != path {
            return fail();
        }
        let file = pin(&path, deadline)?;
        let metadata = file.metadata().map_err(filesystem)?;
        Ok(Self {
            path,
            object: object_identity(&file, &metadata, deadline).map_err(filesystem)?,
            stamp: stamp(&metadata),
        })
    }
    pub(super) fn current(&self, deadline: Instant) -> SecurityResult<File> {
        let file = pin(&self.path, deadline)?;
        let metadata = file.metadata().map_err(filesystem)?;
        if object_identity(&file, &metadata, deadline).map_err(filesystem)? != self.object
            || stamp(&metadata) != self.stamp
        {
            return fail();
        }
        Ok(file)
    }
    pub(super) fn resolve(&self, path: &Path, deadline: Instant) -> SecurityResult<File> {
        let root = self.current(deadline)?;
        let relative = path
            .strip_prefix(&self.path)
            .map_err(|_| ErrorCode::NotReady)?;
        if relative.as_os_str().is_empty() {
            return Ok(root);
        }
        let mut parent = root;
        let mut parts = relative.components().peekable();
        while let Some(part) = parts.next() {
            let std::path::Component::Normal(name) = part else {
                return fail();
            };
            let name = CString::new(name.as_bytes()).map_err(|_| ErrorCode::NotReady)?;
            let file = pin_at(
                parent.as_raw_fd(),
                &name,
                CHILD_RESOLVE,
                parts.peek().is_some(),
                deadline,
            )
            .map_err(filesystem)?;
            let metadata = file.metadata().map_err(filesystem)?;
            immutable(&metadata)?;
            if object_identity(&file, &metadata, deadline)
                .map_err(filesystem)?
                .mount
                != self.object.mount
            {
                return fail();
            }
            parent = file;
        }
        Ok(parent)
    }
}
fn resolve_alias(
    roots: &[StoreRoot],
    path: &Path,
    deadline: Instant,
) -> SecurityResult<Option<File>> {
    let mut path = path.to_owned();
    for _ in 0..32 {
        check(deadline).map_err(filesystem)?;
        if path
            .to_str()
            .is_none_or(|text| text.len() > 4095 || text.chars().any(char::is_control))
        {
            return fail();
        }
        let root = roots
            .iter()
            .find(|root| path.strip_prefix(&root.path).is_ok())
            .ok_or(ErrorCode::NotReady)?;
        let mut parent = root.current(deadline)?;
        let mut current = root.path.clone();
        let mut parts = path
            .strip_prefix(&root.path)
            .map_err(|_| ErrorCode::NotReady)?
            .components();
        let mut alias = None;
        while let Some(part) = parts.next() {
            match part {
                std::path::Component::CurDir => continue,
                std::path::Component::ParentDir => {
                    if current == root.path || !current.pop() {
                        return fail();
                    }
                    parent = root.resolve(&current, deadline)?;
                    continue;
                }
                std::path::Component::Normal(name) => {
                    let name = CString::new(name.as_bytes()).map_err(|_| ErrorCode::NotReady)?;
                    let file =
                        match pin_at(parent.as_raw_fd(), &name, CHILD_RESOLVE, false, deadline) {
                            Ok(file) => file,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                                return Ok(None);
                            }
                            Err(error) => return Err(filesystem(error)),
                        };
                    let metadata = file.metadata().map_err(filesystem)?;
                    if metadata.file_type().is_symlink() {
                        if ![0, crate::pairing::uid()].contains(&metadata.uid()) {
                            return fail();
                        }
                        let mut buffer = [0u8; 4096];
                        let count = unsafe {
                            libc::readlinkat(
                                file.as_raw_fd(),
                                c"".as_ptr(),
                                buffer.as_mut_ptr().cast(),
                                buffer.len(),
                            )
                        };
                        if count <= 0 || count as usize == buffer.len() {
                            return fail();
                        }
                        let target = std::str::from_utf8(&buffer[..count as usize])
                            .map_err(|_| ErrorCode::NotReady)?;
                        let mut target = super::admin::normalized(target, &current)?;
                        for remaining in parts.clone() {
                            target.push(remaining.as_os_str());
                        }
                        alias = Some(target);
                        break;
                    }
                    immutable(&metadata)?;
                    if object_identity(&file, &metadata, deadline)
                        .map_err(filesystem)?
                        .mount
                        != root.object.mount
                    {
                        return fail();
                    }
                    current.push(part.as_os_str());
                    parent = file;
                }
                _ => return fail(),
            }
        }
        if let Some(next) = alias {
            path = next;
        } else {
            return Ok(Some(parent));
        }
    }
    fail()
}

pub(super) struct Image {
    pub(super) path: PathBuf,
    pub(super) file: File,
    pub(super) object: Object,
    stamp: (u64, i64, i64, i64, i64),
}
impl Image {
    fn capture(
        path: PathBuf,
        expected: &str,
        roots: &[StoreRoot],
        deadline: Instant,
    ) -> SecurityResult<Self> {
        if !digest_text(expected) {
            return fail();
        }
        let root = roots
            .iter()
            .find(|root| path.strip_prefix(&root.path).is_ok())
            .ok_or(ErrorCode::NotReady)?;
        let file = root.resolve(&path, deadline)?;
        let metadata = file.metadata().map_err(filesystem)?;
        if !metadata.is_file()
            || metadata.len() > EXECUTABLE_BYTES
            || metadata.mode() & 0o111 == 0
            || metadata.mode() & 0o6000 != 0
        {
            return fail();
        }
        let object = object_identity(&file, &metadata, deadline).map_err(filesystem)?;
        let before = stamp(&metadata);
        if file_digest(&file, EXECUTABLE_BYTES, deadline)? != expected
            || stamp(&file.metadata().map_err(filesystem)?) != before
        {
            return fail();
        }
        Ok(Self {
            path,
            file,
            object,
            stamp: before,
        })
    }
    pub(super) fn current(&self, roots: &[StoreRoot], deadline: Instant) -> SecurityResult<()> {
        let root = roots
            .iter()
            .find(|root| self.path.strip_prefix(&root.path).is_ok())
            .ok_or(ErrorCode::NotReady)?;
        let current = root.resolve(&self.path, deadline)?;
        let metadata = current.metadata().map_err(filesystem)?;
        if object_identity(&current, &metadata, deadline).map_err(filesystem)? != self.object
            || stamp(&metadata) != self.stamp
            || object_identity(
                &self.file,
                &self.file.metadata().map_err(filesystem)?,
                deadline,
            )
            .map_err(filesystem)?
                != self.object
        {
            return fail();
        }
        Ok(())
    }
    fn interpreter(&self, deadline: Instant) -> SecurityResult<Option<PathBuf>> {
        let readable = readable_object(&self.file, self.object, deadline).map_err(filesystem)?;
        let mut header = [0u8; 64];
        readable.read_exact_at(&mut header, 0).map_err(filesystem)?;
        if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 || header[6] != 1 {
            return fail();
        }
        let machine =
            u16::from_le_bytes(header[18..20].try_into().map_err(|_| ErrorCode::NotReady)?);
        let expected = if cfg!(target_arch = "x86_64") {
            62
        } else if cfg!(target_arch = "aarch64") {
            183
        } else {
            return fail();
        };
        if machine != expected || !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3) {
            return fail();
        }
        let start = u64::from_le_bytes(header[32..40].try_into().map_err(|_| ErrorCode::NotReady)?);
        let size = u16::from_le_bytes([header[54], header[55]]) as usize;
        let count = u16::from_le_bytes([header[56], header[57]]) as usize;
        if size != 56
            || count == 0
            || count > 128
            || start
                .checked_add((size * count) as u64)
                .is_none_or(|end| end > self.stamp.0)
        {
            return fail();
        }
        let mut interpreter = None;
        let mut stack = false;
        for index in 0..count {
            check(deadline).map_err(filesystem)?;
            let mut entry = [0u8; 56];
            readable
                .read_exact_at(&mut entry, start + (index * size) as u64)
                .map_err(filesystem)?;
            let kind = u32::from_le_bytes(entry[..4].try_into().map_err(|_| ErrorCode::NotReady)?);
            let flags =
                u32::from_le_bytes(entry[4..8].try_into().map_err(|_| ErrorCode::NotReady)?);
            if kind == 0x6474_e551 {
                if flags & 1 != 0 {
                    return fail();
                }
                stack = true;
            }
            if kind == 3 {
                if interpreter.is_some() {
                    return fail();
                }
                let offset =
                    u64::from_le_bytes(entry[8..16].try_into().map_err(|_| ErrorCode::NotReady)?);
                let length =
                    u64::from_le_bytes(entry[32..40].try_into().map_err(|_| ErrorCode::NotReady)?);
                if !(2..=4096).contains(&length)
                    || offset
                        .checked_add(length)
                        .is_none_or(|end| end > self.stamp.0)
                {
                    return fail();
                }
                let mut buffer = [0u8; 4096];
                readable
                    .read_exact_at(&mut buffer[..length as usize], offset)
                    .map_err(filesystem)?;
                let text = CStr::from_bytes_with_nul(&buffer[..length as usize])
                    .map_err(|_| ErrorCode::NotReady)?
                    .to_str()
                    .map_err(|_| ErrorCode::NotReady)?;
                store_prefix(Path::new(text))?;
                interpreter = Some(PathBuf::from(text));
            }
        }
        if !stack {
            return fail();
        }
        Ok(interpreter)
    }
}

pub(super) struct Runtime {
    pub(super) executable: Image,
    pub(super) interpreter: Image,
    pub(super) roots: Vec<StoreRoot>,
    pub(super) exec_path: PathBuf,
    pub(super) system_config: PathBuf,
    interpreter_request: PathBuf,
}
impl Runtime {
    fn load(manifest: &Path, deadline: Instant) -> SecurityResult<Self> {
        let data = bytes(manifest, MANIFEST_BYTES, None, deadline)?;
        let Value::Object(mut package) = parse_raw_json(&data).map_err(|_| ErrorCode::NotReady)?
        else {
            return fail();
        };
        if package.get("kind").and_then(Value::as_str) != Some("direct-rust")
            || package.get("lockSha256").and_then(Value::as_str)
                != Some(crate::transport::lock_sha256())
        {
            return fail();
        }
        let Some(Value::Object(mut git)) = package.remove("readGit") else {
            return fail();
        };
        let system = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            "x86_64-linux"
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            "aarch64-linux"
        } else {
            return fail();
        };
        if git.get("schema").and_then(Value::as_u64) != Some(1)
            || git.get("available").and_then(Value::as_bool) != Some(true)
            || git.get("system").and_then(Value::as_str) != Some(system)
        {
            return fail();
        }
        let closure = PathBuf::from(field(&mut git, "closureList")?);
        let closure_hash = field(&mut git, "closureSha256")?;
        let registration = PathBuf::from(field(&mut git, "registration")?);
        let registration_hash = field(&mut git, "registrationSha256")?;
        let parent = manifest.parent().ok_or(ErrorCode::NotReady)?;
        if closure != parent.join("runtime-closure.store-paths")
            || registration != parent.join("runtime-closure.registration")
            || !digest_text(&registration_hash)
        {
            return fail();
        }
        let list = bytes(&closure, MANIFEST_BYTES, Some(&closure_hash), deadline)?;
        let text = std::str::from_utf8(&list).map_err(|_| ErrorCode::NotReady)?;
        if !text.ends_with('\n') {
            return fail();
        }
        let mut roots = Vec::new();
        let mut unique = std::collections::HashSet::new();
        for line in text.lines() {
            if roots.len() == MAX_CLOSURE || !unique.insert(line) {
                return fail();
            }
            roots.push(StoreRoot::capture(PathBuf::from(line), deadline)?);
        }
        if roots.is_empty() {
            return fail();
        }
        let registration_file = pin(&registration, deadline)?;
        if file_digest(&registration_file, 8 * MANIFEST_BYTES, deadline)? != registration_hash {
            return fail();
        }
        let executable = Image::capture(
            PathBuf::from(field(&mut git, "executable")?),
            &field(&mut git, "executableSha256")?,
            &roots,
            deadline,
        )?;
        let interpreter = Image::capture(
            PathBuf::from(field(&mut git, "interpreter")?),
            &field(&mut git, "interpreterSha256")?,
            &roots,
            deadline,
        )?;
        if interpreter.interpreter(deadline)?.is_some() {
            return fail();
        }
        let interpreter_request = PathBuf::from(field(&mut git, "interpreterRequest")?);
        if executable.interpreter(deadline)?.as_ref() != Some(&interpreter_request) {
            return fail();
        }
        let actual =
            resolve_alias(&roots, &interpreter_request, deadline)?.ok_or(ErrorCode::NotReady)?;
        if object_identity(&actual, &actual.metadata().map_err(filesystem)?, deadline)
            .map_err(filesystem)?
            != interpreter.object
        {
            return fail();
        }
        let exec_path = PathBuf::from(field(&mut git, "execPath")?);
        let system_config = PathBuf::from(field(&mut git, "systemConfig")?);
        if !system_config.is_absolute()
            || system_config
                .to_str()
                .is_none_or(|value| value.chars().any(char::is_control) || value.len() > 4095)
        {
            return fail();
        }
        let exec_root = roots
            .iter()
            .find(|root| exec_path.strip_prefix(&root.path).is_ok())
            .ok_or(ErrorCode::NotReady)?;
        if !exec_root
            .resolve(&exec_path, deadline)?
            .metadata()
            .map_err(filesystem)?
            .is_dir()
        {
            return fail();
        }
        Ok(Self {
            executable,
            interpreter,
            roots,
            exec_path,
            system_config,
            interpreter_request,
        })
    }
    pub(super) fn current(&self, deadline: Instant) -> SecurityResult<()> {
        self.executable.current(&self.roots, deadline)?;
        self.interpreter.current(&self.roots, deadline)?;
        let current = resolve_alias(&self.roots, &self.interpreter_request, deadline)?
            .ok_or(ErrorCode::NotReady)?;
        if object_identity(&current, &current.metadata().map_err(filesystem)?, deadline)
            .map_err(filesystem)?
            != self.interpreter.object
        {
            return fail();
        }
        Ok(())
    }
    pub(super) fn resolve(&self, path: &Path, deadline: Instant) -> SecurityResult<File> {
        self.roots
            .iter()
            .find(|root| path.strip_prefix(&root.path).is_ok())
            .ok_or(ErrorCode::NotReady)?
            .resolve(path, deadline)
    }
    pub(super) fn request(&self, path: &Path, deadline: Instant) -> SecurityResult<Option<File>> {
        resolve_alias(&self.roots, path, deadline)
    }
}
