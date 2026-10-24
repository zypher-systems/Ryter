//! Embeds the bench's fixtures: each task's `task.toml`, its project
//! (`repo/`) and its hidden tests (`hidden/`). The reference solutions are
//! never embedded. The hidden tests are deflated and masked before they go
//! in, so the binary, which the sandbox lets a model read, does not show
//! them to `grep -a` or `strings`; the runner unmasks them when it scores.
//! That is a cover, not a secret: a model that knew the format could still
//! undo it, and the brief gives it no reason to look.
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The mask the runner takes off again (`task::unmask`).
const MASK: u8 = 0x5A;

const SKIP: &[&str] = &[
    "__pycache__",
    "target",
    "node_modules",
    ".DS_Store",
    "Cargo.lock",
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map(|r| r.flatten().collect())
        .unwrap_or_default();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        if SKIP.contains(&name.to_string_lossy().as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let bench = manifest
        .join("../../bench")
        .canonicalize()
        .expect("the bench directory");
    println!("cargo:rerun-if-changed={}", bench.display());
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let mut code = format!(
        "pub static SCOPE: &str = include_str!({:?});\n",
        bench.join("scope.txt").display().to_string()
    );
    code.push_str("pub static FILES: &[(&str, &[u8], bool)] = &[\n");
    let mut tasks: Vec<_> = fs::read_dir(&bench)
        .expect("read the bench directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("task.toml").is_file())
        .collect();
    tasks.sort();
    for task in tasks {
        let name = task.file_name().unwrap().to_string_lossy().into_owned();
        let mut files = vec![task.join("task.toml")];
        for part in ["repo", "hidden"] {
            walk(&task.join(part), &mut files);
        }
        for file in files {
            let rel = file
                .strip_prefix(&task)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let source = if rel.starts_with("hidden/") {
                masked(&file, &out_dir.join("hidden").join(&name).join(&rel))
            } else {
                file.clone()
            };
            code.push_str(&format!(
                "    ({:?}, include_bytes!({:?}), {}),\n",
                format!("{name}/{rel}"),
                source.display().to_string(),
                executable(&file)
            ));
        }
    }
    code.push_str("];\n");
    fs::write(out_dir.join("fixtures.rs"), code).expect("write fixtures.rs");
}

/// `file` deflated and masked, written under `OUT_DIR`; its path.
fn masked(file: &Path, into: &Path) -> PathBuf {
    let bytes = fs::read(file).expect("read a hidden test");
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&bytes).expect("deflate a hidden test");
    let deflated = encoder.finish().expect("deflate a hidden test");
    let covered: Vec<u8> = deflated.iter().map(|b| b ^ MASK).collect();
    fs::create_dir_all(into.parent().expect("a parent")).expect("OUT_DIR/hidden");
    fs::write(into, covered).expect("write a masked hidden test");
    into.to_path_buf()
}
