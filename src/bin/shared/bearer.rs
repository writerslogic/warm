// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! `Authorization: Bearer` parsing shared by `warm-server` and `warm-mcp`.

use axum::http::{header, HeaderMap};

/// The credential of the request's only `Authorization` header when its scheme is `Bearer`
/// (any case), with leading spaces trimmed. A repeated header, a value that is not visible
/// ASCII, or another scheme gives `None`. The token itself is not validated here.
pub(crate) fn bearer_value(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim_start_matches(' '))
}
