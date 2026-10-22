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

// Jarvis User gets the page of the window WITHOUT the license manager: the marked parts are cut out HERE, at build time, so the
// administrator's page and code are not in the customer's program at all (not just hidden at run time).
fn strip_admin(page: &str) -> String {
    let mut out = String::with_capacity(page.len());
    let mut rest = page;
    loop {
        let a = rest.find("<!--ADMIN-BEGIN-->");
        let b = rest.find("/*ADMIN-BEGIN*/");
        let (start, end_marker) = match (a, b) {
            (Some(x), Some(y)) if x < y => (x, "<!--ADMIN-END-->"),
            (Some(x), None) => (x, "<!--ADMIN-END-->"),
            (_, Some(y)) => (y, "/*ADMIN-END*/"),
            (None, None) => break,
        };
        out.push_str(&rest[..start]);
        match rest[start..].find(end_marker) {
            Some(e) => rest = &rest[start + e + end_marker.len()..],
            None => panic!("ui_page.html: an ADMIN-BEGIN marker has no ADMIN-END"),
        }
    }
    out.push_str(rest);
    out
}

fn main() {
    let page_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src").join("ui_page.html");
    println!("cargo:rerun-if-changed={}", page_path.display());
    let page = std::fs::read_to_string(&page_path).unwrap();
    let page = if std::env::var("CARGO_FEATURE_USER_EDITION").is_ok() { strip_admin(&page) } else { page };
    // the themes and the spheres are one piece of code for the window and for the phone page
    let src_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");
    let kit_path = src_dir.join("orbkit.js");
    println!("cargo:rerun-if-changed={}", kit_path.display());
    let kit = std::fs::read_to_string(&kit_path).unwrap();
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    assert!(page.contains("/*ORBKIT*/"), "ui_page.html: the place of the sphere code is gone");
    std::fs::write(out_dir.join("ui_page.html"), page.replace("/*ORBKIT*/", &kit)).unwrap();
    let phone_path = src_dir.join("remote_page.html");
    println!("cargo:rerun-if-changed={}", phone_path.display());
    let phone = std::fs::read_to_string(&phone_path).unwrap();
    assert!(phone.contains("/*ORBKIT*/"), "remote_page.html: the place of the sphere code is gone");
    std::fs::write(out_dir.join("remote_page.html"), phone.replace("/*ORBKIT*/", &kit)).unwrap();
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
