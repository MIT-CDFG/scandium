// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §27 (strings/i18n pipeline — CP-0.1 seed); plan §2.1 row
//! "Headless CLI v1" (i18n loader ported with proto catalogs as seed, per
//! the wave-1 arbitration the catalogs live under this crate's `i18n/`).
//! Ported from the proto CLI's loader (same authorship): human-facing
//! strings are looked up by MessageId in `i18n/<code>.json` catalogs.
//!
//! Language resolution: `--lang` flag wins over the `APPNAME_LANG`
//! environment variable, which wins over the `en` default
//! ([`resolve_lang`]). Lookup chain per key: requested locale -> `en` ->
//! **key-echo** (the MessageId itself plus `name=value` pairs), so output
//! stays machine-legible and visibly untranslated rather than silently
//! English.
//!
//! Template grammar is the mockup's ICU MessageFormat subset (README
//! divergence table; proto-mirrored grammar):
//! `{name}` placeholders plus cardinal plurals
//! `{name, plural, one {...} other {...}}` where `#` in a branch is the
//! number. Plural categories come from minimal CLDR rules declared per
//! catalog in `_meta.plural_rules` (`one_other`: n == 1 -> one, the en/es
//! default; `other_only`: ja default). The rule of the catalog that
//! supplied the template applies, so an en fallback string pluralizes with
//! en's rule even under `--lang ja`.
//!
//! Args a template does not consume are appended as ` name=value` — that is
//! how optional error context (`field`, `id`, `line` on `cli.error`)
//! surfaces without per-arity ids.
//!
//! Catalog shape tolerated: a flat JSON object `{ "id": "template" }`
//! (canonical, `_meta` block ignored for lookup) or the same map nested
//! under a top-level `"messages"` key. Machine output (`--json`, table data
//! cells, wire constants like PASS/FAIL, command kinds, error codes) is
//! never translated, by design. Catalog dir override for tests:
//! `APPNAME_I18N_DIR`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

/// Minimal CLDR cardinal rules (plan 2.4.3: en/es one|other, ja other).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Rule {
    OneOther,
    OtherOnly,
}

impl Rule {
    fn category(self, n: f64) -> &'static str {
        match self {
            Rule::OneOther if n == 1.0 => "one",
            _ => "other",
        }
    }
}

fn default_rule(lang: &str) -> Rule {
    match lang {
        "ja" => Rule::OtherOnly,
        _ => Rule::OneOther,
    }
}

struct Entry {
    rule: Rule,
    map: HashMap<String, String>,
}

pub struct Catalog {
    /// Lookup chain: requested locale first (when it exists and is not en),
    /// then en. Each entry keeps its own plural rule.
    chain: Vec<Entry>,
}

/// `--lang` flag > `APPNAME_LANG` env > `"en"`.
pub fn resolve_lang(flag: Option<&str>) -> String {
    resolve_lang_from(flag, std::env::var("APPNAME_LANG").ok())
}

fn resolve_lang_from(flag: Option<&str>, env: Option<String>) -> String {
    if let Some(f) = flag {
        if !f.trim().is_empty() {
            return f.trim().to_string();
        }
    }
    if let Some(e) = env {
        if !e.trim().is_empty() {
            return e.trim().to_string();
        }
    }
    "en".to_string()
}

/// The en catalog embedded at compile time (review minor 3): human output
/// NEVER key-echoes for en, even for an installed binary with no catalog
/// directory in sight. The proto resolved a CWD-relative `i18n/` — a silent
/// key-echo trap for installed binaries — recorded as a divergence.
const EMBEDDED_EN: &str = include_str!("../i18n/en.json");

/// Catalog directory resolution order (review minor 3): `APPNAME_I18N_DIR`
/// → executable-adjacent `i18n/` → (for `en`) the embedded catalog →
/// key-echo as last resort. `--lang` precedence is unchanged (language
/// resolution stays [`resolve_lang`]).
fn catalog_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("APPNAME_I18N_DIR") {
        dirs.push(PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("i18n"));
        }
    }
    dirs
}

fn parse_catalog(lang: &str, text: &str) -> Option<Entry> {
    let value: Value = serde_json::from_str(text).ok()?;
    let rule = match value
        .get("_meta")
        .and_then(|m| m.get("plural_rules"))
        .and_then(Value::as_str)
    {
        Some("one_other") => Rule::OneOther,
        Some("other_only") => Rule::OtherOnly,
        _ => default_rule(lang),
    };
    let obj = match value.get("messages") {
        Some(Value::Object(m)) => m.clone(),
        _ => value.as_object()?.clone(),
    };
    let mut map = HashMap::new();
    for (k, v) in obj {
        if k == "_meta" {
            continue;
        }
        if let Value::String(s) = v {
            map.insert(k, s);
        }
    }
    Some(Entry { rule, map })
}

