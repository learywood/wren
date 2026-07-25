use std::{collections::HashSet, ffi::CStr, fmt, path::Path};

use libloading::Library;
use serde_json::Value;
use wren_extension::{
    BUILD_FINGERPRINT, CREATE_SYMBOL, CreateFunction, ExtensionInstance, FINGERPRINT_SYMBOL,
    FingerprintFunction, ToolContext, ToolError, ToolOutput,
};

pub struct LoadedExtension {
    name: String,
    tool_names: Vec<String>,
    instance: Option<ExtensionInstance>,
    _library: Library,
}

impl LoadedExtension {
    pub fn load(path: &Path) -> Result<Self, LoadError> {
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn invoke_tool(
        &mut self,
        name: &str,
        arguments: Value,
        working_directory: &Path,
    ) -> Result<ToolOutput, ToolError> {
        if !self.tool_names.iter().any(|tool_name| tool_name == name) {
            return Err(ToolError::new(
                "unknown_tool",
                format!("no loaded tool is named {name:?}"),
            ));
        }

        let index = self
            .tool_names
            .iter()
            .position(|tool_name| tool_name == name)
            .expect("the tool name was checked above");
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
pub struct LoadError {
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
