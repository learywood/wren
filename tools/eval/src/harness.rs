use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{schema::HarnessSummary, task::hash_bytes};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    Local,
    Environment,
}

impl Authentication {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Environment => "environment",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "harness", rename_all = "snake_case", deny_unknown_fields)]
pub enum HarnessConfig {
    Pi {
        schema_version: u32,
        executable: PathBuf,
        profile: String,
        provider: String,
        model: String,
        reasoning: String,
        tools: Vec<String>,
        authentication: Authentication,
    },
    Codex {
        schema_version: u32,
        executable: PathBuf,
        profile: String,
        provider: String,
        model: String,
        reasoning: String,
        sandbox: String,
        windows_sandbox: String,
        authentication: Authentication,
    },
}

pub struct LoadedHarness {
    pub config: HarnessConfig,
    pub hash: String,
}

impl LoadedHarness {
    pub fn load(path: &Path, expected_kind: &str) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let config: HarnessConfig = toml::from_str(
            str::from_utf8(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        config.validate(expected_kind)?;
        Ok(Self {
            config,
            hash: hash_bytes(&bytes),
        })
    }
}

impl HarnessConfig {
    pub fn validate(&self, expected_kind: &str) -> io::Result<()> {
        let valid = match self {
            Self::Pi {
                schema_version,
                executable,
                profile,
                provider,
                model,
                reasoning,
                tools,
                ..
            } => {
                expected_kind == "pi"
                    && *schema_version == 1
                    && nonempty_path(executable)
                    && nonempty([profile, provider, model, reasoning])
                    && !tools.is_empty()
                    && tools.iter().all(|tool| !tool.trim().is_empty())
            }
            Self::Codex {
                schema_version,
                executable,
                profile,
                provider,
                model,
                reasoning,
                sandbox,
                windows_sandbox,
                ..
            } => {
                expected_kind == "codex"
                    && *schema_version == 1
                    && nonempty_path(executable)
                    && nonempty([
                        profile,
                        provider,
                        model,
                        reasoning,
                        sandbox,
                        windows_sandbox,
                    ])
                    && sandbox == "workspace-write"
                    && windows_sandbox == "unelevated"
            }
        };
        if !valid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid {expected_kind} harness configuration"),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Pi { .. } => "pi",
            Self::Codex { .. } => "codex",
        }
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        match self {
            Self::Pi { executable, .. } | Self::Codex { executable, .. } => executable,
        }
    }

    #[must_use]
    pub const fn authentication(&self) -> Authentication {
        match self {
            Self::Pi { authentication, .. } | Self::Codex { authentication, .. } => *authentication,
        }
    }

    #[must_use]
    pub fn summary(&self, hash: String) -> HarnessSummary {
        match self {
            Self::Pi {
                profile,
                provider,
                model,
                reasoning,
                tools,
                authentication,
                ..
            } => HarnessSummary {
                kind: "pi".to_owned(),
                profile: profile.clone(),
                provider: Some(provider.clone()),
                model: Some(model.clone()),
                reasoning: Some(reasoning.clone()),
                authentication: Some(authentication.name().to_owned()),
                executable: None,
                version: None,
                config_hash: Some(hash),
                permissions: vec![format!("built-in tools: {}", tools.join(","))],
                limitations: vec![
                    "OAuth profile may apply user-global retry and transport settings".to_owned(),
                ],
            },
            Self::Codex {
                profile,
                provider,
                model,
                reasoning,
                sandbox,
                windows_sandbox,
                authentication,
                ..
            } => HarnessSummary {
                kind: "codex".to_owned(),
                profile: profile.clone(),
                provider: Some(provider.clone()),
                model: Some(model.clone()),
                reasoning: Some(reasoning.clone()),
                authentication: Some(authentication.name().to_owned()),
                executable: None,
                version: None,
                config_hash: Some(hash),
                permissions: vec![
                    format!("sandbox: {sandbox}"),
                    format!("Windows sandbox: {windows_sandbox}"),
                    "no built-in tool allowlist".to_owned(),
                ],
                limitations: vec![
                    "skill discovery cannot be fully disabled".to_owned(),
                    "startup network and user-home state updates cannot be fully disabled"
                        .to_owned(),
                    "tool surface is not identical to Pi read/write".to_owned(),
                ],
            },
        }
    }
}

fn nonempty<'a>(values: impl IntoIterator<Item = &'a String>) -> bool {
    values.into_iter().all(|value| !value.trim().is_empty())
}

fn nonempty_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
}

pub fn load_all(root: &Path) -> io::Result<Vec<LoadedHarness>> {
    ["pi", "codex"]
        .into_iter()
        .map(|kind| LoadedHarness::load(&root.join(format!("{kind}.toml")), kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_harnesses_are_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("evals/harnesses");
        let harnesses = load_all(&root).unwrap();
        assert_eq!(harnesses.len(), 2);
        assert_eq!(harnesses[0].hash.len(), 64);
    }
}
