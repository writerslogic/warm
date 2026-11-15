// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Input tree type, the accessibility-tree text parser, page templates and action labels.

use serde::{Deserialize, Serialize};

/// One node of an observation tree. `handle` names the element for actions and is excluded
/// from identity; `props` carries state such as `checked` or `value='x'` and is part of the
/// content identity only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeNode {
    pub role: String,
    pub name: String,
    pub props: String,
    pub handle: Option<String>,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn new(role: &str, name: &str) -> Self {
        Self {
            role: role.into(),
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn with_children(mut self, children: Vec<TreeNode>) -> Self {
        self.children = children;
        self
    }
}

/// An action that led to an observation: a resolved label, or a call such as `click('12')`
/// whose first argument is resolved to `role 'name'` through the previous observation of the
/// same episode in this process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Label(String),
    Call(String),
}

pub(crate) fn trunc(s: &str, chars: usize) -> &str {
    match s.char_indices().nth(chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

pub(crate) fn char_len(s: &str) -> usize {
    s.chars().count()
}

pub(crate) fn label(role: &str, name: &str) -> String {
    if name.is_empty() {
        role.to_string()
    } else {
        format!("{role} '{}'", trunc(name, 80))
    }
}

const SKIP_ROLES: [&str; 12] = [
    "generic",
    "list",
    "listitem",
    "none",
    "LabelText",
    "group",
    "presentation",
    "separator",
    "paragraph",
    "Section",
    "DescriptionList",
    "rowgroup",
];

pub(crate) fn is_skip_role(role: &str) -> bool {
    SKIP_ROLES.contains(&role) || role == "cell_group"
}

pub(crate) fn own_size(role: &str, name: &str) -> u32 {
    if !name.is_empty() || !is_skip_role(role) {
        (char_len(name) + char_len(role) + 4).min(u32::MAX as usize) as u32
    } else {
        0
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The state tokens of the remainder of a line: `value='..'`, `checked[=x]`, `disabled`,
/// `required`, `selected`, `expanded=x`, `pressed=x`, `level=n`, in order of appearance.
fn state_tokens(rest: &str) -> String {
    let chars: Vec<char> = rest.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let word_at = |i: usize, w: &str| -> Option<usize> {
        let wl: Vec<char> = w.chars().collect();
        (chars.len() >= i + wl.len() && chars[i..i + wl.len()] == wl[..]).then_some(i + wl.len())
    };
    let run = |mut j: usize, digits: bool| -> usize {
        while j < chars.len()
            && if digits {
                chars[j].is_ascii_digit()
            } else {
                is_word(chars[j])
            }
        {
            j += 1;
        }
        j
    };
    let mut i = 0;
    while i < chars.len() {
        if let Some(mut j) = word_at(i, "value='") {
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    j += 2;
                } else if chars[j] == '\'' {
                    closed = true;
                    j += 1;
                    break;
                } else {
                    j += 1;
                }
            }
            if closed {
                out.push(chars[i..j].iter().collect());
                i = j;
                continue;
            }
        }
        let boundary = i == 0 || !is_word(chars[i - 1]);
        if boundary {
            let end = |j: usize| j >= chars.len() || !is_word(chars[j]);
            let mut hit = None;
            if let Some(j) = word_at(i, "checked") {
                let e = if j < chars.len() && chars[j] == '=' && run(j + 1, false) > j + 1 {
                    run(j + 1, false)
                } else {
                    j
                };
                hit = Some(e).filter(|&e| end(e));
            }
            for w in ["disabled", "required", "selected"] {
                if hit.is_none() {
                    hit = word_at(i, w).filter(|&j| end(j));
                }
            }
            for (w, digits) in [("expanded=", false), ("pressed=", false), ("level=", true)] {
                if hit.is_none() {
                    if let Some(j) = word_at(i, w) {
                        let e = run(j, digits);
                        if e > j && end(e) {
                            hit = Some(e);
                        }
                    }
                }
            }
            if let Some(e) = hit {
                out.push(chars[i..e].iter().collect());
                i = e;
                continue;
            }
        }
        i += 1;
    }
    out.join(" ")
}

/// Parses the indented accessibility-tree text (one node per line, a tab per level, an
/// optional `[handle]`, a role, an optional quoted name, then state tokens). Lines that do not
/// parse are skipped; returns `None` when no line parses.
pub fn parse_indented(text: &str) -> Option<TreeNode> {
    let mut root: Option<TreeNode> = None;
    // Path of child indices from the root to the node most recently added.
    let mut stack: Vec<usize> = Vec::new();
    fn at<'a>(root: &'a mut TreeNode, path: &[usize]) -> &'a mut TreeNode {
        let mut n = root;
        for &i in path {
            n = &mut n.children[i];
        }
        n
    }
    for line in text.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let depth = line.chars().take_while(|&c| c == '\t').count();
        let mut s = &line[depth..];
        let mut handle = None;
        if let Some(r) = s.strip_prefix('[') {
            if let Some(end) = r.find(']') {
                let digits = &r[..end];
                let after = &r[end + 1..];
                if !digits.is_empty()
                    && digits.bytes().all(|b| b.is_ascii_digit())
                    && after.starts_with(char::is_whitespace)
                {
                    handle = Some(digits.to_string());
                    s = after.trim_start();
                }
            }
        }
        let role_end = s.find(char::is_whitespace).unwrap_or(s.len());
        let role = &s[..role_end];
        if role.is_empty() {
            continue;
        }
        let mut rest = &s[role_end..];
        let mut name = String::new();
        if let Some(r) = rest.strip_prefix(" '") {
            let mut it = r.char_indices();
            let mut close = None;
            while let Some((i, c)) = it.next() {
                if c == '\\' {
                    it.next();
                } else if c == '\'' {
                    close = Some(i);
                    break;
                }
            }
            if let Some(i) = close {
                name = r[..i].replace("\\'", "'");
                rest = &r[i + 1..];
            }
        }
        let node = TreeNode {
            role: role.to_string(),
            name,
            props: state_tokens(rest),
            handle,
            children: Vec::new(),
        };
        while stack.len() + 1 > depth && !stack.is_empty() {
            stack.pop();
        }
        match root.as_mut() {
            None => {
                root = Some(node);
                stack.clear();
            }
            Some(r) => {
                let parent = at(r, &stack);
                parent.children.push(node);
                let idx = parent.children.len() - 1;
                stack.push(idx);
            }
        }
    }
    root
}

fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && b.len() >= i + 3 {
            let h = |c: u8| (c as char).to_digit(16);
            if let (Some(x), Some(y)) = (h(b[i + 1]), h(b[i + 2])) {
                out.push((x * 16 + y) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn replace_runs(s: &str, hex_min: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let mut j = i;
        while j < c.len() && (c[j].is_ascii_digit() || ('a'..='f').contains(&c[j])) {
            j += 1;
        }
        if j - i >= hex_min {
            out.push_str("<h>");
            i = j;
            continue;
        }
        if c[i].is_ascii_digit() {
            while i < c.len() && c[i].is_ascii_digit() {
                i += 1;
            }
            out.push('#');
            continue;
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// A URL with scheme and host removed, query values blanked, hex runs and numbers masked.
pub(crate) fn url_template(url: &str) -> String {
    let mut u = unquote(url);
    for scheme in ["https://", "http://"] {
        if let Some(r) = u.strip_prefix(scheme) {
            u = r.find('/').map_or(String::new(), |i| r[i..].to_string());
            break;
        }
    }
    let mut blanked = String::new();
    let mut chars = u.chars().peekable();
    while let Some(c) = chars.next() {
        blanked.push(c);
        if c == '=' {
            while chars.peek().is_some_and(|&n| n != '&' && n != '?') {
                chars.next();
            }
        }
    }
    trunc(&replace_runs(&blanked, 6), 140).to_string()
}

pub(crate) fn title_template(title: &str) -> String {
    trunc(&replace_runs(title, 8), 90).to_string()
}

fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut o = String::new();
    o.push(quote);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if c == quote => {
                o.push('\\');
                o.push(c)
            }
            c => o.push(c),
        }
    }
    o.push(quote);
    o
}

#[derive(Clone, Debug)]
enum Arg {
    Str(String),
    Lit(String),
}

impl Arg {
    fn text(&self) -> &str {
        match self {
            Arg::Str(s) | Arg::Lit(s) => s,
        }
    }
    fn repr(&self) -> String {
        match self {
            Arg::Str(s) => py_repr_str(s),
            Arg::Lit(s) => s.clone(),
        }
    }
}

fn parse_args(inner: &str) -> Option<Vec<Arg>> {
    let c: Vec<char> = inner.chars().collect();
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if i >= c.len() {
            return Some(args);
        }
        if c[i] == '\'' || c[i] == '"' {
            let q = c[i];
            i += 1;
            let mut s = String::new();
            loop {
                let ch = *c.get(i)?;
                i += 1;
                if ch == '\\' {
                    let e = *c.get(i)?;
                    i += 1;
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        e => e,
                    });
                } else if ch == q {
                    break;
                } else {
                    s.push(ch);
                }
            }
            args.push(Arg::Str(s));
        } else {
            let start = i;
            while i < c.len() && c[i] != ',' {
                i += 1;
            }
            let lit: String = c[start..i].iter().collect::<String>().trim().to_string();
            let ok =
                lit.parse::<f64>().is_ok() || matches!(lit.as_str(), "True" | "False" | "None");
            if !ok {
                return None;
            }
            args.push(Arg::Lit(lit));
        }
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if i < c.len() {
            if c[i] != ',' {
                return None;
            }
            i += 1;
        }
    }
}

