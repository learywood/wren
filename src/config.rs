use std::{
    collections::{BTreeMap, BTreeSet},
    env, fmt, fs, io,
    path::PathBuf,
};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LoadMode {
    Auto,
    Manual,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    extensions: ExtensionConfig,
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let path = config_path()?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(ConfigError::new(format!(
                    "could not read {}: {error}",
                    path.display()
                )));
            }
        };

        toml::from_str(&text).map_err(|error| {
            ConfigError::new(format!("could not parse {}: {error}", path.display()))
        })
    }

    pub fn requested_extensions(&self) -> impl Iterator<Item = &str> {
        self.extensions.load.iter().map(String::as_str)
    }

    pub fn mode_overrides(&self) -> impl Iterator<Item = (&str, LoadMode)> {
        self.extensions
            .overrides
            .iter()
            .filter_map(|(name, value)| value.mode.map(|mode| (name.as_str(), mode)))
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut unique = BTreeSet::new();
        for name in &self.extensions.load {
            validate_extension_id(name)?;
            if !unique.insert(name) {
                return Err(ConfigError::new(format!(
                    "extension {name:?} is requested more than once"
                )));
            }
        }
        for name in self.extensions.overrides.keys() {
            validate_extension_id(name)?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
struct ExtensionConfig {
    #[serde(default)]
    load: Vec<String>,
    #[serde(flatten)]
    overrides: BTreeMap<String, ExtensionOverride>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionOverride {
    mode: Option<LoadMode>,
}

fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(root) = env::var_os("WREN_HOME") {
        if root.is_empty() {
            return Err(ConfigError::new("WREN_HOME is empty"));
        }
        return Ok(PathBuf::from(root).join("config.toml"));
    }

    let home = env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConfigError::new("USERPROFILE is not set"))?;
    Ok(PathBuf::from(home).join(".wren").join("config.toml"))
}

pub fn validate_extension_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty() {
        return Err(ConfigError::new("an extension ID was empty"));
    }
    if id == "." || id == ".." || id.contains(['/', '\\']) {
        return Err(ConfigError::new(format!(
            "extension ID {id:?} is not a single path component"
        )));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}
