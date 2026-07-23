use std::{fmt, ptr::NonNull};

pub const BUILD_FINGERPRINT: &str = concat!(
    "api=1;rustc=",
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
    "api=1;rustc=",
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
}

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
