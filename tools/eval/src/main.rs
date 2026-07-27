#![allow(clippy::missing_errors_doc)]

mod harness;
mod pi;
mod pi_json;
mod run;
mod schema;
mod task;
mod validate;
mod verifier;

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wren-eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> io::Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.first().map(String::as_str) {
        Some("validate") if arguments.len() == 1 => {
            validate::validate(&repository())?;
            Ok(())
        }
        Some("__verify-exact-tree") => verifier::hidden_verify(&arguments[1..]),
        Some("__actor") => hidden_actor(&arguments[1..]),
        Some("run") => {
            let options = parse_run_options(&arguments[1..])?;
            if run::run(&repository(), &options)? {
                Ok(())
            } else {
                Err(io::Error::other(
                    "one or more evaluation attempts did not pass",
                ))
            }
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: wren-eval validate | wren-eval run pi <task> --attempts <n> [--config <path>] [--output <directory>]",
        )),
    }
}

fn parse_run_options(arguments: &[String]) -> io::Result<run::RunOptions> {
    if arguments.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "run requires harness, task, and --attempts <n>",
        ));
    }
    let mut attempts = None;
    let mut config = None;
    let mut output = None;
    let mut index = 2_usize;
    while index < arguments.len() {
        let value = arguments.get(index + 1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} requires a value", arguments[index]),
            )
        })?;
        match arguments[index].as_str() {
            "--attempts" => {
                attempts = Some(
                    value
                        .parse::<u32>()
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
                );
            }
            "--config" => config = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            option => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown run option: {option}"),
                ));
            }
        }
        index += 2;
    }
    Ok(run::RunOptions {
        harness_kind: arguments[0].clone(),
        task_id: arguments[1].clone(),
        attempts: attempts
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "--attempts is required"))?,
        config,
        output,
    })
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("evaluator is inside the Wren workspace")
        .to_owned()
}

fn hidden_actor(arguments: &[String]) -> io::Result<()> {
    if arguments.len() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "__actor requires kind, workspace, and marker paths",
        ));
    }
    let workspace = Path::new(&arguments[1]);
    match arguments[0].as_str() {
        "pass" => fs::write(
            workspace.join("settings.txt"),
            b"project = wren\nrelease_channel = stable\ntelemetry = disabled\n",
        ),
        "unchanged" => Ok(()),
        "timeout" => timeout_actor(Path::new(&arguments[2])),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown validation actor",
        )),
    }
}

fn timeout_actor(marker: &Path) -> io::Result<()> {
    let marker = marker.display().to_string().replace('\'', "''");
    let script =
        format!("Start-Sleep -Seconds 2; Set-Content -NoNewline -Path '{marker}' -Value survived");
    Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::sleep(Duration::from_secs(30));
    Ok(())
}
