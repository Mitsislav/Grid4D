use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::*;

/// The Rust sources under `dir`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The string literal starting at `text[at..]` (after optional whitespace),
/// unescaped; `None` if there is none.
fn literal(text: &str, at: usize) -> Option<String> {
    let rest = text[at..].trim_start();
    let body = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                // A line continuation: the line break and the next line's indent go.
                '\n' => {
                    while chars.clone().next().is_some_and(char::is_whitespace) {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// Every string literal in `text[start..end]`.
fn literals_in(text: &str, start: usize, end: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut at = start;
    while let Some(offset) = text[at..end].find('"') {
        let quote = at + offset;
        let Some(value) = literal(text, quote) else { break };
        // Skip the literal and its quotes (escapes make it longer, never shorter).
        let mut close = quote + 1;
        let bytes = text.as_bytes();
        while close < end && bytes[close] != b'"' {
            close += if bytes[close] == b'\\' { 2 } else { 1 };
        }
        found.push(value);
        at = close + 1;
    }
    found
}

/// Where a file's test module starts (its end without one).
fn test_module(text: &str) -> usize {
    let mut from = 0;
    while let Some(found) = text[from..].find("#[cfg(test)]") {
        let at = from + found;
        let rest = text[at + "#[cfg(test)]".len()..].trim_start();
        if rest.starts_with("mod tests") || rest.starts_with("pub(crate) mod tests") {
            return at;
        }
        from = at + 1;
    }
    text.len()
}

/// The index of the brace closing the one at `open`.
fn closing(text: &str, open: usize, (left, right): (u8, u8)) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut i = open;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' {
                i += 1;
            } else if b == b'"' {
                in_string = false;
            }
        } else if b == b'"' {
            in_string = true;
        } else if b == left {
            depth += 1;
        } else if b == right {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Texts the interface shows, found in the sources: `tr("…")`, `trf("…")`,
/// `key("…")`, the label argument of the label-drawing helpers, undo step
/// and alert names, and every literal in the bodies of `fn label`, `fn hint`,
/// `fn title` and `fn window_label` (here and in the crates whose labels the
/// interface shows).
fn shown_texts() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut ui = Vec::new();
    sources(&root.join("src"), &mut ui);
    let mut others = Vec::new();
    for crate_dir in ["../scene/src", "../render/src", "../geom/src"] {
        sources(&root.join(crate_dir), &mut others);
    }
    let mut plugins = Vec::new();
    sources(&root.join("../../plugins"), &mut plugins);
    let mut texts = BTreeSet::new();
    let label_fns = ["fn label(", "fn hint(", "fn title(", "fn window_label("];
    for (path, is_ui) in ui.iter().map(|p| (p, true)).chain(others.iter().map(|p| (p, false))) {
        if path.components().any(|c| c.as_os_str() == "i18n") {
            continue;
        }
        let Ok(full) = std::fs::read_to_string(path) else {
            continue;
        };
        let text = &full[..test_module(&full)];
        for name in label_fns {
            let mut from = 0;
            while let Some(found) = text[from..].find(name) {
                let start = from + found;
                let Some(open) = text[start..].find('{').map(|o| start + o) else {
                    break;
                };
                let end = closing(text, open, (b'{', b'}'));
                texts.extend(literals_in(text, open, end));
                from = end;
            }
        }
        if !is_ui {
            continue;
        }
        // Calls whose first literal argument is shown.
        let calls = [
            "tr(",
            "trf(",
            "key(",
            "attr_label(",
            "section_heading(",
            "small_label(",
            "small(",
            "command(",
            "planned(",
            "icon_button(",
            "_row(",
            "commit(",
            "edit_textures(",
            "push_change(",
            "record(",
            "record_as_is(",
            "status_hint = Some(",
        ];
        for call in calls {
            let mut from = 0;
            while let Some(found) = text[from..].find(call) {
                let open = from + found + call.len() - 1;
                from = open + 1;
                // A whole word: `tr(` but not `str(`, `attr(`.
                let before = text[..from - call.len()].chars().next_back();
                if call.chars().next().is_some_and(char::is_alphabetic)
                    && before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    && call != "_row("
                {
                    continue;
                }
                let end = closing(text, open, (b'(', b')'));
                if let Some(first) = literals_in(text, open, end).into_iter().next() {
                    texts.insert(first);
                }
            }
        }
        // `label: "…"` (undo steps) and `title: "…"` (alerts).
        for field in ["label: \"", "title: \""] {
            let mut from = 0;
            while let Some(found) = text[from..].find(field) {
                let quote = from + found + field.len() - 1;
                if let Some(value) = literal(text, quote) {
                    texts.insert(value);
                }
                from = quote + 1;
            }
        }
    }
    // Plugins: their names, descriptions, parameter names, hints, choices and
    // commands' messages (`Err(…)`).
    for path in &plugins {
        let Ok(full) = std::fs::read_to_string(path) else {
            continue;
        };
        let text = &full[..test_module(&full)];
        for name in ["fn name(", "fn description("] {
            let mut from = 0;
            while let Some(found) = text[from..].find(name) {
                let start = from + found;
                let Some(open) = text[start..].find('{').map(|o| start + o) else {
                    break;
                };
                let end = closing(text, open, (b'{', b'}'));
                texts.extend(literals_in(text, open, end));
                from = end;
            }
        }
        for field in ["name: \"", "hint: \""] {
            let mut from = 0;
            while let Some(found) = text[from..].find(field) {
                let quote = from + found + field.len() - 1;
                if let Some(value) = literal(text, quote) {
                    texts.insert(value);
                }
                from = quote + 1;
            }
        }
        let mut from = 0;
        while let Some(found) = text[from..].find("Choice(&[") {
            let open = from + found + "Choice(&".len();
            let end = closing(text, open, (b'[', b']'));
            texts.extend(literals_in(text, open, end));
            from = end;
        }
        let mut from = 0;
        while let Some(found) = text[from..].find("Err(") {
            let open = from + found + "Err".len();
            let end = closing(text, open, (b'(', b')'));
            texts.extend(literals_in(text, open, end));
            from = end;
        }
    }
    texts.retain(|t| t.chars().filter(|c| c.is_alphabetic()).count() >= 2 && !t.starts_with("grid4d."));
    texts
}

fn placeholders(text: &str) -> BTreeSet<String> {
    (0..10)
        .map(|i| format!("{{{i}}}"))
        .filter(|p| text.contains(p.as_str()))
        .collect()
}

#[test]
fn the_greek_table_is_sorted_and_keeps_its_placeholders() {
    for pair in el::TABLE.windows(2) {
        assert!(
            pair[0].0 < pair[1].0,
            "not sorted or twice: {:?} / {:?}",
            pair[0].0,
            pair[1].0
        );
    }
    for (english, greek) in el::TABLE {
        assert_eq!(placeholders(english), placeholders(greek), "{english:?} → {greek:?}");
        assert!(!greek.trim().is_empty(), "{english:?} has no Greek");
    }
}

#[test]
fn every_text_the_interface_shows_has_greek_and_none_is_left_over() {
    let shown = shown_texts();
    let missing: Vec<&String> = shown.iter().filter(|t| el::lookup(t).is_none()).collect();
    if !missing.is_empty() {
        let out = std::env::temp_dir().join("grid4d-missing-greek.txt");
        let list: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
        let _ = std::fs::write(&out, list.join("\n\u{1}\n"));
        panic!("{} texts without Greek (listed in {})", missing.len(), out.display());
    }
    let unused: Vec<&str> = el::TABLE
        .iter()
        .map(|&(english, _)| english)
        .filter(|english| !shown.contains(*english))
        .collect();
    assert!(unused.is_empty(), "Greek for texts no code shows: {unused:?}");
}

#[test]
fn english_by_default_greek_on_request_and_placeholders_filled() {
    assert_eq!(Language::default(), Language::English);
    assert_eq!(tr("Delete"), "Delete");
    assert_eq!(trf("{0} objects · {1} selected", &[&3, &1]), "3 objects · 1 selected");
    assert_eq!(tr("no such text"), "no such text");
    assert_eq!(el::lookup("Delete"), Some("Διαγραφή"));
    assert_eq!(el::lookup("no such text"), None);
}
