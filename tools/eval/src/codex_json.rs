use std::io;

use serde_json::{Value, json};

use crate::schema::{Metrics, SCHEMA_VERSION, Transcript, TranscriptEntry};

pub fn normalize(bytes: &[u8]) -> io::Result<Transcript> {
    let mut entries = Vec::new();
    let mut metrics = Metrics::default();
    let mut final_text = None;
    let mut turn_completed = false;
    let mut turn_failed = false;

    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = serde_json::from_slice(line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid Codex JSON on line {}: {error}", index + 1),
            )
        })?;
        match value.get("type").and_then(Value::as_str) {
            Some("item.completed") => {
                let Some(item) = value.get("item") else {
                    continue;
                };
                match item.get("type").and_then(Value::as_str) {
                    Some("agent_message") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            final_text = Some(text.to_owned());
                            entries.push(TranscriptEntry {
                                kind: "assistant_message".to_owned(),
                                name: None,
                                call_id: item.get("id").and_then(Value::as_str).map(str::to_owned),
                                text: Some(text.to_owned()),
                                arguments: None,
                                result: None,
                                error: None,
                            });
                        }
                    }
                    Some("command_execution") => entries.push(TranscriptEntry {
                        kind: "command".to_owned(),
                        name: Some("command_execution".to_owned()),
                        call_id: item.get("id").and_then(Value::as_str).map(str::to_owned),
                        text: item
                            .get("aggregated_output")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        arguments: item
                            .get("command")
                            .cloned()
                            .map(|command| json!({"command": command})),
                        result: item.get("exit_code").cloned(),
                        error: item
                            .get("exit_code")
                            .and_then(Value::as_i64)
                            .map(|code| code != 0),
                    }),
                    Some(kind) if kind.contains("file_change") => entries.push(TranscriptEntry {
                        kind: "file_change".to_owned(),
                        name: Some(kind.to_owned()),
                        call_id: item.get("id").and_then(Value::as_str).map(str::to_owned),
                        text: None,
                        arguments: None,
                        result: Some(item.clone()),
                        error: None,
                    }),
                    _ => {}
                }
            }
            Some("turn.completed") => {
                turn_completed = true;
                add_usage(&mut metrics, value.get("usage"));
            }
            Some("turn.failed") => turn_failed = true,
            _ => {}
        }
    }
    if turn_failed || !turn_completed {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Codex output did not contain a successful turn.completed event",
        ));
    }
    metrics.assistant_turns = Some(1);
    metrics.tool_or_command_calls = Some(
        u64::try_from(
            entries
                .iter()
                .filter(|entry| matches!(entry.kind.as_str(), "command" | "file_change"))
                .count(),
        )
        .expect("entry count fits u64"),
    );
    Ok(Transcript {
        schema_version: SCHEMA_VERSION,
        adapter: "codex".to_owned(),
        final_text,
        entries,
        metrics,
    })
}

fn add_usage(metrics: &mut Metrics, usage: Option<&Value>) {
    let Some(usage) = usage else { return };
    add(&mut metrics.input_tokens, integer(usage, "input_tokens"));
    add(
        &mut metrics.cached_input_tokens,
        integer(usage, "cached_input_tokens"),
    );
    add(
        &mut metrics.cache_write_input_tokens,
        integer(usage, "cache_write_input_tokens"),
    );
    add(&mut metrics.output_tokens, integer(usage, "output_tokens"));
    add(
        &mut metrics.reasoning_tokens,
        integer(usage, "reasoning_output_tokens"),
    );
    if let (Some(input), Some(output)) = (
        integer(usage, "input_tokens"),
        integer(usage, "output_tokens"),
    ) {
        add(
            &mut metrics.total_tokens,
            Some(input.saturating_add(output)),
        );
    }
}

fn integer(value: &Value, name: &str) -> Option<u64> {
    value.get(name).and_then(Value::as_u64)
}

fn add(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0).saturating_add(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_ignores_unknown_items_and_uses_completed_turn_usage() {
        let transcript = normalize(include_bytes!("../tests/fixtures/codex-events.jsonl")).unwrap();
        assert_eq!(transcript.adapter, "codex");
        assert_eq!(transcript.final_text.as_deref(), Some("done"));
        assert_eq!(transcript.metrics.input_tokens, Some(100));
        assert_eq!(transcript.metrics.cached_input_tokens, Some(40));
        assert_eq!(transcript.metrics.output_tokens, Some(12));
        assert_eq!(transcript.metrics.reasoning_tokens, Some(3));
        assert_eq!(transcript.metrics.total_tokens, Some(112));
        assert_eq!(transcript.metrics.tool_or_command_calls, Some(1));
        assert_eq!(transcript.metrics.cost_usd, None);
    }

    #[test]
    fn failed_or_malformed_protocol_is_rejected() {
        assert!(normalize(b"not-json\n").is_err());
        assert!(normalize(b"{\"type\":\"turn.failed\"}\n").is_err());
    }
}
