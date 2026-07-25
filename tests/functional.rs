use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
static READ_EXTENSION: OnceLock<PathBuf> = OnceLock::new();
static FIXTURE_EXTENSION: OnceLock<PathBuf> = OnceLock::new();

#[test]
fn harness_starts_and_stops_without_installed_extensions() {
    let harness = HarnessInstallation::new();
    let output = harness
        .command()
        .output()
        .expect("compiled Wren harness should execute");

    assert_success(&output, "");
}

#[test]
fn configured_extensions_support_auto_and_manual_loading() {
    let automatic = HarnessInstallation::new();
    automatic.install_extension("read", read_extension(), "auto");
    assert_success(
        &automatic.invoke_tool(Path::new("."), "read", r#"{"path":"Cargo.toml"}"#),
        &fs::read_to_string("Cargo.toml")
            .expect("Cargo.toml should be readable")
            .replace("\r\n", "\n"),
    );

    let manual = HarnessInstallation::new();
    manual.install_extension("read", read_extension(), "manual");
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
}

#[test]
fn extension_discovery_reports_configuration_installation_and_conflict_errors() {
    let missing = HarnessInstallation::new();
    missing.write_config("[extensions]\nload = [\"missing\"]\n");
    assert_stderr_contains(
        &missing.command().output().expect("Wren should execute"),
        "requested extension \"missing\" is not installed",
    );

    let malformed = HarnessInstallation::new();
    malformed.write_config("not TOML");
    assert_stderr_contains(
        &malformed.command().output().expect("Wren should execute"),
        "configuration error: could not parse",
    );

    let missing_library = HarnessInstallation::new();
    missing_library.install_extension("read", Path::new("missing.dll"), "auto");
    assert_stderr_contains(
        &missing_library
            .command()
            .output()
            .expect("Wren should execute"),
        "could not load extension \"read\"",
    );

    let conflicting = HarnessInstallation::new();
    conflicting.install_extension("read", read_extension(), "auto");
    conflicting.install_extension("functional-test-fixture", fixture_extension(), "auto");
    assert_stderr_contains(
        &conflicting.command().output().expect("Wren should execute"),
        "is registered by both extension",
    );

    let removed_flag = HarnessInstallation::new()
        .command()
        .arg("--extension")
        .arg(read_extension())
        .output()
        .expect("Wren should execute");
    assert_stderr_contains(&removed_flag, "usage: wren [tool <name> --args <json>]");
}

#[test]
fn read_tool_reads_ranges_and_paths_through_the_harness() {
    let harness = read_harness();
    let directory = TestDirectory::new();
    let text_path = directory.path().join("sample.txt");
    fs::write(&text_path, b"alpha\r\nbeta\r\ngamma\r\n").expect("text fixture should be writable");

    let relative = invoke_read(
        &harness,
        directory.path(),
        serde_json::json!({"path": "sample.txt", "limit": 2}).to_string(),
    );
    assert_success(
        &relative,
        "alpha\nbeta\n\n[Showing lines 1-2. Use offset=3 to continue.]",
    );

    let absolute = invoke_read(
        &harness,
        directory.path(),
        serde_json::json!({"path": text_path, "offset": 3, "limit": 20}).to_string(),
    );
    assert_success(&absolute, "gamma\n");

    fs::write(directory.path().join("empty.txt"), []).expect("empty fixture should be writable");
    assert_success(
        &invoke_read(
            &harness,
            directory.path(),
            serde_json::json!({"path": "empty.txt"}).to_string(),
        ),
        "",
    );
    assert_error(
        &invoke_read(
            &harness,
            directory.path(),
            serde_json::json!({"path": "empty.txt", "offset": 2}).to_string(),
        ),
        "invalid_range",
    );
}

#[test]
fn read_tool_bounds_output_through_the_harness() {
    let harness = read_harness();
    let directory = TestDirectory::new();
    fs::write(directory.path().join("many-lines.txt"), "x\n".repeat(2_001))
        .expect("line-limit fixture should be writable");
    let line_limited = invoke_read(
        &harness,
        directory.path(),
        serde_json::json!({"path": "many-lines.txt"}).to_string(),
    );
    assert!(line_limited.status.success());
    assert!(line_limited.stdout.len() <= 50 * 1_024);
    assert!(
        stdout(&line_limited).ends_with("[Showing lines 1-2000. Use offset=2001 to continue.]")
    );

    let wide_lines = format!("{}\n", "a".repeat(1_000)).repeat(60);
    fs::write(directory.path().join("many-bytes.txt"), wide_lines)
        .expect("byte-limit fixture should be writable");
    let byte_limited = invoke_read(
        &harness,
        directory.path(),
        serde_json::json!({"path": "many-bytes.txt"}).to_string(),
    );
    assert!(byte_limited.status.success());
    assert!(byte_limited.stdout.len() <= 50 * 1_024);
    assert!(stdout(&byte_limited).contains("Use offset="));

    fs::write(
        directory.path().join("long-line.txt"),
        format!("{}\nafter\n", "é".repeat(30_000)),
    )
    .expect("long-line fixture should be writable");
    let long_line = invoke_read(
        &harness,
        directory.path(),
        serde_json::json!({"path": "long-line.txt"}).to_string(),
    );
    assert!(long_line.status.success());
    assert!(long_line.stdout.len() <= 50 * 1_024);
    let long_line_stdout = stdout(&long_line);
    assert!(long_line_stdout.contains("[Line 1 truncated.]"));
    assert!(long_line_stdout.ends_with("[Showing lines 1-1. Use offset=2 to continue.]"));
}

#[test]
fn read_tool_reports_structured_errors_through_the_harness() {
    let harness = read_harness();
    let directory = TestDirectory::new();
    fs::write(directory.path().join("sample.txt"), b"sample")
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
        assert_error(&invoke_read(&harness, directory.path(), arguments), kind);
    }

    fs::write(directory.path().join("invalid.txt"), [0xff, 0xfe])
        .expect("invalid UTF-8 fixture should be writable");
    assert_error(
        &invoke_read(
            &harness,
            directory.path(),
            serde_json::json!({"path": "invalid.txt"}).to_string(),
        ),
        "invalid_utf8",
    );

    let locked_path = directory.path().join("locked.txt");
    fs::write(&locked_path, b"locked").expect("locked fixture should be writable");
    let locked_file = open_without_sharing(&locked_path);
    assert_error(
        &invoke_read(
            &harness,
            directory.path(),
            serde_json::json!({"path": "locked.txt"}).to_string(),
        ),
        "permission_denied",
    );
    drop(locked_file);

    let unknown = harness.invoke_tool(
        directory.path(),
        "unknown",
        &serde_json::json!({}).to_string(),
    );
    assert_error(&unknown, "unknown_tool");
}

