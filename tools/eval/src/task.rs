use std::{
    fmt::Write as _,
    fs, io,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use wren_test_support::workspace::{copy_tree_contents, require_contained_relative_path};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: u32,
    pub prompt: PathBuf,
    pub fixture: PathBuf,
    pub timeout_seconds: u64,
    pub verifier: VerifierConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VerifierConfig {
    ExactTreeV1 { expected: PathBuf },
}

#[derive(Clone, Debug)]
pub struct Task {
    pub directory: PathBuf,
    pub manifest: TaskManifest,
    pub prompt: Vec<u8>,
    pub manifest_hash: String,
}

impl Task {
    pub fn load(tasks_root: &Path, id: &str) -> io::Result<Self> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "task ID contains unsupported characters",
            ));
        }
        let directory = tasks_root.join(id);
        let manifest_bytes = fs::read(directory.join("task.toml"))?;
        let manifest: TaskManifest = toml::from_str(
            str::from_utf8(&manifest_bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        validate_manifest(&directory, id, &manifest)?;
        let prompt = fs::read(directory.join(&manifest.prompt))?;
        if prompt.is_empty() || prompt.iter().all(u8::is_ascii_whitespace) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "task prompt must not be empty",
            ));
        }
        Ok(Self {
            directory,
            manifest,
            prompt,
            manifest_hash: hash_bytes(&manifest_bytes),
        })
    }

    #[must_use]
    pub fn fixture(&self) -> PathBuf {
        self.directory.join(&self.manifest.fixture)
    }

    #[must_use]
    pub fn expected(&self) -> PathBuf {
        match &self.manifest.verifier {
            VerifierConfig::ExactTreeV1 { expected } => self.directory.join(expected),
        }
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        Duration::from_secs(self.manifest.timeout_seconds)
    }

    pub fn prepare_workspace(&self, destination: &Path) -> io::Result<()> {
        copy_tree_contents(&self.fixture(), destination)
    }
}

pub fn load_all(tasks_root: &Path) -> io::Result<Vec<Task>> {
    let mut directories = fs::read_dir(tasks_root)?.collect::<Result<Vec<_>, _>>()?;
    directories.sort_by_key(fs::DirEntry::file_name);
    directories
        .into_iter()
        .filter_map(|entry| match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => Some(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "task ID is not UTF-8"))
                    .and_then(|id| Task::load(tasks_root, &id)),
            ),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().fold(
        String::with_capacity(digest.len() * 2),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    )
}

fn validate_manifest(directory: &Path, id: &str, manifest: &TaskManifest) -> io::Result<()> {
    if manifest.schema_version != 1
        || manifest.id != id
        || manifest.version == 0
        || manifest.timeout_seconds == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "task schema, identity, version, or timeout is invalid",
        ));
    }
    if directory.file_name().and_then(|name| name.to_str()) != Some(manifest.id.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "task directory name must equal its ID",
        ));
    }
    require_contained_relative_path(&manifest.prompt)?;
    require_contained_relative_path(&manifest.fixture)?;
    let expected = match &manifest.verifier {
        VerifierConfig::ExactTreeV1 { expected } => expected,
    };
    require_contained_relative_path(expected)?;
    validate_tree(&directory.join(&manifest.fixture), "fixture")?;
    validate_tree(&directory.join(expected), "expected")
}

fn validate_tree(path: &Path, name: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("task {name} must be a plain directory"),
        ));
    }
    let mut count = 0_usize;
    validate_entries(path, &mut count)?;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("task {name} tree must not be empty"),
        ));
    }
    Ok(())
}

fn validate_entries(path: &Path, count: &mut usize) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "task trees must not contain reparse points",
            ));
        }
        if metadata.is_dir() {
            *count += 1;
            validate_entries(&entry.path(), count)?;
        } else if metadata.is_file() {
            *count += 1;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "task trees must contain only regular files and directories",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_task_is_valid_and_hash_is_stable_length() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("evals/tasks");
        let task = Task::load(&root, "exact-file-edit").unwrap();
        assert_eq!(task.manifest.id, "exact-file-edit");
        assert_eq!(task.manifest_hash.len(), 64);
        assert!(!task.prompt.is_empty());
    }
}
