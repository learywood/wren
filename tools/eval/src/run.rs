use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use wren_test_support::{
    EnvironmentPolicy, IsolatedWorkspace, ProcessRequest, TreeCleanup,
    artifacts::{atomic_write, atomic_write_json, capture_git, publish_directory},
    resolve_windows_command, run_process,
};

use crate::{
    harness::{HarnessConfig, LoadedHarness},
    pi,
    schema::{
        AttemptResult, Classification, EvidenceKind, Failure, HarnessProcess, Metrics, ReasonCode,
        RunRecord, SCHEMA_VERSION, Transcript, VerifierResult, classify,
    },
    task::Task,
    validate::{
        initialize_git, millis, process_failure, tree_cleanup_name, unique_id, unix_millis,
        verifier_failure,
    },
    verifier::{VerifierExecution, run_exact_verifier},
};

const ATTEMPT_ARTIFACTS: [&str; 10] = [
    "result.json",
    "prompt.txt",
    "harness.jsonl",
    "harness.stderr.txt",
    "transcript.json",
    "verifier.json",
    "verifier.stdout.txt",
    "verifier.stderr.txt",
    "git-status.txt",
    "git.diff",
];

pub struct RunOptions {
    pub harness_kind: String,
    pub task_id: String,
    pub attempts: u32,
    pub config: Option<PathBuf>,
    pub output: Option<PathBuf>,
}

pub fn run(repository: &Path, options: &RunOptions) -> io::Result<bool> {
    if options.attempts == 0 || options.harness_kind != "pi" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "run requires pi and a positive attempt count",
        ));
    }
    let task = Task::load(&repository.join("evals/tasks"), &options.task_id)?;
    let config_path = options.config.clone().unwrap_or_else(|| {
        repository
            .join("evals/harnesses")
            .join(format!("{}.toml", options.harness_kind))
    });
    let loaded = LoadedHarness::load(&config_path, &options.harness_kind)?;
    let executable = resolve_windows_command(loaded.config.executable())?;
    let git = resolve_windows_command(Path::new("git.exe"))?;
    let started = Instant::now();
    let started_unix_ms = unix_millis()?;
    let run_id = unique_id()?;
    let output = options
        .output
        .clone()
        .unwrap_or_else(|| repository.join("target/eval/results").join(&run_id));
    if output.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("output already exists: {}", output.display()),
        ));
    }
    let parent = output
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output has no parent"))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".{run_id}.staging"));
    fs::create_dir(&staging)?;
    let version = harness_version(&executable, &staging)?;

    let mut results = Vec::new();
    let mut attempt_paths = Vec::new();
    for index in 1..=options.attempts {
        let attempt_name = format!("attempt-{index:03}");
        let result = run_attempt(
            repository,
            &task,
            &loaded.config,
            &executable,
            &git,
            &staging,
            &attempt_name,
            index,
        )?;
        results.push(result);
        attempt_paths.push(attempt_name);
    }

    let pass_count = count(&results, Classification::Pass);
    let task_failure_count = count(&results, Classification::TaskFailure);
    let infrastructure_failure_count = count(&results, Classification::InfrastructureFailure);
    let mut summary = loaded.config.summary(loaded.hash);
    summary.executable = Some(executable.display().to_string());
    summary.version = Some(version);
    let record = RunRecord {
        schema_version: SCHEMA_VERSION,
        run_id,
        evidence_kind: EvidenceKind::BehavioralEvaluation,
        started_unix_ms,
        duration_ms: millis(started.elapsed()),
        task_id: task.manifest.id.clone(),
        task_version: task.manifest.version,
        task_manifest_hash: task.manifest_hash,
        harness: summary,
        requested_attempts: options.attempts,
        completed_attempts: u32::try_from(results.len()).expect("attempt count fits u32"),
        pass_count,
        task_failure_count,
        infrastructure_failure_count,
        observed_pass_rate: f64::from(pass_count) / f64::from(options.attempts),
        metrics: aggregate_metrics(&results),
        attempt_paths,
    };
    atomic_write_json(&staging.join("run.json"), &record)?;
    publish_directory(&staging, &output)?;
    println!(
        "evaluation complete: pass={pass_count}, task_failure={task_failure_count}, infrastructure_failure={infrastructure_failure_count}; {}",
        output.display()
    );
    Ok(pass_count == options.attempts)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_attempt(
    repository: &Path,
    task: &Task,
    config: &HarnessConfig,
    executable: &Path,
    git: &Path,
    run_staging: &Path,
    attempt_name: &str,
    index: u32,
) -> io::Result<AttemptResult> {
    let started = Instant::now();
    let started_unix_ms = unix_millis()?;
    let mut isolated =
        IsolatedWorkspace::create(&repository.join("target/eval/workspaces"), config.kind())?;
    task.prepare_workspace(isolated.workspace())?;
    initialize_git(git, isolated.workspace(), isolated.artifacts())?;
    let attempt_staging = run_staging.join(format!(".{attempt_name}.staging"));
    let attempt_output = run_staging.join(attempt_name);
    fs::create_dir(&attempt_staging)?;
    atomic_write(&attempt_staging.join("prompt.txt"), &task.prompt)?;

    let arguments = pi::arguments(config)?;
    let environment = Ok::<_, io::Error>(pi::environment(config, &isolated));
    let stdout_path = attempt_staging.join("harness.jsonl");
    let stderr_path = attempt_staging.join("harness.stderr.txt");
    let (mut infrastructure_failure, process) = match environment {
        Ok(environment) => {
            let request = ProcessRequest {
                program: executable.to_owned(),
                arguments,
                working_directory: isolated.workspace().to_owned(),
                stdin: &task.prompt,
                environment,
                timeout: task.timeout(),
                stdout_path: stdout_path.clone(),
                stderr_path,
            };
            match run_process(&request) {
                Ok(result) => (process_failure(&result), Some(result)),
                Err(error) => (
                    Some(failure(
                        ReasonCode::ProcessFailure,
                        format!("harness process failed: {error}"),
                    )),
                    None,
                ),
            }
        }
        Err(error) => {
            fs::write(&stdout_path, [])?;
            fs::write(&stderr_path, [])?;
            (Some(failure(ReasonCode::Setup, error.to_string())), None)
        }
    };

    let normalized = if infrastructure_failure.is_none() {
        let bytes = fs::read(&stdout_path)?;
        let result = pi::normalize(&bytes);
        match result {
            Ok(transcript) => Some(transcript),
            Err(error) => {
                infrastructure_failure = Some(failure(
                    ReasonCode::ProtocolParse,
                    format!("harness protocol could not be normalized: {error}"),
                ));
                None
            }
        }
    } else {
        None
    };
    let transcript = normalized.unwrap_or_else(|| empty_transcript(config.kind()));
    atomic_write_json(&attempt_staging.join("transcript.json"), &transcript)?;

    capture_git(
        git,
        isolated.workspace(),
        &attempt_staging.join("git-status.txt"),
        &attempt_staging.join("git.diff"),
    )?;
    let verifier = match run_exact_verifier(
        &fs::canonicalize(std::env::current_exe()?)?,
        &task.expected(),
        isolated.workspace(),
        &attempt_staging,
    ) {
        Ok(verifier) => verifier,
        Err(error) => {
            if infrastructure_failure.is_none() {
                infrastructure_failure = Some(failure(
                    ReasonCode::VerifierLaunch,
                    format!("verifier failed: {error}"),
                ));
            }
            VerifierExecution {
                exit_code: None,
                timed_out: false,
                tree_cleanup: TreeCleanup::Clean,
                report: None,
            }
        }
    };
    if infrastructure_failure.is_none() {
        infrastructure_failure = verifier_failure(&verifier);
    }
    let verifier_passed = verifier.report.as_ref().map(|report| report.passed);
    if let Err(error) = isolated.finish() {
        infrastructure_failure = Some(failure(
            ReasonCode::Cleanup,
            format!("workspace cleanup failed: {error}"),
        ));
    }
    let (classification, failure) = classify(infrastructure_failure, verifier_passed);
    let harness_process = process.as_ref().map_or_else(
        || HarnessProcess {
            exit_code: None,
            timed_out: false,
            tree_cleanup: "not_started".to_owned(),
        },
        |result| HarnessProcess {
            exit_code: result.exit_code,
            timed_out: result.timed_out,
            tree_cleanup: tree_cleanup_name(result.tree_cleanup).to_owned(),
        },
    );
    let result = AttemptResult {
        schema_version: SCHEMA_VERSION,
        index,
        harness: config.kind().to_owned(),
        classification,
        failure,
        started_unix_ms,
        duration_ms: millis(started.elapsed()),
        harness_process,
        verifier: VerifierResult {
            exit_code: verifier.exit_code,
            passed: verifier_passed,
        },
        metrics: transcript.metrics,
        artifacts: ATTEMPT_ARTIFACTS.into_iter().map(str::to_owned).collect(),
    };
    atomic_write_json(&attempt_staging.join("result.json"), &result)?;
    publish_directory(&attempt_staging, &attempt_output)?;
    Ok(result)
}

