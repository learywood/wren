use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Serialize, de::DeserializeOwned};

use crate::{EnvironmentPolicy, ProcessRequest, TreeCleanup, run_process};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no parent"))?;
    fs::create_dir_all(parent)?;
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("atomic destination already exists: {}", path.display()),
        ));
    }
    let temporary = temporary_path(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn atomic_write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

pub fn publish_directory(staging: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "artifact destination already exists: {}",
                destination.display()
            ),
        ));
    }
    fs::rename(staging, destination)
}

pub fn capture_git(
    git: &Path,
    repository: &Path,
    status_path: &Path,
    diff_path: &Path,
) -> io::Result<()> {
    let transient = status_path.with_extension("git-transient.txt");
    run_git(
        git,
        repository,
        vec!["add".into(), "--intent-to-add".into(), "--all".into()],
        &transient,
        &transient.with_extension("stderr.txt"),
    )?;
    let _ = fs::remove_file(&transient);
    let _ = fs::remove_file(transient.with_extension("stderr.txt"));
    run_git(
        git,
        repository,
        vec!["status".into(), "--porcelain=v1".into()],
        status_path,
        &status_path.with_extension("stderr.txt"),
    )?;
    run_git(
        git,
        repository,
        vec!["diff".into(), "--binary".into(), "HEAD".into(), "--".into()],
        diff_path,
        &diff_path.with_extension("stderr.txt"),
    )?;
    let _ = fs::remove_file(status_path.with_extension("stderr.txt"));
    let _ = fs::remove_file(diff_path.with_extension("stderr.txt"));
    Ok(())
}

fn run_git(
    git: &Path,
    repository: &Path,
    arguments: Vec<OsString>,
    stdout_path: &Path,
    stderr_path: &Path,
) -> io::Result<()> {
    let request = ProcessRequest {
        program: git.to_owned(),
        arguments,
        working_directory: repository.to_owned(),
        stdin: &[],
        environment: EnvironmentPolicy::inherit(),
        timeout: Duration::from_secs(30),
        stdout_path: stdout_path.to_owned(),
        stderr_path: stderr_path.to_owned(),
    };
    let result = run_process(&request)?;
    if result.exit_code != Some(0) || result.timed_out || result.tree_cleanup != TreeCleanup::Clean
    {
        return Err(io::Error::other(format!(
            "Git artifact command failed: exit={:?}, timed_out={}, cleanup={:?}; see {}",
            result.exit_code,
            result.timed_out,
            result.tree_cleanup,
            stderr_path.display()
        )));
    }
    Ok(())
}

fn temporary_path(path: &Path) -> io::Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file has no name"))?
        .to_string_lossy();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let counter = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    Ok(path.with_file_name(format!(
        ".{file_name}.{timestamp}-{}-{counter}.tmp",
        std::process::id()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Record {
        value: u32,
    }

    #[test]
    fn json_round_trip_uses_published_destination() {
        let root = Path::new("target/test-support/artifacts");
        fs::create_dir_all(root).unwrap();
        let path = root.join(format!("record-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        atomic_write_json(&path, &Record { value: 7 }).unwrap();
        assert_eq!(read_json::<Record>(&path).unwrap(), Record { value: 7 });
        assert!(atomic_write_json(&path, &Record { value: 8 }).is_err());
        fs::remove_file(path).unwrap();
    }
}
