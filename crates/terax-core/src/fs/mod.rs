use anyhow::{Context, Result};
use std::{fs, path::{Path, PathBuf}};
use walkdir::WalkDir;

pub fn read_text_limited(path: &Path, max_bytes: usize) -> Result<String> {
    let meta = fs::metadata(path).with_context(|| format!("metadata {}", path.display()))?;
    if meta.len() as usize > max_bytes {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let slice = &bytes[..max_bytes.min(bytes.len())];
        let mut s = String::from_utf8_lossy(slice).to_string();
        s.push_str(&format!("\n\n[truncated: file is {} bytes, limit is {} bytes]", meta.len(), max_bytes));
        Ok(s)
    } else { fs::read_to_string(path).with_context(|| format!("read text {}", path.display())) }
}

pub fn tree_summary(root: &Path, max_entries: usize) -> String {
    let mut out = String::new();
    for (i, e) in WalkDir::new(root).max_depth(3).into_iter().filter_map(Result::ok).take(max_entries).enumerate() {
        if i > 0 { out.push('\n'); }
        let p = e.path().strip_prefix(root).unwrap_or(e.path());
        out.push_str(&p.display().to_string());
        if e.path().is_dir() { out.push('/'); }
    }
    out
}

pub fn canonical_or_original(path: PathBuf) -> PathBuf { fs::canonicalize(&path).unwrap_or(path) }


pub fn find_project_root(start: &Path) -> PathBuf {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() || cur.join("TERAX.md").exists() || cur.join("Cargo.toml").exists() || cur.join("package.json").exists() {
            return cur;
        }
        if !cur.pop() { return start.to_path_buf(); }
    }
}

pub fn workspace_index(root: &Path, max_entries: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!("Workspace: {}\n", root.display()));
    out.push_str("Files:\n");
    for e in WalkDir::new(root).max_depth(4).into_iter().filter_map(Result::ok).take(max_entries) {
        let p = e.path();
        let rel = p.strip_prefix(root).unwrap_or(p);
        let name = rel.display().to_string();
        if name.contains("node_modules") || name.contains("target/") || name.contains(".git/") { continue; }
        if p.is_dir() { out.push_str(&format!("  {}/\n", name)); }
        else if let Ok(meta)=std::fs::metadata(p) { out.push_str(&format!("  {} ({} bytes)\n", name, meta.len())); }
    }
    out
}
