//! The configuration schema, its defaults, and loading.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use deskhand_policy::Override;
use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;

pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-5-5";
pub const DEFAULT_MAX_TOKENS: u32 = 4096;
pub const FILE_NAME: &str = "config.toml";

/// Written when no config file exists. Must parse to [`Config::default`]
/// (enforced by a test).
pub const TEMPLATE: &str = r#"# DeskHand configuration. Changes apply as soon as you save this file.
# If the file has an error, DeskHand keeps the last working settings and
# shows the error in Settings.

# Shortcut that shows and hides the panel.
# Modifiers: Ctrl, Alt, Shift, Win (at least one of Ctrl, Alt, or Win).
# Keys: A-Z, 0-9, F1-F24, Space.
hotkey = "Ctrl+Alt+D"

[provider]
kind = "anthropic"
model = "claude-sonnet-5-5"
max_tokens = 4096

# To use a local or OpenAI-compatible server instead, replace the
# [provider] section above with:
#
# [provider]
# kind = "openai_compatible"
# base_url = "http://localhost:11434/v1"   # Ollama
# model = "qwen3"
# max_tokens = 4096
# thinking = false   # Qwen3-family models: answer without a reasoning phase

# Per-tool overrides. "ask" requires your confirmation each time; "deny"
# disables the tool. Overrides can only make a tool stricter.
[tools]
# move_window = "ask"
# focus_window = "deny"

# Folders DeskHand may search and work in (open, create folders, move,
# recycle). File tools refuse anything outside them. Empty means Desktop,
# Documents, Downloads, Pictures, Music, and Videos. Use absolute paths, or ~
# for your profile folder, e.g. roots = ["~/Projects", "D:/Archive"]
[search]
roots = []

# Compare mode sends the same question and context to a second model and
# shows both answers side by side. Tools are not used in compare mode.
# Uncomment and adjust, for example for OpenAI (key goes in Settings):
#
# [compare]
# kind = "openai_compatible"
# base_url = "https://api.openai.com/v1"
# model = "<an OpenAI model name>"
#
# or for a local model through Ollama (no key needed):
#
# [compare]
# kind = "openai_compatible"
# base_url = "http://localhost:11434/v1"
# model = "qwen3"
# thinking = false   # Qwen3-family models: answer without a reasoning phase
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[serde(with = "hotkey_serde")]
    pub hotkey: Hotkey,
    pub provider: ProviderConfig,
    /// Tool name to override. Names not registered by any tool are kept
    /// (a plugin may register them later) and reported as warnings.
    pub tools: BTreeMap<String, Override>,
    pub search: SearchConfig,
    /// A second model for compare mode. None disables compare.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare: Option<ProviderConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    /// Absolute paths, or `~`-prefixed paths under the profile folder. Empty
    /// means the standard user folders. Resolved by the app, which knows the
    /// profile folder.
    pub roots: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Alt+D".parse().expect("default hotkey is valid"),
            provider: ProviderConfig::default(),
            tools: BTreeMap::new(),
            search: SearchConfig::default(),
            compare: None,
        }
    }
}

