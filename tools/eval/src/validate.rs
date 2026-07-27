use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use wren_test_support::{
    EnvironmentPolicy, IsolatedWorkspace, ProcessRequest, TreeCleanup,
    artifacts::{atomic_write, atomic_write_json, capture_git, publish_directory, read_json},
    environment::verifier_child,
    resolve_windows_command, run_process,
};

use crate::{
    harness,
    schema::{
        AttemptResult, Classification, EvidenceKind, Failure, HarnessProcess, HarnessSummary,
        Metrics, ReasonCode, RunRecord, SCHEMA_VERSION, Transcript, VerifierResult, classify,
    },
    task::{self, Task},
    verifier::run_exact_verifier,
};

const ARTIFACTS: [&str; 9] = [
    "result.json",
    "prompt.txt",
    "harness.jsonl",
    "harness.stderr.txt",
    "transcript.json",
    "verifier.json",
    "verifier.stdout.txt",
    "verifier.stderr.txt",
    "git-status.txt",
];

#[derive(Clone, Copy)]
enum Actor {
    Pass,
    Unchanged,
    Timeout,
}

impl Actor {
    const fn name(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Unchanged => "unchanged",
            Self::Timeout => "timeout",
        }
    }

    const fn expected(self) -> Classification {
        match self {
            Self::Pass => Classification::Pass,
            Self::Unchanged => Classification::TaskFailure,
            Self::Timeout => Classification::InfrastructureFailure,
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn validate(repository: &Path) -> io::Result<PathBuf> {
    let tasks_root = repository.join("evals/tasks");
    let tasks = task::load_all(&tasks_root)?;
    if tasks.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no evaluation tasks found",
        ));
    }
    let harnesses = harness::load_all(&repository.join("evals/harnesses"))?;
    for loaded in &harnesses {
        let _validated_fields = (
            loaded.config.kind(),
            loaded.config.executable(),
            loaded.config.authentication(),
            loaded.config.summary(loaded.hash.clone()),
        );
    }
    let task = tasks
        .iter()
        .find(|task| task.manifest.id == "exact-file-edit")
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "exact-file-edit task is missing")
        })?;
    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let git = resolve_windows_command(Path::new("git.exe"))?;
    let started = Instant::now();
    let started_unix_ms = unix_millis()?;
    let run_id = unique_id()?;
    let parent = repository.join("target/eval/validation");
    fs::create_dir_all(&parent)?;
    let staging = parent.join(format!(".{run_id}.staging"));
    let output = parent.join(&run_id);
    fs::create_dir(&staging)?;

    let actors = [Actor::Pass, Actor::Unchanged, Actor::Timeout];
    let mut results = Vec::new();
    let mut attempt_paths = Vec::new();
    for (index, actor) in actors.into_iter().enumerate() {
        let attempt_name = format!("attempt-{:03}", index + 1);
        let result = run_actor(
            repository,
            task,
            &executable,
            &git,
            &staging,
            &attempt_name,
            u32::try_from(index + 1).expect("three validation attempts fit u32"),
            actor,
        )?;
        if result.classification != actor.expected() {
            return Err(io::Error::other(format!(
                "validation actor {} produced {:?}, expected {:?}",
                actor.name(),
                result.classification,
                actor.expected()
            )));
        }
        let read_back: AttemptResult = read_json(&staging.join(&attempt_name).join("result.json"))?;
        if read_back != result {
            return Err(io::Error::other("attempt result JSON did not round-trip"));
        }
        results.push(result);
        attempt_paths.push(attempt_name);
    }

    let pass_count = count(&results, Classification::Pass);
    let task_failure_count = count(&results, Classification::TaskFailure);
    let infrastructure_failure_count = count(&results, Classification::InfrastructureFailure);
    let record = RunRecord {
        schema_version: SCHEMA_VERSION,
        run_id,
        evidence_kind: EvidenceKind::EvaluatorValidation,
        started_unix_ms,
        duration_ms: millis(started.elapsed()),
        task_id: task.manifest.id.clone(),
        task_version: task.manifest.version,
        task_manifest_hash: task.manifest_hash.clone(),
        harness: HarnessSummary {
            kind: "validation-actors".to_owned(),
            profile: "credentialless-self-test".to_owned(),
            provider: None,
            model: None,
            reasoning: None,
            authentication: None,
            executable: Some(executable.display().to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            config_hash: None,
            permissions: vec!["isolated test workspace".to_owned()],
            limitations: vec!["no agent or provider was invoked".to_owned()],
        },
        requested_attempts: 3,
        completed_attempts: 3,
        pass_count,
        task_failure_count,
        infrastructure_failure_count,
        observed_pass_rate: f64::from(pass_count) / 3.0,
        metrics: Metrics::default(),
        attempt_paths,
    };
    atomic_write_json(&staging.join("run.json"), &record)?;
    let read_back: RunRecord = read_json(&staging.join("run.json"))?;
    if read_back != record {
        return Err(io::Error::other("run JSON did not round-trip"));
    }
    publish_directory(&staging, &output)?;
    println!(
        "evaluator validation passed: pass={pass_count}, task_failure={task_failure_count}, infrastructure_failure={infrastructure_failure_count}; {}",
        output.display()
    );
    println!("No agent, Pi, Codex, provider, or behavioral claim was tested.");
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn run_actor(
    repository: &Path,
    task: &Task,
    executable: &Path,
    git: &Path,
    run_staging: &Path,
    attempt_name: &str,
    index: u32,
    actor: Actor,
) -> io::Result<AttemptResult> {
    let started = Instant::now();
    let started_unix_ms = unix_millis()?;
    let mut isolated =
        IsolatedWorkspace::create(&repository.join("target/eval/workspaces"), actor.name())?;
    task.prepare_workspace(isolated.workspace())?;
    initialize_git(git, isolated.workspace(), isolated.artifacts())?;

    let attempt_staging = run_staging.join(format!(".{attempt_name}.staging"));
    let attempt_output = run_staging.join(attempt_name);
    fs::create_dir(&attempt_staging)?;
    atomic_write(&attempt_staging.join("prompt.txt"), &task.prompt)?;
    let transcript = Transcript {
        schema_version: SCHEMA_VERSION,
        adapter: format!("validation-actor-{}", actor.name()),
        final_text: None,
        entries: Vec::new(),
        metrics: Metrics::default(),
    };
    atomic_write_json(&attempt_staging.join("transcript.json"), &transcript)?;

    let marker = isolated.root().join("descendant-finished.txt");
    let mut arguments = vec![OsString::from("__actor"), OsString::from(actor.name())];
    arguments.push(isolated.workspace().as_os_str().to_owned());
    arguments.push(marker.as_os_str().to_owned());
    let request = ProcessRequest {
        program: executable.to_owned(),
        arguments,
        working_directory: isolated.workspace().to_owned(),
        stdin: &task.prompt,
        environment: verifier_child(),
        timeout: if matches!(actor, Actor::Timeout) {
            Duration::from_millis(500)
        } else {
            task.timeout()
        },
        stdout_path: attempt_staging.join("harness.jsonl"),
        stderr_path: attempt_staging.join("harness.stderr.txt"),
    };
    let process = run_process(&request)?;
    if matches!(actor, Actor::Timeout) {
        std::thread::sleep(Duration::from_secs(3));
        if marker.exists() {
            return Err(io::Error::other("timed-out validation descendant survived"));
        }
    }

    capture_git(
        git,
        isolated.workspace(),
        &attempt_staging.join("git-status.txt"),
        &attempt_staging.join("git.diff"),
    )?;
    let verifier = run_exact_verifier(
        executable,
        &task.expected(),
        isolated.workspace(),
        &attempt_staging,
    )?;

    let mut infrastructure_failure = process_failure(&process);
    if infrastructure_failure.is_none() {
        infrastructure_failure = verifier_failure(&verifier);
    }
    let verifier_passed = verifier.report.as_ref().map(|report| report.passed);
    isolated.finish().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("validation workspace cleanup failed: {error}"),
        )
    })?;
    let (classification, failure) = classify(infrastructure_failure, verifier_passed);
    let result = AttemptResult {
        schema_version: SCHEMA_VERSION,
        index,
        harness: format!("validation-{}", actor.name()),
        classification,
        failure,
        started_unix_ms,
        duration_ms: millis(started.elapsed()),
        harness_process: HarnessProcess {
            exit_code: process.exit_code,
            timed_out: process.timed_out,
            tree_cleanup: tree_cleanup_name(process.tree_cleanup).to_owned(),
        },
        verifier: VerifierResult {
            exit_code: verifier.exit_code,
            passed: verifier_passed,
        },
        metrics: Metrics::default(),
        artifacts: ARTIFACTS
            .into_iter()
            .chain(["git.diff"])
            .map(str::to_owned)
            .collect(),
    };
    atomic_write_json(&attempt_staging.join("result.json"), &result)?;
    publish_directory(&attempt_staging, &attempt_output)?;
    Ok(result)
}

