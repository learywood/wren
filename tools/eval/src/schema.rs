use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    EvaluatorValidation,
    BehavioralEvaluation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Pass,
    TaskFailure,
    InfrastructureFailure,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    Setup,
    ExecutableResolution,
    VersionResolution,
    ProcessFailure,
    Timeout,
    TreeCleanup,
    HarnessExit,
    ProtocolParse,
    ProtocolIncomplete,
    VerifierLaunch,
    VerifierTimeout,
    VerifierProtocol,
    Artifacts,
    Cleanup,
    VerificationFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub code: ReasonCode,
    pub message: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub assistant_turns: Option<u64>,
    pub tool_or_command_calls: Option<u64>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessProcess {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub tree_cleanup: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierResult {
    pub exit_code: Option<i32>,
    pub passed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptResult {
    pub schema_version: u32,
    pub index: u32,
    pub harness: String,
    pub classification: Classification,
    pub failure: Option<Failure>,
    pub started_unix_ms: u64,
    pub duration_ms: u64,
    pub harness_process: HarnessProcess,
    pub verifier: VerifierResult,
    pub metrics: Metrics,
    pub artifacts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub evidence_kind: EvidenceKind,
    pub started_unix_ms: u64,
    pub duration_ms: u64,
    pub task_id: String,
    pub task_version: u32,
    pub task_manifest_hash: String,
    pub harness: HarnessSummary,
    pub requested_attempts: u32,
    pub completed_attempts: u32,
    pub pass_count: u32,
    pub task_failure_count: u32,
    pub infrastructure_failure_count: u32,
    pub observed_pass_rate: f64,
    pub metrics: Metrics,
    pub attempt_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessSummary {
    pub kind: String,
    pub profile: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub authentication: Option<String>,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub config_hash: Option<String>,
    pub permissions: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transcript {
    pub schema_version: u32,
    pub adapter: String,
    pub final_text: Option<String>,
    pub entries: Vec<TranscriptEntry>,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptEntry {
    pub kind: String,
    pub name: Option<String>,
    pub call_id: Option<String>,
    pub text: Option<String>,
    pub arguments: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<bool>,
}

pub fn classify(
    infrastructure_failure: Option<Failure>,
    verifier_passed: Option<bool>,
) -> (Classification, Option<Failure>) {
    if let Some(failure) = infrastructure_failure {
        return (Classification::InfrastructureFailure, Some(failure));
    }
    if verifier_passed == Some(true) {
        return (Classification::Pass, None);
    }
    (
        Classification::TaskFailure,
        Some(Failure {
            code: ReasonCode::VerificationFailed,
            message: "workspace did not match the expected tree".to_owned(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infrastructure_failure_takes_precedence_over_verifier_result() {
        let failure = Failure {
            code: ReasonCode::Timeout,
            message: "timed out".to_owned(),
        };
        assert_eq!(
            classify(Some(failure.clone()), Some(true)),
            (Classification::InfrastructureFailure, Some(failure))
        );
    }

    #[test]
    fn verifier_decides_pass_or_task_failure_after_sound_infrastructure() {
        assert_eq!(classify(None, Some(true)), (Classification::Pass, None));
        assert_eq!(classify(None, Some(false)).0, Classification::TaskFailure);
    }
}
