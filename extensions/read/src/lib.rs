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

fn seek_to_line<R: BufRead>(reader: &mut R, offset: usize, path: &Path) -> Result<(), ToolError> {
    let mut line = 1_usize;
    while line < offset {
        let (consumed, ended_line) = {
            let buffer = reader
                .fill_buf()
                .map_err(|error| map_io_error(error, "read", path))?;
            if buffer.is_empty() {
                return Err(ToolError::new(
                    "invalid_range",
                    format!("offset {offset} is beyond the end of the file"),
                ));
            }
            let consumed = memchr(b'\n', buffer).map_or(buffer.len(), |index| index + 1);
            (consumed, buffer.get(consumed - 1) == Some(&b'\n'))
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

fn collect<R: BufRead>(
    reader: &mut R,
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

fn has_more<R: BufRead>(reader: &mut R, path: &Path) -> Result<bool, ToolError> {
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

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    fn test_path() -> &'static Path {
        Path::new("fixture.txt")
    }

    #[test]
    fn arguments_apply_defaults_and_reject_empty_or_zero_values() {
        let arguments: ReadArguments =
            serde_json::from_value(json!({"path": "sample.txt"})).unwrap();
        assert_eq!(arguments.offset, 1);
        assert_eq!(arguments.limit, MAX_LINES);
        assert!(arguments.validate().is_ok());

        for value in [
            json!({"path": ""}),
            json!({"path": "sample.txt", "offset": 0}),
            json!({"path": "sample.txt", "limit": 0}),
        ] {
            let arguments: ReadArguments = serde_json::from_value(value).unwrap();
            assert_eq!(
                arguments.validate().unwrap_err().kind(),
                "invalid_arguments"
            );
        }
    }

    #[test]
    fn requested_limit_reports_next_offset() {
        let output = collect_output(b"one\ntwo\nthree\n", 2, TruncationReason::Requested);
        assert_eq!(
            output.text(),
            "one\ntwo\n\n[Showing lines 1-2. Use offset=3 to continue.]"
        );
        assert_eq!(
            output.details(),
            &json!({
                "start_line": 1,
                "end_line": 2,
                "truncated": true,
                "truncation_reason": "requested_limit",
                "next_offset": 3,
            })
        );
    }

    #[test]
    fn line_limit_distinguishes_exactly_two_thousand_from_more() {
        let exact = "x\n".repeat(MAX_LINES);
        let output = collect_output(exact.as_bytes(), MAX_LINES, TruncationReason::Lines);
        assert!(!output.details()["truncated"].as_bool().unwrap());
        assert!(!output.text().contains("Use offset="));

        let extra = "x\n".repeat(MAX_LINES + 1);
        let output = collect_output(extra.as_bytes(), MAX_LINES, TruncationReason::Lines);
        assert_eq!(output.details()["truncation_reason"], "line_limit");
        assert!(output.text().ends_with("Use offset=2001 to continue.]"));
    }

    #[test]
    fn normalizes_crlf_split_across_reader_buffer_boundary() {
        let mut reader = BufReader::with_capacity(2, Cursor::new(b"a\r\nb\r\n"));
        let collected =
            collect(&mut reader, MAX_LINES, TruncationReason::Lines, test_path()).unwrap();
        let output = format_output(&collected, 1).unwrap();
        assert_eq!(output.text(), "a\nb\n");
    }

    #[test]
    fn byte_truncation_does_not_split_multibyte_utf8_and_fits_notice() {
        let text = format!("a{}", "é".repeat(30_000));
        let output = collect_output(text.as_bytes(), MAX_LINES, TruncationReason::Lines);
        assert!(output.text().is_char_boundary(output.text().len()));
        assert!(output.text().len() <= MAX_OUTPUT_BYTES);
        assert!(output.text().contains("[Line 1 truncated.]"));
        assert!(
            output
                .text()
                .ends_with("[Showing lines 1-1. Use offset=2 to continue.]")
        );
        assert_eq!(output.details()["truncation_reason"], "byte_limit");
    }

    #[test]
    fn complete_lines_are_removed_until_the_continuation_notice_fits() {
        let line = format!("{}\n", "a".repeat(25_580));
        let input = format!("{line}{line}{}\n", "tail".repeat(100));
        let output = collect_output(input.as_bytes(), MAX_LINES, TruncationReason::Lines);
        assert!(output.text().len() <= MAX_OUTPUT_BYTES);
        assert_eq!(output.details()["end_line"], 1);
        assert_eq!(output.details()["next_offset"], 2);
        assert_eq!(output.details()["truncation_reason"], "byte_limit");
    }

    #[test]
    fn notice_separator_handles_empty_terminated_and_unterminated_text() {
        assert_eq!(notice_separator(b""), "");
        assert_eq!(notice_separator(b"line\n"), "\n");
        assert_eq!(notice_separator(b"line"), "\n\n");
    }

    #[test]
    fn decode_prefix_drops_only_an_incomplete_final_character() {
        assert_eq!(decode_prefix(b"ok\xc3").unwrap(), "ok");
        assert_eq!(decode_prefix(b"ok\xc3\xa9").unwrap(), "oké");
        assert_eq!(decode_prefix(b"ok\xff").unwrap_err().kind(), "invalid_utf8");
        assert_eq!(
            decode_complete(b"ok\xff").unwrap_err().kind(),
            "invalid_utf8"
        );
    }

    #[test]
    fn seeking_works_with_generic_buffered_readers() {
        let mut reader = BufReader::with_capacity(1, Cursor::new(b"one\ntwo\n"));
        seek_to_line(&mut reader, 2, test_path()).unwrap();
        let collected =
            collect(&mut reader, MAX_LINES, TruncationReason::Lines, test_path()).unwrap();
        assert_eq!(format_output(&collected, 2).unwrap().text(), "two\n");
    }

    fn collect_output(input: &[u8], limit: usize, reason: TruncationReason) -> ToolOutput {
        let mut reader = BufReader::new(Cursor::new(input));
        let collected = collect(&mut reader, limit, reason, test_path()).unwrap();
        format_output(&collected, 1).unwrap()
    }
}