fn initialize_git(git: &Path, workspace: &Path, artifacts: &Path) -> io::Result<()> {
    for arguments in [
        vec!["init", "--quiet"],
        vec!["add", "--all"],
        vec![
            "-c",
            "user.name=Wren Evaluator",
            "-c",
            "user.email=wren-eval@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "baseline",
        ],
    ] {
        run_checked(
            git,
            arguments.into_iter().map(OsString::from).collect(),
            workspace,
            artifacts,
        )?;
    }
    Ok(())
}

fn run_checked(
    program: &Path,
    arguments: Vec<OsString>,
    working_directory: &Path,
    artifacts: &Path,
) -> io::Result<()> {
    let stdout = artifacts.join("setup.stdout.txt");
    let stderr = artifacts.join("setup.stderr.txt");
    let request = ProcessRequest {
        program: program.to_owned(),
        arguments,
        working_directory: working_directory.to_owned(),
        stdin: &[],
        environment: EnvironmentPolicy::inherit(),
        timeout: Duration::from_secs(30),
        stdout_path: stdout.clone(),
        stderr_path: stderr.clone(),
    };
    let result = run_process(&request)?;
    if result.exit_code != Some(0) || result.timed_out || result.tree_cleanup != TreeCleanup::Clean
    {
        return Err(io::Error::other(format!(
            "setup command failed; see {}",
            stderr.display()
        )));
    }
    let _ = fs::remove_file(stdout);
    let _ = fs::remove_file(stderr);
    Ok(())
}

