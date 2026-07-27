use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Value, json};
use wren_extension::{
    Extension, ExtensionError, ExtensionMetadata, Tool, ToolContext, ToolDefinition, ToolError,
    ToolOutput,
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

const INPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Path to the file to create or replace (relative to the working directory or absolute)."
    },
    "content": {
      "type": "string",
      "description": "Complete text content to write."
    }
  },
  "required": ["path", "content"],
  "additionalProperties": false
}"#;

#[derive(Default)]
struct WriteExtension {
    tool: WriteTool,
}

impl Extension for WriteExtension {
    fn initialize(&mut self) -> Result<ExtensionMetadata<'_>, ExtensionError> {
        Ok(ExtensionMetadata::new("write"))
    }

    fn tool(&mut self, index: usize) -> Option<&mut dyn Tool> {
        (index == 0).then_some(&mut self.tool)
    }
}

#[derive(Default)]
struct WriteTool;

impl Tool for WriteTool {
    fn definition(&self) -> ToolDefinition<'_> {
        ToolDefinition::new(
            "write",
            "Write complete UTF-8 text to a local file. Relative paths use the working directory. Creates the file and missing parent directories, or replaces an existing regular file.",
            INPUT_SCHEMA,
        )
    }

    fn invoke(
        &mut self,
        arguments: Value,
        context: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        let arguments: WriteArguments = serde_json::from_value(arguments).map_err(|error| {
            ToolError::new(
                "invalid_arguments",
                format!("invalid write arguments: {error}"),
            )
        })?;
        arguments.validate()?;

        let path = resolve_path(&arguments.path, context.working_directory());
        write_path(&path, &arguments.path, &arguments.content)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArguments {
    path: String,
    content: String,
}

impl WriteArguments {
    fn validate(&self) -> Result<(), ToolError> {
        if self.path.is_empty() {
            return Err(ToolError::new(
                "invalid_arguments",
                "path must not be empty",
            ));
        }
        Ok(())
    }
}

fn resolve_path(supplied_path: &str, working_directory: &Path) -> PathBuf {
    let path = Path::new(supplied_path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        working_directory.join(path)
    }
}

fn write_path(path: &Path, supplied_path: &str, content: &str) -> Result<ToolOutput, ToolError> {
    inspect_destination(path)?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| map_io_error(&error, Operation::CreateParents, path))?;
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| map_io_error(&error, Operation::Open, path))?;
    file.write_all(content.as_bytes())
        .map_err(|error| map_io_error(&error, Operation::Write, path))?;

    let bytes_written = content.len();
    Ok(ToolOutput::with_details(
        format!("Successfully wrote {bytes_written} bytes to {supplied_path}"),
        json!({
            "path": supplied_path,
            "resolved_path": path.to_string_lossy(),
            "bytes_written": bytes_written,
        }),
    ))
}

