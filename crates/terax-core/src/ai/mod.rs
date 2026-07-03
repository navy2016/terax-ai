use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage { pub role: String, pub content: String }
#[derive(Serialize)]
struct ChatRequest { model: String, messages: Vec<ChatMessage>, stream: bool }
#[derive(Deserialize)]
struct ChatResponse { choices: Vec<ChatChoice> }
#[derive(Deserialize)]
struct ChatChoice { message: ChatMessage }

pub fn chat(messages: Vec<ChatMessage>) -> Result<String> {
    let cfg = Config::load_default()?;
    let provider = cfg.resolve_provider()?;
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    let req = ChatRequest { model: provider.model, messages, stream: false };
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(180)).build()?;
    let res = client.post(url).bearer_auth(provider.api_key).json(&req).send().context("AI request failed")?;
    let status = res.status();
    let body = res.text().context("read AI response")?;
    if !status.is_success() { return Err(anyhow!("AI HTTP {}: {}", status, body)); }
    let parsed: ChatResponse = serde_json::from_str(&body).context("parse AI response")?;
    parsed.choices.into_iter().next().map(|c| c.message.content).ok_or_else(|| anyhow!("AI response has no choices"))
}

pub fn user(content: impl Into<String>) -> ChatMessage { ChatMessage { role: "user".into(), content: content.into() } }
pub fn assistant(content: impl Into<String>) -> ChatMessage { ChatMessage { role: "assistant".into(), content: content.into() } }
pub fn system(content: impl Into<String>) -> ChatMessage { ChatMessage { role: "system".into(), content: content.into() } }
