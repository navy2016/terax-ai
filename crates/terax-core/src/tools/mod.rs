use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolKind {
    ReadFile,
    WriteFile,
    EditFile,
    GitStatus,
    GitDiff,
    TerminalTail,
    TerminalSend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    pub id: String,
    pub kind: ToolKind,
    pub summary: String,
    pub payload: String,
    pub risk: ToolRisk,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub ok: bool,
    pub output: String,
}

impl ToolRequest {
    pub fn terminal_send(cmd: impl Into<String>) -> Self {
        let cmd = cmd.into();
        let risk = classify_command_risk(&cmd);
        Self {
            id: format!("terminal-send-{}", now_millis()),
            kind: ToolKind::TerminalSend,
            summary: format!("Run shell command: {cmd}"),
            payload: cmd,
            risk,
        }
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn classify_command_risk(cmd: &str) -> ToolRisk {
    let c = cmd.to_lowercase();
    let high = ["rm -rf", "mkfs", "dd ", ":(){", "chmod -r 777", "curl ", "wget "];
    if high.iter().any(|p| c.contains(p)) && (c.contains("| sh") || c.contains("| bash") || c.contains("rm -rf") || c.contains("mkfs") || c.contains("dd ")) {
        return ToolRisk::High;
    }
    let medium = ["rm ", "mv ", "chmod ", "chown ", "git push", "git reset", "git clean", "apk add", "npm install", "cargo install"];
    if medium.iter().any(|p| c.contains(p)) { ToolRisk::Medium } else { ToolRisk::Low }
}
