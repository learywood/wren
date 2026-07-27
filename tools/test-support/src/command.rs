use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
};

pub fn resolve_windows_command(configured: &Path) -> io::Result<PathBuf> {
    let has_path = configured.is_absolute()
        || configured
            .components()
            .any(|component| matches!(component, Component::RootDir | Component::Prefix(_)))
        || configured
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
    if has_path {
        return canonical_regular_file(configured);
    }

    let path = env::var_os("PATH")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is not set"))?;
    let extensions = candidate_extensions(configured);
    for directory in env::split_paths(&path) {
        for extension in &extensions {
            let candidate = if extension.is_empty() {
                directory.join(configured)
            } else {
                directory.join(format!("{}{}", configured.to_string_lossy(), extension))
            };
            if candidate.is_file() {
                return fs::canonicalize(candidate);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("could not resolve {} in PATH", configured.display()),
    ))
}

fn candidate_extensions(configured: &Path) -> Vec<String> {
    if configured.extension().is_some() {
        return vec![String::new()];
    }
    env::var_os("PATHEXT").map_or_else(
        || vec![".EXE".to_owned(), ".CMD".to_owned(), ".BAT".to_owned()],
        |value| {
            value
                .to_string_lossy()
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(str::to_owned)
                .collect()
        },
    )
}

fn canonical_regular_file(path: &Path) -> io::Result<PathBuf> {
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("command is not a regular file: {}", path.display()),
        ));
    }
    fs::canonicalize(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_absolute_windows_executable() {
        let resolved = resolve_windows_command(Path::new(r"C:\Windows\System32\cmd.exe")).unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.is_file());
    }

    #[test]
    fn missing_explicit_command_is_rejected() {
        assert!(resolve_windows_command(Path::new(r"C:\missing\wren-eval.exe")).is_err());
    }
}
