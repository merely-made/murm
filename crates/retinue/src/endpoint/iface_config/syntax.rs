//! The ConfigObj subset RNS config files use: `[section]`, `[[subsection]]`, `key = value`,
//! `#` comments, quoted values, comma lists and ConfigObj's booleans.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::RnsConfigError;

/// One `[section]` or `[[subsection]]`: its name and keys in file order.
#[derive(Clone, Debug, Default)]
pub(super) struct Section {
    pub(super) name: String,
    pub(super) keys: Vec<(String, String)>,
}

/// The two parts of a config RNS's interface setup reads.
#[derive(Debug, Default)]
pub(super) struct Document {
    pub(super) reticulum: Section,
    pub(super) interfaces: Vec<Section>,
}

enum Place {
    Reticulum,
    Interfaces,
    Interface,
    Other,
}

pub(super) fn parse(text: &str) -> Result<Document, RnsConfigError> {
    let mut doc = Document::default();
    let mut place = Place::Other;
    for (index, raw) in text.lines().enumerate() {
        let syntax = || RnsConfigError::Syntax { line: index + 1 };
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(inner) = line.strip_prefix("[[") {
            let name = inner.split_once("]]").ok_or_else(syntax)?.0.trim();
            if !matches!(place, Place::Interfaces | Place::Interface) {
                place = Place::Other;
                continue;
            }
            if name.is_empty() || doc.interfaces.iter().any(|s| s.name == name) {
                return Err(syntax());
            }
            doc.interfaces.push(Section {
                name: name.to_string(),
                keys: Vec::new(),
            });
            place = Place::Interface;
        } else if let Some(inner) = line.strip_prefix('[') {
            let name = inner.split_once(']').ok_or_else(syntax)?.0.trim();
            place = match name {
                "reticulum" => Place::Reticulum,
                "interfaces" => Place::Interfaces,
                _ => Place::Other,
            };
        } else {
            let (key, value) = line.split_once('=').ok_or_else(syntax)?;
            let (key, value) = (key.trim().to_string(), unquote(value).ok_or_else(syntax)?);
            let section = match place {
                Place::Reticulum => &mut doc.reticulum,
                Place::Interface => doc.interfaces.last_mut().expect("in a subsection"),
                Place::Interfaces | Place::Other => continue,
            };
            if key.is_empty() || section.keys.iter().any(|(k, _)| *k == key) {
                return Err(syntax());
            }
            section.keys.push((key, value));
        }
    }
    Ok(doc)
}

/// A value without its quotes or trailing comment. A quoted value ends at its closing
/// quote; an unquoted one at `#`.
fn unquote(value: &str) -> Option<String> {
    let value = value.trim();
    match value.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let (inner, rest) = value[1..].split_once(quote)?;
            let rest = rest.trim();
            (rest.is_empty() || rest.starts_with('#')).then(|| inner.to_string())
        }
        _ => Some(value.split('#').next().unwrap_or("").trim().to_string()),
    }
}

/// ConfigObj's `as_bool` (any case).
pub(super) fn boolean(value: &str) -> Option<bool> {
    const TRUE: [&str; 4] = ["true", "yes", "on", "1"];
    const FALSE: [&str; 4] = ["false", "no", "off", "0"];
    let is = |words: [&str; 4]| words.iter().any(|w| value.eq_ignore_ascii_case(w));
    if is(TRUE) {
        Some(true)
    } else if is(FALSE) {
        Some(false)
    } else {
        None
    }
}

/// ConfigObj's comma list, as `as_list` reads `devices`.
#[cfg(all(feature = "auto", unix))]
pub(super) fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Split a command line as Python's `shlex.split` does in POSIX mode: whitespace
/// separates words, single quotes are literal, double quotes take `\` before `"`, `\`, `$`
/// and `` ` ``, and a bare `\` escapes the next character. `None` for an open quote or a
/// trailing escape.
#[cfg(any(feature = "pipe", test))]
pub(super) fn shell_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => words.extend(word.take()),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => w.push(c),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            c @ ('"' | '\\' | '$' | '`') => w.push(c),
                            '\n' => {}
                            c => {
                                w.push('\\');
                                w.push(c);
                            }
                        },
                        c => w.push(c),
                    }
                }
            }
            '\\' => word.get_or_insert_with(String::new).push(chars.next()?),
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Some(words)
}
