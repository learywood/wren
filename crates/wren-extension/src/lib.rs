use std::{fmt, path::Path, ptr::NonNull};

use serde_json::Value;

pub const BUILD_FINGERPRINT: &str = concat!(
    "api=2;rustc=",
    env!("WREN_EXTENSION_RUSTC_COMMIT"),
    ";target=",
    env!("WREN_EXTENSION_TARGET"),
    ";profile=",
    env!("WREN_EXTENSION_PROFILE"),
    ";panic=",
    env!("WREN_EXTENSION_PANIC"),
);

#[doc(hidden)]
pub const BUILD_FINGERPRINT_C: &[u8] = concat!(
    "api=2;rustc=",
    env!("WREN_EXTENSION_RUSTC_COMMIT"),
    ";target=",
    env!("WREN_EXTENSION_TARGET"),
    ";profile=",
    env!("WREN_EXTENSION_PROFILE"),
    ";panic=",
    env!("WREN_EXTENSION_PANIC"),
    "\0",
)
.as_bytes();

#[doc(hidden)]
pub const CREATE_SYMBOL: &[u8] = b"wren_extension_create_v1\0";
#[doc(hidden)]
pub const FINGERPRINT_SYMBOL: &[u8] = b"wren_extension_build_fingerprint_v1\0";

#[doc(hidden)]
pub type CreateFunction = unsafe fn() -> ExtensionInstance;
#[doc(hidden)]
pub type FingerprintFunction = unsafe extern "C" fn() -> *const core::ffi::c_char;

pub trait Extension {
    /// Initializes the extension and returns its metadata.
    ///
    /// Implementations must not panic. Wren calls this method once.
    ///
    /// # Errors
    ///
    /// Returns an error when the extension cannot initialize.
    fn initialize(&mut self) -> Result<ExtensionMetadata<'_>, ExtensionError>;

    /// Returns the model-callable tool at `index`, if one exists.
    ///
    /// Tool indexes must be contiguous from zero and remain stable after initialization.
    fn tool(&mut self, _index: usize) -> Option<&mut dyn Tool> {
        None
    }
}

pub trait Tool {
    fn definition(&self) -> ToolDefinition<'_>;

    /// Invokes the tool with validated JSON arguments.
    ///
    /// Implementations must not panic.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the arguments or operation fail.
    fn invoke(
        &mut self,
        arguments: Value,
        context: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError>;
}

#[derive(Debug)]
pub struct ToolDefinition<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a str,
}

impl<'a> ToolDefinition<'a> {
    #[must_use]
    pub const fn new(name: &'a str, description: &'a str, input_schema: &'a str) -> Self {
        Self {
            name,
            description,
            input_schema,
        }
    }

    #[must_use]
    pub const fn name(&self) -> &str {
        self.name
    }

    #[must_use]
    pub const fn description(&self) -> &str {
        self.description
    }

    #[must_use]
    pub const fn input_schema(&self) -> &str {
        self.input_schema
    }
}

#[derive(Debug)]
pub struct ToolContext<'a> {
    working_directory: &'a Path,
}

impl<'a> ToolContext<'a> {
    #[doc(hidden)]
    #[must_use]
    pub const fn new(working_directory: &'a Path) -> Self {
        Self { working_directory }
    }

    #[must_use]
    pub const fn working_directory(&self) -> &Path {
        self.working_directory
    }
}

#[derive(Debug)]
pub struct ToolOutput {
    text: String,
    details: Value,
}

impl ToolOutput {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            details: Value::Null,
        }
    }

    #[must_use]
    pub fn with_details(text: impl Into<String>, details: Value) -> Self {
        Self {
            text: text.into(),
            details,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn details(&self) -> &Value {
        &self.details
    }
}

#[derive(Debug)]
pub struct ToolError {
    kind: String,
    message: String,
}

impl ToolError {
    #[must_use]
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ToolError {}

#[derive(Debug)]
pub struct ExtensionMetadata<'a> {
    name: &'a str,
}

impl<'a> ExtensionMetadata<'a> {
    #[must_use]
    pub const fn new(name: &'a str) -> Self {
        Self { name }
    }

    #[must_use]
    pub const fn name(&self) -> &str {
        self.name
    }
}

#[derive(Debug)]
pub struct ExtensionError {
    message: &'static str,
}

impl ExtensionError {
    #[must_use]
    pub const fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ExtensionError {}

#[doc(hidden)]
pub struct ExtensionInstance {
    extension: NonNull<dyn Extension>,
    destroy: unsafe fn(*mut dyn Extension),
}

impl ExtensionInstance {
    #[doc(hidden)]
    #[must_use]
    pub fn new(extension: Box<dyn Extension>) -> Self {
        unsafe fn destroy(extension: *mut dyn Extension) {
            // SAFETY: `ExtensionInstance` calls this exactly once with the pointer
            // produced by `Box::into_raw` below.
            unsafe { drop(Box::from_raw(extension)) };
        }

        let extension =
            NonNull::new(Box::into_raw(extension)).expect("Box pointers are never null");
        Self { extension, destroy }
    }

    #[doc(hidden)]
    pub fn extension_mut(&mut self) -> &mut dyn Extension {
        // SAFETY: the pointer remains owned and valid until this instance is dropped.
        unsafe { self.extension.as_mut() }
    }
}

impl Drop for ExtensionInstance {
    fn drop(&mut self) {
        // SAFETY: the matching destroy function came from the library that created
        // the instance and is called before that library is unloaded.
        unsafe { (self.destroy)(self.extension.as_ptr()) };
    }
}

#[macro_export]
macro_rules! export_extension {
    ($extension:expr) => {
        #[doc(hidden)]
        #[unsafe(export_name = "wren_extension_build_fingerprint_v1")]
        pub extern "C" fn __wren_extension_build_fingerprint_v1() -> *const ::core::ffi::c_char {
            $crate::BUILD_FINGERPRINT_C.as_ptr().cast()
        }

        #[doc(hidden)]
        #[unsafe(export_name = "wren_extension_create_v1")]
        pub fn __wren_extension_create_v1() -> $crate::ExtensionInstance {
            $crate::ExtensionInstance::new(::std::boxed::Box::new($extension))
        }
    };
}
