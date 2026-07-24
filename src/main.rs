#[cfg(feature = "profiling")]
macro_rules! profile_scope {
    ($name:literal) => {
        tracy_client::span!($name)
    };
}

#[cfg(not(feature = "profiling"))]
macro_rules! profile_scope {
    ($name:literal) => {
        ()
    };
}

mod extension;
#[cfg(feature = "profiling")]
mod profile;

use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

use extension::LoadedExtension;

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
        let _run = profile_scope!("wren.run");
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
    match extension_path(env::args_os().skip(1)) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(path)) => {
            let _load = profile_scope!("wren.extension.load");
            match LoadedExtension::load(&path) {
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
        Err(message) => {
            eprintln!("wren: {message}");
            ExitCode::FAILURE
        }
    }
}

fn extension_path(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<Option<PathBuf>, &'static str> {
    let Some(argument) = arguments.next() else {
        return Ok(None);
    };
    if argument != "--extension" {
        return Err("usage: wren [--extension <path>]");
    }

    let path = arguments
        .next()
        .ok_or("--extension requires a library path")?;
    if arguments.next().is_some() {
        return Err("usage: wren [--extension <path>]");
    }

    Ok(Some(path.into()))
}
