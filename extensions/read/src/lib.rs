use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader},
    path::Path,
};

use memchr::memchr;
use serde::Deserialize;
use serde_json::{Value, json};
use wren_extension::{
    Extension, ExtensionError, ExtensionMetadata, Tool, ToolContext, ToolDefinition, ToolError,
    ToolOutput,
};

const MAX_LINES: usize = 2_000;
const MAX_OUTPUT_BYTES: usize = 50 * 1_024;
const READER_CAPACITY: usize = 64 * 1_024;

const INPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "path": { "type": "string" },
    "offset": { "type": "integer", "minimum": 1, "default": 1 },
    "limit": { "type": "integer", "minimum": 1, "default": 2000 }
  },
  "required": ["path"],
  "additionalProperties": false
}"#;

#[derive(Default)]
struct ReadExtension {
    tool: ReadTool,
}

impl Extension for ReadExtension {
    fn initialize(&mut self) -> Result<ExtensionMetadata<'_>, ExtensionError> {
        Ok(ExtensionMetadata::new("read"))
    }

    fn tool(&mut self, index: usize) -> Option<&mut dyn Tool> {
        (index == 0).then_some(&mut self.tool)
    }
}

#[derive(Default)]
struct ReadTool;

impl Tool for ReadTool {
    fn definition(&self) -> ToolDefinition<'_> {
        ToolDefinition::new(
            "read",
            "Read a bounded section of a local text file",
            INPUT_SCHEMA,
        )
    }

    fn invoke(
        &mut self,
        arguments: Value,
        context: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        let arguments: ReadArguments = serde_json::from_value(arguments).map_err(|error| {
            ToolError::new(
                "invalid_arguments",
                format!("invalid read arguments: {error}"),
            )
        })?;
        arguments.validate()?;

        let supplied_path = Path::new(&arguments.path);
        let path = if supplied_path.is_absolute() {
            supplied_path.to_owned()
        } else {
            context.working_directory().join(supplied_path)
        };

        read_path(&path, arguments.offset, arguments.limit)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    path: String,
    #[serde(default = "default_offset")]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

impl ReadArguments {
    fn validate(&self) -> Result<(), ToolError> {
        if self.path.is_empty() {
            return Err(ToolError::new(
                "invalid_arguments",
                "path must not be empty",
            ));
        }
        if self.offset == 0 {
            return Err(ToolError::new(
                "invalid_arguments",
                "offset must be a positive one-based line number",
            ));
        }
        if self.limit == 0 {
            return Err(ToolError::new(
                "invalid_arguments",
                "limit must be positive",
            ));
        }
        Ok(())
    }
}

const fn default_offset() -> usize {
    1
}

const fn default_limit() -> usize {
    MAX_LINES
}

fn read_path(path: &Path, offset: usize, requested_limit: usize) -> Result<ToolOutput, ToolError> {
    let metadata = fs::metadata(path).map_err(|error| map_io_error(error, "inspect", path))?;
    if !metadata.is_file() {
        return Err(ToolError::new(
            "not_regular_file",
            format!("{} is not a regular file", path.display()),
        ));
    }

    let file = File::open(path).map_err(|error| map_io_error(error, "open", path))?;
    let mut reader = BufReader::with_capacity(READER_CAPACITY, file);
    seek_to_line(&mut reader, offset, path)?;

    let line_limit = requested_limit.min(MAX_LINES);
    let limit_reason = if requested_limit < MAX_LINES {
        TruncationReason::Requested
    } else {
        TruncationReason::Lines
    };
    let collected = collect(&mut reader, line_limit, limit_reason, path)?;

    if !collected.saw_line && offset > 1 {
        return Err(ToolError::new(
            "invalid_range",
            format!("offset {offset} is beyond the end of the file"),
        ));
    }

    format_output(&collected, offset)
}

fn seek_to_line(reader: &mut BufReader<File>, offset: usize, path: &Path) -> Result<(), ToolError> {
    let mut line = 1_usize;
    while line < offset {
        let consumed = {
            let buffer = reader
                .fill_buf()
                .map_err(|error| map_io_error(error, "read", path))?;
            if buffer.is_empty() {
                return Err(ToolError::new(
                    "invalid_range",
                    format!("offset {offset} is beyond the end of the file"),
                ));
            }
            memchr(b'\n', buffer).map_or(buffer.len(), |index| index + 1)
        };
        let ended_line = {
            let buffer = reader.buffer();
            buffer.get(consumed - 1) == Some(&b'\n')
        };
        reader.consume(consumed);
        if ended_line {
            line = line.checked_add(1).ok_or_else(|| {
                ToolError::new("io", "the file contains too many lines to address")
            })?;
        }
    }
    Ok(())
}

struct Collected {
    bytes: Vec<u8>,
    line_ends: Vec<usize>,
    line_count: usize,
    saw_line: bool,
    truncation: Option<TruncationReason>,
}

#[derive(Clone, Copy)]
enum TruncationReason {
    Requested,
    Lines,
    Bytes,
}

impl TruncationReason {
    const fn name(self) -> &'static str {
        match self {
            Self::Requested => "requested_limit",
            Self::Lines => "line_limit",
            Self::Bytes => "byte_limit",
        }
    }
}