fn process_failure(process: &wren_test_support::ProcessResult) -> Option<Failure> {
    if process.timed_out {
        Some(failure(ReasonCode::Timeout, "harness timed out"))
    } else if process.tree_cleanup != TreeCleanup::Clean {
        Some(failure(
            ReasonCode::TreeCleanup,
            "harness process tree required forced cleanup",
        ))
    } else if process.exit_code != Some(0) {
        Some(failure(
            ReasonCode::HarnessExit,
            "harness exited unsuccessfully",
        ))
    } else {
        None
    }
}

fn verifier_failure(verifier: &crate::verifier::VerifierExecution) -> Option<Failure> {
    if verifier.timed_out {
        Some(failure(ReasonCode::VerifierTimeout, "verifier timed out"))
    } else if verifier.tree_cleanup != TreeCleanup::Clean || verifier.exit_code != Some(0) {
        Some(failure(
            ReasonCode::VerifierLaunch,
            "verifier process failed",
        ))
    } else if verifier.report.is_none() {
        Some(failure(
            ReasonCode::VerifierProtocol,
            "verifier did not emit a valid report",
        ))
    } else {
        None
    }
}

fn failure(code: ReasonCode, message: &str) -> Failure {
    Failure {
        code,
        message: message.to_owned(),
    }
}

fn count(results: &[AttemptResult], class: Classification) -> u32 {
    u32::try_from(
        results
            .iter()
            .filter(|result| result.classification == class)
            .count(),
    )
    .expect("attempt count fits u32")
}

const fn tree_cleanup_name(cleanup: TreeCleanup) -> &'static str {
    match cleanup {
        TreeCleanup::Clean => "clean",
        TreeCleanup::Terminated => "terminated",
    }
}

fn unique_id() -> io::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    Ok(format!("{nanos}-{}", std::process::id()))
}

fn unix_millis() -> io::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    u64::try_from(millis).map_err(io::Error::other)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
