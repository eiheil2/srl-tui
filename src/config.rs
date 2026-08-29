//! Configuration persistence for the flashcards app.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Application configuration that persists between sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The currently selected theme name.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Maximum new cards introduced per study session.
    #[serde(default = "default_new_per_session")]
    pub new_per_session: u32,
}

fn default_theme() -> String {
    "default".to_string()
}

fn default_new_per_session() -> u32 {
    20
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            new_per_session: default_new_per_session(),
        }
    }
}

impl Config {
    /// Get the default config file path.
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("flashcards")
            .join("config.toml")
    }

    /// Load config from disk, returning default if file doesn't exist.
    pub fn load() -> Result<Self> {
        let path = Self::default_path();

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;

        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse config file")?;

        Ok(config)
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

        let content = toml::to_string_pretty(self).with_context(|| "Failed to serialize config")?;

        fs::write(&path, content)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;

        Ok(())
    }
}