fn collect(
    reader: &mut BufReader<File>,
    line_limit: usize,
    limit_reason: TruncationReason,
    path: &Path,
) -> Result<Collected, ToolError> {
    let mut bytes = Vec::with_capacity(MAX_OUTPUT_BYTES);
    let mut line_ends = Vec::with_capacity(line_limit.min(MAX_LINES));
    let mut line_count = 0_usize;
    let mut line_started = false;
    let mut saw_line = false;
    let truncation;

    loop {
        if line_count == line_limit {
            truncation = has_more(reader, path)?.then_some(limit_reason);
            break;
        }

        if bytes.len() == MAX_OUTPUT_BYTES {
            if has_more(reader, path)? {
                saw_line = true;
                truncation = Some(TruncationReason::Bytes);
            } else {
                if line_started {
                    line_count += 1;
                    line_ends.push(bytes.len());
                }
                truncation = None;
            }
            break;
        }

        let remaining = MAX_OUTPUT_BYTES - bytes.len();
        let step = {
            let buffer = reader
                .fill_buf()
                .map_err(|error| map_io_error(error, "read", path))?;
            if buffer.is_empty() {
                ReadStep::End
            } else if let Some(newline) = memchr(b'\n', buffer) {
                let before_newline = &buffer[..newline];
                let delta = normalized_line_delta(&bytes, before_newline);
                if delta <= remaining {
                    bytes.extend_from_slice(before_newline);
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                    bytes.push(b'\n');
                    ReadStep::Line(newline + 1)
                } else {
                    let take = remaining.min(before_newline.len());
                    bytes.extend_from_slice(&before_newline[..take]);
                    ReadStep::Full(take)
                }
            } else {
                let take = remaining.min(buffer.len());
                bytes.extend_from_slice(&buffer[..take]);
                if take == buffer.len() {
                    ReadStep::Chunk(take)
                } else {
                    ReadStep::Full(take)
                }
            }
        };

        match step {
            ReadStep::End => {
                if line_started {
                    line_count += 1;
                    line_ends.push(bytes.len());
                }
                truncation = None;
                break;
            }
            ReadStep::Line(consumed) => {
                reader.consume(consumed);
                saw_line = true;
                line_started = false;
                line_count += 1;
                line_ends.push(bytes.len());
            }
            ReadStep::Chunk(consumed) => {
                reader.consume(consumed);
                saw_line = true;
                line_started = true;
            }
            ReadStep::Full(consumed) => {
                reader.consume(consumed);
                saw_line = true;
                truncation = Some(TruncationReason::Bytes);
                break;
            }
        }
    }

    Ok(Collected {
        bytes,
        line_ends,
        line_count,
        saw_line,
        truncation,
    })
}

enum ReadStep {
    End,
    Line(usize),
    Chunk(usize),
    Full(usize),
}

fn normalized_line_delta(output: &[u8], before_newline: &[u8]) -> usize {
    if before_newline.last() == Some(&b'\r') {
        before_newline.len()
    } else if before_newline.is_empty() && output.last() == Some(&b'\r') {
        0
    } else {
        before_newline.len() + 1
    }
}

fn has_more(reader: &mut BufReader<File>, path: &Path) -> Result<bool, ToolError> {
    reader
        .fill_buf()
        .map(|buffer| !buffer.is_empty())
        .map_err(|error| map_io_error(error, "read", path))
}

