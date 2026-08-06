//! Native platform glue — no crates.io build deps.
//! - macOS: compile AppKit UI with system clang
//! - Windows: embed app icon + version via system windres

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=native/macos/interpres_gui.m");
    println!("cargo:rerun-if-changed=native/macos/interpres_gui.h");
    println!("cargo:rerun-if-changed=native/windows/interpres.rc");
    println!("cargo:rerun-if-changed=assets/logo-256.png");
    println!("cargo:rerun-if-changed=assets/logo.png");
    println!("cargo:rerun-if-changed=assets/Interpres.ico");

    if target.contains("apple-darwin") || target.contains("apple-ios") {
        build_macos_gui(&manifest, &out_dir);
        return;
    }

    if target.contains("windows") {
        build_windows_resources(&manifest, &out_dir);
    }
}

fn build_macos_gui(manifest: &Path, out_dir: &Path) {
    let src = manifest.join("native/macos/interpres_gui.m");
    let obj = out_dir.join("interpres_gui.o");

    let status = Command::new("clang")
        .args(["-fobjc-arc", "-fmodules", "-O2", "-c", "-o"])
        .arg(&obj)
        .arg(&src)
        .arg("-I")
        .arg(manifest.join("native/macos"))
        .status()
        .expect("failed to spawn clang — install Xcode Command Line Tools");

    if !status.success() {
        panic!("clang failed to compile native/macos/interpres_gui.m");
    }

    println!("cargo:rustc-link-arg={}", obj.display());
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Cocoa");
}

fn build_windows_resources(manifest: &Path, out_dir: &Path) {
    let ico = manifest.join("assets/Interpres.ico");
    // Prefer checked-in ico; otherwise build from logo-256.png (Vista+ PNG-in-ICO).
    if !ico.is_file() {
        let png = if manifest.join("assets/logo-256.png").is_file() {
            manifest.join("assets/logo-256.png")
        } else {
            manifest.join("assets/logo.png")
        };
        if png.is_file() {
            if let Err(e) = write_png_as_ico(&png, &ico) {
                println!("cargo:warning=could not create assets/Interpres.ico from PNG: {e}");
            }
        }
    }

    if !ico.is_file() {
        println!("cargo:warning=no assets/Interpres.ico — Windows exe will have default icon");
        return;
    }

    // Generate .rc with absolute icon path so windres finds it from any cwd.
    let rc_gen = out_dir.join("interpres_icon.rc");
    let ico_escaped = ico.display().to_string().replace('\\', "\\\\");
    let rc_body = format!(
        "/* auto-generated — do not edit */\n\
         1 ICON \"{ico_escaped}\"\n\
         \n\
         1 VERSIONINFO\n\
         FILEVERSION    0,2,0,0\n\
         PRODUCTVERSION 0,2,0,0\n\
         FILEFLAGSMASK  0x3fL\n\
         FILEFLAGS      0x0L\n\
         FILEOS         0x40004L\n\
         FILETYPE       0x1L\n\
         FILESUBTYPE    0x0L\n\
         BEGIN\n\
           BLOCK \"StringFileInfo\"\n\
           BEGIN\n\
             BLOCK \"040904b0\"\n\
             BEGIN\n\
               VALUE \"CompanyName\",      \"Interpres Contributors\"\n\
               VALUE \"FileDescription\",  \"Interpres Live Captions companion\"\n\
               VALUE \"FileVersion\",      \"0.2.0\"\n\
               VALUE \"InternalName\",     \"interpres\"\n\
               VALUE \"LegalCopyright\",   \"MIT OR Apache-2.0\"\n\
               VALUE \"OriginalFilename\", \"interpres.exe\"\n\
               VALUE \"ProductName\",      \"Interpres\"\n\
               VALUE \"ProductVersion\",   \"0.2.0\"\n\
             END\n\
           END\n\
           BLOCK \"VarFileInfo\"\n\
           BEGIN\n\
             VALUE \"Translation\", 0x409, 1200\n\
           END\n\
         END\n"
    );
    fs::write(&rc_gen, rc_body).expect("write generated rc");

    let res_obj = out_dir.join("interpres_res.o");
    let windres = find_windres();
    let status = Command::new(&windres)
        .arg("-i")
        .arg(&rc_gen)
        .arg("-o")
        .arg(&res_obj)
        .arg("--input-format=rc")
        .arg("--output-format=coff")
        .status();

    match status {
        Ok(s) if s.success() && res_obj.is_file() => {
            // Link resource object into the final PE (Explorer / taskbar icon).
            println!("cargo:rustc-link-arg={}", res_obj.display());
            println!("cargo:warning=embedded Windows icon via {}", windres.display());
        }
        Ok(s) => {
            println!(
                "cargo:warning=windres ({}) failed (exit {:?}) — exe icon not embedded",
                windres.display(),
                s.code()
            );
        }
        Err(e) => {
            println!(
                "cargo:warning=windres not run ({e}; looked for {}). Install MinGW windres or set WINDRES=",
                windres.display()
            );
        }
    }
}

