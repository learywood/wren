use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

const HYPERFINE_VERSION: &str = "1.20.0";
const TRACY_VERSION: &str = "0.13.1";
const TRACY_ARCHIVE_URL: &str =
    "https://github.com/wolfpld/tracy/releases/download/v0.13.1/windows-0.13.1.zip";
const TRACY_ARCHIVE_SHA256: &str =
    "ee6db1a7e71a12deb5973a8dbfdf9f36d3635bec0e0b31b1cc74f28de7dac4c9";
const WARMUP_RUNS: &str = "20";
const MINIMUM_RUNS: &str = "100";
const PROFILE_CAPTURE_SECONDS: &str = "1";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(15);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wren-perf: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);
    let Some(command) = arguments.next() else {
        print_help();
        return Ok(());
    };
    let command = command
        .to_str()
        .ok_or_else(|| "command must be valid Unicode".to_owned())?;

    match command {
        "setup" => {
            require_no_arguments(arguments)?;
            setup()
        }
        "startup" => {
            require_no_arguments(arguments)?;
            startup()
        }
        "profile-startup" => {
            require_no_arguments(arguments)?;
            profile_startup()
        }
        "view-profile" => {
            require_no_arguments(arguments)?;
            view_profile()
        }
        "clean" => {
            require_no_arguments(arguments)?;
            clean()
        }
        "compare" => {
            let baseline = arguments
                .next()
                .ok_or_else(|| "compare requires a baseline binary".to_owned())?;
            let candidate = arguments
                .next()
                .ok_or_else(|| "compare requires a candidate binary".to_owned())?;
            require_no_arguments(arguments)?;
            compare(&baseline, &candidate)
        }
        "help" | "--help" | "-h" => {
            require_no_arguments(arguments)?;
            print_help();
            Ok(())
        }
        _ => Err(format!(
            "unknown command `{command}`; run `cargo perf --help`"
        )),
    }
}

fn print_help() {
    println!(
        "Wren performance tasks\n\n\
Usage:\n  cargo perf <command>\n\n\
Commands:\n  setup                          Install pinned tools under target/perf-tools\n  startup                        Measure startup of the release Wren harness\n  compare <baseline> <candidate> Compare two already-built release binaries\n  profile-startup                Record and export an instrumented startup profile\n  view-profile                   Open the recorded profile in Tracy Profiler\n  clean                          Remove generated outputs and abandoned staging data\n  help                           Print this help\n\n\
Tools:\n  hyperfine {HYPERFINE_VERSION}\n  Tracy     {TRACY_VERSION}"
    );
}

fn require_no_arguments(mut arguments: impl Iterator<Item = OsString>) -> Result<(), String> {
    if arguments.next().is_some() {
        Err("unexpected argument; run `cargo perf --help`".to_owned())
    } else {
        Ok(())
    }
}

fn setup() -> Result<(), String> {
    let root = repository_root();
    let tools = tool_root(&root);
    fs::create_dir_all(&tools)
        .map_err(|error| format!("failed to create `{}`: {error}", tools.display()))?;
    remove_abandoned_staging(&tools)?;

    install_hyperfine(&root, &tools)?;
    install_tracy(&root, &tools)
}

fn install_hyperfine(root: &Path, tools: &Path) -> Result<(), String> {
    let destination = hyperfine_root(tools);
    let executable = hyperfine_executable(&destination);
    if tool_has_version(&executable, "hyperfine", HYPERFINE_VERSION) {
        println!(
            "Using hyperfine {HYPERFINE_VERSION} at `{}`.",
            executable.display()
        );
        return Ok(());
    }

    println!("Installing hyperfine {HYPERFINE_VERSION}...");
    let staging = StagingDirectory::new(tools, "hyperfine")?;
    let mut command = cargo_command(root);
    command
        .arg("install")
        .arg("--locked")
        .arg("--force")
        .arg("--root")
        .arg(staging.path())
        .arg("--version")
        .arg(HYPERFINE_VERSION)
        .arg("hyperfine");
    run_checked(
        &mut command,
        &format!("install hyperfine {HYPERFINE_VERSION}"),
    )?;

    let staged_executable = hyperfine_executable(staging.path());
    if !tool_has_version(&staged_executable, "hyperfine", HYPERFINE_VERSION) {
        return Err(format!(
            "hyperfine installation did not create version {HYPERFINE_VERSION} at `{}`",
            staged_executable.display()
        ));
    }
    staging.publish(&destination)?;
    println!(
        "Installed hyperfine {HYPERFINE_VERSION} at `{}`.",
        executable.display()
    );
    Ok(())
}

