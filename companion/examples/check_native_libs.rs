// SPDX-License-Identifier: GPL-3.0-or-later
//! Static actual-APK gate. JNI loading and real 16KiB devices remain separate runtime gates.
#[path = "smoke/native_inventory.rs"]
mod native_inventory;
use clap::{Arg, Command, value_parser};
use serde_json::{Value, json};
use std::{fs,io,path::PathBuf};
fn fail(s: &str) -> io::Error {
    io::Error::other(s)
}
fn cached(group: &str, name: &str, version: &str) -> io::Result<PathBuf> {
    let home = std::env::var_os("GRADLE_USER_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".gradle")
        });
    let base = home
        .join("caches/modules-2/files-2.1")
        .join(group)
        .join(name)
        .join(version);
    let mut found = Vec::new();
    for dir in fs::read_dir(&base)? {
        let p = dir?.path().join(format!("{name}-{version}.aar"));
        if p.is_file() {
            found.push(p);
        }
    }
    if found.len() != 1 {
        return Err(fail(
            "exact resolved Maven AAR unavailable/ambiguous; pass its explicit --*-aar path",
        ));
    }
    Ok(found.remove(0))
}
fn run() -> io::Result<Value> {
    let args=Command::new("check_native_libs").about("Actual APK all-ten ELF/ZIP/source-transform/provenance/debug-signer inventory; no device pass").arg(Arg::new("apk").required(true).value_parser(value_parser!(PathBuf))).args(["native-output","jna-aar","camera-aar","graphics-aar"].map(|n|Arg::new(n).long(n).value_parser(value_parser!(PathBuf)))).get_matches();
    let apk = fs::canonicalize(args.get_one::<PathBuf>("apk").expect("required APK"))?;
    let native = args
        .get_one::<PathBuf>("native-output")
        .cloned()
        .or_else(|| std::env::var_os("GOATR_IROH_ANDROID").map(PathBuf::from))
        .ok_or_else(|| fail("GOATR_IROH_ANDROID actual source-built native output required"))?;
    let jna = args
        .get_one::<PathBuf>("jna-aar")
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| cached("net.java.dev.jna", "jna", "5.19.1"))?;
    let camera = args
        .get_one::<PathBuf>("camera-aar")
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| cached("androidx.camera", "camera-core", "1.6.1"))?;
    let graphics = args
        .get_one::<PathBuf>("graphics-aar")
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| cached("androidx.graphics", "graphics-path", "1.0.1"))?;
    let sdk=std::env::var_os("ANDROID_HOME").or_else(||std::env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from).ok_or_else(||fail("pinned Android SDK unavailable"))?;
    native_inventory::inspect(&native_inventory::InventoryInputs {
        apk,native:fs::canonicalize(native)?,jna:fs::canonicalize(jna)?,camera:fs::canonicalize(camera)?,
        graphics:fs::canonicalize(graphics)?,sdk:fs::canonicalize(sdk)?,
        retained_source:fs::canonicalize("third_party/iroh-ffi/source.json")?,scratch_parent:fs::canonicalize(std::env::temp_dir())?,
    })
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            println!(
                "{}",
                json!({"gate":"native-inventory","status":"blocked","reason":e.to_string()})
            );
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn elf_alignment_rejects_wrong_machine_truncation_and_subpage_load() {
        let mut b = vec![0u8; 120];
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        b[16..18].copy_from_slice(&3u16.to_le_bytes());
        b[18..20].copy_from_slice(&62u16.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&1u32.to_le_bytes());
        b[112..120].copy_from_slice(&16384u64.to_le_bytes());
        assert!(native_inventory::elf(&b, "x86_64").is_ok());
        assert!(native_inventory::elf(&b, "arm64-v8a").is_err());
        b[112..120].copy_from_slice(&4096u64.to_le_bytes());
        assert!(native_inventory::elf(&b, "x86_64").is_err());
        assert!(native_inventory::elf(&b[..80], "x86_64").is_err());
    }
}