fn load_lang_file(lang: &str) -> Option<Entry> {
    for dir in catalog_dirs() {
        let path = dir.join(format!("{lang}.json"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(entry) = parse_catalog(lang, &text) {
                return Some(entry);
            }
        }
    }
    if lang == "en" {
        return parse_catalog("en", EMBEDDED_EN);
    }
    None
}

impl Catalog {
    /// Load the chain for `lang`. Missing files are not an error: the CLI
    /// falls back per the chain above.
    pub fn load(lang: &str) -> Catalog {
        let mut chain = Vec::new();
        if lang != "en" {
            if let Some(entry) = load_lang_file(lang) {
                chain.push(entry);
            }
        }
        if let Some(en) = load_lang_file("en") {
            chain.push(en);
        }
        Catalog { chain }
    }

    /// Translate `key`; key-echo fallback when the key is uncataloged.
    pub fn tr(&self, key: &str, args: &[(&str, String)]) -> String {
        for entry in &self.chain {
            if let Some(template) = entry.map.get(key) {
                return expand(template, args, entry.rule);
            }
        }
        // Key-echo: the id itself + all args, visibly untranslated.
        let mut out = String::from(key);
        for (name, value) in args {
            out.push(' ');
            out.push_str(name);
            out.push('=');
            out.push_str(value);
        }
        out
    }
}

/// Render `template` with `args` under `rule`; unconsumed args are appended
/// as ` name=value` pairs (in their given order).
fn expand<'a>(template: &str, args: &'a [(&'a str, String)], rule: Rule) -> String {
    let mut consumed: Vec<&'a str> = Vec::new();
    let mut text = expand_plurals(template, args, rule, &mut consumed);
    for (name, value) in args {
        let needle = format!("{{{name}}}");
        if text.contains(&needle) {
            text = text.replace(&needle, value);
            consumed.push(name);
        }
    }
    for (name, value) in args {
        if !consumed.contains(name) {
            text.push(' ');
            text.push_str(name);
            text.push('=');
            text.push_str(value);
        }
    }
    text
}

/// Replace every `{name, plural, sel {..} ..}` block with its chosen
/// branch (`#` -> the argument's value). Malformed blocks are left verbatim.
fn expand_plurals<'a>(
    template: &str,
    args: &'a [(&'a str, String)],
    rule: Rule,
    consumed: &mut Vec<&'a str>,
) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '{' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let Some(end) = matching_brace(&chars, i) else {
            // unbalanced: emit the rest verbatim
            out.extend(&chars[i..]);
            break;
        };
        let inner: String = chars[i + 1..end].iter().collect();
        match parse_plural(&inner) {
            Some((name, branches)) => {
                let value = args
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v.as_str());
                let category = value
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|n| rule.category(n))
                    .unwrap_or("other");
                let branch = branches
                    .iter()
                    .find(|(sel, _)| *sel == category)
                    .or_else(|| branches.iter().find(|(sel, _)| *sel == "other"));
                match (branch, value) {
                    (Some((_, body)), Some(v)) => {
                        if let Some((argname, _)) = args.iter().find(|(n, _)| *n == name) {
                            consumed.push(argname);
                        }
                        out.push_str(&body.replace('#', v));
                    }
                    (Some((_, body)), None) => out.push_str(body),
                    (None, _) => {
                        // no usable branch: keep the block visible
                        out.push('{');
                        out.push_str(&inner);
                        out.push('}');
                    }
                }
            }
            None => {
                // not a plural block ({name} placeholder or plain braces):
                // keep verbatim for the placeholder pass
                out.push('{');
                out.push_str(&inner);
                out.push('}');
            }
        }
        i = end + 1;
    }
    out
}

fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &ch) in chars.iter().enumerate().skip(open) {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// `"name, plural, one {..} other {..}"` -> `(name, [(selector, body)...])`.
fn parse_plural(inner: &str) -> Option<(String, Vec<(String, String)>)> {
    let (head, rest) = inner.split_once(',')?;
    let (kw, branches_src) = rest.split_once(',')?;
    if kw.trim() != "plural" {
        return None;
    }
    let name = head.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let chars: Vec<char> = branches_src.chars().collect();
    let mut branches = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let sel_start = i;
        while i < chars.len() && chars[i] != '{' {
            i += 1;
        }
        if i >= chars.len() {
            return None; // selector without branch
        }
        let selector: String = chars[sel_start..i]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if selector != "one" && selector != "other" {
            return None; // unsupported selector
        }
        let end = matching_brace(&chars, i)?;
        let body: String = chars[i + 1..end].iter().collect();
        branches.push((selector, body));
        i = end + 1;
    }
    if branches.iter().any(|(s, _)| s == "other") {
        Some((name.to_string(), branches))
    } else {
        None // an ICU plural requires an `other` branch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(n, v)| (*n, v.to_string())).collect()
    }

    #[test]
    fn lang_resolution_flag_wins_over_env_wins_over_default() {
        assert_eq!(resolve_lang_from(Some("es"), Some("ja".into())), "es");
        assert_eq!(resolve_lang_from(None, Some("ja".into())), "ja");
        assert_eq!(resolve_lang_from(None, None), "en");
        assert_eq!(resolve_lang_from(Some("  "), Some(" ".into())), "en");
        assert_eq!(resolve_lang_from(None, Some(" es ".into())), "es");
    }

    #[test]
    fn placeholder_interpolation_and_unconsumed_append() {
        let a = args(&[("code", "error.x"), ("message", "boom"), ("id", "u-1")]);
        assert_eq!(
            expand("Error {code}: {message}", &a, Rule::OneOther),
            "Error error.x: boom id=u-1"
        );
    }

    #[test]
    fn plural_one_other_rule() {
        let t = "{count, plural, one {# layer} other {# layers}}";
        assert_eq!(
            expand(t, &args(&[("count", "1")]), Rule::OneOther),
            "1 layer"
        );
        assert_eq!(
            expand(t, &args(&[("count", "3")]), Rule::OneOther),
            "3 layers"
        );
        assert_eq!(
            expand(t, &args(&[("count", "0")]), Rule::OneOther),
            "0 layers"
        );
    }

    #[test]
    fn plural_other_only_rule_and_other_only_template() {
        let t = "{count, plural, other {レイヤー # 枚}}";
        assert_eq!(
            expand(t, &args(&[("count", "1")]), Rule::OtherOnly),
            "レイヤー 1 枚"
        );
        // an en template evaluated under ja's rule still picks other for 1
        let en = "{count, plural, one {# layer} other {# layers}}";
        assert_eq!(
            expand(en, &args(&[("count", "1")]), Rule::OtherOnly),
            "1 layers"
        );
    }

    #[test]
    fn plural_mixed_with_placeholders() {
        let t = "{performed, plural, one {Undid # step} other {Undid # steps}} — cursor {cursor}/{length}";
        assert_eq!(
            expand(
                t,
                &args(&[("performed", "2"), ("cursor", "4"), ("length", "6")]),
                Rule::OneOther
            ),
            "Undid 2 steps — cursor 4/6"
        );
    }

    #[test]
    fn malformed_blocks_stay_visible() {
        // unsupported selector -> block kept verbatim (audit keeps catalogs clean)
        let t = "{n, plural, few {x} other {y}}";
        assert_eq!(
            expand(t, &args(&[("n", "1")]), Rule::OneOther),
            "{n, plural, few {x} other {y}} n=1"
        );
        // unbalanced brace -> emitted as-is
        let t2 = "broken {n";
        assert_eq!(expand(t2, &[], Rule::OneOther), "broken {n");
    }

    #[test]
    fn key_echo_when_uncataloged() {
        let cat = Catalog { chain: Vec::new() };
        assert_eq!(
            cat.tr(
                "cli.apply.ok",
                &args(&[("kind", "filter.apply"), ("id", "u")])
            ),
            "cli.apply.ok kind=filter.apply id=u"
        );
    }

    #[test]
    fn chain_prefers_requested_locale_and_keeps_per_locale_rules() {
        let mut ja = HashMap::new();
        ja.insert("k".to_string(), "{n, plural, other {#枚}}".to_string());
        let mut en = HashMap::new();
        en.insert(
            "k".to_string(),
            "{n, plural, one {# item} other {# items}}".to_string(),
        );
        en.insert("only_en".to_string(), "fallback".to_string());
        let cat = Catalog {
            chain: vec![
                Entry {
                    rule: Rule::OtherOnly,
                    map: ja,
                },
                Entry {
                    rule: Rule::OneOther,
                    map: en,
                },
            ],
        };
        assert_eq!(cat.tr("k", &args(&[("n", "1")])), "1枚");
        // en fallback template pluralizes with en's rule
        assert_eq!(cat.tr("only_en", &[]), "fallback");
    }
}