fn format_output(collected: &Collected, offset: usize) -> Result<ToolOutput, ToolError> {
    let Some(initial_reason) = collected.truncation else {
        let text = decode_complete(&collected.bytes)?;
        return Ok(ToolOutput::with_details(
            text,
            details(offset, collected.line_count, None, None),
        ));
    };

    for displayed_lines in (1..=collected.line_count).rev() {
        let text_end = collected.line_ends[displayed_lines - 1];
        let end_line = offset + displayed_lines - 1;
        let next_offset = end_line + 1;
        let notice = continuation_notice(offset, end_line, next_offset);
        let separator = notice_separator(&collected.bytes[..text_end]);
        if text_end + separator.len() + notice.len() <= MAX_OUTPUT_BYTES {
            let reason = if displayed_lines == collected.line_count {
                initial_reason
            } else {
                TruncationReason::Bytes
            };
            let mut text = decode_complete(&collected.bytes[..text_end])?;
            text.push_str(separator);
            text.push_str(&notice);
            return Ok(ToolOutput::with_details(
                text,
                details(offset, displayed_lines, Some(reason), Some(next_offset)),
            ));
        }
    }

    format_truncated_first_line(&collected.bytes, offset)
}

fn format_truncated_first_line(bytes: &[u8], offset: usize) -> Result<ToolOutput, ToolError> {
    let next_offset = offset
        .checked_add(1)
        .ok_or_else(|| ToolError::new("io", "the next line number exceeds the supported range"))?;
    let marker = format!("[Line {offset} truncated.]");
    let notice = continuation_notice(offset, offset, next_offset);
    let notice_bytes = marker.len() + 1 + notice.len();
    let content_limit = MAX_OUTPUT_BYTES
        .checked_sub(notice_bytes + 2)
        .expect("fixed notices fit within the output limit");
    let prefix = decode_prefix(&bytes[..bytes.len().min(content_limit)])?;
    let mut text = prefix.to_owned();
    text.push_str(notice_separator(text.as_bytes()));
    text.push_str(&marker);
    text.push('\n');
    text.push_str(&notice);

    Ok(ToolOutput::with_details(
        text,
        details(offset, 1, Some(TruncationReason::Bytes), Some(next_offset)),
    ))
}

fn continuation_notice(start: usize, end: usize, next: usize) -> String {
    format!("[Showing lines {start}-{end}. Use offset={next} to continue.]")
}

fn notice_separator(text: &[u8]) -> &'static str {
    if text.is_empty() {
        ""
    } else if text.ends_with(b"\n") {
        "\n"
    } else {
        "\n\n"
    }
}

fn decode_complete(bytes: &[u8]) -> Result<String, ToolError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| {
        ToolError::new(
            "invalid_utf8",
            "the requested file content is not valid UTF-8",
        )
    })
}

fn decode_prefix(bytes: &[u8]) -> Result<&str, ToolError> {
    match str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.error_len().is_none() => str::from_utf8(&bytes[..error.valid_up_to()])
            .map_err(|_| {
                ToolError::new(
                    "invalid_utf8",
                    "the requested file content is not valid UTF-8",
                )
            }),
        Err(_) => Err(ToolError::new(
            "invalid_utf8",
            "the requested file content is not valid UTF-8",
        )),
    }
}

fn details(
    offset: usize,
    displayed_lines: usize,
    truncation: Option<TruncationReason>,
    next_offset: Option<usize>,
) -> Value {
    json!({
        "start_line": offset,
        "end_line": (displayed_lines > 0).then(|| offset + displayed_lines - 1),
        "truncated": truncation.is_some(),
        "truncation_reason": truncation.map(TruncationReason::name),
        "next_offset": next_offset,
    })
}

fn map_io_error(error: io::Error, operation: &str, path: &Path) -> ToolError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        _ if matches!(error.raw_os_error(), Some(5 | 32 | 33)) => "permission_denied",
        _ => "io",
    };
    let message = error.to_string();
    drop(error);
    ToolError::new(
        kind,
        format!("could not {operation} {}: {message}", path.display()),
    )
}

wren_extension::export_extension!(ReadExtension::default());
