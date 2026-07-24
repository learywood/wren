use std::{ffi::CStr, fmt, path::Path};

use libloading::Library;
use wren_extension::{
    BUILD_FINGERPRINT, CREATE_SYMBOL, CreateFunction, ExtensionInstance, FINGERPRINT_SYMBOL,
    FingerprintFunction,
};

pub struct LoadedExtension {
    name: String,
    instance: Option<ExtensionInstance>,
    _library: Library,
}

impl LoadedExtension {
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        // SAFETY: native extensions are trusted code. The library remains loaded
        // for at least as long as every value and function pointer obtained from it.
        let library = {
            let _open = profile_scope!("wren.extension.open_library");
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
            let _initialize = profile_scope!("wren.extension.initialize");
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

        Ok(Self {
            name,
            instance: Some(instance),
            _library: library,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
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
