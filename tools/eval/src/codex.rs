use std::{env, ffi::OsString, io, path::Path};

use wren_test_support::{
    EnvironmentPolicy, IsolatedWorkspace,
    environment::{codex_environment_child, codex_local_child},
};

use crate::{
    codex_json,
    harness::{Authentication, HarnessConfig},
    schema::Transcript,
};

const CONFIG_PINS: [&str; 15] = [
    "approval_policy=\"never\"",
    "project_doc_max_bytes=0",
    "web_search=\"disabled\"",
    "check_for_update_on_startup=false",
    "analytics.enabled=false",
    "otel.exporter=\"none\"",
    "otel.metrics_exporter=\"none\"",
    "otel.trace_exporter=\"none\"",
    "features.apps=false",
    "features.remote_plugin=false",
    "features.hooks=false",
    "features.memories=false",
    "features.goals=false",
    "features.multi_agent=false",
    "features.shell_snapshot=false",
];

pub fn arguments(config: &HarnessConfig, workspace: &Path) -> io::Result<Vec<OsString>> {
    let HarnessConfig::Codex {
        provider,
        model,
        reasoning,
        sandbox,
        windows_sandbox,
        ..
    } = config
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected Codex config",
        ));
    };
    let mut arguments = vec![
        "exec".into(),
        "--json".into(),
        "--ephemeral".into(),
        "--ignore-user-config".into(),
        "--ignore-rules".into(),
        "--strict-config".into(),
        "-C".into(),
        workspace.as_os_str().to_owned(),
        "--sandbox".into(),
        sandbox.as_str().into(),
        "-m".into(),
        model.as_str().into(),
        "-c".into(),
        format!("model_provider={provider:?}").into(),
        "-c".into(),
        format!("model_reasoning_effort={reasoning:?}").into(),
        "-c".into(),
        format!("windows.sandbox={windows_sandbox:?}").into(),
    ];
    for pin in CONFIG_PINS {
        arguments.push("-c".into());
        arguments.push(pin.into());
    }
    arguments.push("-".into());
    Ok(arguments)
}

pub fn environment(
    config: &HarnessConfig,
    workspace: &IsolatedWorkspace,
) -> io::Result<EnvironmentPolicy> {
    match config.authentication() {
        Authentication::Local => Ok(codex_local_child(workspace.wren_home())),
        Authentication::Environment => {
            if env::var_os("CODEX_API_KEY").is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "CODEX_API_KEY is required for Codex environment authentication",
                ));
            }
            Ok(codex_environment_child(
                workspace.wren_home(),
                workspace.harness_home(),
            ))
        }
    }
}

pub fn normalize(bytes: &[u8]) -> io::Result<Transcript> {
    codex_json::normalize(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::LoadedHarness;

    #[test]
    fn fixed_arguments_pin_non_elevated_ephemeral_execution_without_prompt() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let loaded =
            LoadedHarness::load(&root.join("evals/harnesses/codex.toml"), "codex").unwrap();
        let arguments = arguments(&loaded.config, Path::new(r"C:\workspace")).unwrap();
        let rendered = arguments
            .iter()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("--ephemeral")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed("--strict-config")));
        assert!(rendered.contains(&std::borrow::Cow::Borrowed(
            "windows.sandbox=\"unelevated\""
        )));
        assert_eq!(rendered.last(), Some(&std::borrow::Cow::Borrowed("-")));
        assert!(
            !rendered
                .iter()
                .any(|argument| argument.contains("release_channel"))
        );
    }
}
