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

#[test]
fn harness_starts_and_stops() {
    let mut harness = Command::new(env!("CARGO_BIN_EXE_wren"))
        .spawn()
        .expect("compiled Wren harness should start");

    let status = harness.wait().expect("Wren harness should stop");

    assert!(status.success(), "Wren harness exited with {status}");
}

#[test]
fn harness_loads_and_executes_extension() {
    let library = build_extension(
        "wren-fixture-extension",
        "wren_fixture_extension",
        "functional-fixture",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_wren"))
        .arg("--extension")
        .arg(&library)
        .output()
        .expect("compiled Wren harness should execute");

    assert!(
        output.status.success(),
        "Wren harness exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("harness output should be UTF-8"),
        "initialized extension: functional-test-fixture\n"
    );
}

#[test]
fn read_tool_reads_ranges_and_paths_through_the_harness() {
    let directory = TestDirectory::new();
    let text_path = directory.path().join("sample.txt");
    fs::write(&text_path, b"alpha\r\nbeta\r\ngamma\r\n").expect("text fixture should be writable");

    let relative = invoke_read(
        directory.path(),
        serde_json::json!({"path": "sample.txt", "limit": 2}).to_string(),
    );
    assert_success(
        &relative,
        "alpha\nbeta\n\n[Showing lines 1-2. Use offset=3 to continue.]",
    );

    let absolute = invoke_read(
        directory.path(),
        serde_json::json!({"path": text_path, "offset": 3, "limit": 20}).to_string(),
    );
    assert_success(&absolute, "gamma\n");

    fs::write(directory.path().join("empty.txt"), []).expect("empty fixture should be writable");
    assert_success(
        &invoke_read(
            directory.path(),
            serde_json::json!({"path": "empty.txt"}).to_string(),
        ),
        "",
    );
    assert_error(
        &invoke_read(
            directory.path(),
            serde_json::json!({"path": "empty.txt", "offset": 2}).to_string(),
        ),
        "invalid_range",
    );
}

#[test]
fn read_tool_bounds_output_through_the_harness() {
    let directory = TestDirectory::new();
    fs::write(directory.path().join("many-lines.txt"), "x\n".repeat(2_001))
        .expect("line-limit fixture should be writable");
    let line_limited = invoke_read(
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
        assert_error(&invoke_read(directory.path(), arguments), kind);
    }

    fs::write(directory.path().join("invalid.txt"), [0xff, 0xfe])
        .expect("invalid UTF-8 fixture should be writable");
    assert_error(
        &invoke_read(
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
            directory.path(),
            serde_json::json!({"path": "locked.txt"}).to_string(),
        ),
        "permission_denied",
    );
    drop(locked_file);

    let unknown = invoke_tool(
        directory.path(),
        "unknown",
        serde_json::json!({}).to_string(),
    );
    assert_error(&unknown, "unknown_tool");
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

fn invoke_read(working_directory: &Path, arguments: String) -> Output {
    invoke_tool(working_directory, "read", arguments)
}

fn invoke_tool(working_directory: &Path, name: &str, arguments: String) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wren"))
        .current_dir(working_directory)
        .arg("--extension")
        .arg(read_extension())
        .arg("tool")
        .arg(name)
        .arg("--args")
        .arg(arguments)
        .output()
        .expect("Wren tool command should execute")
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