fn find_windres() -> PathBuf {
    if let Ok(p) = env::var("WINDRES") {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return pb;
        }
    }
    // Prefer windres next to the C compiler when using gnu toolchain.
    if let Ok(cc) = env::var("CC") {
        let cc_path = PathBuf::from(&cc);
        if let Some(dir) = cc_path.parent() {
            for name in ["windres.exe", "windres"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    // `where windres` (Windows) / `which` not needed if on PATH.
    if let Ok(out) = Command::new("where.exe").arg("windres").output() {
        if out.status.success() {
            if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().next() {
                let p = PathBuf::from(line.trim());
                if p.is_file() {
                    return p;
                }
            }
        }
    }
    // Common WinGet MinGW layouts when PATH is incomplete in cargo's environment.
    if let Ok(local) = env::var("LOCALAPPDATA") {
        let winget = PathBuf::from(local).join("Microsoft\\WinGet\\Packages");
        if let Ok(entries) = fs::read_dir(&winget) {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().to_string();
                if !(name.contains("WinLibs") || name.contains("mingw") || name.contains("MinGW") || name.contains("LLVM-MinGW"))
                {
                    continue;
                }
                for sub in [
                    "mingw64\\bin\\windres.exe",
                    "bin\\windres.exe",
                    // llvm-mingw unpack layout
                ] {
                    let cand = ent.path().join(sub);
                    if cand.is_file() {
                        return cand;
                    }
                }
                // Nested versioned folder (llvm-mingw-*-ucrt-x86_64\bin\windres.exe)
                if let Ok(walk) = fs::read_dir(ent.path()) {
                    for inner in walk.flatten() {
                        let cand = inner.path().join("bin\\windres.exe");
                        if cand.is_file() {
                            return cand;
                        }
                        let cand = inner.path().join("mingw64\\bin\\windres.exe");
                        if cand.is_file() {
                            return cand;
                        }
                    }
                }
            }
        }
    }
    PathBuf::from("windres.exe")
}

/// Write a Vista+ ICO that stores the PNG payload as-is (no crates).
fn write_png_as_ico(png_path: &Path, ico_path: &Path) -> std::io::Result<()> {
    let png = fs::read(png_path)?;
    if png.len() < 24 || !png.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a PNG",
        ));
    }
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());

    // ICONDIR (6) + ICONDIRENTRY (16) + PNG
    let mut out = Vec::with_capacity(6 + 16 + png.len());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type icon
    out.extend_from_slice(&1u16.to_le_bytes()); // count

    // width/height: 0 means 256 in classic ICO
    let wb = if w >= 256 { 0u8 } else { w as u8 };
    let hb = if h >= 256 { 0u8 } else { h as u8 };
    out.push(wb);
    out.push(hb);
    out.push(0); // color count
    out.push(0); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bit count
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes()); // offset = 6+16
    out.extend_from_slice(&png);

    if let Some(parent) = ico_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(ico_path, out)?;
    Ok(())
}
