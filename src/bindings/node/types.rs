// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use napi_derive::napi;

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 15)] as char);
    }
    encoded
}

#[napi(object)]
pub struct AuditEntryJs {
    pub timestamp_ms: f64,
    pub op: String,
    pub id_hash: String,
    pub signed: bool,
    pub effect_id: Option<String>,
    pub signature: String,
    pub signable_bytes: String,
}

impl From<crate::AuditEntry> for AuditEntryJs {
    fn from(entry: crate::AuditEntry) -> Self {
        Self {
            timestamp_ms: entry.timestamp_ms as f64,
            op: match entry.op {
                crate::AuditOp::Memorize => "memorize",
                crate::AuditOp::Delete => "delete",
                crate::AuditOp::Compact => "compact",
            }
            .into(),
            id_hash: hex_encode(&entry.id_hash),
            signed: entry.signature != [0; 64],
            effect_id: entry.effect_id.map(|id| hex_encode(&id)),
            signature: hex_encode(&entry.signature),
            signable_bytes: hex_encode(&entry.signable_bytes()),
        }
    }
}

/// A stored document as `listDocuments` returns it: identity and provenance, not text.
#[napi(object)]
pub struct ListedDocument {
    pub id: String,
    pub version: String,
    pub source_uri: Option<String>,
    pub metadata: serde_json::Value,
    pub chunk_count: u32,
}

impl From<crate::core::documents::DocumentInfo> for ListedDocument {
    fn from(record: crate::core::documents::DocumentInfo) -> Self {
        Self {
            id: record.id,
            version: record.version,
            source_uri: record.source_uri,
            metadata: record.metadata,
            chunk_count: record.chunk_count as u32,
        }
    }
}

#[napi(object)]
pub struct StructuralResultJs {
    pub entity_id: String,
    pub confidence: f64,
    pub path: String,
}

#[napi(object)]
pub struct MultiHopResultJs {
    pub entity_id: String,
    pub confidence: f64,
    pub method: String,
}

#[napi(object)]
pub struct CleanupResultJs {
    pub id: String,
    pub confidence: f64,
}

#[napi(object)]
pub struct GovernanceReportJs {
    pub composites_merged: u32,
    pub composites_forgotten: u32,
    pub atoms_forgotten: u32,
    pub idf_refreshed: bool,
    pub atoms_refined: u32,
}

#[napi(object)]
pub struct GoalJs {
    pub name: String,
    pub utility: f64,
}

#[napi(object)]
pub struct PlannedActionJs {
    pub subject: String,
    pub relation: String,
    pub object: String,
    pub depth: u32,
    pub cost: f64,
}

#[napi(object)]
pub struct PlanJs {
    pub goal: String,
    pub actions: Vec<PlannedActionJs>,
    pub complete: bool,
    pub total_cost: f64,
}

#[napi(object)]
pub struct QuestionJs {
    pub text: String,
    pub goal_relevance: f64,
}
