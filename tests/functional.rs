use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use wren_test_support::{
    IsolatedWorkspace, ProcessRequest, ReleaseInstallation, TreeCleanup, environment::wren_child,
    run_process,
};

static RELEASE_INSTALLATION: OnceLock<ReleaseInstallation> = OnceLock::new();
static FIXTURE_EXTENSION: OnceLock<PathBuf> = OnceLock::new();
static NEXT_CAPTURE: AtomicUsize = AtomicUsize::new(0);

#[test]
fn installed_release_starts_and_auto_loads_packaged_read_extension() {
    let mut harness = HarnessInstallation::new();
    assert_success(&harness.run(Path::new("."), []), "");
    assert_success(
        &harness.invoke_tool(Path::new("."), "read", r#"{"path":"Cargo.toml"}"#),
        &fs::read_to_string("Cargo.toml")
            .expect("Cargo.toml should be readable")
            .replace("\r\n", "\n"),
    );
    harness.finish();
}

#[test]
fn configured_extensions_support_auto_and_manual_loading() {
    let mut automatic = HarnessInstallation::new();
    assert_success(
        &automatic.invoke_tool(Path::new("."), "read", r#"{"path":"Cargo.toml"}"#),
        &fs::read_to_string("Cargo.toml")
            .expect("Cargo.toml should be readable")
            .replace("\r\n", "\n"),
    );

    let mut manual = HarnessInstallation::new();
    manual.set_read_manifest_mode("manual");
    assert_error(
        &manual.invoke_tool(Path::new("."), "read", r#"{"path":"Cargo.toml"}"#),
        "unknown_tool",
    );
    manual.write_config("[extensions]\nload = [\"read\"]\n");
    assert_success(
        &manual.invoke_tool(Path::new("."), "read", r#"{"path":"Cargo.toml"}"#),
        &fs::read_to_string("Cargo.toml")
            .expect("Cargo.toml should be readable")
            .replace("\r\n", "\n"),
    );

    automatic.write_config("[extensions.read]\nmode = \"manual\"\n");
    assert_error(
        &automatic.invoke_tool(Path::new("."), "read", r#"{"path":"Cargo.toml"}"#),
        "unknown_tool",
    );
    automatic.finish();
    manual.finish();
}

#[test]
fn extension_discovery_reports_configuration_installation_and_conflict_errors() {
    let mut missing = HarnessInstallation::new();
    missing.write_config("[extensions]\nload = [\"missing\"]\n");
    assert_stderr_contains(
        &missing.run(Path::new("."), []),
        "requested extension \"missing\" is not installed",
    );

    let mut malformed = HarnessInstallation::new();
    malformed.write_config("not TOML");
    assert_stderr_contains(
        &malformed.run(Path::new("."), []),
        "configuration error: could not parse",
    );

    let mut missing_library = HarnessInstallation::new();
    missing_library.write_read_manifest(
        "id = \"read\"\ngeneration = \"missing\"\nlibrary = \"generations/missing/missing.dll\"\nmode = \"auto\"\n",
    );
    assert_stderr_contains(
        &missing_library.run(Path::new("."), []),
        "could not load extension \"read\"",
    );

    let mut conflicting = HarnessInstallation::new();
    conflicting.install_fixture_extension();
    assert_stderr_contains(
        &conflicting.run(Path::new("."), []),
        "is registered by both extension",
    );

    let mut removed_flag = HarnessInstallation::new();
    let read_library = removed_flag
        .installation
        .read_library()
        .as_os_str()
        .to_owned();
    assert_stderr_contains(
        &removed_flag.run(
            Path::new("."),
            [OsString::from("--extension"), read_library],
        ),
        "usage: wren [tool <name> --args <json>]",
    );

    missing.finish();
    malformed.finish();
    missing_library.finish();
    conflicting.finish();
    removed_flag.finish();
}

#[test]
fn read_tool_reads_ranges_and_paths_through_the_installed_release() {
    let mut harness = HarnessInstallation::new();
    let text_path = harness.workspace().join("sample.txt");
    fs::write(&text_path, b"alpha\r\nbeta\r\ngamma\r\n").expect("text fixture should be writable");

    let relative = invoke_read(
        &harness,
        serde_json::json!({"path": "sample.txt", "limit": 2}).to_string(),
    );
    assert_success(
        &relative,
        "alpha\nbeta\n\n[Showing lines 1-2. Use offset=3 to continue.]",
    );

    let absolute = invoke_read(
        &harness,
        serde_json::json!({"path": text_path, "offset": 3, "limit": 20}).to_string(),
    );
    assert_success(&absolute, "gamma\n");

    fs::write(harness.workspace().join("empty.txt"), []).expect("empty fixture should be writable");
    assert_success(
        &invoke_read(
            &harness,
            serde_json::json!({"path": "empty.txt"}).to_string(),
        ),
        "",
    );
    assert_error(
        &invoke_read(
            &harness,
            serde_json::json!({"path": "empty.txt", "offset": 2}).to_string(),
        ),
        "invalid_range",
    );
    harness.finish();
}

#[test]
fn read_tool_bounds_output_through_the_installed_release() {
    let mut harness = HarnessInstallation::new();
    fs::write(
        harness.workspace().join("many-lines.txt"),
        "x\n".repeat(2_001),
    )
    .expect("line-limit fixture should be writable");
    let line_limited = invoke_read(
        &harness,
        serde_json::json!({"path": "many-lines.txt"}).to_string(),
    );
    assert!(line_limited.succeeded());
    assert!(line_limited.stdout.len() <= 50 * 1_024);
    assert!(
        line_limited
            .stdout_text()
            .ends_with("[Showing lines 1-2000. Use offset=2001 to continue.]")
    );
    assert_success(
        &invoke_read(
            &harness,
            serde_json::json!({"path": "many-lines.txt", "offset": 2001}).to_string(),
        ),
        "x\n",
    );

    let wide_lines = format!("{}\n", "a".repeat(1_000)).repeat(60);
    fs::write(harness.workspace().join("many-bytes.txt"), wide_lines)
        .expect("byte-limit fixture should be writable");
    let byte_limited = invoke_read(
        &harness,
        serde_json::json!({"path": "many-bytes.txt"}).to_string(),
    );
    assert!(byte_limited.succeeded());
    assert!(byte_limited.stdout.len() <= 50 * 1_024);
    assert!(byte_limited.stdout_text().contains("Use offset="));

    fs::write(
        harness.workspace().join("long-line.txt"),
        format!("{}\nafter\n", "é".repeat(30_000)),
    )
    .expect("long-line fixture should be writable");
    let long_line = invoke_read(
        &harness,
        serde_json::json!({"path": "long-line.txt"}).to_string(),
    );
    assert!(long_line.succeeded());
    assert!(long_line.stdout.len() <= 50 * 1_024);
    let long_line_stdout = long_line.stdout_text();
    assert!(long_line_stdout.contains("[Line 1 truncated.]"));
    assert!(long_line_stdout.ends_with("[Showing lines 1-1. Use offset=2 to continue.]"));
    harness.finish();
}

#[test]
fn read_tool_reports_structured_errors_through_the_installed_release() {
    let mut harness = HarnessInstallation::new();
    fs::write(harness.workspace().join("sample.txt"), b"sample")
        .expect("text fixture should be writable");

    for (arguments, kind) in [
        (
            serde_json::json!({"path": "sample.txt", "offset": 0}).to_string(),
            "invalid_arguments",
        ),
        ("{".to_owned(), "invalid_arguments"),
        (
            serde_json::json!({"path": "missing.txt"}).to_string(),
            "not_found",
        ),
        (
            serde_json::json!({"path": "."}).to_string(),
            "not_regular_file",
        ),
    ] {
        assert_error(&invoke_read(&harness, arguments), kind);
    }

    fs::write(harness.workspace().join("invalid.txt"), [0xff, 0xfe])
        .expect("invalid UTF-8 fixture should be writable");
    assert_error(
        &invoke_read(
            &harness,
            serde_json::json!({"path": "invalid.txt"}).to_string(),
        ),
        "invalid_utf8",
    );

    let locked_path = harness.workspace().join("locked.txt");
    fs::write(&locked_path, b"locked").expect("locked fixture should be writable");
    let locked_file = open_without_sharing(&locked_path);
    assert_error(
        &invoke_read(
            &harness,
            serde_json::json!({"path": "locked.txt"}).to_string(),
        ),
        "permission_denied",
    );
    drop(locked_file);

    assert_error(
        &harness.invoke_tool(
            harness.workspace(),
            "unknown",
            &serde_json::json!({}).to_string(),
        ),
        "unknown_tool",
    );
    harness.finish();
}

fn release_installation() -> &'static ReleaseInstallation {
    RELEASE_INSTALLATION.get_or_init(|| {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        let root = repository.join("target").join(format!(
            "functional-install-{timestamp}-{}",
            std::process::id()
        ));
        ReleaseInstallation::install(&repository, &root)
            .expect("cargo install-wren should produce a complete release installation")
    })
}

fn fixture_extension() -> &'static Path {
    FIXTURE_EXTENSION
        .get_or_init(|| {
            let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let target = repository.join("target").join("functional-fixture");
            let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let build = Command::new(cargo)
                .current_dir(&repository)
                .args([
                    "build",
                    "--quiet",
                    "--release",
                    "--locked",
                    "--package",
                    "wren-fixture-extension",
                    "--target-dir",
                ])
                .arg(&target)
                .status()
                .expect("fixture extension should build");
            assert!(
                build.success(),
                "fixture extension build exited with {build}"
            );
            target.join("release").join(format!(
                "{}wren_fixture_extension{}",
                env::consts::DLL_PREFIX,
                env::consts::DLL_SUFFIX
            ))
        })
        .as_path()
}

fn invoke_read(harness: &HarnessInstallation, arguments: impl AsRef<str>) -> HarnessOutput {
    harness.invoke_tool(harness.workspace(), "read", arguments.as_ref())
}

fn assert_success(output: &HarnessOutput, expected_stdout: &str) {
    assert!(
        output.succeeded(),
        "Wren exited with {:?}: {}",
        output.exit_code,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout_text(), expected_stdout);
    assert!(output.stderr.is_empty());
}

fn assert_error(output: &HarnessOutput, kind: &str) {
    assert!(!output.succeeded(), "Wren unexpectedly succeeded");
    assert!(
        output.stdout.is_empty(),
        "failed tools must not write stdout"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!("wren: {kind}:")),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_stderr_contains(output: &HarnessOutput, expected: &str) {
    assert!(!output.succeeded(), "Wren unexpectedly succeeded");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
fn open_without_sharing(path: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .expect("locked fixture should open")
}

struct HarnessInstallation {
    directory: IsolatedWorkspace,
    installation: ReleaseInstallation,
}

impl HarnessInstallation {
    fn new() -> Self {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let directory =
            IsolatedWorkspace::create(&repository.join("target/functional"), "scenario")
                .expect("scenario workspace should be creatable");
        let installation = release_installation()
            .clone_to(&directory.root().join("installation"))
            .expect("release installation should be clonable");
        Self {
            directory,
            installation,
        }
    }

    fn workspace(&self) -> &Path {
        self.directory.workspace()
    }

    fn run(
        &self,
        working_directory: &Path,
        arguments: impl IntoIterator<Item = OsString>,
    ) -> HarnessOutput {
        let capture = NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed);
        let stdout_path = self
            .directory
            .artifacts()
            .join(format!("command-{capture}.stdout.txt"));
        let stderr_path = self
            .directory
            .artifacts()
            .join(format!("command-{capture}.stderr.txt"));
        let request = ProcessRequest {
            program: self.installation.executable().to_owned(),
            arguments: arguments.into_iter().collect(),
            working_directory: working_directory.to_owned(),
            stdin: &[],
            environment: wren_child(self.directory.wren_home()),
            timeout: Duration::from_secs(30),
            stdout_path: stdout_path.clone(),
            stderr_path: stderr_path.clone(),
        };
        let result = run_process(&request).expect("installed Wren process should execute");
        assert!(!result.timed_out, "installed Wren process timed out");
        assert_eq!(result.tree_cleanup, TreeCleanup::Clean);
        HarnessOutput {
            exit_code: result.exit_code,
            stdout: fs::read(stdout_path).expect("stdout capture should be readable"),
            stderr: fs::read(stderr_path).expect("stderr capture should be readable"),
        }
    }

    fn invoke_tool(&self, working_directory: &Path, name: &str, arguments: &str) -> HarnessOutput {
        self.run(
            working_directory,
            [
                OsString::from("tool"),
                OsString::from(name),
                OsString::from("--args"),
                OsString::from(arguments),
            ],
        )
    }

    fn set_read_manifest_mode(&self, mode: &str) {
        let manifest_path = self.read_manifest_path();
        let manifest =
            fs::read_to_string(&manifest_path).expect("read manifest should be readable");
        let updated = manifest.replace("mode = \"auto\"", &format!("mode = {mode:?}"));
        assert_ne!(manifest, updated, "installed read mode should be auto");
        fs::write(manifest_path, updated).expect("read manifest should be writable");
    }

    fn write_read_manifest(&self, manifest: &str) {
        fs::write(self.read_manifest_path(), manifest).expect("read manifest should be writable");
    }

    fn read_manifest_path(&self) -> PathBuf {
        self.installation
            .root()
            .join("bin/extensions/read/extension.toml")
    }

    fn install_fixture_extension(&self) {
        let extension = self
            .installation
            .root()
            .join("bin/extensions/functional-test-fixture");
        let generation = extension.join("generations/test");
        fs::create_dir_all(&generation).expect("fixture generation should be creatable");
        let library = fixture_extension();
        let library_name = library
            .file_name()
            .expect("fixture library should have a file name");
        fs::copy(library, generation.join(library_name))
            .expect("fixture library should be installable");
        fs::write(
            extension.join("extension.toml"),
            format!(
                "id = \"functional-test-fixture\"\ngeneration = \"test\"\nlibrary = \"generations/test/{}\"\nmode = \"auto\"\n",
                library_name.to_string_lossy()
            ),
        )
        .expect("fixture manifest should be writable");
    }

    fn write_config(&self, config: &str) {
        fs::write(self.directory.wren_home().join("config.toml"), config)
            .expect("Wren configuration should be writable");
    }

    fn finish(&mut self) {
        self.directory
            .finish()
            .expect("scenario workspace should clean up");
    }
}

struct HarnessOutput {
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl HarnessOutput {
    fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }

    fn stdout_text(&self) -> &str {
        str::from_utf8(&self.stdout).expect("tool stdout should be UTF-8")
    }
}