fn parse_action(a: &str) -> (String, Vec<Arg>) {
    let t = a.trim_end();
    let name_len = t
        .char_indices()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    if name_len == 0 || !t[name_len..].starts_with('(') || !t.ends_with(')') {
        return (a.to_string(), Vec::new());
    }
    let inner = &t[name_len + 1..t.len() - 1];
    let args = parse_args(inner).unwrap_or_else(|| vec![Arg::Str(inner.to_string())]);
    (t[..name_len].to_string(), args)
}

fn args_repr(args: &[Arg]) -> String {
    match args {
        [] => "()".into(),
        [a] => format!("({},)", a.repr()),
        _ => format!(
            "({})",
            args.iter().map(Arg::repr).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// The action's label, resolving a handle through `handles` (handle to `role 'name'`).
pub(crate) fn action_label(a: &str, handles: &dyn Fn(&str) -> Option<String>) -> String {
    let (f, args) = parse_action(a);
    const TARGETED: [&str; 11] = [
        "click",
        "hover",
        "fill",
        "select_option",
        "dblclick",
        "press",
        "focus",
        "check",
        "uncheck",
        "clear",
        "upload_file",
    ];
    if (TARGETED.contains(&f.as_str()) || f == "drag_and_drop") && !args.is_empty() {
        let target = handles(args[0].text()).unwrap_or_else(|| format!("bid {}", args[0].text()));
        let extra = if (f == "fill" || f == "select_option") && args.len() > 1 {
            format!(" <- '{}'", trunc(args[1].text(), 60))
        } else {
            String::new()
        };
        return format!("{f} {target}{extra}");
    }
    if f == "scroll" {
        return "scroll".into();
    }
    if f == "keyboard_press" && !args.is_empty() {
        return format!("press key {}", args[0].text());
    }
    if f == "goto" && !args.is_empty() {
        return format!("goto {}", url_template(args[0].text()));
    }
    if f.is_empty() {
        "noop".into()
    } else {
        format!("{f}{}", trunc(&args_repr(&args), 60))
    }
}

/// Whether a line reads as an error message.
pub(crate) fn is_error_line(line: &str) -> bool {
    const PHRASES: [&str; 12] = [
        "error",
        "invalid",
        "not allowed",
        "denied",
        "failed",
        "failure",
        "cannot",
        "unable",
        "forbidden",
        "exception",
        "required field",
        "not found",
    ];
    let lower = line.to_lowercase();
    let c: Vec<char> = lower.chars().collect();
    let hit = |p: &str| {
        let pc: Vec<char> = p.chars().collect();
        if pc.len() > c.len() {
            return false;
        }
        (0..=c.len() - pc.len()).any(|i| {
            c[i..i + pc.len()] == pc[..]
                && (i == 0 || !is_word(c[i - 1]))
                && (i + pc.len() == c.len() || !is_word(c[i + pc.len()]))
        })
    };
    PHRASES.iter().any(|p| hit(p)) || hit("no results")
}
