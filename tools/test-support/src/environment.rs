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

const CODEX_EXECUTION_VARIABLES: [&str; 8] = [
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
    "CODEX_PERMISSION_PROFILE",
    "CODEX_NON_INTERACTIVE",
    "CODEX_CI",
    "CODEX_STARTING_DIFF",
    "CODEX_ROLLOUT_TRACE_ROOT",
    "CODEX_THREAD_ID",
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
pub fn pi_environment_child(home: &Path, pi_home: &Path) -> EnvironmentPolicy {
    pi_child(home).set("PI_CODING_AGENT_DIR", pi_home.as_os_str())
}

#[must_use]
pub fn codex_local_child(home: &Path) -> EnvironmentPolicy {
    scrub_codex_execution(pi_child(home))
        .remove("CODEX_API_KEY")
        .remove("OPENAI_API_KEY")
}

#[must_use]
pub fn codex_environment_child(home: &Path, codex_home: &Path) -> EnvironmentPolicy {
    scrub_codex_execution(pi_child(home))
        .remove("OPENAI_API_KEY")
        .set("CODEX_HOME", codex_home.as_os_str())
}

fn scrub_codex_execution(policy: EnvironmentPolicy) -> EnvironmentPolicy {
    let policy = CODEX_EXECUTION_VARIABLES
        .iter()
        .copied()
        .fold(policy, EnvironmentPolicy::remove);
    env::vars_os()
        .filter(|(name, _)| name.to_string_lossy().starts_with("CODEX_NETWORK_"))
        .fold(policy, |policy, (name, _)| policy.remove(name))
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

    #[test]
    fn codex_children_scrub_execution_metadata_and_select_auth_mode() {
        let local = codex_local_child(Path::new(r"C:\wren-home"));
        for variable in CODEX_EXECUTION_VARIABLES {
            assert!(local.removed.contains(OsStr::new(variable)));
        }
        assert!(local.removed.contains(OsStr::new("CODEX_API_KEY")));
        assert!(local.removed.contains(OsStr::new("OPENAI_API_KEY")));

        let environment =
            codex_environment_child(Path::new(r"C:\wren-home"), Path::new(r"C:\codex-home"));
        assert!(!environment.removed.contains(OsStr::new("CODEX_API_KEY")));
        assert_eq!(
            environment.values.get(OsStr::new("CODEX_HOME")),
            Some(&OsString::from(r"C:\codex-home"))
        );
    }
}
