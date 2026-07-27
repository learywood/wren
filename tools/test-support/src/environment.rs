use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{OsStr, OsString},
    path::Path,
    process::Command,
};

const PI_SESSION_VARIABLES: [&str; 6] = [
    "PI_CODING_AGENT",
    "PI_SESSION_ID",
    "PI_SESSION_FILE",
    "PI_PROVIDER",
    "PI_MODEL",
    "PI_REASONING_LEVEL",
];

/// An explicit policy for the environment inherited by a child process.
pub struct EnvironmentPolicy {
    clear: bool,
    values: BTreeMap<OsString, OsString>,
    removed: BTreeSet<OsString>,
}

impl EnvironmentPolicy {
    #[must_use]
    pub const fn inherit() -> Self {
        Self {
            clear: false,
            values: BTreeMap::new(),
            removed: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self {
            clear: true,
            values: BTreeMap::new(),
            removed: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn set(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        let name = name.into();
        self.removed.remove(&name);
        self.values.insert(name, value.into());
        self
    }

    #[must_use]
    pub fn remove(mut self, name: impl Into<OsString>) -> Self {
        let name = name.into();
        self.values.remove(&name);
        self.removed.insert(name);
        self
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        if self.clear {
            command.env_clear();
        }
        command.envs(&self.values);
        for name in &self.removed {
            command.env_remove(name);
        }
    }
}

#[must_use]
pub fn wren_child(home: &Path) -> EnvironmentPolicy {
    EnvironmentPolicy::inherit().set("WREN_HOME", home.as_os_str())
}

#[must_use]
pub fn pi_child(home: &Path) -> EnvironmentPolicy {
    PI_SESSION_VARIABLES
        .iter()
        .copied()
        .fold(wren_child(home), EnvironmentPolicy::remove)
}

#[must_use]
pub fn verifier_child() -> EnvironmentPolicy {
    ["SystemRoot", "WINDIR", "TEMP", "TMP"]
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|value| (name, value)))
        .fold(EnvironmentPolicy::empty(), |policy, (name, value)| {
            policy.set(name, value)
        })
}

#[must_use]
pub fn has_variable(policy: &EnvironmentPolicy, name: &OsStr) -> bool {
    policy.values.contains_key(name) && !policy.removed.contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_children_remove_parent_session_metadata() {
        let policy = pi_child(Path::new(r"C:\isolated-home"));
        for variable in PI_SESSION_VARIABLES {
            assert!(policy.removed.contains(OsStr::new(variable)));
        }
        assert_eq!(
            policy.values.get(OsStr::new("WREN_HOME")),
            Some(&OsString::from(r"C:\isolated-home"))
        );
    }

    #[test]
    fn verifier_environment_is_cleared_and_contains_no_provider_variables() {
        let policy = verifier_child();
        assert!(policy.clear);
        assert!(!has_variable(&policy, OsStr::new("OPENAI_API_KEY")));
        assert!(!has_variable(&policy, OsStr::new("PI_CODING_AGENT_DIR")));
    }
}