fn install_tracy(root: &Path, tools: &Path) -> Result<(), String> {
    let destination = tracy_root(tools);
    if tracy_installation_is_valid(&destination) {
        println!(
            "Using Tracy {TRACY_VERSION} at `{}`.",
            destination.display()
        );
        return Ok(());
    }

    println!("Installing Tracy {TRACY_VERSION}...");
    let staging = StagingDirectory::new(tools, "tracy")?;
    let archive = staging.path().join("windows.zip");

    let mut download = Command::new("curl.exe");
    download
        .current_dir(root)
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&archive)
        .arg(TRACY_ARCHIVE_URL);
    run_checked(&mut download, "download the Tracy archive")?;

    let checksum = sha256(&archive)?;
    if checksum != TRACY_ARCHIVE_SHA256 {
        return Err(format!(
            "Tracy archive checksum mismatch: expected {TRACY_ARCHIVE_SHA256}, found {checksum}"
        ));
    }

    let mut extract = Command::new("tar.exe");
    extract
        .current_dir(root)
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .args([
            "tracy-capture.exe",
            "tracy-csvexport.exe",
            "tracy-profiler.exe",
        ]);
    run_checked(&mut extract, "extract the Tracy archive")?;
    remove_file_if_present(&archive)?;
    fs::write(
        staging.path().join("archive.sha256"),
        format!("{TRACY_ARCHIVE_SHA256}\n"),
    )
    .map_err(|error| format!("failed to write the Tracy checksum marker: {error}"))?;

    if !tracy_installation_is_valid(staging.path()) {
        return Err("the Tracy archive did not contain the required Windows tools".to_owned());
    }
    staging.publish(&destination)?;
    println!(
        "Installed Tracy {TRACY_VERSION} at `{}`.",
        destination.display()
    );
    Ok(())
}

fn startup() -> Result<(), String> {
    let root = repository_root();
    build_wren(&root, "release", false)?;

    let output = output_root(&root)?;
    let staging = StagingDirectory::new(&output, "startup")?;
    let wren = profile_executable(&root, "release");
    let hyperfine = require_hyperfine(&root)?;
    let expression = hyperfine_expression(&wren)?;

    let mut command = Command::new(hyperfine);
    command
        .current_dir(&root)
        .args([
            "--shell=none",
            "--warmup",
            WARMUP_RUNS,
            "--min-runs",
            MINIMUM_RUNS,
            "--command-name",
            "Wren startup",
            "--export-json",
        ])
        .arg(staging.path().join("results.json"))
        .arg("--export-markdown")
        .arg(staging.path().join("results.md"))
        .arg(expression);
    run_checked(&mut command, "measure Wren startup")?;
    staging.publish(&output.join("startup"))
}

fn compare(baseline: &OsStr, candidate: &OsStr) -> Result<(), String> {
    let root = repository_root();
    let baseline = existing_binary(baseline)?;
    let candidate = existing_binary(candidate)?;
    let output = output_root(&root)?;
    let staging = StagingDirectory::new(&output, "startup-comparison")?;
    let hyperfine = require_hyperfine(&root)?;
    let baseline_expression = hyperfine_expression(&baseline)?;
    let candidate_expression = hyperfine_expression(&candidate)?;

    let mut command = Command::new(hyperfine);
    command
        .current_dir(&root)
        .args([
            "--shell=none",
            "--warmup",
            WARMUP_RUNS,
            "--min-runs",
            MINIMUM_RUNS,
            "--reference",
        ])
        .arg(baseline_expression)
        .args([
            "--reference-name",
            "baseline",
            "--command-name",
            "candidate",
        ])
        .arg("--export-json")
        .arg(staging.path().join("results.json"))
        .arg("--export-markdown")
        .arg(staging.path().join("results.md"))
        .arg(candidate_expression);
    run_checked(&mut command, "compare Wren startup")?;
    staging.publish(&output.join("startup-comparison"))
}