fn inspect_destination(path: &Path) -> Result<(), ToolError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if is_not_found(&error) => return Ok(()),
        Err(error) => return Err(map_io_error(&error, Operation::Inspect, path)),
    };

    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ToolError::new(
            "unsupported_reparse_point",
            format!(
                "{} is a reparse point; write only supports regular files",
                path.display()
            ),
        ));
    }
    if !metadata.is_file() {
        return Err(ToolError::new(
            "not_regular_file",
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Operation {
    Inspect,
    CreateParents,
    Open,
    Write,
}

impl Operation {
    const fn name(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::CreateParents => "create parent directories for",
            Self::Open => "open for writing",
            Self::Write => "write",
        }
    }
}

fn is_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || matches!(error.raw_os_error(), Some(2 | 3))
}

fn map_io_error(error: &io::Error, operation: Operation, path: &Path) -> ToolError {
    let raw_error = error.raw_os_error();
    let (kind, message_operation) = if error.kind() == io::ErrorKind::InvalidInput
        || matches!(raw_error, Some(123 | 161 | 206))
    {
        ("invalid_path", operation)
    } else if error.kind() == io::ErrorKind::NotADirectory
        || raw_error == Some(267)
        || (matches!(operation, Operation::CreateParents)
            && error.kind() == io::ErrorKind::AlreadyExists)
    {
        ("invalid_destination", Operation::CreateParents)
    } else if is_not_found(error) {
        ("not_found", operation)
    } else if error.kind() == io::ErrorKind::PermissionDenied
        || matches!(raw_error, Some(5 | 32 | 33))
    {
        ("permission_denied", operation)
    } else {
        ("io", operation)
    };
    ToolError::new(
        kind,
        format!(
            "could not {} {}: {error}",
            message_operation.name(),
            path.display()
        ),
    )
}

wren_extension::export_extension!(WriteExtension::default());

#[cfg(test)]
mod tests {
    use std::{
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn definition_has_the_stable_name_description_and_strict_schema() {
        let definition = WriteTool.definition();
        assert_eq!(definition.name(), "write");
        assert_eq!(
            definition.description(),
            "Write complete UTF-8 text to a local file. Relative paths use the working directory. Creates the file and missing parent directories, or replaces an existing regular file."
        );
        assert_eq!(
            serde_json::from_str::<Value>(definition.input_schema()).unwrap(),
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to create or replace (relative to the working directory or absolute)."
                    },
                    "content": {
                        "type": "string",
                        "description": "Complete text content to write."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            })
        );
    }

    #[test]
    fn validation_rejects_missing_wrong_unknown_and_empty_paths() {
        let root = TestRoot::new();
        let context = ToolContext::new(root.path());
        let mut tool = WriteTool;

        for arguments in [
            json!({"content": "text"}),
            json!({"path": "file.txt"}),
            json!({"path": 1, "content": "text"}),
            json!({"path": "file.txt", "content": false}),
            json!({"path": "file.txt", "content": "text", "extra": true}),
        ] {
            let error = tool.invoke(arguments, &context).unwrap_err();
            assert_eq!(error.kind(), "invalid_arguments");
            assert!(error.message().starts_with("invalid write arguments: "));
        }

        let error = tool
            .invoke(json!({"path": "", "content": ""}), &context)
            .unwrap_err();
        assert_eq!(error.kind(), "invalid_arguments");
        assert_eq!(error.message(), "path must not be empty");
    }

    #[test]
    fn resolves_relative_paths_from_the_working_directory_and_keeps_absolute_paths() {
        let root = TestRoot::new();
        assert_eq!(
            resolve_path("nested\\file.txt", root.path()),
            root.path().join("nested/file.txt")
        );
        let absolute = root.path().join("absolute.txt");
        assert_eq!(
            resolve_path(absolute.to_str().unwrap(), Path::new(r"C:\ignored")),
            absolute
        );
    }

    #[test]
    fn creates_files_and_nested_parents() {
        let root = TestRoot::new();
        let output = invoke(&root, "nested\\more\\file.txt", "created").unwrap();
        assert_eq!(
            fs::read(root.path().join("nested/more/file.txt")).unwrap(),
            b"created"
        );
        assert_eq!(
            output.text(),
            "Successfully wrote 7 bytes to nested\\more\\file.txt"
        );
    }

    #[test]
    fn replacement_truncates_existing_regular_files() {
        let root = TestRoot::new();
        let path = root.path().join("replace.txt");
        fs::write(&path, b"a much longer original").unwrap();
        invoke(&root, "replace.txt", "short").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"short");
    }

    #[test]
    fn writes_exact_utf8_bom_nul_and_newline_bytes_and_reports_details() {
        let root = TestRoot::new();
        let content = "\u{feff}hé\0\r\nline\nlone\rno final newline";
        let output = invoke(&root, "exact.txt", content).unwrap();
        let path = root.path().join("exact.txt");
        assert_eq!(fs::read(&path).unwrap(), content.as_bytes());
        assert_eq!(
            output.text(),
            format!("Successfully wrote {} bytes to exact.txt", content.len())
        );
        assert_eq!(
            output.details(),
            &json!({
                "path": "exact.txt",
                "resolved_path": path.to_string_lossy(),
                "bytes_written": content.len(),
            })
        );
    }

    #[test]
    fn empty_content_creates_a_zero_byte_file() {
        let root = TestRoot::new();
        let output = invoke(&root, "empty.txt", "").unwrap();
        assert_eq!(
            fs::metadata(root.path().join("empty.txt")).unwrap().len(),
            0
        );
        assert_eq!(output.text(), "Successfully wrote 0 bytes to empty.txt");
    }

    #[test]
    fn rejects_directories_and_non_directory_parents() {
        let root = TestRoot::new();
        fs::create_dir(root.path().join("directory")).unwrap();
        let error = invoke(&root, "directory", "text").unwrap_err();
        assert_eq!(error.kind(), "not_regular_file");

        fs::write(root.path().join("parent-file"), b"original").unwrap();
        let error = invoke(&root, "parent-file\\child.txt", "text").unwrap_err();
        assert_eq!(error.kind(), "invalid_destination");
        assert!(
            error
                .message()
                .starts_with("could not create parent directories for ")
        );
    }

    #[test]
    fn rejects_a_final_directory_junction_as_a_reparse_point() {
        let root = TestRoot::new();
        let target = root.path().join("target");
        let junction = root.path().join("junction");
        fs::create_dir(&target).unwrap();
        create_directory_junction(&junction, &target);

        let error = invoke(&root, "junction", "text").unwrap_err();
        assert_eq!(error.kind(), "unsupported_reparse_point");
        assert!(
            error
                .message()
                .ends_with("is a reparse point; write only supports regular files")
        );
        fs::remove_dir(junction).unwrap();
    }

    #[test]
    fn maps_stable_windows_and_io_error_kinds() {
        for (error, operation, expected, expected_operation) in [
            (
                io::Error::from_raw_os_error(5),
                Operation::Open,
                "permission_denied",
                Operation::Open,
            ),
            (
                io::Error::from_raw_os_error(32),
                Operation::Open,
                "permission_denied",
                Operation::Open,
            ),
            (
                io::Error::from_raw_os_error(33),
                Operation::Write,
                "permission_denied",
                Operation::Write,
            ),
            (
                io::Error::from_raw_os_error(123),
                Operation::Open,
                "invalid_path",
                Operation::Open,
            ),
            (
                io::Error::from_raw_os_error(161),
                Operation::Inspect,
                "invalid_path",
                Operation::Inspect,
            ),
            (
                io::Error::from_raw_os_error(206),
                Operation::Open,
                "invalid_path",
                Operation::Open,
            ),
            (
                io::Error::from_raw_os_error(267),
                Operation::Inspect,
                "invalid_destination",
                Operation::CreateParents,
            ),
            (
                io::Error::new(io::ErrorKind::NotADirectory, "parent is a file"),
                Operation::Inspect,
                "invalid_destination",
                Operation::CreateParents,
            ),
            (
                io::Error::from_raw_os_error(2),
                Operation::Open,
                "not_found",
                Operation::Open,
            ),
            (
                io::Error::from_raw_os_error(3),
                Operation::Write,
                "not_found",
                Operation::Write,
            ),
            (
                io::Error::other("device failure"),
                Operation::Write,
                "io",
                Operation::Write,
            ),
        ] {
            let mapped = map_io_error(&error, operation, Path::new("fixture.txt"));
            assert_eq!(mapped.kind(), expected);
            assert!(mapped.message().starts_with(&format!(
                "could not {} fixture.txt: ",
                expected_operation.name()
            )));
        }
    }

    fn invoke(root: &TestRoot, path: &str, content: &str) -> Result<ToolOutput, ToolError> {
        WriteTool.invoke(
            json!({"path": path, "content": content}),
            &ToolContext::new(root.path()),
        )
    }

    fn create_directory_junction(junction: &Path, target: &Path) {
        let junction = junction.to_string_lossy().replace('/', "\\");
        let target = target.to_string_lossy().replace('/', "\\");
        let output = Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(junction)
            .arg(target)
            .output()
            .expect("cmd.exe should create the junction fixture");
        assert!(
            output.status.success(),
            "mklink failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct TestRoot {
        path: PathBuf,
    }

    impl TestRoot {
        fn new() -> Self {
            let counter = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("wren-write-test-{}-{counter}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
