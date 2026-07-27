use std::{
    fs, io,
    os::windows::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const CLEANUP_ATTEMPTS: usize = 10;
const CLEANUP_DELAY: Duration = Duration::from_millis(50);
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub struct IsolatedWorkspace {
    root: PathBuf,
    workspace: PathBuf,
    wren_home: PathBuf,
    harness_home: PathBuf,
    artifacts: PathBuf,
    cleaned: bool,
}

impl IsolatedWorkspace {
    pub fn create(parent: &Path, label: &str) -> io::Result<Self> {
        validate_label(label)?;
        fs::create_dir_all(parent)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let counter = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = parent.join(format!(
            "{label}-{timestamp}-{}-{counter}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let wren_home = root.join("wren-home");
        let harness_home = root.join("harness-home");
        let artifacts = root.join("artifacts");
        for directory in [&workspace, &wren_home, &harness_home, &artifacts] {
            fs::create_dir_all(directory)?;
        }
        Ok(Self {
            root,
            workspace,
            wren_home,
            harness_home,
            artifacts,
            cleaned: false,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub fn wren_home(&self) -> &Path {
        &self.wren_home
    }

    #[must_use]
    pub fn harness_home(&self) -> &Path {
        &self.harness_home
    }

    #[must_use]
    pub fn artifacts(&self) -> &Path {
        &self.artifacts
    }

    pub fn copy_fixture(&self, source: &Path) -> io::Result<()> {
        copy_tree_contents(source, &self.workspace)
    }

    pub fn finish(&mut self) -> io::Result<()> {
        if self.cleaned {
            return Ok(());
        }
        remove_tree_with_retry(&self.root)?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for IsolatedWorkspace {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = remove_tree_with_retry(&self.root);
        }
    }
}

pub fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy destination already exists: {}", destination.display()),
        ));
    }
    fs::create_dir_all(destination)?;
    copy_tree_contents(source, destination)
}

pub fn copy_tree_contents(source: &Path, destination: &Path) -> io::Result<()> {
    require_plain_directory(source)?;
    fs::create_dir_all(destination)?;
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        reject_reparse(&source_path, &metadata)?;
        if metadata.is_dir() {
            fs::create_dir(&destination_path)?;
            copy_tree_contents(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "fixture entry is not a regular file or directory: {}",
                    source_path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub fn require_contained_relative_path(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path must be a contained relative path: {}", path.display()),
        ));
    }
    Ok(())
}

fn validate_label(label: &str) -> io::Result<()> {
    if label.is_empty()
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace label must contain only ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn require_plain_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    reject_reparse(path, &metadata)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("expected a directory: {}", path.display()),
        ));
    }
    Ok(())
}

fn reject_reparse(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("reparse points are not allowed: {}", path.display()),
        ));
    }
    Ok(())
}

fn remove_tree_with_retry(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last_error = None;
    for attempt in 0..CLEANUP_ATTEMPTS {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < CLEANUP_ATTEMPTS {
            thread::sleep(CLEANUP_DELAY);
        }
    }
    let error = last_error.expect("at least one cleanup attempt was made");
    Err(io::Error::new(
        error.kind(),
        format!(
            "could not remove {} after bounded retries: {error}",
            path.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_unique_and_clean_up_explicitly() {
        let parent = Path::new("target/test-support");
        let mut first = IsolatedWorkspace::create(parent, "unique").unwrap();
        let mut second = IsolatedWorkspace::create(parent, "unique").unwrap();
        assert_ne!(first.root(), second.root());
        let first_path = first.root().to_owned();
        first.finish().unwrap();
        assert!(!first_path.exists());
        second.finish().unwrap();
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        for path in [
            Path::new(""),
            Path::new("..\\outside"),
            Path::new(r"C:\outside"),
        ] {
            assert!(require_contained_relative_path(path).is_err());
        }
        assert!(require_contained_relative_path(Path::new("fixture/settings.txt")).is_ok());
    }
}
