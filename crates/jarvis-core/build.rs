// The hashes of the command packs of this build (resources/commands): written next to the program, see src/integrity.rs.
// A command file that is not in this list, or differs from it, is not run with the powers it declares.

use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("..").join("..").join("resources").join("commands");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut text = String::new();
    for f in files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        // a file that is only a leftover of an editor is not a command file
        if rel.ends_with('~') || rel.ends_with(".bak") || rel.ends_with(".swp") {
            continue;
        }
        let bytes = std::fs::read(&f).unwrap();
        let mut h = Sha256::new();
        h.update(&bytes);
        let mut hex = String::new();
        for b in h.finalize() {
            write!(hex, "{:02x}", b).unwrap();
        }
        println!("cargo:rerun-if-changed={}", f.display());
        writeln!(text, "{}  {}", hex, rel).unwrap();
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("commands_manifest.txt");
    std::fs::write(out, text).unwrap();
}
