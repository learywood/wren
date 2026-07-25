#[cfg(feature = "profiling")]
macro_rules! profile_scope {
    ($name:literal) => {
        let _profile_scope = tracy_client::span!($name);
    };
}

#[cfg(not(feature = "profiling"))]
macro_rules! profile_scope {
    ($name:literal) => {};
}

mod extension;
#[cfg(feature = "profiling")]
mod profile;

use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use extension::LoadedExtension;
use serde_json::Value;

const USAGE: &str = "usage: wren [--extension <path> [tool <name> --args <json>]]";

fn main() -> ExitCode {
    #[cfg(feature = "profiling")]
    let session = match profile::Session::start() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("wren: {error}");
            return ExitCode::FAILURE;
        }
    };

    let result = {
        profile_scope!("wren.run");
        run()
    };

    #[cfg(feature = "profiling")]
    if let Err(error) = session.finish() {
        eprintln!("wren: {error}");
        return ExitCode::FAILURE;
    }

    result
}

fn run() -> ExitCode {
    match command(env::args_os().skip(1)) {
        Ok(Command::Start) => ExitCode::SUCCESS,
        Ok(Command::Load { path }) => load_extension(&path),
        Ok(Command::Invoke {
            path,
            tool_name,
            arguments,
        }) => invoke_tool(&path, &tool_name, &arguments),
        Err(message) => {
            eprintln!("wren: {message}");
            ExitCode::FAILURE
        }
    }
}

fn load_extension(path: &Path) -> ExitCode {
    profile_scope!("wren.extension.load");
    match LoadedExtension::load(path) {
        Ok(extension) => {
            println!("initialized extension: {}", extension.name());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wren: {error}");
            ExitCode::FAILURE
        }
    }
}

fn invoke_tool(path: &Path, tool_name: &str, arguments: &str) -> ExitCode {
    let value: Value = match serde_json::from_str(arguments) {
        Ok(value @ Value::Object(_)) => value,
        Ok(_) => {
            eprintln!("wren: invalid_arguments: --args must be a JSON object");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("wren: invalid_arguments: --args is not valid JSON: {error}");
            return ExitCode::FAILURE;
        }
    };
    let working_directory = match env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("wren: could not determine the working directory: {error}");
            return ExitCode::FAILURE;
        }
    };

    profile_scope!("wren.extension.load");
    let mut extension = match LoadedExtension::load(path) {
        Ok(extension) => extension,
        Err(error) => {
            eprintln!("wren: {error}");
            return ExitCode::FAILURE;
        }
    };

    match extension.invoke_tool(tool_name, value, &working_directory) {
        Ok(output) => {
            if let Err(error) = io::stdout().write_all(output.text().as_bytes()) {
                eprintln!("wren: could not write tool output: {error}");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("wren: {error}");
            ExitCode::FAILURE
        }
    }
}

enum Command {
    Start,
    Load {
        path: PathBuf,
    },
    Invoke {
        path: PathBuf,
        tool_name: String,
        arguments: String,
    },
}

fn command(mut arguments: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let Some(argument) = arguments.next() else {
        return Ok(Command::Start);
    };
    if argument != "--extension" {
        return Err(USAGE.to_owned());
    }

    let path = arguments
        .next()
        .ok_or_else(|| "--extension requires a library path".to_owned())?;
    let Some(subcommand) = arguments.next() else {
        return Ok(Command::Load { path: path.into() });
    };
    if subcommand != "tool" {
        return Err(USAGE.to_owned());
    }

    let tool_name = arguments
        .next()
        .ok_or_else(|| "tool requires a name".to_owned())?
        .into_string()
        .map_err(|_| "the tool name must be Unicode".to_owned())?;
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--args")) {
        return Err("tool requires --args <json>".to_owned());
    }
    let json = arguments
        .next()
        .ok_or_else(|| "--args requires JSON".to_owned())?
        .into_string()
        .map_err(|_| "--args must be Unicode JSON".to_owned())?;
    if arguments.next().is_some() {
        return Err(USAGE.to_owned());
    }

    Ok(Command::Invoke {
        path: path.into(),
        tool_name,
        arguments: json,
    })
}
