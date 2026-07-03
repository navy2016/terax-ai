use anyhow::{Context, Result};
use std::{path::Path, process::Command};

fn run_git(root: &Path, args: &[&str], max: usize) -> Result<String> {
    let out = Command::new("git").args(args).current_dir(root).output().with_context(|| format!("git {}", args.join(" ")))?;
    let mut s = String::new();
    if !out.stdout.is_empty() { s.push_str(&String::from_utf8_lossy(&out.stdout)); }
    if !out.stderr.is_empty() { s.push_str(&String::from_utf8_lossy(&out.stderr)); }
    if s.len() > max { s.truncate(max); s.push_str("\n[truncated]"); }
    Ok(s)
}

pub fn status(root: &Path) -> Result<String> { run_git(root, &["status", "--short", "--branch"], 64 * 1024) }
pub fn diff(root: &Path) -> Result<String> { run_git(root, &["diff", "--no-ext-diff"], 128 * 1024) }
pub fn staged_diff(root: &Path) -> Result<String> { run_git(root, &["diff", "--cached", "--no-ext-diff"], 128 * 1024) }
