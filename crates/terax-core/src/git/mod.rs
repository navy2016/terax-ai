use anyhow::{Context, Result};
use std::{path::Path, process::Command};

#[derive(Debug, Clone)]
pub struct GitEntry {
    pub status: String,
    pub path: String,
    pub staged: bool,
}

fn run_git(root: &Path, args: &[&str], max: usize) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(root).output().with_context(|| format!("git {}", args.join(" ")))?;
    let mut s = String::new();
    if !out.stdout.is_empty() { s.push_str(&String::from_utf8_lossy(&out.stdout)); }
    if !out.stderr.is_empty() { s.push_str(&String::from_utf8_lossy(&out.stderr)); }
    if !out.status.success() {
        anyhow::bail!("git {} failed: {}", args.join(" "), s.trim());
    }
    if s.len() > max { s.truncate(max); s.push_str("\n[truncated]"); }
    Ok(s)
}

pub fn status(root: &Path) -> Result<String> { run_git(root, &["status", "--short", "--branch"], 64 * 1024) }
pub fn diff(root: &Path) -> Result<String> { run_git(root, &["diff", "--no-ext-diff"], 128 * 1024) }
pub fn staged_diff(root: &Path) -> Result<String> { run_git(root, &["diff", "--cached", "--no-ext-diff"], 128 * 1024) }

pub fn status_entries(root: &Path) -> Result<Vec<GitEntry>> {
    let s = run_git(root, &["status", "--short"], 64 * 1024)?;
    let mut out = Vec::new();
    for line in s.lines() {
        if line.len() < 3 { continue; }
        let status = line[..2].to_string();
        let path = line[3..].trim().to_string();
        let staged = !status.chars().next().unwrap_or(' ').is_whitespace() && !status.starts_with("??");
        out.push(GitEntry { status, path, staged });
    }
    Ok(out)
}

pub fn diff_path(root: &Path, path: &str) -> Result<String> {
    run_git(root, &["diff", "--no-ext-diff", "--", path], 128 * 1024)
}

pub fn staged_diff_path(root: &Path, path: &str) -> Result<String> {
    run_git(root, &["diff", "--cached", "--no-ext-diff", "--", path], 128 * 1024)
}

pub fn stage(root: &Path, path: &str) -> Result<String> {
    run_git(root, &["add", "--", path], 32 * 1024)
}

pub fn unstage(root: &Path, path: &str) -> Result<String> {
    run_git(root, &["restore", "--staged", "--", path], 32 * 1024)
}

pub fn branch(root: &Path) -> Result<String> { run_git(root, &["branch", "--show-current"], 8 * 1024).map(|s| s.trim().to_string()) }
pub fn remote(root: &Path) -> Result<String> { run_git(root, &["remote", "-v"], 16 * 1024) }
pub fn set_remote_origin(root: &Path, url: &str) -> Result<String> {
    let has = Command::new("git").args(["remote", "get-url", "origin"]).current_dir(root).output().map(|o| o.status.success()).unwrap_or(false);
    if has { run_git(root, &["remote", "set-url", "origin", url], 32 * 1024) } else { run_git(root, &["remote", "add", "origin", url], 32 * 1024) }
}
pub fn commit(root: &Path, message: &str) -> Result<String> { run_git(root, &["commit", "-m", message], 128 * 1024) }
pub fn clone_repo(parent: &Path, url: &str) -> Result<String> { run_git(parent, &["clone", url], 256 * 1024) }
