use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const HYPERFINE_VERSION: &str = "1.20.0";
const SAMPLY_VERSION: &str = "0.13.1";
const WARMUP_RUNS: &str = "20";
const MINIMUM_RUNS: &str = "100";
const PROFILE_RATE: &str = "10000";
const PROFILE_ITERATIONS: &str = "1000";

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
Commands:\n  setup                         Install pinned performance tools under target/perf-tools\n  startup                       Measure startup of the release Wren harness\n  compare <baseline> <candidate> Compare two already-built harness binaries\n  profile-startup               Record a repeated-startup Samply profile\n  view-profile                  Open the recorded profile in the platform UI\n  help                           Print this help\n\n\
Tools:\n  hyperfine {HYPERFINE_VERSION}\n  samply    {SAMPLY_VERSION}"
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

    install_tool(&root, &tools, "hyperfine", HYPERFINE_VERSION)?;
    install_tool(&root, &tools, "samply", SAMPLY_VERSION)
}

fn install_tool(root: &Path, tools: &Path, package: &str, version: &str) -> Result<(), String> {
    let installed = tool_executable(tools, package);
    if tool_has_version(&installed, package, version) {
        println!("Using {package} {version} at `{}`.", installed.display());
        return Ok(());
    }

    println!("Installing {package} {version}...");
    let mut command = cargo_command(root);
    command
        .arg("install")
        .arg("--locked")
        .arg("--force")
        .arg("--root")
        .arg(tools)
        .arg("--version")
        .arg(version)
        .arg(package);
    run_checked(&mut command, &format!("install {package} {version}"))?;

    if tool_has_version(&installed, package, version) {
        Ok(())
    } else {
        Err(format!(
            "{package} installation did not create version {version} at `{}`",
            installed.display()
        ))
    }
}

fn startup() -> Result<(), String> {
    let root = repository_root();
    build_wren(&root, "release")?;

    let output = output_directory(&root)?;
    let wren = profile_executable(&root, "release");
    let hyperfine = require_tool(&root, "hyperfine")?;
    let json = output.join("startup.json");
    let markdown = output.join("startup.md");
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
        .arg(json)
        .arg("--export-markdown")
        .arg(markdown)
        .arg(expression);
    run_checked(&mut command, "measure Wren startup")
}

fn compare(baseline: &OsStr, candidate: &OsStr) -> Result<(), String> {
    let root = repository_root();
    let baseline = existing_binary(baseline)?;
    let candidate = existing_binary(candidate)?;
    let output = output_directory(&root)?;
    let hyperfine = require_tool(&root, "hyperfine")?;
    let json = output.join("startup-comparison.json");
    let markdown = output.join("startup-comparison.md");
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
        .arg(json)
        .arg("--export-markdown")
        .arg(markdown)
        .arg(candidate_expression);
    run_checked(&mut command, "compare Wren startup")
}

