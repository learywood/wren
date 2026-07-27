use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;

use crate::{
    EnvironmentPolicy, ProcessRequest, TreeCleanup, copy_tree, process::run_process,
    workspace::require_contained_relative_path,
};

#[derive(Clone)]
pub struct ReleaseInstallation {
    root: PathBuf,
    executable: PathBuf,
    read_library: PathBuf,
    write_library: PathBuf,
}

impl ReleaseInstallation {
    pub fn install(repository: &Path, root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let request = ProcessRequest {
            program: PathBuf::from(cargo),
            arguments: vec!["install-wren".into()],
            working_directory: repository.to_owned(),
            stdin: &[],
            environment: EnvironmentPolicy::inherit().set("CARGO_INSTALL_ROOT", root.as_os_str()),
            timeout: Duration::from_mins(15),
            stdout_path: root.join("install.stdout.txt"),
            stderr_path: root.join("install.stderr.txt"),
        };
        let result = run_process(&request)?;
        if result.timed_out
            || result.tree_cleanup != TreeCleanup::Clean
            || result.exit_code != Some(0)
        {
            return Err(io::Error::other(format!(
                "cargo install-wren failed: exit={:?}, timed_out={}, cleanup={:?}; see {}",
                result.exit_code,
                result.timed_out,
                result.tree_cleanup,
                request.stderr_path.display()
            )));
        }
        Self::open(root)
    }

    pub fn open(root: &Path) -> io::Result<Self> {
        let executable = root.join("bin").join("wren.exe");
        require_regular_file(&executable, "installed Wren executable")?;

        let read_library = installed_extension_library(root, "read")?;
        let write_library = installed_extension_library(root, "write")?;

        Ok(Self {
            root: root.to_owned(),
            executable,
            read_library,
            write_library,
        })
    }

    pub fn clone_to(&self, destination: &Path) -> io::Result<Self> {
        fs::create_dir(destination)?;
        copy_tree(&self.root.join("bin"), &destination.join("bin"))?;
        Self::open(destination)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    #[must_use]
    pub fn read_library(&self) -> &Path {
        &self.read_library
    }

    #[must_use]
    pub fn write_library(&self) -> &Path {
        &self.write_library
    }
}

fn installed_extension_library(root: &Path, id: &str) -> io::Result<PathBuf> {
    let extension_directory = root.join("bin").join("extensions").join(id);
    let manifest_path = extension_directory.join("extension.toml");
    require_regular_file(&manifest_path, &format!("installed {id} manifest"))?;
    let manifest: ExtensionManifest = toml::from_str(&fs::read_to_string(&manifest_path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if manifest.id != id || manifest.generation.is_empty() || manifest.mode != "auto" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("installed {id} manifest has an invalid identity, generation, or mode"),
        ));
    }
    let relative_library = Path::new(&manifest.library);
    require_contained_relative_path(relative_library)?;
    let expected_prefix = Path::new("generations").join(&manifest.generation);
    if !relative_library.starts_with(&expected_prefix) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("installed {id} library is outside its selected generation"),
        ));
    }
    let library = extension_directory.join(relative_library);
    require_regular_file(&library, &format!("installed {id} generation library"))?;
    Ok(library)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionManifest {
    id: String,
    generation: String,
    library: String,
    mode: String,
}

fn require_regular_file(path: &Path, description: &str) -> io::Result<()> {
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{description} is missing: {}", path.display()),
        ));
    }
    Ok(())
}
