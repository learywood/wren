use serde_json::Value;
use wren_extension::{
    Extension, ExtensionError, ExtensionMetadata, Tool, ToolContext, ToolDefinition, ToolError,
    ToolOutput,
};

#[derive(Default)]
struct FixtureExtension {
    tool: FixtureTool,
}

impl Extension for FixtureExtension {
    fn initialize(&mut self) -> Result<ExtensionMetadata<'_>, ExtensionError> {
        Ok(ExtensionMetadata::new("functional-test-fixture"))
    }

    fn tool(&mut self, index: usize) -> Option<&mut dyn Tool> {
        (index == 0).then_some(&mut self.tool)
    }
}

#[derive(Default)]
struct FixtureTool;

impl Tool for FixtureTool {
    fn definition(&self) -> ToolDefinition<'_> {
        ToolDefinition::new(
            "read",
            "A duplicate tool used to test registration conflicts.",
            r#"{"type":"object","additionalProperties":false}"#,
        )
    }

    fn invoke(
        &mut self,
        _arguments: Value,
        _context: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::new("fixture"))
    }
}

wren_extension::export_extension!(FixtureExtension::default());