mod hotkey_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::hotkey::Hotkey;

    pub fn serialize<S: Serializer>(h: &Hotkey, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(h)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Hotkey, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderConfig {
    #[serde(rename_all = "snake_case")]
    Anthropic {
        #[serde(default = "default_anthropic_model")]
        model: String,
        /// Overrides `https://api.anthropic.com`, e.g. for a proxy. The API
        /// key is sent to this URL.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
    },
    #[serde(rename_all = "snake_case")]
    OpenaiCompatible {
        /// Including the version path, e.g. `http://localhost:11434/v1`.
        base_url: String,
        model: String,
        #[serde(default = "default_max_tokens")]
        max_tokens: u32,
        /// Turns the model's reasoning phase on or off per request, for
        /// servers that read `chat_template_kwargs.enable_thinking`
        /// (llama.cpp, vLLM) with models that support it (Qwen3 family).
        /// Unset sends nothing, so other APIs never see the parameter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking: Option<bool>,
    },
}

fn default_anthropic_model() -> String {
    DEFAULT_ANTHROPIC_MODEL.into()
}

fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::Anthropic {
            model: default_anthropic_model(),
            base_url: None,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

impl ProviderConfig {
    pub fn model(&self) -> &str {
        match self {
            Self::Anthropic { model, .. } | Self::OpenaiCompatible { model, .. } => model,
        }
    }

    fn validate(&self) -> Result<(), String> {
        let (model, base_url, max_tokens) = match self {
            Self::Anthropic {
                model,
                base_url,
                max_tokens,
            } => (model, base_url.as_deref(), *max_tokens),
            Self::OpenaiCompatible {
                model,
                base_url,
                max_tokens,
                ..
            } => (model, Some(base_url.as_str()), *max_tokens),
        };
        if model.trim().is_empty() {
            return Err("provider.model is empty".into());
        }
        if let Some(url) = base_url
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(format!(
                "provider.base_url `{url}` must start with https:// or http://"
            ));
        }
        if max_tokens == 0 {
            return Err("provider.max_tokens must be at least 1".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Io { path: PathBuf, message: String },
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Invalid(m) => write!(f, "config error: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn io(path: &Path, e: std::io::Error) -> ConfigError {
    ConfigError::Io {
        path: path.to_owned(),
        message: e.to_string(),
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(text)
            .map_err(|e| ConfigError::Invalid(e.to_string().trim().to_owned()))?;
        config.provider.validate().map_err(ConfigError::Invalid)?;
        if let Some(compare) = &config.compare {
            compare.validate().map_err(|e| {
                ConfigError::Invalid(format!("[compare] {}", e.replace("provider.", "")))
            })?;
        }
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|e| io(path, e))?;
        Self::parse(&text)
    }

    /// Loads `path`, first writing [`TEMPLATE`] there if no file exists.
    /// An existing file is never overwritten, even if it is invalid.
    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
            }
            fs::write(path, TEMPLATE).map_err(|e| io(path, e))?;
        }
        Self::load(path)
    }

    /// Override entries naming tools that are not in `known`.
    pub fn unknown_tools<'a>(&'a self, known: &[String]) -> Vec<&'a str> {
        self.tools
            .keys()
            .filter(|t| !known.contains(t))
            .map(String::as_str)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_the_default() {
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
    }

    #[test]
    fn empty_file_is_the_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn parses_openai_compatible_and_overrides() {
        let c = Config::parse(
            r#"
            hotkey = "Win+Shift+Space"
            [provider]
            kind = "openai_compatible"
            base_url = "http://localhost:11434/v1"
            model = "qwen3"
            [tools]
            move_window = "ask"
            focus_window = "deny"
            "#,
        )
        .unwrap();
        assert_eq!(c.hotkey.to_string(), "Shift+Win+Space");
        assert_eq!(
            c.provider,
            ProviderConfig::OpenaiCompatible {
                base_url: "http://localhost:11434/v1".into(),
                model: "qwen3".into(),
                max_tokens: DEFAULT_MAX_TOKENS,
                thinking: None,
            }
        );
        assert_eq!(c.tools["move_window"], Override::Ask);
        assert_eq!(c.tools["focus_window"], Override::Deny);
    }

    #[test]
    fn anthropic_fields_default() {
        let c = Config::parse("[provider]\nkind = \"anthropic\"\n").unwrap();
        assert_eq!(c.provider, ProviderConfig::default());
    }

    #[test]
    fn rejects_invalid_values_with_useful_messages() {
        let cases = [
            ("hotkey = \"D\"", "Ctrl, Alt, or Win"),
            ("colour = \"blue\"", "unknown field"),
            ("[tools]\nmove_window = \"allow\"", "unknown variant"),
            ("[provider]\nkind = \"gemini\"", "unknown variant"),
            (
                "[provider]\nkind = \"openai_compatible\"\nmodel = \"x\"",
                "base_url",
            ),
            (
                "[provider]\nkind = \"openai_compatible\"\nbase_url = \"localhost:11434\"\nmodel = \"x\"",
                "must start with",
            ),
            (
                "[provider]\nkind = \"anthropic\"\nmodel = \" \"",
                "model is empty",
            ),
            (
                "[provider]\nkind = \"anthropic\"\nmax_tokens = 0",
                "max_tokens",
            ),
            ("hotkey = ", "hotkey"),
        ];
        for (text, needle) in cases {
            let err = Config::parse(text).unwrap_err().to_string();
            assert!(
                err.contains(needle),
                "`{text}` gave `{err}`, expected `{needle}`"
            );
        }
    }

    #[test]
    fn parses_thinking_and_rejects_unknown_provider_keys() {
        let c = Config::parse(
            "[compare]\nkind = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:8090/v1\"\nmodel = \"q\"\nthinking = false",
        )
        .unwrap();
        assert!(matches!(
            c.compare,
            Some(ProviderConfig::OpenaiCompatible {
                thinking: Some(false),
                ..
            })
        ));
        // A typo is an error, not a silently ignored setting.
        let err = Config::parse(
            "[compare]\nkind = \"openai_compatible\"\nbase_url = \"http://x/v1\"\nmodel = \"q\"\nthinkng = false",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("thinkng"), "{err}");
        // Anthropic has no such setting.
        assert!(Config::parse("[provider]\nkind = \"anthropic\"\nthinking = false").is_err());
    }

    #[test]
    fn parses_and_validates_compare() {
        let c = Config::parse(
            "[compare]\nkind = \"openai_compatible\"\nbase_url = \"http://localhost:11434/v1\"\nmodel = \"qwen3\"",
        )
        .unwrap();
        assert_eq!(c.compare.unwrap().model(), "qwen3");
        let err = Config::parse("[compare]\nkind = \"anthropic\"\nmodel = \"\"")
            .unwrap_err()
            .to_string();
        assert!(err.contains("[compare] model is empty"), "{err}");
    }

    #[test]
    fn parses_search_roots() {
        let c = Config::parse("[search]\nroots = [\"~/Projects\", \"D:/Archive\"]").unwrap();
        assert_eq!(c.search.roots, ["~/Projects", "D:/Archive"]);
        assert!(Config::parse("[search]\nfolders = []").is_err());
    }

    #[test]
    fn reports_unknown_tool_overrides() {
        let c = Config::parse("[tools]\nmove_window = \"ask\"\nmove_windw = \"deny\"").unwrap();
        let known = vec!["move_window".to_owned()];
        assert_eq!(c.unknown_tools(&known), ["move_windw"]);
    }

    #[test]
    fn load_or_create_writes_template_once_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(FILE_NAME);

        assert_eq!(Config::load_or_create(&path).unwrap(), Config::default());
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);

        fs::write(&path, "hotkey = \"oops\"").unwrap();
        assert!(Config::load_or_create(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "hotkey = \"oops\"");
    }
}
