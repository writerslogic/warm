// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Native metadata filters: exact equality, `$eq` and `$in`, with list-valued fields matching
//! when any element matches. Any other operator is rejected rather than matching nothing.

use anyhow::{bail, ensure, Result};
use serde_json::Value;

/// Operators a metadata filter accepts.
pub const SUPPORTED_FILTER_OPERATORS: &[&str] = &["$eq", "$in"];
const MAX_IN_VALUES: usize = 1024;

/// A filter used an operator the engine does not evaluate.
#[derive(Debug, Clone)]
pub struct UnsupportedFilterOperator {
    pub operator: String,
    pub supported: &'static [&'static str],
}

impl std::fmt::Display for UnsupportedFilterOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unsupported metadata filter operator {:?}; supported operators: {}",
            self.operator,
            if self.supported.is_empty() {
                "none (scalar equality only)".to_owned()
            } else {
                self.supported.join(", ")
            }
        )
    }
}

impl std::error::Error for UnsupportedFilterOperator {}

/// The error for `operator`, naming the operators `supported` accepts.
pub(crate) fn unsupported_operator(
    operator: &str,
    supported: &'static [&'static str],
) -> anyhow::Error {
    anyhow::Error::new(UnsupportedFilterOperator {
        operator: operator.to_owned(),
        supported,
    })
}

fn unsupported(operator: &str) -> anyhow::Error {
    unsupported_operator(operator, SUPPORTED_FILTER_OPERATORS)
}

pub(crate) fn operators(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value
        .as_object()
        .filter(|object| object.keys().any(|key| key.starts_with('$')))
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

/// Checks a filter once, before any document is evaluated.
pub fn validate_filter(filter: Option<&Value>) -> Result<()> {
    let Some(filter) = filter else {
        return Ok(());
    };
    let Some(filter) = filter.as_object() else {
        bail!("filter must be a JSON object of metadata terms");
    };
    for (key, term) in filter {
        if key.starts_with('$') {
            return Err(unsupported(key));
        }
        let Some(ops) = operators(term) else {
            continue;
        };
        for (op, operand) in ops {
            match op.as_str() {
                "$eq" => {}
                "$in" => {
                    let Some(values) = operand.as_array() else {
                        bail!("$in on {key:?} requires an array");
                    };
                    ensure!(
                        values.len() <= MAX_IN_VALUES && values.iter().all(is_scalar),
                        "$in on {key:?} takes at most {MAX_IN_VALUES} scalar values"
                    );
                }
                _ => return Err(unsupported(op)),
            }
        }
        ensure!(
            ops.keys().all(|op| op.starts_with('$')),
            "filter term {key:?} mixes operators with plain keys"
        );
    }
    Ok(())
}

/// A stored value equals `expected`, or is a list holding an element that does.
fn equals(stored: &Value, expected: &Value) -> bool {
    stored == expected
        || stored
            .as_array()
            .is_some_and(|items| !expected.is_array() && items.iter().any(|item| item == expected))
}

fn term_matches(stored: Option<&Value>, term: &Value) -> bool {
    let Some(stored) = stored else {
        return false;
    };
    let Some(ops) = operators(term) else {
        return equals(stored, term);
    };
    ops.iter().all(|(op, operand)| match op.as_str() {
        "$eq" => equals(stored, operand),
        "$in" => operand
            .as_array()
            .is_some_and(|set| set.iter().any(|value| equals(stored, value))),
        _ => false,
    })
}

/// Whether `metadata` satisfies every term of an already validated `filter`.
pub(crate) fn matches_metadata(metadata: &Value, filter: Option<&Value>) -> bool {
    filter.and_then(Value::as_object).is_none_or(|filter| {
        filter
            .iter()
            .all(|(key, term)| term_matches(metadata.get(key), term))
    })
}

/// The scalar a term compares by equality, which the metadata index can narrow by.
pub(crate) fn indexed_equality(term: &Value) -> Option<&Value> {
    match operators(term) {
        None => Some(term),
        Some(ops) if ops.len() == 1 => ops.get("$eq"),
        Some(_) => None,
    }
    .filter(|value| is_scalar(value))
}
