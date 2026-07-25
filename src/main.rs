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

mod config;
mod extension;
#[cfg(feature = "profiling")]
mod profile;

use std::{
    env,
    ffi::OsString,
    io::{self, Write},
    process::ExitCode,
};

use config::Config;
use extension::ExtensionRegistry;
use serde_json::Value;

const USAGE: &str = "usage: wren [tool <name> --args <json>]";

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
    let command = match command(env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("wren: {message}");
            return ExitCode::FAILURE;
        }
    };

    match command {
        Command::Start => start(),
        Command::Invoke {
            tool_name,
            arguments,
        } => invoke_tool(&tool_name, &arguments),
    }
}

fn start() -> ExitCode {
    match start_registry() {
        Ok(_registry) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wren: {error}");
            ExitCode::FAILURE
        }
    }
}

fn start_registry() -> Result<ExtensionRegistry, String> {
    let config = Config::load().map_err(|error| format!("configuration error: {error}"))?;
    let executable = env::current_exe()
        .map_err(|error| format!("could not determine the executable path: {error}"))?;
    profile_scope!("wren.extension.registry.start");
    ExtensionRegistry::start(&executable, &config).map_err(|error| error.to_string())
}

fn invoke_tool(tool_name: &str, arguments: &str) -> ExitCode {
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
    let mut registry = match start_registry() {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("wren: {error}");
            return ExitCode::FAILURE;
        }
    };

    match registry.invoke_tool(tool_name, value, &working_directory) {
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
    Invoke {
        tool_name: String,
        arguments: String,
    },
}

fn command(mut arguments: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let Some(argument) = arguments.next() else {
        return Ok(Command::Start);
    };
    if argument != "tool" {
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
        tool_name,
        arguments: json,
    })
}
