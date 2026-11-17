// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! Form state: the controls of one observation (text fields, selects, checkboxes, buttons) with
//! their values and selected options, parsed at ingest from the accessibility-tree text of the
//! step and rendered at query time as a compact table before the page text.
//!
//! The input is the indented text of an accessibility tree, one node per line:
//! `[id] role 'name' props`, where `props` is the rest of the line (`value='x', checked=true,
//! required`). The parser and the rendering are specified by the reference renderer of the
//! LongMemEval-V2 adapter (`benchmarks/lme2/`), and the parity test
//! (`tests/experience_form_parity.rs`) holds them to its output byte for byte. They follow its
//! character classes: whitespace is Python's `str.isspace`, word characters are letters, digits
//! and `_`, and names and values are rendered with Python's `repr`. Nothing depends on a site.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Controls kept for one step; the parser stops at this many.
pub const MAX_CONTROLS: usize = 120;
/// Options kept for one combobox or listbox; the others are counted.
pub const MAX_OPTIONS: usize = 25;

const CONTROL_ROLES: [&str; 12] = [
    "textbox",
    "searchbox",
    "combobox",
    "listbox",
    "checkbox",
    "radio",
    "switch",
    "spinbutton",
    "button",
    "tab",
    "menuitemcheckbox",
    "menuitemradio",
];

/// The header of a rendered form table.
pub const FORM_TABLE_HEADER: &str =
    "- Form controls (structured from the page text; * = selected option):\n";

/// One option of a combobox or listbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
}

/// One form control of a step. The state strings are as the page gave them (`true`, `False`),
/// with surrounding single quotes removed; a flag is `Some(false)` when the page says
/// `required=False` and `Some(true)` when it is present otherwise.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Control {
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
    /// The first [`MAX_OPTIONS`] options of a combobox or listbox, in page order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ControlOption>,
    /// Options beyond the first [`MAX_OPTIONS`].
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more_options: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Control {
    /// Bytes of text the control holds, for size accounting.
    pub(crate) fn text_bytes(&self) -> usize {
        let opt = |o: &Option<String>| o.as_ref().map_or(0, String::len);
        self.role.len()
            + self.name.len()
            + opt(&self.value)
            + opt(&self.checked)
            + opt(&self.selected)
            + opt(&self.expanded)
            + opt(&self.pressed)
            + self
                .options
                .iter()
                .map(|o| o.label.len() + 1)
                .sum::<usize>()
    }

    /// The longest string the control holds.
    pub(crate) fn longest_string(&self) -> usize {
        let opt = |o: &Option<String>| o.as_ref().map_or(0, String::len);
        [
            self.role.len(),
            self.name.len(),
            opt(&self.value),
            opt(&self.checked),
            opt(&self.selected),
            opt(&self.expanded),
            opt(&self.pressed),
        ]
        .into_iter()
        .chain(self.options.iter().map(|o| o.label.len()))
        .max()
        .unwrap_or(0)
    }

    /// The table row: `role 'name' state, state options: *selected | other | (+n more)`.
    pub fn row(&self) -> String {
        let mut state: Vec<String> = Vec::new();
        if let Some(v) = &self.value {
            state.push(format!("value={}", py_repr(v)));
        }
        for (k, v) in [
            ("checked", &self.checked),
            ("selected", &self.selected),
            ("expanded", &self.expanded),
            ("pressed", &self.pressed),
        ] {
            if let Some(v) = v {
                state.push(format!("{k}={v}"));
            }
        }
        for (k, v) in [
            ("required", self.required),
            ("disabled", self.disabled),
            ("readonly", self.readonly),
        ] {
            match v {
                Some(true) => state.push(k.to_string()),
                Some(false) => state.push(format!("not {k}")),
                None => {}
            }
        }
        let mut row = format!("{} {}", self.role, py_repr(&self.name));
        if !state.is_empty() {
            row.push(' ');
            row.push_str(&state.join(", "));
        }
        if !self.options.is_empty() {
            let labels: Vec<String> = self
                .options
                .iter()
                .map(|o| {
                    if o.selected {
                        format!("*{}", o.label)
                    } else {
                        o.label.clone()
                    }
                })
                .collect();
            row.push_str(" options: ");
            row.push_str(&labels.join(" | "));
            if self.more_options > 0 {
                row.push_str(&format!(" | (+{} more)", self.more_options));
            }
        }
        row
    }
}

/// Python's `str.isspace`.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n'
            | '\u{0b}'
            | '\u{0c}'
            | '\r'
            | '\u{1c}'..='\u{1f}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn strip_space(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Whether Python's `repr` shows the character as is: not a control, format, private-use,