fn harness_version(executable: &Path, staging: &Path) -> io::Result<String> {
    let stdout = staging.join("version.stdout.txt");
    let stderr = staging.join("version.stderr.txt");
    let request = ProcessRequest {
        program: executable.to_owned(),
        arguments: vec!["--version".into()],
        working_directory: staging.to_owned(),
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
            "could not query harness version; see {}",
            stderr.display()
        )));
    }
    let version = fs::read_to_string(&stdout)?.trim().to_owned();
    if version.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "harness version output is empty",
        ));
    }
    fs::remove_file(stdout)?;
    fs::remove_file(stderr)?;
    Ok(version)
}

fn empty_transcript(adapter: &str) -> Transcript {
    Transcript {
        schema_version: SCHEMA_VERSION,
        adapter: adapter.to_owned(),
        final_text: None,
        entries: Vec::new(),
        metrics: Metrics::default(),
    }
}

const fn failure(code: ReasonCode, message: String) -> Failure {
    Failure { code, message }
}

fn count(results: &[AttemptResult], classification: Classification) -> u32 {
    u32::try_from(
        results
            .iter()
            .filter(|result| result.classification == classification)
            .count(),
    )
    .expect("attempt count fits u32")
}

fn aggregate_metrics(results: &[AttemptResult]) -> Metrics {
    let mut total = Metrics::default();
    for metrics in results.iter().map(|result| &result.metrics) {
        add(&mut total.assistant_turns, metrics.assistant_turns);
        add(
            &mut total.tool_or_command_calls,
            metrics.tool_or_command_calls,
        );
        add(&mut total.input_tokens, metrics.input_tokens);
        add(&mut total.cached_input_tokens, metrics.cached_input_tokens);
        add(
            &mut total.cache_write_input_tokens,
            metrics.cache_write_input_tokens,
        );
        add(&mut total.output_tokens, metrics.output_tokens);
        add(&mut total.reasoning_tokens, metrics.reasoning_tokens);
        add(&mut total.total_tokens, metrics.total_tokens);
        if let Some(cost) = metrics.cost_usd {
            total.cost_usd = Some(total.cost_usd.unwrap_or(0.0) + cost);
        }
    }
    total
}

fn add(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0).saturating_add(value));
    }
}