fn profile_startup() -> Result<(), String> {
    let root = repository_root();
    build_wren(&root, "profiling")?;

    let output = output_directory(&root)?.join("startup-profile.json.gz");
    let wren = profile_executable(&root, "profiling");
    let samply = require_tool(&root, "samply")?;

    remove_if_present(&output)?;
    #[cfg(target_os = "windows")]
    remove_windows_profile_outputs(&output)?;

    let mut command = Command::new(samply);
    command
        .current_dir(&root)
        .args([
            "record",
            "--rate",
            PROFILE_RATE,
            "--iteration-count",
            PROFILE_ITERATIONS,
            "--profile-name",
            "Wren startup",
            "--main-thread-only",
            "--reuse-threads",
            "--save-only",
            "--output",
        ])
        .arg(&output);
    #[cfg(target_os = "windows")]
    command.arg("--keep-etl");
    command.arg("--").arg(wren);
    run_checked(&mut command, "profile Wren startup")?;

    #[cfg(target_os = "windows")]
    create_windows_profile_report(&root, &output)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn view_profile() -> Result<(), String> {
    let root = repository_root();
    let profile = root.join("target/perf/startup-profile.json.gz");
    require_file(&profile, "profile")?;

    let samply = require_tool(&root, "samply")?;
    let mut command = Command::new(samply);
    command.current_dir(&root).arg("load").arg(profile);
    run_checked(&mut command, "open the Wren startup profile")
}

#[cfg(target_os = "windows")]
fn view_profile() -> Result<(), String> {
    let root = repository_root();
    let profile = root.join("target/perf/startup-profile.etl");
    require_file(&profile, "Windows profile")?;

    let wpa = windows_performance_tool("wpa")?;
    let mut command = Command::new(wpa);
    command.current_dir(&root).arg(profile);
    run_checked(&mut command, "open the Wren startup profile")
}

#[cfg(target_os = "windows")]
fn remove_windows_profile_outputs(profile: &Path) -> Result<(), String> {
    for path in windows_profile_paths(profile) {
        remove_if_present(&path)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn create_windows_profile_report(root: &Path, profile: &Path) -> Result<(), String> {
    let [kernel_trace, merged_trace, report] = windows_profile_paths(profile);
    require_file(&kernel_trace, "Samply kernel trace")?;

    let xperf = windows_performance_tool("xperf")?;
    let mut merge = Command::new(&xperf);
    merge
        .current_dir(root)
        .arg("-merge")
        .arg(&kernel_trace)
        .arg(&merged_trace);
    run_checked(&mut merge, "merge the Windows startup trace")?;

    let symbol_cache = root.join("target/perf/symcache");
    fs::create_dir_all(&symbol_cache).map_err(|error| {
        format!(
            "failed to create symbol cache `{}`: {error}",
            symbol_cache.display()
        )
    })?;

    let mut report_command = Command::new(xperf);
    report_command
        .current_dir(root)
        .env("_NT_SYMBOL_PATH", root.join("target/profiling"))
        .env("_NT_SYMCACHE_PATH", symbol_cache)
        .arg("-i")
        .arg(&merged_trace)
        .arg("-symbols")
        .arg("-o")
        .arg(&report)
        .args(["-a", "profile", "-detail"]);
    run_checked(
        &mut report_command,
        "create the symbolized Windows startup report",
    )?;

    println!("Windows startup profile report: `{}`", report.display());
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_profile_paths(profile: &Path) -> [PathBuf; 3] {
    let directory = profile
        .parent()
        .expect("startup profile output must have a parent directory");
    [
        directory.join("startup-profile.kernel.etl"),
        directory.join("startup-profile.etl"),
        directory.join("startup-profile.txt"),
    ]
}

#[cfg(target_os = "windows")]
fn windows_performance_tool(name: &str) -> Result<PathBuf, String> {
    let executable_name = format!("{name}.exe");
    if let Some(path) = env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(&executable_name))
            .find(|candidate| candidate.is_file())
    }) {
        return Ok(path);
    }

    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        let path = PathBuf::from(program_files)
            .join("Windows Kits/10/Windows Performance Toolkit")
            .join(&executable_name);
        if path.is_file() {
            return Ok(path);
        }
    }

    Err(format!(
        "{executable_name} was not found; install the Windows Performance Toolkit"
    ))
}

fn build_wren(root: &Path, profile: &str) -> Result<(), String> {
    let mut command = cargo_command(root);
    command
        .args(["build", "--locked", "--package", "wren", "--profile"])
        .arg(profile);
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

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to remove `{}`: {error}", path.display())),
    }
}

fn require_file(path: &Path, description: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!(
            "{description} `{}` does not exist; run `cargo perf profile-startup` first",
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

fn tool_executable(tools: &Path, name: &str) -> PathBuf {
    tools
        .join("bin")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX))
}

fn require_tool(root: &Path, name: &str) -> Result<PathBuf, String> {
    let executable = tool_executable(&tool_root(root), name);
    let version = match name {
        "hyperfine" => HYPERFINE_VERSION,
        "samply" => SAMPLY_VERSION,
        _ => return Err(format!("unknown performance tool `{name}`")),
    };
    if tool_has_version(&executable, name, version) {
        Ok(executable)
    } else {
        Err(format!(
            "pinned {name} {version} executable not found at `{}`; run `cargo perf setup`",
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

fn output_directory(root: &Path) -> Result<PathBuf, String> {
    let output = root.join("target/perf");
    fs::create_dir_all(&output)
        .map_err(|error| format!("failed to create `{}`: {error}", output.display()))?;
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