fn profile_startup() -> Result<(), String> {
    let root = repository_root();
    let capture_tool = require_tracy_tool(&root, "tracy-capture")?;
    let export_tool = require_tracy_tool(&root, "tracy-csvexport")?;
    build_wren(&root, "profiling", true)?;

    let output = output_root(&root)?;
    let staging = StagingDirectory::new(&output, "startup-profile")?;
    let trace = staging.path().join("profile.tracy");
    let csv = staging.path().join("zones.csv");
    let wren = profile_executable(&root, "profiling");

    let mut capture = Command::new(capture_tool)
        .current_dir(&root)
        .arg("-f")
        .args(["-s", PROFILE_CAPTURE_SECONDS])
        .arg("-o")
        .arg(&trace)
        .spawn()
        .map_err(|error| format!("failed to start Tracy capture: {error}"))?;

    thread::sleep(Duration::from_millis(100));
    let mut wren_command = Command::new(wren);
    wren_command
        .current_dir(&root)
        .env("WREN_TRACY_CAPTURE", "1");
    if let Err(error) = run_checked(&mut wren_command, "run instrumented Wren startup") {
        terminate(&mut capture);
        return Err(error);
    }

    let capture_status = wait_bounded(&mut capture, PROCESS_TIMEOUT, "Tracy capture")?;
    if !capture_status.success() {
        return Err(format!("Tracy capture exited with {capture_status}"));
    }
    require_nonempty_file(&trace, "Tracy profile")?;

    let csv_file = File::create(&csv)
        .map_err(|error| format!("failed to create `{}`: {error}", csv.display()))?;
    let mut export = Command::new(export_tool);
    export
        .current_dir(&root)
        .arg("-u")
        .arg(&trace)
        .stdout(Stdio::from(csv_file));
    run_checked(&mut export, "export the Tracy startup profile")?;
    drop(export);
    validate_profile_csv(&csv)?;

    staging.publish(&output.join("startup-profile"))?;
    println!(
        "Startup profile: `{}`",
        output.join("startup-profile/profile.tracy").display()
    );
    println!(
        "Agent-readable zones: `{}`",
        output.join("startup-profile/zones.csv").display()
    );
    Ok(())
}

fn view_profile() -> Result<(), String> {
    let root = repository_root();
    let profile = root.join("target/perf/startup-profile/profile.tracy");
    require_nonempty_file(&profile, "Tracy profile")?;

    let profiler = require_tracy_tool(&root, "tracy-profiler")?;
    Command::new(profiler)
        .current_dir(&root)
        .arg(&profile)
        .spawn()
        .map_err(|error| format!("failed to open the Tracy profile: {error}"))?;
    println!("Opened `{}` in Tracy Profiler.", profile.display());
    Ok(())
}

fn clean() -> Result<(), String> {
    let root = repository_root();
    for directory in [
        "target/perf",
        "target/perf-baseline",
        "target/perf-candidate",
    ] {
        remove_directory_if_present(&root.join(directory))?;
    }
    remove_abandoned_staging(&tool_root(&root))?;
    println!("Removed generated performance outputs and abandoned staging data.");
    Ok(())
}

fn build_wren(root: &Path, profile: &str, profiling: bool) -> Result<(), String> {
    let mut command = cargo_command(root);
    command
        .args(["build", "--locked", "--package", "wren", "--profile"])
        .arg(profile);
    if profiling {
        command.args(["--features", "profiling"]);
    }
    run_checked(
        &mut command,
        &format!("build Wren with the {profile} profile"),
    )
}

fn cargo_command(root: &Path) -> Command {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command.current_dir(root);
    command
}

fn run_checked(command: &mut Command, description: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("failed to {description}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "failed to {description}: process exited with {status}"
        ))
    }
}

fn wait_bounded(
    child: &mut Child,
    timeout: Duration,
    description: &str,
) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(PROCESS_POLL_INTERVAL),
            Ok(None) => {
                terminate(child);
                return Err(format!("{description} did not finish within 15 seconds"));
            }
            Err(error) => {
                terminate(child);
                return Err(format!("failed to wait for {description}: {error}"));
            }
        }
    }
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("failed to open `{}`: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_profile_csv(path: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?;
    let mut lines = contents.lines();
    if lines.next() == Some("name,src_file,src_line,ns_since_start,exec_time_ns,thread,value")
        && lines.any(|line| line.starts_with("wren.run,"))
    {
        Ok(())
    } else {
        Err(format!(
            "Tracy CSV `{}` did not contain a resolved `wren.run` zone",
            path.display()
        ))
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("performance task crate must be under tools/perf")
        .to_owned()
}

fn tool_root(root: &Path) -> PathBuf {
    root.join("target/perf-tools")
}

fn hyperfine_root(tools: &Path) -> PathBuf {
    tools.join(format!("hyperfine-{HYPERFINE_VERSION}"))
}

fn hyperfine_executable(root: &Path) -> PathBuf {
    root.join("bin")
        .join(format!("hyperfine{}", env::consts::EXE_SUFFIX))
}

fn tracy_root(tools: &Path) -> PathBuf {
    tools.join(format!("tracy-{TRACY_VERSION}"))
}

fn tracy_executable(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{name}{}", env::consts::EXE_SUFFIX))
}

fn tracy_installation_is_valid(root: &Path) -> bool {
    ["tracy-capture", "tracy-csvexport", "tracy-profiler"]
        .iter()
        .all(|name| tracy_executable(root, name).is_file())
        && fs::read_to_string(root.join("archive.sha256"))
            .is_ok_and(|value| value.trim() == TRACY_ARCHIVE_SHA256)
}

fn require_hyperfine(root: &Path) -> Result<PathBuf, String> {
    let executable = hyperfine_executable(&hyperfine_root(&tool_root(root)));
    if tool_has_version(&executable, "hyperfine", HYPERFINE_VERSION) {
        Ok(executable)
    } else {
        Err(format!(
            "pinned hyperfine {HYPERFINE_VERSION} executable not found at `{}`; run `cargo perf setup`",
            executable.display()
        ))
    }
}

fn require_tracy_tool(root: &Path, name: &str) -> Result<PathBuf, String> {
    let tools = tracy_root(&tool_root(root));
    let executable = tracy_executable(&tools, name);
    if tracy_installation_is_valid(&tools) && executable.is_file() {
        Ok(executable)
    } else {
        Err(format!(
            "pinned Tracy {TRACY_VERSION} tool not found at `{}`; run `cargo perf setup`",
            executable.display()
        ))
    }
}

fn tool_has_version(executable: &Path, name: &str, version: &str) -> bool {
    Command::new(executable)
        .arg("--version")
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == format!("{name} {version}")
        })
}

