use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    os::windows::fs::MetadataExt,
    path::Path,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use wren_test_support::{
    ProcessRequest, TreeCleanup, artifacts::atomic_write_json, environment::verifier_child,
    run_process,
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactTreeReport {
    pub schema_version: u32,
    pub passed: bool,
    pub missing: Vec<String>,
    pub unexpected: Vec<String>,
    pub changed: Vec<String>,
}

pub struct VerifierExecution {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub tree_cleanup: TreeCleanup,
    pub report: Option<ExactTreeReport>,
}

pub fn compare_exact_tree(expected: &Path, actual: &Path) -> io::Result<ExactTreeReport> {
    let expected_entries = tree_entries(expected, false)?;
    let actual_entries = tree_entries(actual, true)?;
    let expected_paths = expected_entries.keys().cloned().collect::<BTreeSet<_>>();
    let actual_paths = actual_entries.keys().cloned().collect::<BTreeSet<_>>();
    let missing = expected_paths
        .difference(&actual_paths)
        .cloned()
        .collect::<Vec<_>>();
    let unexpected = actual_paths
        .difference(&expected_paths)
        .cloned()
        .collect::<Vec<_>>();
    let changed = expected_paths
        .intersection(&actual_paths)
        .filter(|path| expected_entries.get(*path) != actual_entries.get(*path))
        .cloned()
        .collect::<Vec<_>>();
    Ok(ExactTreeReport {
        schema_version: 1,
        passed: missing.is_empty() && unexpected.is_empty() && changed.is_empty(),
        missing,
        unexpected,
        changed,
    })
}

pub fn run_exact_verifier(
    executable: &Path,
    expected: &Path,
    actual: &Path,
    artifacts: &Path,
) -> io::Result<VerifierExecution> {
    let stdout_path = artifacts.join("verifier.stdout.txt");
    let stderr_path = artifacts.join("verifier.stderr.txt");
    let request = ProcessRequest {
        program: executable.to_owned(),
        arguments: vec![
            "__verify-exact-tree".into(),
            expected.as_os_str().to_owned(),
            actual.as_os_str().to_owned(),
        ],
        working_directory: actual.to_owned(),
        stdin: &[],
        environment: verifier_child(),
        timeout: Duration::from_secs(30),
        stdout_path: stdout_path.clone(),
        stderr_path,
    };
    let process = run_process(&request)?;
    let report = if process.exit_code == Some(0)
        && !process.timed_out
        && process.tree_cleanup == TreeCleanup::Clean
    {
        serde_json::from_slice::<ExactTreeReport>(&fs::read(&stdout_path)?).ok()
    } else {
        None
    };
    if let Some(report) = &report {
        atomic_write_json(&artifacts.join("verifier.json"), report)?;
    }
    Ok(VerifierExecution {
        exit_code: process.exit_code,
        timed_out: process.timed_out,
        tree_cleanup: process.tree_cleanup,
        report,
    })
}

pub fn hidden_verify(arguments: &[String]) -> io::Result<()> {
    if arguments.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "__verify-exact-tree requires expected and actual paths",
        ));
    }
    let report = compare_exact_tree(Path::new(&arguments[0]), Path::new(&arguments[1]))?;
    serde_json::to_writer(io::stdout().lock(), &report).map_err(io::Error::other)
}

#[derive(Eq, PartialEq)]
enum Entry {
    Directory,
    File(Vec<u8>),
    ReparsePoint,
}

fn tree_entries(root: &Path, exclude_git: bool) -> io::Result<BTreeMap<String, Entry>> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "exact-tree root is not a plain directory: {}",
                root.display()
            ),
        ));
    }
    let mut entries = BTreeMap::new();
    visit(root, root, exclude_git, &mut entries)?;
    Ok(entries)
}

fn visit(
    root: &Path,
    directory: &Path,
    exclude_git: bool,
    entries: &mut BTreeMap<String, Entry>,
) -> io::Result<()> {
    let mut children = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .expect("visited path remains below root");
        if exclude_git && relative == Path::new(".git") {
            continue;
        }
        let key = portable_path(relative)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            entries.insert(key, Entry::ReparsePoint);
        } else if metadata.is_dir() {
            entries.insert(key, Entry::Directory);
            visit(root, &path, exclude_git, entries)?;
        } else if metadata.is_file() {
            entries.insert(key, Entry::File(fs::read(path)?));
        } else {
            entries.insert(key, Entry::ReparsePoint);
        }
    }
    Ok(())
}

fn portable_path(path: &Path) -> io::Result<String> {
    let parts =
        path.components()
            .map(|component| {
                component.as_os_str().to_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "tree path is not UTF-8")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wren_test_support::IsolatedWorkspace;

    #[test]
    fn exact_tree_reports_missing_unexpected_and_changed_paths() {
        let mut root =
            IsolatedWorkspace::create(Path::new("target/eval-tests"), "verifier").unwrap();
        let expected = root.root().join("expected");
        let actual = root.workspace().to_owned();
        fs::create_dir(&expected).unwrap();
        fs::write(expected.join("same.txt"), b"same").unwrap();
        fs::write(expected.join("changed.txt"), b"before").unwrap();
        fs::write(expected.join("missing.txt"), b"missing").unwrap();
        fs::write(actual.join("same.txt"), b"same").unwrap();
        fs::write(actual.join("changed.txt"), b"after").unwrap();
        fs::write(actual.join("extra.txt"), b"extra").unwrap();
        fs::create_dir(actual.join(".git")).unwrap();
        fs::write(actual.join(".git/ignored"), b"ignored").unwrap();

        let report = compare_exact_tree(&expected, &actual).unwrap();
        assert!(!report.passed);
        assert_eq!(report.missing, ["missing.txt"]);
        assert_eq!(report.unexpected, ["extra.txt"]);
        assert_eq!(report.changed, ["changed.txt"]);
        root.finish().unwrap();
    }
}
