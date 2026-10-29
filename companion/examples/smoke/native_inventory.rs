// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared examples-only actual APK inspection; never a JNI/device/provider or delivery pass.
use goatr::{pairing::{hex,private_dir,uid},protocol::Uuid};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use std::{collections::BTreeSet,fs::{self,File},io::{self,Read},os::unix::fs::{DirBuilderExt,MetadataExt},path::{Path,PathBuf},process::{Command as Process,Stdio}};

pub(crate) struct InventoryInputs {
    pub apk:PathBuf,
    pub native:PathBuf,
    pub jna:PathBuf,
    pub camera:PathBuf,
    pub graphics:PathBuf,
    pub sdk:PathBuf,
    pub retained_source:PathBuf,
    pub scratch_parent:PathBuf,
}
fn fail(message:&str)->io::Error {io::Error::other(message)}
fn sdk_tool(root:&Path,name:&str)->PathBuf {root.join("build-tools/36.0.0").join(name)}

fn sha(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex(&digest)
}
fn bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut b = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut b)?;
    if b.len() as u64 > limit {
        return Err(fail("artifact exceeds inventory bound"));
    }
    Ok(b)
}
fn command(mut c: Process, limit: u64) -> io::Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    // SAFETY: the hook only clears capabilities with async-signal-safe syscalls.
    unsafe {
        use std::os::unix::process::CommandExt;
        c.pre_exec(goatr::codex_relay::drop_inherited_capabilities);
    }
    let mut child = c.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| fail("inspection stdout missing"))?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        child.kill()?;
        child.wait()?;
        return Err(fail("inspection output exceeded bound"));
    }
    if !child.wait()?.success() {
        return Err(fail("required native inspection tool failed"));
    }
    Ok(bytes)
}
fn member(archive: &Path, name: &str) -> io::Result<Vec<u8>> {
    let mut c = Process::new("unzip");
    c.arg("-p").arg(archive).arg(name);
    command(c, 64 * 1024 * 1024)
}
fn entries(archive: &Path) -> io::Result<Vec<String>> {
    let mut c = Process::new("unzip");
    c.arg("-Z1").arg(archive);
    let text = String::from_utf8(command(c, 4 * 1024 * 1024)?).map_err(io::Error::other)?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for line in text.lines() {
        if line.contains("..")
            || line.starts_with('/')
            || line.chars().any(char::is_control)
            || !seen.insert(line.to_owned())
        {
            return Err(fail("unsafe/duplicate archive member"));
        }
        result.push(line.to_owned());
    }
    Ok(result)
}
fn n16(b: &[u8], o: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2)
            .ok_or_else(|| fail("truncated ELF"))?
            .try_into()
            .map_err(io::Error::other)?,
    ))
}
fn n32(b: &[u8], o: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .ok_or_else(|| fail("truncated ELF"))?
            .try_into()
            .map_err(io::Error::other)?,
    ))
}
fn n64(b: &[u8], o: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(o..o + 8)
            .ok_or_else(|| fail("truncated ELF"))?
            .try_into()
            .map_err(io::Error::other)?,
    ))
}
pub(crate) fn elf(bytes: &[u8], abi: &str) -> io::Result<Value> {
    if bytes.get(..6) != Some(b"\x7fELF\x02\x01")
        || n16(bytes, 16)? != 3
        || n16(bytes, 18)? != if abi == "arm64-v8a" { 183 } else { 62 }
    {
        return Err(fail(
            "native member is not the intended ELF64 little-endian DYN ABI",
        ));
    }
    let ph = usize::try_from(n64(bytes, 32)?).map_err(io::Error::other)?;
    let size = usize::from(n16(bytes, 54)?);
    let count = usize::from(n16(bytes, 56)?);
    if size < 56 || count > 128 {
        return Err(fail("unsupported ELF program header bounds"));
    }
    let table = count
        .checked_mul(size)
        .and_then(|n| ph.checked_add(n))
        .ok_or_else(|| fail("ELF program table overflow"))?;
    if table > bytes.len() {
        return Err(fail("truncated ELF program table"));
    }
    let mut loads = Vec::new();
    for i in 0..count {
        let p = ph + i * size;
        if n32(bytes, p)? == 1 {
            let offset = n64(bytes, p + 8)?;
            let address = n64(bytes, p + 16)?;
            let alignment = n64(bytes, p + 48)?;
            if alignment < 16384
                || !alignment.is_power_of_two()
                || offset % 16384 != address % 16384
            {
                return Err(fail("ELF LOAD segment fails 16KiB alignment/congruence"));
            }
            loads.push(json!({"offset":offset,"vaddr":address,"alignment":alignment}));
        }
    }
    if loads.is_empty() {
        return Err(fail("ELF has no LOAD segments"));
    }
    Ok(json!({"abi":abi,"sha256":sha(bytes),"bytes":bytes.len(),"loads":loads}))
}
fn graph(manifest: &Value) -> io::Result<Vec<Value>> {
    let mut shared = Vec::new();
    for p in manifest["packages"]
        .as_array()
        .ok_or_else(|| fail("dependency packages absent"))?
    {
        let name = p["name"]
            .as_str()
            .ok_or_else(|| fail("dependency name absent"))?;
        if ["iroh", "iroh-base", "iroh-relay", "rustls"].contains(&name) {
            let version = p["version"]
                .as_str()
                .ok_or_else(|| fail("dependency version absent"))?;
            if version != if name == "rustls" { "0.23.45" } else { "1.3.0" } {
                return Err(fail("duplicate/insecure shared dependency selection"));
            }
            if p["source"].as_str() != Some("registry+https://github.com/rust-lang/crates.io-index")
                || p["checksum"].as_str().is_none_or(|s| {
                    s.len() != 64
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                })
            {
                return Err(fail("shared source/checksum identity is null or invalid"));
            }
            shared.push(json!({"name":name,"version":version,"source":p["source"],"checksum":p["checksum"]}));
        }
    }
    shared.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    if shared.len() != 4 {
        return Err(fail("exact four shared provenance selections required"));
    }
    Ok(shared)
}
struct Scratch(PathBuf, u64, u64);
impl Scratch {
    fn new(parent:&Path) -> io::Result<Self> {
        let p = parent.join(format!("goatr-native-{}", Uuid::random().0));
        fs::DirBuilder::new().mode(0o700).create(&p)?;
        private_dir(&p, false)?;
        let m = fs::metadata(&p)?;
        Ok(Self(p, m.dev(), m.ino()))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.0)
            && m.uid() == uid()
            && m.dev() == self.1
            && m.ino() == self.2
            && m.mode() & 0o7777 == 0o700
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
pub(crate) fn inspect(inputs:&InventoryInputs)->io::Result<Value> {
    for path in [&inputs.apk,&inputs.native,&inputs.jna,&inputs.camera,&inputs.graphics,&inputs.sdk,&inputs.retained_source,&inputs.scratch_parent] {
        if !path.is_absolute()||fs::canonicalize(path)?!=*path {return Err(fail("inventory inputs must be explicit canonical paths"));}
    }
    let apk=inputs.apk.as_path();let native=inputs.native.as_path();let jna=inputs.jna.as_path();
    let camera=inputs.camera.as_path();let graphics=inputs.graphics.as_path();let sdk=inputs.sdk.as_path();
    let apk_entries = entries(&apk)?;
    let names = [
        "libandroidx.graphics.path.so",
        "libimage_processing_util_jni.so",
        "libiroh_ffi.so",
        "libjnidispatch.so",
        "libsurface_util_jni.so",
    ];
    let abis = ["arm64-v8a", "x86_64"];
    let expected: BTreeSet<_> = abis
        .iter()
        .flat_map(|a| names.iter().map(move |n| format!("lib/{a}/{n}")))
        .collect();
    let actual: BTreeSet<_> = apk_entries
        .iter()
        .filter(|p| !p.ends_with('/') && (p.starts_with("lib/") || p.ends_with(".so")))
        .cloned()
        .collect();
    if actual != expected {
        return Err(fail(
            "APK native inventory differs from ALL ten intended entries; extras are not ignored",
        ));
    }
    if apk_entries.iter().any(|p| {
        p.ends_with(".jar")
            || p.ends_with(".dll")
            || p.ends_with(".dylib")
            || p.contains("linux-x86")
    }) {
        return Err(fail("desktop/JNA JAR/native resource packaged into APK"));
    }
    let jna_entries = entries(&jna)?;
    if jna_entries.iter().any(|p| {
        p.ends_with(".dll") || p.ends_with(".dylib") || p.contains("linux-") || p.contains("win32-")
    }) {
        return Err(fail("JNA AAR unexpectedly contains desktop native payload"));
    }
    if sha(&bounded(&jna, 128 * 1024 * 1024)?)
        != "b57125cb7d16253f0d65a80f7d3a4c3664effa711b8bdbb7f87fb572ce1624ed"
    {
        return Err(fail(
            "JNA AAR differs from retained verified Maven archive identity",
        ));
    }
    let mut listing = Process::new("unzip");
    listing.arg("-lv").arg(&apk);
    let listing =
        String::from_utf8(command(listing, 4 * 1024 * 1024)?).map_err(io::Error::other)?;
    for entry in &actual {
        let row = listing
            .lines()
            .find(|line| line.split_whitespace().last() == Some(entry.as_str()))
            .ok_or_else(|| fail("APK native ZIP method unavailable"))?;
        if row.split_whitespace().nth(1) != Some("Stored") {
            return Err(fail(
                "all ten native APK members must be uncompressed Stored, never ignored by zipalign",
            ));
        }
    }
    if sha(&bounded(&camera, 128 * 1024 * 1024)?)
        != "a36f1c323851b51215ba476640e1eb4153882b0c368c0376e9fb905e689df84a"
        || sha(&bounded(&graphics, 128 * 1024 * 1024)?)
            != "8ca4032b6d79b351f0b59ad4b580eddbb9423e1652f7c958830687f1eee2ec03"
    {
        return Err(fail(
            "Maven JNI AAR differs from retained publisher source-input identity",
        ));
    }
    let manifest: Value =
        serde_json::from_slice(&bounded(&native.join("manifest.json"), 1024 * 1024)?)
            .map_err(io::Error::other)?;
    let companion: Value =
        serde_json::from_slice(include_bytes!("../../dependencies.json")).map_err(io::Error::other)?;
    let ffi: Value = serde_json::from_slice(&bounded(
        &native.join("goatr-dependencies.json"),
        1024 * 1024,
    )?)
    .map_err(io::Error::other)?;
    let shared = graph(&companion)?;
    if shared != graph(&ffi)?
        || shared
            != manifest["sharedProvenance"]
                .as_array()
                .ok_or_else(|| fail("FFI shared provenance absent"))?
                .clone()
    {
        return Err(fail(
            "independent locks differ in exact four shared transport identities",
        ));
    }
    let retained_source: Value = serde_json::from_slice(&bounded(
        &inputs.retained_source,
        1024 * 1024,
    )?)
    .map_err(io::Error::other)?;
    if manifest["identity"] != retained_source {
        return Err(fail(
            "native producer source/patch/lock/context/advisory identity differs from retained repository source",
        ));
    }
    if sha(include_bytes!("../../Cargo.lock")) != companion["lockSha256"].as_str().unwrap_or("")
        || sha(&bounded(&native.join("Cargo.lock"), 1024 * 1024)?)
            != manifest["identity"]["lockSha256"].as_str().unwrap_or("")
    {
        return Err(fail("actual retained lock SHA mismatch"));
    }
    for (name, digest) in manifest["artifacts"]
        .as_object()
        .ok_or_else(|| fail("FFI artifact identities absent"))?
    {
        if sha(&bounded(&native.join(name), 64 * 1024 * 1024)?) != digest.as_str().unwrap_or("") {
            return Err(fail(
                "generated API/context/native producer source-copy digest mismatch",
            ));
        }
    }
    let strip = sdk.join("ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip");
    let scratch = Scratch::new(&inputs.scratch_parent)?;
    let mut members = Vec::new();
    for abi in abis {
        for name in names {
            let entry = format!("lib/{abi}/{name}");
            let bytes = member(&apk, &entry)?;
            let inspected = elf(&bytes, abi)?;
            let (raw, provenance) = match name {
                "libiroh_ffi.so" => (
                    bounded(
                        &native.join(format!("jniLibs/{abi}/{name}")),
                        64 * 1024 * 1024,
                    )?,
                    json!({"producer":native,"identity":manifest["identity"]}),
                ),
                "libjnidispatch.so" => (
                    member(&jna, &format!("jni/{abi}/{name}"))?,
                    json!({"coordinate":"net.java.dev.jna:jna:5.19.1@aar","archive":jna,"archiveSha256":sha(&bounded(&jna,128*1024*1024)?)}),
                ),
                _ => {
                    let archive = if name == "libandroidx.graphics.path.so" {
                        &graphics
                    } else {
                        &camera
                    };
                    (
                        member(archive, &format!("jni/{abi}/{name}"))?,
                        json!({"coordinate":if name=="libandroidx.graphics.path.so"{"androidx.graphics:graphics-path:1.0.1"}else{"androidx.camera:camera-core:1.6.1"},"archive":archive,"publisher":"separate Maven JNI upstream build/source-input manifests retained in packaged notices; not FFI attestation"}),
                    )
                }
            };
            let inspection = scratch.0.join(format!("inspect-{abi}-{name}"));
            fs::write(&inspection, &bytes)?;
            let mut readelf =
                Process::new(sdk.join(
                    "ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf",
                ));
            readelf.args(["-W", "-d"]).arg(&inspection);
            let dynamic = String::from_utf8(command(readelf, 65536)?).map_err(io::Error::other)?;
            let mut needed = Vec::new();
            for line in dynamic.lines().filter(|s| s.contains("(NEEDED)")) {
                let library = line
                    .rsplit_once('[')
                    .and_then(|(_, v)| v.strip_suffix(']'))
                    .ok_or_else(|| fail("native DT_NEEDED representation unavailable"))?;
                if ![
                    "libandroid.so",
                    "libc.so",
                    "libdl.so",
                    "libjnigraphics.so",
                    "liblog.so",
                    "libm.so",
                ]
                .contains(&library)
                    && !names.contains(&library)
                {
                    return Err(fail(
                        "unexpected native application/platform DT_NEEDED dependency",
                    ));
                }
                needed.push(library.to_owned());
            }
            let inspected = json!({"elf":inspected,"needed":needed,"inspectionTool":"pinned NDK28.2 llvm-readelf -W -d"});
            let transform = if name == "libiroh_ffi.so" || name == "libjnidispatch.so" {
                let path = scratch.0.join(format!("{abi}-{name}"));
                fs::write(&path, &raw)?;
                let mut cmd = Process::new(&strip);
                cmd.arg("--strip-unneeded").arg(&path);
                command(cmd, 4096)?;
                if bounded(&path, 64 * 1024 * 1024)? != bytes {
                    return Err(fail(
                        "actual APK differs from pinned NDK28.2 strip-unneeded transform of raw producer bytes",
                    ));
                }
                Some(json!({"tool":strip,"argument":"--strip-unneeded","rawSha256":sha(&raw)}))
            } else {
                if raw != bytes {
                    return Err(fail("actual Maven JNI member source-copy mismatch"));
                }
                None
            };
            members.push(
                json!({"entry":entry,"elf":inspected,"source":provenance,"transform":transform}),
            );
        }
    }
    let mut zipalign = Process::new(sdk_tool(sdk,"zipalign"));
    zipalign.args(["-c", "-P", "16", "-v", "4"]).arg(&apk);
    command(zipalign, 128 * 1024)?;
    let mut signer = Process::new(sdk_tool(sdk,"apksigner"));
    signer
        .args(["verify", "--verbose", "--print-certs"])
        .arg(&apk);
    let signature = String::from_utf8(command(signer, 65536)?).map_err(io::Error::other)?;
    if !signature.contains("CN=Android Debug") {
        return Err(fail(
            "checkpoint APK does not have the disclosed Android debug signer",
        ));
    }
    let notices = String::from_utf8(member(&apk, "assets/android-third-party-notices.txt")?)
        .map_err(io::Error::other)?;
    for required in [
        "987b9ac",
        "afae031e",
        "15251192",
        "11766943",
        "8a05a22",
        "Redistribution and use in source and binary forms",
        "PATENTS",
        "GPL",
        "Apache",
    ] {
        if !notices.contains(required) {
            return Err(fail(
                "packaged JNI publisher/source/libyuv/license notice chain incomplete",
            ));
        }
    }
    Ok(
        json!({"gate":"native-inventory","status":"pass","apk":apk,"apkSha256":sha(&bounded(&apk,512*1024*1024)?),"members":members,"count":10,"zip":"actual zipalign -c -P16 -v4","sharedProvenance":shared,"companionFeatures":companion["directDependencies"],"advisoryEvidence":companion["advisories"],"sqlite":companion["sqlite"],"signer":signature,"noticesSha256":sha(notices.as_bytes()),"limits":["Static source/ELF/ZIP/debug-signer gate only; no JNI load or real16KiB/device/provider pass","Retained advisory DB omits both required GHSAs; exact secure selections checked, full audits remain coordinator evidence","Publisher source-input manifests are not native byte reproduction or cryptographic DSSE/GPL distribution certification"]}),
    )
}