fn read_harness() -> HarnessInstallation {
    let harness = HarnessInstallation::new();
    harness.install_extension("read", read_extension(), "auto");
    harness
}

fn read_extension() -> &'static Path {
    READ_EXTENSION
        .get_or_init(|| {
            build_extension(
                "wren-read-extension",
                "wren_read_extension",
                "functional-read",
            )
        })
        .as_path()
}

fn fixture_extension() -> &'static Path {
    FIXTURE_EXTENSION
        .get_or_init(|| {
            build_extension(
                "wren-fixture-extension",
                "wren_fixture_extension",
                "functional-fixture",
            )
        })
        .as_path()
}

fn build_extension(package: &str, library_name: &str, target_name: &str) -> PathBuf {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = repository.join("target").join(target_name);
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let build = Command::new(cargo)
        .current_dir(&repository)
        .args(["build", "--quiet", "--package", package, "--target-dir"])
        .arg(&target)
        .status()
        .expect("extension should build");
    assert!(build.success(), "extension build exited with {build}");

    target.join("debug").join(format!(
        "{}{}{}",
        env::consts::DLL_PREFIX,
        library_name,
        env::consts::DLL_SUFFIX
    ))
}

fn invoke_read(
    harness: &HarnessInstallation,
    working_directory: &Path,
    arguments: impl AsRef<str>,
) -> Output {
    harness.invoke_tool(working_directory, "read", arguments.as_ref())
}

fn assert_success(output: &Output, expected_stdout: &str) {
    assert!(
        output.status.success(),
        "Wren exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout(output), expected_stdout);
    assert!(output.stderr.is_empty());
}

fn assert_error(output: &Output, kind: &str) {
    assert!(!output.status.success(), "Wren unexpectedly succeeded");
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

fn assert_stderr_contains(output: &Output, expected: &str) {
    assert!(!output.status.success(), "Wren unexpectedly succeeded");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout(output: &Output) -> &str {
    str::from_utf8(&output.stdout).expect("tool stdout should be UTF-8")
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
    directory: TestDirectory,
    executable: PathBuf,
    home: PathBuf,
}

impl HarnessInstallation {
    fn new() -> Self {
        let directory = TestDirectory::new();
        let executable = directory
            .path()
            .join(format!("wren{}", env::consts::EXE_SUFFIX));
        fs::copy(env!("CARGO_BIN_EXE_wren"), &executable)
            .expect("Wren executable should be copied into its installation");
        let home = directory.path().join("home");
        fs::create_dir(&home).expect("Wren home should be creatable");
        Self {
            directory,
            executable,
            home,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.env("WREN_HOME", &self.home);
        command
    }

    fn invoke_tool(&self, working_directory: &Path, name: &str, arguments: &str) -> Output {
        self.command()
            .current_dir(working_directory)
            .arg("tool")
            .arg(name)
            .arg("--args")
            .arg(arguments)
            .output()
            .expect("Wren tool command should execute")
    }

    fn install_extension(&self, id: &str, library: &Path, mode: &str) {
        let extension = self.directory.path().join("extensions").join(id);
        let generation = extension.join("generations").join("test");
        fs::create_dir_all(&generation).expect("extension directory should be creatable");
        let library_name = library
            .file_name()
            .expect("extension library should have a file name");
        if library.exists() {
            fs::copy(library, generation.join(library_name))
                .expect("extension library should be installable");
        }
        let relative_library = Path::new("generations").join("test").join(library_name);
        fs::write(
            extension.join("extension.toml"),
            format!(
                "id = {id:?}\ngeneration = \"test\"\nlibrary = {:?}\nmode = {mode:?}\n",
                relative_library.to_string_lossy().replace('\\', "/")
            ),
        )
        .expect("extension manifest should be writable");
    }

    fn write_config(&self, config: &str) {
        fs::write(self.home.join("config.toml"), config)
            .expect("Wren configuration should be writable");
    }
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let id = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("wren-functional-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("test directory should be creatable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("test directory should be removable");
    }
}
