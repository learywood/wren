use std::{
    collections::{BTreeMap, HashSet},
    ffi::CStr,
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

use libloading::Library;
use serde::Deserialize;
use serde_json::Value;
use wren_extension::{
    BUILD_FINGERPRINT, CREATE_SYMBOL, CreateFunction, ExtensionInstance, FINGERPRINT_SYMBOL,
    FingerprintFunction, ToolContext, ToolError, ToolOutput,
};

use crate::config::{Config, LoadMode, validate_extension_id};

pub struct ExtensionRegistry {
    extensions: BTreeMap<String, InstalledExtension>,
    tool_owners: BTreeMap<String, String>,
}

impl ExtensionRegistry {
    pub fn start(executable: &Path, config: &Config) -> Result<Self, RegistryError> {
        config.validate().map_err(RegistryError::configuration)?;
        let executable_directory = executable.parent().ok_or_else(|| {
            RegistryError::new(format!(
                "the executable has no parent directory: {}",
                executable.display()
            ))
        })?;
        let extensions_directory = executable_directory.join("extensions");
        let mut registry = Self {
            extensions: discover_extensions(&extensions_directory)?,
            tool_owners: BTreeMap::new(),
        };

        for (id, mode) in config.mode_overrides() {
            let extension = registry.extensions.get_mut(id).ok_or_else(|| {
                RegistryError::new(format!("configured extension {id:?} is not installed"))
            })?;
            extension.mode = mode;
        }
        for id in config.requested_extensions() {
            if !registry.extensions.contains_key(id) {
                return Err(RegistryError::new(format!(
                    "requested extension {id:?} is not installed"
                )));
            }
        }

        let automatic = registry
            .extensions
            .iter()
            .filter(|(_, extension)| extension.mode == LoadMode::Auto)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in automatic {
            registry.load(&id)?;
        }
        for id in config.requested_extensions() {
            registry.load(id)?;
        }

        Ok(registry)
    }

    pub fn load(&mut self, id: &str) -> Result<(), RegistryError> {
        let extension = self
            .extensions
            .get_mut(id)
            .ok_or_else(|| RegistryError::new(format!("extension {id:?} is not installed")))?;
        match &extension.state {
            ExtensionState::Active(_) => return Ok(()),
            ExtensionState::Failed(message) => {
                return Err(RegistryError::new(format!(
                    "extension {id:?} previously failed to load: {message}"
                )));
            }
            ExtensionState::Installed => {}
            ExtensionState::Loading => {
                return Err(RegistryError::new(format!(
                    "extension {id:?} is already loading"
                )));
            }
        }

        let library = extension.library.clone();
        extension.state = ExtensionState::Loading;
        let loaded = match LoadedExtension::load(&library) {
            Ok(loaded) => loaded,
            Err(error) => {
                let message = error.to_string();
                self.extensions
                    .get_mut(id)
                    .expect("the loading extension remains registered")
                    .state = ExtensionState::Failed(message.clone());
                return Err(RegistryError::new(format!(
                    "could not load extension {id:?}: {message}"
                )));
            }
        };

        if loaded.name() != id {
            let message = format!(
                "manifest ID {id:?} does not match initialized name {:?}",
                loaded.name()
            );
            self.extensions
                .get_mut(id)
                .expect("the loading extension remains registered")
                .state = ExtensionState::Failed(message.clone());
            return Err(RegistryError::new(message));
        }

        for tool_name in loaded.tool_names() {
            if let Some(owner) = self.tool_owners.get(tool_name) {
                let message = format!(
                    "tool {tool_name:?} is registered by both extension {owner:?} and {id:?}"
                );
                self.extensions
                    .get_mut(id)
                    .expect("the loading extension remains registered")
                    .state = ExtensionState::Failed(message.clone());
                return Err(RegistryError::new(message));
            }
        }
        for tool_name in loaded.tool_names() {
            self.tool_owners.insert(tool_name.to_owned(), id.to_owned());
        }
        self.extensions
            .get_mut(id)
            .expect("the loading extension remains registered")
            .state = ExtensionState::Active(loaded);
        Ok(())
    }

    pub fn invoke_tool(
        &mut self,
        name: &str,
        arguments: Value,
        working_directory: &Path,
    ) -> Result<ToolOutput, ToolError> {
        let Some(owner) = self.tool_owners.get(name) else {
            return Err(ToolError::new(
                "unknown_tool",
                format!("no loaded tool is named {name:?}"),
            ));
        };
        let extension = self
            .extensions
            .get_mut(owner)
            .expect("tool owners refer to registered extensions");
        let ExtensionState::Active(extension) = &mut extension.state else {
            return Err(ToolError::new(
                "tool_unavailable",
                format!("tool {name:?} is no longer available"),
            ));
        };
        extension.invoke_tool(name, arguments, working_directory)
    }
}

struct InstalledExtension {
    _generation: String,
    library: PathBuf,
    mode: LoadMode,
    state: ExtensionState,
}

enum ExtensionState {
    Installed,
    Loading,
    Active(LoadedExtension),
    Failed(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionManifest {
    id: String,
    generation: String,
    library: PathBuf,
    mode: LoadMode,
}

fn discover_extensions(
    extensions_directory: &Path,
) -> Result<BTreeMap<String, InstalledExtension>, RegistryError> {
    let entries = match fs::read_dir(extensions_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(RegistryError::new(format!(
                "could not inspect {}: {error}",
                extensions_directory.display()
            )));
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            RegistryError::new(format!(
                "could not inspect {}: {error}",
                extensions_directory.display()
            ))
        })?;
        if entry
            .file_type()
            .map_err(|error| {
                RegistryError::new(format!(
                    "could not inspect {}: {error}",
                    entry.path().display()
                ))
            })?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    directories.sort();

    let mut extensions = BTreeMap::new();
    for directory in directories {
        let manifest_path = directory.join("extension.toml");
        let text = fs::read_to_string(&manifest_path).map_err(|error| {
            RegistryError::new(format!(
                "could not read {}: {error}",
                manifest_path.display()
            ))
        })?;
        let manifest: ExtensionManifest = toml::from_str(&text).map_err(|error| {
            RegistryError::new(format!(
                "could not parse {}: {error}",
                manifest_path.display()
            ))
        })?;
        validate_extension_id(&manifest.id).map_err(RegistryError::configuration)?;
        if manifest.generation.is_empty() {
            return Err(RegistryError::new(format!(
                "extension {:?} has an empty generation",
                manifest.id
            )));
        }
        let directory_id = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                RegistryError::new(format!(
                    "extension directory name is not Unicode: {}",
                    directory.display()
                ))
            })?;
        if directory_id != manifest.id {
            return Err(RegistryError::new(format!(
                "extension directory {directory_id:?} contains manifest ID {:?}",
                manifest.id
            )));
        }
        if manifest.library.as_os_str().is_empty()
            || manifest
                .library
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(RegistryError::new(format!(
                "extension {:?} has an invalid library path",
                manifest.id
            )));
        }
        let id = manifest.id;
        let installed = InstalledExtension {
            _generation: manifest.generation,
            library: directory.join(manifest.library),
            mode: manifest.mode,
            state: ExtensionState::Installed,
        };
        if extensions.insert(id.clone(), installed).is_some() {
            return Err(RegistryError::new(format!(
                "duplicate installed extension ID {id:?}"
            )));
        }
    }
    Ok(extensions)
}