/// line or paragraph separator, or space separator other than ASCII space, and not a
/// noncharacter. Other unassigned code points are shown as is, which can differ from a Python
/// whose Unicode tables know them as unassigned.
fn is_printable(c: char) -> bool {
    let u = c as u32;
    if (0x20..0x7f).contains(&u) {
        return true;
    }
    if u < 0xa0 {
        return false;
    }
    const NOT_PRINTABLE: &[(u32, u32)] = &[
        (0xa0, 0xa0),
        (0xad, 0xad),
        (0x600, 0x605),
        (0x61c, 0x61c),
        (0x6dd, 0x6dd),
        (0x70f, 0x70f),
        (0x890, 0x891),
        (0x8e2, 0x8e2),
        (0x1680, 0x1680),
        (0x180e, 0x180e),
        (0x2000, 0x200f),
        (0x2028, 0x202f),
        (0x205f, 0x206f),
        (0x3000, 0x3000),
        (0xe000, 0xf8ff),
        (0xfdd0, 0xfdef),
        (0xfeff, 0xfeff),
        (0xfff9, 0xfffb),
        (0x110bd, 0x110bd),
        (0x110cd, 0x110cd),
        (0x13430, 0x1343f),
        (0x1bca0, 0x1bca3),
        (0x1d173, 0x1d17a),
        (0xe0001, 0xe0001),
        (0xe0020, 0xe007f),
        (0xf0000, 0x10ffff),
    ];
    if u & 0xfffe == 0xfffe {
        return false;
    }
    !NOT_PRINTABLE.iter().any(|&(a, b)| (a..=b).contains(&u))
}

/// Python's `repr` of a string.
pub(crate) fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut o = String::with_capacity(s.len() + 2);
    o.push(quote);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if c == quote => {
                o.push('\\');
                o.push(c);
            }
            c if is_printable(c) => o.push(c),
            c if (c as u32) < 0x100 => o.push_str(&format!("\\x{:02x}", c as u32)),
            c if (c as u32) < 0x10000 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push_str(&format!("\\U{:08x}", c as u32)),
        }
    }
    o.push(quote);
    o
}

/// One parsed line: indentation in characters, role, stripped name and the rest of the line.
struct Line<'a> {
    depth: usize,
    role: &'a str,
    name: &'a str,
    rest: &'a str,
}

/// The end of a quoted run starting at the opening quote `at`: the byte just past the closing
/// quote. A backslash escapes the next character; `None` when the quote is not closed.
fn quoted_end(s: &str, at: usize) -> Option<usize> {
    let mut it = s[at + 1..].char_indices();
    while let Some((i, c)) = it.next() {
        match c {
            '\\' => {
                it.next()?;
            }
            '\'' => return Some(at + 1 + i + 1),
            _ => {}
        }
    }
    None
}

fn word_end(s: &str, from: usize) -> usize {
    s[from..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(s.len(), |(i, _)| from + i)
}

fn space_end(s: &str, from: usize) -> usize {
    s[from..]
        .char_indices()
        .find(|&(_, c)| !is_space(c))
        .map_or(s.len(), |(i, _)| from + i)
}

/// `^(\s*)\[(\w+)\]\s+(\w+)\s*(?:'((?:[^'\\]|\\.)*)')?(.*)$`
fn parse_line(line: &str) -> Option<Line<'_>> {
    let start = space_end(line, 0);
    let depth = line[..start].chars().count();
    let s = &line[start..];
    let r = s.strip_prefix('[')?;
    let id_end = word_end(r, 0);
    if id_end == 0 {
        return None;
    }
    let r = r[id_end..].strip_prefix(']')?;
    let role_start = space_end(r, 0);
    if role_start == 0 {
        return None;
    }
    let role_end = word_end(r, role_start);
    if role_end == role_start {
        return None;
    }
    let role = &r[role_start..role_end];
    let after = space_end(r, role_end);
    let (name, rest) = match r[after..].starts_with('\'') {
        true => match quoted_end(r, after) {
            Some(end) => (strip_space(&r[after + 1..end - 1]), &r[end..]),
            None => ("", &r[after..]),
        },
        false => ("", &r[after..]),
    };
    Some(Line {
        depth,
        role,
        name,
        rest,
    })
}

