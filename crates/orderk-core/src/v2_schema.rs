use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const V2_SOURCE_SCHEMA_VERSION: &str = "orderk.v2.source.v1";
pub const V2_CHUNK_SCHEMA_VERSION: &str = "orderk.v2.chunk.v1";
pub const V2_WIKI_OBJECT_SCHEMA_VERSION: &str = "orderk.v2.wiki_object.v1";
pub const V2_EDGE_SCHEMA_VERSION: &str = "orderk.v2.edge.v1";
pub const V2_PROPOSAL_SCHEMA_VERSION: &str = "orderk.v2.proposal.v1";
pub const V2_AUDIT_SCHEMA_VERSION: &str = "orderk.v2.audit.v1";
pub const V2_PROFILE_SCHEMA_VERSION: &str = "orderk.v2.profile.v1";
pub const V2_SEARCH_CONTRACT_SCHEMA_VERSION: &str = "orderk.v2.search_contract.v1";
pub const V2_GATE_RESULT_SCHEMA_VERSION: &str = "orderk.v2.gate_result.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V2Relation {
    Supports,
    Refines,
    Contradicts,
    Replaces,
    DependsOn,
    PartOf,
}

impl V2Relation {
    pub const ALLOWED: [&'static str; 6] = [
        "supports",
        "refines",
        "contradicts",
        "replaces",
        "depends_on",
        "part_of",
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V2EdgeStatus {
    Proposal,
    Active,
    Rejected,
    Superseded,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V2WikiObjectKind {
    Concept,
    Entity,
    Claim,
    Decision,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FallbackInvocation {
    NotCalled,
    Called,
    CalledUnparseableFallback,
    CalledFailedDegraded,
    CalledTimeoutDegraded,
    NotCalledNoCandidates,
    NotUsed,
}

impl FallbackInvocation {
    pub const ALLOWED: [&'static str; 7] = [
        "not_called",
        "called",
        "called_unparseable_fallback",
        "called_failed_degraded",
        "called_timeout_degraded",
        "not_called_no_candidates",
        "not_used",
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V2TraceLevel {
    Off,
    Compact,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V2GateState {
    Pass,
    Fail,
    Blocked,
    NeedsManual,
    AdvisoryWarn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V2GateSeverity {
    Hard,
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2NodeRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl V2NodeRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: None,
        }
    }

    pub fn with_path(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: Some(path.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2EvidenceRef {
    pub source_path: String,
    pub chunk_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<(usize, usize)>,
}

impl V2EvidenceRef {
    pub fn new(source_path: impl Into<String>, chunk_id: impl Into<String>) -> Self {
        Self {
            source_path: source_path.into(),
            chunk_id: chunk_id.into(),
            quote_hash: None,
            line_range: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2Source {
    pub schema_version: String,
    pub path: String,
    pub hash: String,
    pub size_bytes: u64,
    pub mtime: i64,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2Chunk {
    pub schema_version: String,
    pub id: String,
    pub source_path: String,
    pub source_hash: String,
    pub line_start: usize,
    pub line_end: usize,
    pub text_hash: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2WikiObject {
    pub schema_version: String,
    pub id: String,
    pub kind: V2WikiObjectKind,
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source_refs: Vec<V2EvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct V2Edge {
    pub schema_version: String,
    pub id: String,
    pub from: V2NodeRef,
    pub to: V2NodeRef,
    pub relation: V2Relation,
    pub status: V2EdgeStatus,
    pub confidence: f32,
    #[serde(default)]
    pub evidence: Vec<V2EvidenceRef>,
    pub created_at: String,
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct V2Proposal {
    pub schema_version: String,
    pub id: String,
    pub run_id: String,
    pub relation: V2Relation,
    pub from: V2NodeRef,
    pub to: V2NodeRef,
    pub confidence: f32,
    #[serde(default)]
    pub evidence: Vec<V2EvidenceRef>,
    #[serde(default)]
    pub suggested_patch: Option<Value>,
    pub status: V2EdgeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2AuditEvent {
    pub schema_version: String,
    pub id: String,
    pub run_id: String,
    pub event: String,
    pub actor: String,
    pub created_at: String,
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2ModelSlot {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dim: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct V2Profile {
    pub schema_version: String,
    pub profile_id: String,
    pub embedding: V2ModelSlot,
    pub reranker: V2ModelSlot,
    pub llm: V2ModelSlot,
    pub trace_level: V2TraceLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct V2SearchContract {
    pub schema_version: String,
    pub query: String,
    pub mode: String,
    pub reasoning_triggered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_reason: Option<String>,
    pub trace_level: V2TraceLevel,
    pub fallback_invocation: FallbackInvocation,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub metrics: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct V2GateResult {
    pub schema_version: String,
    pub gate_id: String,
    pub ok: bool,
    pub state: V2GateState,
    pub severity: V2GateSeverity,
    pub unattended_safe: bool,
    pub manual_review_required: bool,
    #[serde(default)]
    pub thresholds: Value,
    #[serde(default)]
    pub metrics: Value,
    #[serde(default)]
    pub failures: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub artifacts: Value,
    pub next_action_on_failure: String,
}

impl V2GateResult {
    pub fn pass(gate_id: impl Into<String>) -> Self {
        Self {
            schema_version: V2_GATE_RESULT_SCHEMA_VERSION.to_string(),
            gate_id: gate_id.into(),
            ok: true,
            state: V2GateState::Pass,
            severity: V2GateSeverity::Hard,
            unattended_safe: true,
            manual_review_required: false,
            thresholds: json!({}),
            metrics: json!({}),
            failures: Vec::new(),
            warnings: Vec::new(),
            artifacts: json!({}),
            next_action_on_failure: "none".to_string(),
        }
    }

    pub fn fail(gate_id: impl Into<String>, failures: Vec<String>) -> Self {
        Self {
            ok: false,
            state: V2GateState::Fail,
            failures,
            next_action_on_failure: "fix failures and rerun the gate".to_string(),
            ..Self::pass(gate_id)
        }
    }
}

#[cfg(test)]
mod v2_schema_contract_tests {
    use super::*;

    #[test]
    fn v2_schema_contract_serializes_edge_and_fallback_vocab() {
        let edge = V2Edge {
            schema_version: V2_EDGE_SCHEMA_VERSION.to_string(),
            id: "edge-1".to_string(),
            from: V2NodeRef::new("raw/a.md"),
            to: V2NodeRef::new("wiki/concepts/b.md"),
            relation: V2Relation::Supports,
            status: V2EdgeStatus::Proposal,
            confidence: 0.81,
            evidence: vec![V2EvidenceRef::new("raw/a.md", "chunk-1")],
            created_at: "2026-06-04T00:00:00Z".to_string(),
            run_id: "run-1".to_string(),
        };
        let json = serde_json::to_value(edge).unwrap();
        assert_eq!(json["schema_version"], "orderk.v2.edge.v1");
        assert_eq!(json["relation"], "supports");
        assert_eq!(json["status"], "proposal");

        let fallback = serde_json::to_value(FallbackInvocation::CalledUnparseableFallback).unwrap();
        assert_eq!(fallback, "called_unparseable_fallback");
        assert_eq!(V2Relation::ALLOWED.len(), 6);
        assert_eq!(FallbackInvocation::ALLOWED.len(), 7);
    }

    #[test]
    fn v2_search_contract_serializes_schema_trace_and_fallback_vocab() {
        let contract = V2SearchContract {
            schema_version: V2_SEARCH_CONTRACT_SCHEMA_VERSION.to_string(),
            query: "sqlite vec semantic search".to_string(),
            mode: "standard".to_string(),
            reasoning_triggered: false,
            trigger_reason: None,
            trace_level: V2TraceLevel::Compact,
            fallback_invocation: FallbackInvocation::NotCalled,
            warnings: Vec::new(),
            metrics: BTreeMap::new(),
        };
        let json = serde_json::to_value(contract).unwrap();
        assert_eq!(json["schema_version"], "orderk.v2.search_contract.v1");
        assert_eq!(json["trace_level"], "compact");
        assert_eq!(json["fallback_invocation"], "not_called");
        assert!(json.get("results").is_none());
        assert!(json.get("profile").is_none());
        assert!(json.get("latency_ms").is_none());
    }

    #[test]
    fn v2_gate_result_has_common_release_decision_envelope() {
        let gate = V2GateResult::pass("fixture_integrity");
        let json = serde_json::to_value(gate).unwrap();
        assert_eq!(json["schema_version"], "orderk.v2.gate_result.v1");
        assert_eq!(json["state"], "pass");
        assert_eq!(json["severity"], "hard");
        assert_eq!(json["manual_review_required"], false);
    }
}
