use std::{ffi::OsString, io};

use wren_test_support::{
    EnvironmentPolicy, IsolatedWorkspace,
    environment::{pi_child, pi_environment_child},
};

use crate::{
    harness::{Authentication, HarnessConfig},
    pi_json,
    schema::Transcript,
};

pub fn arguments(config: &HarnessConfig) -> io::Result<Vec<OsString>> {
    let HarnessConfig::Pi {
        provider,
        model,
        reasoning,
        tools,
        ..
    } = config
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected Pi config",
        ));
    };
    Ok(vec![
        "--mode".into(),
        "json".into(),
        "--offline".into(),
        "--provider".into(),
        provider.into(),
        "--model".into(),
        model.into(),
        "--models".into(),
        format!("{provider}/{model}").into(),
        "--thinking".into(),
        reasoning.into(),
        "--no-session".into(),
        "--tools".into(),
        tools.join(",").into(),
        "--no-extensions".into(),
        "--no-skills".into(),
        "--no-prompt-templates".into(),
        "--no-themes".into(),
        "--no-context-files".into(),
        "--no-approve".into(),
    ])
}

#[must_use]
pub fn environment(config: &HarnessConfig, workspace: &IsolatedWorkspace) -> EnvironmentPolicy {
    match config.authentication() {
        Authentication::Local => pi_child(workspace.wren_home()),
        Authentication::Environment => {
            pi_environment_child(workspace.wren_home(), workspace.harness_home())
        }
    }
}

pub fn normalize(bytes: &[u8]) -> io::Result<Transcript> {
    pi_json::normalize(bytes)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::harness::LoadedHarness;

    #[test]
    fn fixed_arguments_pin_resources_and_exclude_prompt() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let loaded = LoadedHarness::load(&root.join("evals/harnesses/pi.toml"), "pi").unwrap();
        let arguments = arguments(&loaded.config).unwrap();
        let rendered = arguments
            .iter()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("--no-session")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("--no-context-files")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("read,write")));
        assert!(
            !rendered
                .iter()
                .any(|argument| argument.contains("release_channel"))
        );
    }
}