/// The `key=value` properties and bare flags of the rest of a line, as the reference reads
/// them: properties are `(\w+)=('(?:[^'\\]|\\.)*'|[^,]+)` found left to right, a later one
/// replacing an earlier one of the same key, the value stripped of single quotes at both ends;
/// flags are the comma-separated pieces without `=`, stripped of whitespace.
struct Props<'a> {
    pairs: Vec<(&'a str, &'a str)>,
    rest: &'a str,
}

impl<'a> Props<'a> {
    fn parse(rest: &'a str) -> Self {
        let mut pairs = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            let c = rest[i..].chars().next().expect("in bounds");
            if !is_word(c) {
                i += c.len_utf8();
                continue;
            }
            let key_end = word_end(rest, i);
            let value = rest[key_end..]
                .strip_prefix('=')
                .and_then(|_| Self::value(rest, key_end + 1));
            match value {
                Some(end) => {
                    let v = rest[key_end + 1..end].trim_matches('\'');
                    pairs.push((&rest[i..key_end], v));
                    i = end;
                }
                None => i = key_end,
            }
        }
        Props { pairs, rest }
    }

    /// The end of a value starting at `at`: a closed quoted run, else a nonempty run up to the
    /// next comma.
    fn value(s: &str, at: usize) -> Option<usize> {
        if s[at..].starts_with('\'') {
            if let Some(end) = quoted_end(s, at) {
                return Some(end);
            }
        }
        let end = s[at..].find(',').map_or(s.len(), |i| at + i);
        (end > at).then_some(end)
    }

    fn get(&self, key: &str) -> Option<&'a str> {
        self.pairs.iter().rev().find(|p| p.0 == key).map(|p| p.1)
    }

    fn flag(&self, key: &str) -> bool {
        self.rest
            .split(',')
            .any(|f| !f.contains('=') && strip_space(f) == key)
    }

    fn state(&self, key: &str) -> Option<bool> {
        let v = self.get(key);
        (v.is_some() || self.flag(key)).then_some(!matches!(v, Some("False" | "false")))
    }
}

fn is_true(v: Option<&str>) -> bool {
    matches!(v, Some("True" | "true"))
}

/// The controls of one accessibility-tree text, at most [`MAX_CONTROLS`]. Lines that do not
/// have the form `[id] role ...`, or are longer than `max_line_bytes`, are skipped. An unnamed
/// button with no state is not a control.
pub fn extract_controls(text: &str, max_line_bytes: usize) -> Vec<Control> {
    let lines: Vec<Line<'_>> = text
        .split('\n')
        .filter(|l| l.len() <= max_line_bytes)
        .filter_map(parse_line)
        .collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if out.len() >= MAX_CONTROLS {
            break;
        }
        if !CONTROL_ROLES.contains(&line.role) {
            continue;
        }
        let p = Props::parse(line.rest);
        let mut c = Control {
            role: line.role.to_string(),
            name: line.name.to_string(),
            value: p
                .get("value")
                .filter(|v| !v.is_empty() && *v != "None")
                .map(str::to_string),
            checked: p.get("checked").map(str::to_string),
            selected: p.get("selected").map(str::to_string),
            expanded: p.get("expanded").map(str::to_string),
            pressed: p.get("pressed").map(str::to_string),
            required: p.state("required"),
            disabled: p.state("disabled"),
            readonly: p.state("readonly"),
            ..Control::default()
        };
        if matches!(line.role, "combobox" | "listbox") {
            let mut total = 0u32;
            for child in lines[i + 1..].iter().take_while(|n| n.depth > line.depth) {
                if child.role != "option" {
                    continue;
                }
                total = total.saturating_add(1);
                if c.options.len() < MAX_OPTIONS {
                    c.options.push(ControlOption {
                        label: child.name.to_string(),
                        selected: is_true(Props::parse(child.rest).get("selected")),
                    });
                }
            }
            c.more_options = total.saturating_sub(c.options.len() as u32);
        }
        let stateless = c.value.is_none()
            && c.checked.is_none()
            && c.selected.is_none()
            && c.expanded.is_none()
            && c.pressed.is_none()
            && c.required.is_none()
            && c.disabled.is_none()
            && c.readonly.is_none();
        if c.role == "button" && c.name.is_empty() && stateless {
            continue;
        }
        out.push(c);
    }
    out
}

