use std::{collections::BTreeMap, io};

use serde_json::Value;

use crate::schema::{Metrics, SCHEMA_VERSION, Transcript, TranscriptEntry};

#[allow(clippy::too_many_lines)]
pub fn normalize(bytes: &[u8]) -> io::Result<Transcript> {
    let mut entries = Vec::new();
    let mut starts = BTreeMap::<String, (String, Value)>::new();
    let mut metrics = Metrics::default();
    let mut assistant_turns = 0_u64;
    let mut final_text = None;

    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = serde_json::from_slice(line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid Pi JSON on line {}: {error}", index + 1),
            )
        })?;
        match value.get("type").and_then(Value::as_str) {
            Some("message_end") => {
                let Some(message) = value.get("message") else {
                    continue;
                };
                if message.get("role").and_then(Value::as_str) != Some("assistant") {
                    continue;
                }
                assistant_turns = assistant_turns.saturating_add(1);
                add_pi_usage(&mut metrics, message.get("usage"));
                let text = content_text(message.get("content"));
                if !text.is_empty() {
                    final_text = Some(text.clone());
                    entries.push(TranscriptEntry {
                        kind: "assistant_message".to_owned(),
                        name: None,
                        call_id: None,
                        text: Some(text),
                        arguments: None,
                        result: None,
                        error: None,
                    });
                }
            }
            Some("tool_execution_start") => {
                if let Some(id) = value.get("toolCallId").and_then(Value::as_str) {
                    starts.insert(
                        id.to_owned(),
                        (
                            value
                                .get("toolName")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_owned(),
                            value.get("args").cloned().unwrap_or(Value::Null),
                        ),
                    );
                }
            }
            Some("tool_execution_end") => {
                let id = value
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                let (name, arguments) = starts.remove(&id).unwrap_or_else(|| {
                    (
                        value
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned(),
                        Value::Null,
                    )
                });
                entries.push(TranscriptEntry {
                    kind: "tool".to_owned(),
                    name: Some(name),
                    call_id: Some(id),
                    text: None,
                    arguments: Some(arguments),
                    result: value.get("result").cloned(),
                    error: value.get("isError").and_then(Value::as_bool),
                });
            }
            _ => {}
        }
    }
    if assistant_turns == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Pi output contains no final assistant message_end event",
        ));
    }
    if !starts.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Pi output contains incomplete tool executions",
        ));
    }
    metrics.assistant_turns = Some(assistant_turns);
    metrics.tool_or_command_calls = Some(
        u64::try_from(entries.iter().filter(|entry| entry.kind == "tool").count())
            .expect("entry count fits u64"),
    );
    Ok(Transcript {
        schema_version: SCHEMA_VERSION,
        adapter: "pi".to_owned(),
        final_text,
        entries,
        metrics,
    })
}

fn content_text(content: Option<&Value>) -> String {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn add_pi_usage(metrics: &mut Metrics, usage: Option<&Value>) {
    let Some(usage) = usage else { return };
    add(&mut metrics.input_tokens, integer(usage, "input"));
    add(&mut metrics.output_tokens, integer(usage, "output"));
    add(
        &mut metrics.cached_input_tokens,
        integer(usage, "cacheRead"),
    );
    add(
        &mut metrics.cache_write_input_tokens,
        integer(usage, "cacheWrite"),
    );
    add(&mut metrics.reasoning_tokens, integer(usage, "reasoning"));
    add(&mut metrics.total_tokens, integer(usage, "totalTokens"));
    let cost = usage
        .get("cost")
        .and_then(|cost| cost.get("total"))
        .and_then(Value::as_f64);
    if let Some(cost) = cost {
        metrics.cost_usd = Some(metrics.cost_usd.unwrap_or(0.0) + cost);
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
    fn fixture_ignores_unknown_events_and_counts_final_usage_once() {
        let transcript = normalize(include_bytes!("../tests/fixtures/pi-events.jsonl")).unwrap();
        assert_eq!(transcript.adapter, "pi");
        assert_eq!(transcript.metrics.assistant_turns, Some(2));
        assert_eq!(transcript.metrics.tool_or_command_calls, Some(1));
        assert_eq!(transcript.metrics.input_tokens, Some(15));
        assert_eq!(transcript.metrics.reasoning_tokens, Some(3));
        assert_eq!(transcript.metrics.cost_usd, Some(0.25));
        assert_eq!(transcript.final_text.as_deref(), Some("done"));
    }

    #[test]
    fn malformed_or_incomplete_protocol_is_rejected() {
        assert!(normalize(b"not-json\n").is_err());
        assert!(normalize(br#"{"type":"unknown"}"#).is_err());
    }
}