fn profile_executable(root: &Path, profile: &str) -> PathBuf {
    root.join("target")
        .join(profile)
        .join(format!("wren{}", env::consts::EXE_SUFFIX))
}

fn output_root(root: &Path) -> Result<PathBuf, String> {
    let output = root.join("target/perf");
    fs::create_dir_all(&output)
        .map_err(|error| format!("failed to create `{}`: {error}", output.display()))?;
    remove_abandoned_staging(&output)?;
    Ok(output)
}

fn existing_binary(value: &OsStr) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to read the current directory: {error}"))?
            .join(path)
    };

    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("binary `{}` does not exist", path.display()))
    }
}

fn hyperfine_expression(path: &Path) -> Result<String, String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("binary path `{}` is not valid Unicode", path.display()))?;
    Ok(format!("'{}'", path.replace('\'', "'\\''")))
}

fn require_nonempty_file(path: &Path, description: &str) -> Result<(), String> {
    if path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        Ok(())
    } else {
        Err(format!(
            "{description} `{}` does not exist or is empty; run `cargo perf profile-startup` first",
            path.display()
        ))
    }
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove `{}`: {error}", path.display())),
    }
}

fn remove_directory_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove `{}`: {error}", path.display())),
    }
}

fn remove_abandoned_staging(parent: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to read `{}`: {error}", parent.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read an entry under `{}`: {error}",
                parent.display()
            )
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".staging-") {
            remove_directory_if_present(&entry.path())?;
        } else if let Some(backup_name) = name.strip_prefix(".backup-") {
            let destination_name = backup_name
                .rsplit_once('-')
                .map_or(backup_name, |(destination, _process_id)| destination);
            let destination = parent.join(destination_name);
            if destination.exists() {
                remove_directory_if_present(&entry.path())?;
            } else {
                fs::rename(entry.path(), &destination).map_err(|error| {
                    format!(
                        "failed to restore `{}` from abandoned publication: {error}",
                        destination.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

struct StagingDirectory {
    path: PathBuf,
    published: bool,
}

impl StagingDirectory {
    fn new(parent: &Path, label: &str) -> Result<Self, String> {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
        let path = parent.join(format!(".staging-{label}-{}", std::process::id()));
        remove_directory_if_present(&path)?;
        fs::create_dir(&path)
            .map_err(|error| format!("failed to create `{}`: {error}", path.display()))?;
        Ok(Self {
            path,
            published: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn publish(mut self, destination: &Path) -> Result<(), String> {
        let parent = destination
            .parent()
            .ok_or_else(|| format!("destination `{}` has no parent", destination.display()))?;
        let name = destination
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("destination `{}` has no file name", destination.display()))?;
        let backup = parent.join(format!(".backup-{name}-{}", std::process::id()));
        remove_directory_if_present(&backup)?;

        let had_previous = destination.exists();
        if had_previous {
            fs::rename(destination, &backup).map_err(|error| {
                format!(
                    "failed to stage previous output `{}`: {error}",
                    destination.display()
                )
            })?;
        }

        if let Err(error) = fs::rename(&self.path, destination) {
            if had_previous {
                let _ = fs::rename(&backup, destination);
            }
            return Err(format!(
                "failed to publish `{}`: {error}",
                destination.display()
            ));
        }
        self.published = true;

        if had_previous && let Err(error) = remove_directory_if_present(&backup) {
            eprintln!(
                "wren-perf: warning: published output but could not remove backup `{}`: {error}",
                backup.display()
            );
        }
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