pub struct LoadedExtension {
    name: String,
    tool_names: Vec<String>,
    instance: Option<ExtensionInstance>,
    _library: Library,
}

impl LoadedExtension {
    fn load(path: &Path) -> Result<Self, LoadError> {
        // SAFETY: native extensions are trusted code. The library remains loaded
        // for at least as long as every value and function pointer obtained from it.
        let library = {
            profile_scope!("wren.extension.open_library");
            unsafe { Library::new(path) }.map_err(|error| {
                LoadError::new(format!("could not load {}: {error}", path.display()))
            })?
        };

        let fingerprint = {
            // SAFETY: only the stable fingerprint function is called before its
            // result confirms that the Rust ABI matches the harness.
            let function = unsafe { library.get::<FingerprintFunction>(FINGERPRINT_SYMBOL) }
                .map_err(|error| LoadError::new(format!("missing build fingerprint: {error}")))?;
            // SAFETY: the contract requires a pointer to a static, null-terminated string.
            let pointer = unsafe { function() };
            if pointer.is_null() {
                return Err(LoadError::new("the build fingerprint was null"));
            }
            // SAFETY: the extension contract guarantees a valid static C string.
            unsafe { CStr::from_ptr(pointer) }
                .to_str()
                .map_err(|_| LoadError::new("the build fingerprint was not UTF-8"))?
                .to_owned()
        };

        if fingerprint != BUILD_FINGERPRINT {
            return Err(LoadError::new(format!(
                "incompatible extension build: expected {BUILD_FINGERPRINT}, found {fingerprint}"
            )));
        }

        let create = {
            // SAFETY: the matching fingerprint establishes the native Rust ABI
            // used by the constructor and its return value.
            let function =
                unsafe { library.get::<CreateFunction>(CREATE_SYMBOL) }.map_err(|error| {
                    LoadError::new(format!("missing extension constructor: {error}"))
                })?;
            *function
        };
        // SAFETY: the symbol and its native ABI were validated above.
        let mut instance = unsafe { create() };
        let name = {
            profile_scope!("wren.extension.initialize");
            instance
                .extension_mut()
                .initialize()
                .map_err(|error| {
                    LoadError::new(format!("extension initialization failed: {error}"))
                })?
                .name()
                .to_owned()
        };

        if name.is_empty() {
            return Err(LoadError::new("the extension name was empty"));
        }

        let tool_names = validate_tools(&mut instance)?;

        Ok(Self {
            name,
            tool_names,
            instance: Some(instance),
            _library: library,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn tool_names(&self) -> impl Iterator<Item = &str> {
        self.tool_names.iter().map(String::as_str)
    }

    fn invoke_tool(
        &mut self,
        name: &str,
        arguments: Value,
        working_directory: &Path,
    ) -> Result<ToolOutput, ToolError> {
        let index = self
            .tool_names
            .iter()
            .position(|tool_name| tool_name == name)
            .ok_or_else(|| {
                ToolError::new(
                    "tool_unavailable",
                    format!("tool {name:?} is no longer available"),
                )
            })?;
        let instance = self
            .instance
            .as_mut()
            .expect("the extension instance exists until drop");
        let Some(tool) = instance.extension_mut().tool(index) else {
            return Err(ToolError::new(
                "tool_unavailable",
                format!("tool {name:?} is no longer available"),
            ));
        };
        if tool.definition().name() != name {
            return Err(ToolError::new(
                "tool_unavailable",
                format!("tool {name:?} changed its registration"),
            ));
        }
        tool.invoke(arguments, &ToolContext::new(working_directory))
    }
}

fn validate_tools(instance: &mut ExtensionInstance) -> Result<Vec<String>, LoadError> {
    let mut names = Vec::new();
    let mut unique_names = HashSet::new();
    let mut index = 0_usize;

    while let Some(tool) = instance.extension_mut().tool(index) {
        let definition = tool.definition();
        if definition.name().is_empty() {
            return Err(LoadError::new("a tool name was empty"));
        }
        if definition.description().is_empty() {
            return Err(LoadError::new(format!(
                "tool {:?} has an empty description",
                definition.name()
            )));
        }
        let schema: Value = serde_json::from_str(definition.input_schema()).map_err(|error| {
            LoadError::new(format!(
                "tool {:?} has invalid JSON Schema: {error}",
                definition.name()
            ))
        })?;
        if !schema.is_object() {
            return Err(LoadError::new(format!(
                "tool {:?} has a non-object JSON Schema",
                definition.name()
            )));
        }
        if !unique_names.insert(definition.name().to_owned()) {
            return Err(LoadError::new(format!(
                "duplicate tool name {:?}",
                definition.name()
            )));
        }
        names.push(definition.name().to_owned());
        index = index
            .checked_add(1)
            .ok_or_else(|| LoadError::new("the extension exposed too many tools"))?;
    }

    Ok(names)
}

impl Drop for LoadedExtension {
    fn drop(&mut self) {
        // Drop extension-owned state through its matching library code before
        // `Library` is dropped and unloads that code.
        drop(self.instance.take());
    }
}

#[derive(Debug)]
struct LoadError {
    message: String,
}

impl LoadError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LoadError {}

#[derive(Debug)]
pub struct RegistryError {
    message: String,
}

impl RegistryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn configuration(error: impl fmt::Display) -> Self {
        Self::new(format!("invalid extension configuration: {error}"))
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RegistryError {}
