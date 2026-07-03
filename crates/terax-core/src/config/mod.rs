use anyhow::{Context, Result};
use serde::Deserialize;
use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub default_provider: Option<String>,
    pub providers: Option<std::collections::BTreeMap<String, ProviderConfig>>,
    pub ai: Option<AiOptions>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiOptions {
    pub max_context_bytes: Option<usize>,
    pub auto_include_current_file: Option<bool>,
    pub auto_include_git_status: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Config {
    pub fn load_default() -> Result<Self> {
        let home = env::var("HOME").context("HOME is not set")?;
        let path = PathBuf::from(home).join(".config/terax-tui/config.toml");
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }
    pub fn resolve_provider(&self) -> Result<ResolvedProvider> {
        if let Some(providers) = &self.providers {
            let id = self.default_provider.as_deref().unwrap_or_else(|| providers.keys().next().map(String::as_str).unwrap_or(""));
            let p = providers.get(id).with_context(|| format!("provider not found: {id}"))?;
            let api_key = match (&p.api_key, &p.api_key_env) {
                (Some(v), _) => v.clone(),
                (None, Some(env_name)) => env::var(env_name).with_context(|| format!("env {env_name} is not set"))?,
                _ => anyhow::bail!("provider api_key or api_key_env required"),
            };
            return Ok(ResolvedProvider { base_url: p.base_url.clone(), api_key, model: p.model.clone() });
        }
        Ok(ResolvedProvider {
            base_url: self.base_url.clone().context("base_url missing")?,
            api_key: self.api_key.clone().context("api_key missing")?,
            model: self.model.clone().context("model missing")?,
        })
    }
    pub fn max_context_bytes(&self) -> usize { self.ai.as_ref().and_then(|a| a.max_context_bytes).unwrap_or(64 * 1024) }
}
