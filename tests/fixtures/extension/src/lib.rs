use wren_extension::{Extension, ExtensionError, ExtensionMetadata};

#[derive(Default)]
struct FixtureExtension;

impl Extension for FixtureExtension {
    fn initialize(&mut self) -> Result<ExtensionMetadata<'_>, ExtensionError> {
        Ok(ExtensionMetadata::new("functional-test-fixture"))
    }
}

wren_extension::export_extension!(FixtureExtension);
