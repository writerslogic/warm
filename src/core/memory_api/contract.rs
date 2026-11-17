// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! JSON envelopes carried by `Memory::call`; payload schemas reuse the native API types.

pub use super::CapabilitiesRequest;
use schemars::JsonSchema;

use super::{
    Explanation, FactRecord, ForgetOptions, ForgetScope, RecallOptions, Recalled, RememberInput,
    Remembered,
};

#[derive(JsonSchema)]
#[serde(untagged)]
pub enum RememberRequest {
    Fact(Box<RememberInput>),
    Observation(Box<ObservationInput>),
    Batch(RememberBatch),
}

/// One item of a batch: a fact, or a step of an agent trajectory.
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum RememberItem {
    Fact(Box<RememberInput>),
    Observation(Box<ObservationInput>),
}

/// A step of an agent trajectory, stored by the experience core. Recognized by `episode`
/// (string), `step` (non-negative integer) and `tree` (object). Bounds: 128 levels, 200000 nodes,
/// 16 KiB per string; over HTTP the JSON parser's limit of 128 levels allows 63 tree levels.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationInput {
    #[schemars(length(min = 1, max = 256))]
    pub episode: String,
    pub step: u32,
    pub tree: ObservationNode,
    pub url: Option<String>,
    /// `{"label": text}` or `{"call": text}`: the action taken before this observation.
    pub action: Option<serde_json::Map<String, serde_json::Value>>,
    pub goal: Option<String>,
    pub outcome: Option<String>,
}

/// A node of an accessibility tree. `children` hold nodes of the same shape.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationNode {
    #[schemars(length(min = 1))]
    pub role: String,
    pub name: Option<String>,
    pub props: Option<String>,
    pub handle: Option<String>,
    pub children: Option<Vec<serde_json::Value>>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RememberBatch {
    #[schemars(length(min = 1))]
    pub facts: Vec<RememberItem>,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecallRequest {
    pub query: String,
    #[serde(flatten)]
    pub options: RecallOptions,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExplainRequest {
    pub query: String,
    pub id: String,
    #[serde(flatten)]
    pub options: RecallOptions,
}

#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgetRequest {
    pub scope: ForgetScope,
    #[serde(flatten)]
    pub options: ForgetOptions,
}

#[derive(JsonSchema)]
/// `get` and `history` retain their legacy acceptance of extra request fields.
pub struct IdRequest {
    pub id: String,
}

#[derive(JsonSchema)]
pub struct RememberReply {
    pub remembered: Vec<Remembered>,
}

#[derive(JsonSchema)]
pub struct RecallReply {
    pub results: Vec<Recalled>,
}

#[derive(JsonSchema)]
pub struct CapabilitiesReply {
    pub capabilities: super::MemoryCapabilities,
}

#[derive(JsonSchema)]
pub struct ExplainReply {
    pub explanation: Explanation,
}

#[derive(JsonSchema)]
pub struct GetReply {
    pub fact: FactOrNull,
}

#[derive(JsonSchema)]
#[serde(untagged)]
pub enum FactOrNull {
    Fact(Box<FactRecord>),
    Missing(()),
}

#[derive(JsonSchema)]
pub struct HistoryReply {
    pub history: Vec<FactRecord>,
}