/// The form tables of a sequence of states, one string per state in order: the header, the
/// rows not shown for an earlier state of the sequence, and a count of the rows left out
/// because they were. A state without controls gives an empty string. At most
/// [`MAX_CONTROLS`] rows of a state are considered.
pub fn render_form_tables<'a>(states: impl IntoIterator<Item = &'a [Control]>) -> Vec<String> {
    let mut shown: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for controls in states {
        let rows: Vec<String> = controls
            .iter()
            .take(MAX_CONTROLS)
            .map(Control::row)
            .collect();
        if rows.is_empty() {
            out.push(String::new());
            continue;
        }
        let new: Vec<&String> = rows.iter().filter(|r| !shown.contains(*r)).collect();
        let mut block = String::from(FORM_TABLE_HEADER);
        block.push_str(
            &new.iter()
                .map(|r| format!("  {r}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if new.len() < rows.len() {
            block.push_str(&format!(
                "\n  [{} controls unchanged from an earlier state omitted]",
                rows.len() - new.len()
            ));
        }
        block.push('\n');
        shown.extend(rows);
        out.push(block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: usize = 16 * 1024;

    fn rows(text: &str) -> Vec<String> {
        extract_controls(text, LINE)
            .iter()
            .map(Control::row)
            .collect()
    }

    #[test]
    fn each_control_role_is_kept_and_other_roles_are_not() {
        let mut text = String::from("RootWebArea 'Form'\n");
        for (i, role) in CONTROL_ROLES.iter().enumerate() {
            text.push_str(&format!("\t[{i}] {role} 'Field {i}'\n"));
        }
        text.push_str("\t[90] link 'Home'\n\t[91] StaticText 'Label'\n\t[92] option 'Loose'\n");
        let got = rows(&text);
        let want: Vec<String> = CONTROL_ROLES
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{r} 'Field {i}'"))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn properties_and_flags_render_in_a_fixed_order() {
        let got = rows(
            "[1] textbox 'Email', readonly, required, disabled=False, value='a@b.c', pressed=true, expanded=False, selected=x, checked='true'\n\
             [2] checkbox 'Agree' checked=false\n\
             [3] textbox 'Empty' value=''\n\
             [4] textbox 'Null', value=None\n\
             [5] button '', disabled\n\
             [6] button ''\n\
             [7] button 'Go'\n\
             [8] spinbutton 'Qty', value='3', required=false",
        );
        assert_eq!(
            got,
            vec![
                "textbox 'Email' value='a@b.c', checked=true, selected=x, expanded=False, pressed=true, required, not disabled, readonly",
                "checkbox 'Agree' checked=false",
                "textbox 'Empty'",
                "textbox 'Null'",
                "button '' disabled",
                "button 'Go'",
                "spinbutton 'Qty' value='3', not required",
            ]
        );
    }

    #[test]
    fn nested_options_are_listed_with_the_selected_ones_marked() {
        let got = rows(
            "[10] combobox 'Sort by', hasPopup='listbox', expanded=True\n\
             \t[11] option 'Newest', selected=True\n\
             \t[12] group 'More'\n\
             \t\t[13] option 'Oldest', selected=False\n\
             \tStaticText 'not a node line'\n\
             \t[14] option 'Price'\n\
             [15] listbox 'Tags'\n\
             \t[16] option 'a', selected=true\n\
             \t[17] option 'b', selected=true\n\
             [18] option 'after'",
        );
        assert_eq!(
            got,
            vec![
                "combobox 'Sort by' expanded=True options: *Newest | Oldest | Price",
                "listbox 'Tags' options: *a | *b",
            ]
        );
    }

    #[test]
    fn names_and_values_render_as_python_repr() {
        let got = rows(
            "[1] textbox 'It\\'s' value='say \"hi\"'\n\
             [2] textbox 'tab\there', value='a\\\\b'\n\
             [3] textbox '\u{a0}nbsp\u{200b}' value='\u{1f600}'\n\
             [4] textbox 'He said \"x\" it\\'s'",
        );
        assert_eq!(
            got,
            vec![
                "textbox \"It\\\\'s\" value='say \"hi\"'",
                "textbox 'tab\\there' value='a\\\\\\\\b'",
                "textbox 'nbsp\\u200b' value='\u{1f600}'",
                "textbox 'He said \"x\" it\\\\\\'s'",
            ]
        );
    }

    #[test]
    fn malformed_lines_are_skipped_without_panic() {
        let inputs = [
            "",
            "\n\n",
            "[",
            "[]",
            "[1]",
            "[1] ",
            "[1]textbox 'x'",
            "[1 textbox 'x'",
            "[-] textbox 'x'",
            "textbox 'no id'",
            "[1] textbox 'unclosed",
            "[1] textbox 'trailing backslash\\",
            "[1] textbox value=",
            "[1] textbox =x, ,=, value",
            "[1] textbox '\u{e9}' value='\u{1f600}",
            "[\u{e9}] textbox 'unicode id'",
            "\u{a0}\u{3000}[2] checkbox 'deep'",
        ];
        for text in inputs {
            let _ = extract_controls(text, LINE);
        }
        assert_eq!(rows("[1] textbox 'unclosed"), vec!["textbox ''"]);
        assert_eq!(
            rows("[1] textbox value='open, x"),
            vec!["textbox '' value='open'"]
        );
        assert_eq!(rows("[1] textbox = 'n'"), vec!["textbox ''"]);
        assert_eq!(
            rows("[\u{e9}] textbox 'unicode id'"),
            vec!["textbox 'unicode id'"]
        );
        // Non-ASCII whitespace indents, as in the reference.
        assert_eq!(
            extract_controls("\u{3000}[2] checkbox 'deep'", LINE).len(),
            1
        );
    }

    #[test]
    fn lines_over_the_byte_bound_are_skipped() {
        let name = "n".repeat(100);
        let line = format!("[1] textbox '{name}'");
        assert_eq!(extract_controls(&line, line.len()).len(), 1);
        assert_eq!(extract_controls(&line, line.len() - 1).len(), 0);
    }

    fn listbox(options: usize) -> String {
        let mut t = String::from("[1] listbox 'L'\n");
        for i in 0..options {
            t.push_str(&format!("\t[{}] option 'o{i}'\n", i + 2));
        }
        t
    }

    #[test]
    fn options_cap_at_limit_minus_one_limit_and_limit_plus_one() {
        for (n, kept, more) in [
            (MAX_OPTIONS - 1, MAX_OPTIONS - 1, 0),
            (MAX_OPTIONS, MAX_OPTIONS, 0),
            (MAX_OPTIONS + 1, MAX_OPTIONS, 1),
        ] {
            let c = &extract_controls(&listbox(n), LINE)[0];
            assert_eq!(
                (c.options.len(), c.more_options),
                (kept, more as u32),
                "{n}"
            );
            assert_eq!(c.row().ends_with(" | (+1 more)"), more == 1, "{n}");
        }
    }

    fn boxes(n: usize) -> String {
        (0..n).map(|i| format!("[{i}] checkbox 'c{i}'\n")).collect()
    }

    #[test]
    fn controls_cap_at_limit_minus_one_limit_and_limit_plus_one() {
        for (n, kept) in [
            (MAX_CONTROLS - 1, MAX_CONTROLS - 1),
            (MAX_CONTROLS, MAX_CONTROLS),
            (MAX_CONTROLS + 1, MAX_CONTROLS),
        ] {
            assert_eq!(extract_controls(&boxes(n), LINE).len(), kept, "{n}");
        }
    }

    #[test]
    fn rendering_caps_rows_per_state() {
        let many: Vec<Control> = (0..MAX_CONTROLS + 1)
            .map(|i| Control {
                role: "checkbox".into(),
                name: format!("c{i}"),
                ..Control::default()
            })
            .collect();
        for n in [MAX_CONTROLS - 1, MAX_CONTROLS, MAX_CONTROLS + 1] {
            let t = render_form_tables([&many[..n]]);
            let rows = t[0].lines().count() - 1;
            assert_eq!(rows, n.min(MAX_CONTROLS), "{n}");
        }
    }

    #[test]
    fn tables_omit_rows_shown_for_an_earlier_state() {
        let a = extract_controls("[1] textbox 'q' value='x'\n[2] button 'Go'", LINE);
        let b = extract_controls("[1] textbox 'q' value='y'\n[2] button 'Go'", LINE);
        let t = render_form_tables([&a[..], &[][..], &b[..], &a[..]]);
        assert_eq!(
            t,
            vec![
                format!("{FORM_TABLE_HEADER}  textbox 'q' value='x'\n  button 'Go'\n"),
                String::new(),
                format!(
                    "{FORM_TABLE_HEADER}  textbox 'q' value='y'\n  [1 controls unchanged from an earlier state omitted]\n"
                ),
                format!(
                    "{FORM_TABLE_HEADER}\n  [2 controls unchanged from an earlier state omitted]\n"
                ),
            ]
        );
    }

    #[test]
    fn many_nested_listboxes_stay_linear_in_the_control_cap() {
        let mut t = String::new();
        for i in 0..2000 {
            t.push_str(&"\t".repeat(i % 100));
            t.push_str(&format!("[{i}] listbox 'l'\n"));
        }
        assert_eq!(extract_controls(&t, LINE).len(), MAX_CONTROLS);
    }
}
