//! Embeds the icon, the manifest and a version resource into the exe.
//!
//! The version block is generated from `Cargo.toml` instead of being written
//! by hand, so `Cargo.toml` stays the only place a release number is bumped.
//! Without it the exe has no version at all: Properties shows nothing and
//! users cannot say which build they run.

use std::fmt::Write as _;

fn main() {
    println!("cargo:rerun-if-changed=rasklad.exe.manifest");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();

    let numbers: Vec<u32> = version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let major = numbers.first().copied().unwrap_or(0);
    let minor = numbers.get(1).copied().unwrap_or(0);
    let patch = numbers.get(2).copied().unwrap_or(0);

    // Forward slashes: the resource compiler treats a backslash inside a
    // string literal as an escape.
    let icon = root.join("assets/icon.ico").display().to_string().replace('\\', "/");
    let manifest = root.join("rasklad.exe.manifest").display().to_string().replace('\\', "/");

    let mut rc = String::new();
    let _ = writeln!(rc, "1 ICON \"{icon}\"");
    let _ = writeln!(rc, "1 RT_MANIFEST \"{manifest}\"");
    let _ = writeln!(rc, "1 VERSIONINFO");
    let _ = writeln!(rc, "FILEVERSION {major},{minor},{patch},0");
    let _ = writeln!(rc, "PRODUCTVERSION {major},{minor},{patch},0");
    let _ = writeln!(rc, "FILEOS 0x40004L");
    let _ = writeln!(rc, "FILETYPE 0x1L");
    let _ = writeln!(rc, "BEGIN");
    let _ = writeln!(rc, "  BLOCK \"StringFileInfo\"");
    let _ = writeln!(rc, "  BEGIN");
    let _ = writeln!(rc, "    BLOCK \"040904B0\"");
    let _ = writeln!(rc, "    BEGIN");
    let _ = writeln!(rc, "      VALUE \"FileDescription\", \"Rasklad - keyboard layout switcher\"");
    let _ = writeln!(rc, "      VALUE \"FileVersion\", \"{version}\"");
    let _ = writeln!(rc, "      VALUE \"InternalName\", \"rasklad\"");
    let _ = writeln!(rc, "      VALUE \"OriginalFilename\", \"rasklad.exe\"");
    let _ = writeln!(rc, "      VALUE \"ProductName\", \"Rasklad\"");
    let _ = writeln!(rc, "      VALUE \"ProductVersion\", \"{version}\"");
    let _ = writeln!(rc, "      VALUE \"LegalCopyright\", \"GPL-3.0\"");
    let _ = writeln!(rc, "    END");
    let _ = writeln!(rc, "  END");
    let _ = writeln!(rc, "  BLOCK \"VarFileInfo\"");
    let _ = writeln!(rc, "  BEGIN");
    let _ = writeln!(rc, "    VALUE \"Translation\", 0x409, 1200");
    let _ = writeln!(rc, "  END");
    let _ = writeln!(rc, "END");

    let rc_path = out.join("resources.rc");
    std::fs::write(&rc_path, rc).expect("write generated rc");
    let _ = embed_resource::compile(&rc_path, embed_resource::NONE);
}
