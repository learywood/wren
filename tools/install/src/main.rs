use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::Hasher,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    match install() {
        Ok(executable) => {
            println!("installed Wren at {}", executable.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("install-wren: {error}");
            ExitCode::FAILURE
        }
    }
}

fn install() -> Result<PathBuf, String> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the installer is inside the Wren workspace");
    let target = repository.join("target").join("install-wren");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(repository)
        .args([
            "build",
            "--release",
            "--locked",
            "--package",
            "wren",
            "--package",
            "wren-read-extension",
            "--package",
            "wren-write-extension",
            "--target-dir",
        ])
        .arg(&target)
        .status()
        .map_err(|error| format!("could not start Cargo: {error}"))?;
    if !status.success() {
        return Err(format!("Cargo build failed with {status}"));
    }

    let installation_root = installation_root()?;
    let bin = installation_root.join("bin");
    fs::create_dir_all(&bin)
        .map_err(|error| format!("could not create {}: {error}", bin.display()))?;

    let executable_name = format!("wren{}", env::consts::EXE_SUFFIX);
    let built_executable = target.join("release").join(&executable_name);
    let installed_executable = bin.join(&executable_name);
    copy(&built_executable, &installed_executable)?;

    let release = target.join("release");
    install_extension(&bin, &release, "read", "wren_read_extension")?;
    install_extension(&bin, &release, "write", "wren_write_extension")?;

    Ok(installed_executable)
}

fn install_extension(
    bin: &Path,
    release: &Path,
    id: &str,
    library_stem: &str,
) -> Result<(), String> {
    let library_name = format!(
        "{}{library_stem}{}",
        env::consts::DLL_PREFIX,
        env::consts::DLL_SUFFIX
    );
    let built_library = release.join(&library_name);
    let library_bytes = fs::read(&built_library)
        .map_err(|error| format!("could not read {}: {error}", built_library.display()))?;
    let mut hasher = DefaultHasher::new();
    hasher.write(&library_bytes);
    let generation = format!("{:016x}", hasher.finish());
    let extension = bin.join("extensions").join(id);
    let generation_directory = extension.join("generations").join(&generation);
    fs::create_dir_all(&generation_directory).map_err(|error| {
        format!(
            "could not create {}: {error}",
            generation_directory.display()
        )
    })?;
    let installed_library = generation_directory.join(&library_name);
    if !installed_library.exists() {
        fs::write(&installed_library, library_bytes)
            .map_err(|error| format!("could not write {}: {error}", installed_library.display()))?;
    }

    let relative_library = format!("generations/{generation}/{library_name}");
    let manifest = format!(
        "id = \"{id}\"\ngeneration = \"{generation}\"\nlibrary = \"{relative_library}\"\nmode = \"auto\"\n"
    );
    let manifest_path = extension.join("extension.toml");
    fs::write(&manifest_path, manifest)
        .map_err(|error| format!("could not write {}: {error}", manifest_path.display()))?;
    Ok(())
}

fn installation_root() -> Result<PathBuf, String> {
    if let Some(root) = env::var_os("CARGO_INSTALL_ROOT").filter(|value| !value.is_empty()) {
        return Ok(root.into());
    }
    if let Some(root) = env::var_os("CARGO_HOME").filter(|value| !value.is_empty()) {
        return Ok(root.into());
    }
    let home = env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "CARGO_INSTALL_ROOT, CARGO_HOME, and USERPROFILE are not set".to_owned())?;
    Ok(PathBuf::from(home).join(".cargo"))
}

fn copy(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}
