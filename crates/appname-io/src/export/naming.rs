// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §26.2 "Output-template substitution grammar" [NORMATIVE]
//! (shared by batch and §29.2 slice naming), the collision policy, and
//! the §35.2 path-traversal guard; §29.2 [NORMATIVE] naming (`{slice}`
//! token, `@2x`/`@3x` stem suffixes, default template
//! `{name}-{slice}.{ext}`).
//!
//! Grammar: `{variable}` tokens substitute; everything else is literal;
//! `{{`/`}}` escape literal braces; unknown variables are a VALIDATION
//! error (CLI exit 2), never silent passthrough. Substituted VALUES are
//! traversal-guarded: `/`, `\`, and `..` produced by substitution are
//! replaced with `_`.
//!
//! **The containment invariant (DEF-1, ruled fix).** The per-value
//! guard alone is NOT the §35.2 guarantee: guarded values can COMPOSE
//! into a traversal (slice `"."` through `{slice}{slice}/…` composes
//! to `../…`), and literal template text can be absolute. The real
//! invariant is two-layered: (1) values are per-value guarded at
//! substitution; (2) every RESOLVED name passes [`check_contained`]
//! before any write — never absolute, and no `..` SEGMENT anywhere in
//! the composed path. Literal template text may therefore create
//! RELATIVE subpaths (`web/{name}.{ext}` is legitimate and kept); it
//! cannot escape the destination. Every path producer (slice export,
//! Export-As, batch) runs the final check; a rejection is a typed
//! per-file failure, never a write.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::CodecError;

/// §29.2 default slice template.
pub const DEFAULT_SLICE_TEMPLATE: &str = "{name}-{slice}.{ext}";

/// Variable values for one substitution run. `w`/`h` are the
/// POST-processing dimensions (§26.2); `seq` is 1-based.
#[derive(Debug, Clone, Default)]
pub struct TemplateVars {
    pub name: String,
    pub ext: String,
    pub slice: Option<String>,
    pub seq: Option<u64>,
    pub w: Option<u32>,
    pub h: Option<u32>,
    /// Job start time as (year, month, day) — the `{date:...}` source.
    /// ASCII digits always (§26.2 locale-independent rule).
    pub date_ymd: Option<(i32, u32, u32)>,
}

/// §35.2/§26.2 [NORMATIVE] traversal guard over ONE substituted value:
/// `/`, `\`, and `..` are replaced with `_`. Layer 1 of the invariant
/// (module doc) — [`check_contained`] over the composed result is
/// layer 2, and the only one that is load-bearing for containment.
pub fn guard_value(v: &str) -> String {
    let mut s = v.replace(['/', '\\'], "_");
    while let Some(i) = s.find("..") {
        s.replace_range(i..i + 2, "_");
    }
    s
}

/// **DEF-1 final containment check** (§35.2/§26.2 [NORMATIVE]; the
/// ruled fix): run on every RESOLVED output name, after all
/// substitution/suffixing/collision composition and BEFORE any write.
/// Rejects (typed) any name that is absolute — a leading separator or
/// `Path::is_absolute` (drive prefixes on Windows), which would make
/// `dest.join` discard `dest` entirely — and any name containing a
/// `..` SEGMENT when split on `/` and `\` (per-value-guarded pieces
/// can still COMPOSE into one). Relative subdir names
/// (`web/hero.png`) pass: literal template text chooses paths INSIDE
/// the destination, never outside it.
pub fn check_contained(name: &str) -> Result<(), CodecError> {
    let escape = |detail: String| CodecError::OptionOutOfRange {
        field: "template",
        detail,
    };
    if name.is_empty() {
        return Err(escape("resolved output name is empty".into()));
    }
    if name.starts_with('/') || name.starts_with('\\') || Path::new(name).is_absolute() {
        return Err(escape(format!(
            "resolved output name `{name}` is absolute — outputs never leave the \
             destination directory (§35.2/§26.2 guard)"
        )));
    }
    for segment in name.split(['/', '\\']) {
        if segment == ".." {
            return Err(escape(format!(
                "resolved output name `{name}` contains a `..` segment — outputs never \
                 leave the destination directory (§35.2/§26.2 guard)"
            )));
        }
    }
    Ok(())
}

/// Substitute `template` with `vars` (§26.2 grammar). Unknown variables
/// and malformed braces answer typed (`OptionOutOfRange` — the CLI maps
/// it to exit 2); no filesystem access happens here.
pub fn substitute(template: &str, vars: &TemplateVars) -> Result<String, CodecError> {
    let mut out = String::with_capacity(template.len() + 16);
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                out.push('{');
                i += 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                let end = template[i + 1..]
                    .find('}')
                    .map(|o| i + 1 + o)
                    .ok_or_else(|| CodecError::OptionOutOfRange {
                        field: "template",
                        detail: format!("unterminated `{{` at byte {i}"),
                    })?;
                let token = &template[i + 1..end];
                out.push_str(&resolve(token, vars)?);
                i = end + 1;
            }
            b'}' => {
                return Err(CodecError::OptionOutOfRange {
                    field: "template",
                    detail: format!("stray `}}` at byte {i} (escape literal braces as `}}}}`)"),
                })
            }
            _ => {
                // copy one UTF-8 scalar
                let ch = template[i..].chars().next().expect("in-bounds");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Ok(out)
}

fn resolve(token: &str, vars: &TemplateVars) -> Result<String, CodecError> {
    let unknown = |t: &str| CodecError::OptionOutOfRange {
        field: "template",
        detail: format!("unknown template variable `{{{t}}}` (§26.2 token table)"),
    };
    // {seq} / {seq:N}
    if token == "seq" || token.starts_with("seq:") {
        let seq = vars.seq.ok_or_else(|| CodecError::OptionOutOfRange {
            field: "template",
            detail: "`{seq}` outside a batch context (§26.2)".into(),
        })?;
        let pad = if let Some(n) = token.strip_prefix("seq:") {
            n.parse::<usize>().map_err(|_| unknown(token))?
        } else {
            0
        };
        return Ok(format!("{seq:0pad$}"));
    }
    // {date:<pattern>} — ICU pattern subset (yyyy, MM, dd literal-joined;
    // the full ICU engine joins with the i18n wave). ASCII digits always.
    if let Some(pattern) = token.strip_prefix("date:") {
        let (y, m, d) = vars.date_ymd.ok_or_else(|| CodecError::OptionOutOfRange {
            field: "template",
            detail: "`{date:...}` outside a dated context (§26.2)".into(),
        })?;
        let mut s = String::new();
        let mut rest = pattern;
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix("yyyy") {
                s.push_str(&format!("{y:04}"));
                rest = r;
            } else if let Some(r) = rest.strip_prefix("MM") {
                s.push_str(&format!("{m:02}"));
                rest = r;
            } else if let Some(r) = rest.strip_prefix("dd") {
                s.push_str(&format!("{d:02}"));
                rest = r;
            } else {
                let ch = rest.chars().next().expect("non-empty");
                if ch.is_ascii_alphanumeric() {
                    // an ICU field we do not implement yet — typed, not
                    // silently literal
                    return Err(CodecError::OptionOutOfRange {
                        field: "template",
                        detail: format!(
                            "unsupported date pattern field in `{{{token}}}` (yyyy/MM/dd subset)"
                        ),
                    });
                }
                s.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
        return Ok(s);
    }
    match token {
        "name" => Ok(guard_value(&vars.name)),
        "ext" => Ok(guard_value(&vars.ext)),
        "slice" => {
            vars.slice
                .as_deref()
                .map(guard_value)
                .ok_or_else(|| CodecError::OptionOutOfRange {
                    field: "template",
                    detail: "`{slice}` outside a slice-export context (§26.2/§29.2)".into(),
                })
        }
        "w" => vars.w.map(|v| v.to_string()).ok_or_else(|| unknown("w")),
        "h" => vars.h.map(|v| v.to_string()).ok_or_else(|| unknown("h")),
        other => Err(unknown(other)),
    }
}

/// §29.2 [NORMATIVE]: scale variants ≠ 1× append `@2x`/`@3x` to the STEM
/// (the template has no scale token). Non-integer scales format with the
/// shortest exact representation (`@1.5x`).
pub fn apply_scale_suffix(filename: &str, scale: f32) -> String {
    if (scale - 1.0).abs() < f32::EPSILON {
        return filename.to_string();
    }
    let suffix = if (scale.fract()).abs() < f32::EPSILON {
        format!("@{}x", scale as u32)
    } else {
        format!("@{scale}x")
    };
    match filename.rfind('.') {
        Some(dot) if dot > 0 => format!("{}{suffix}{}", &filename[..dot], &filename[dot..]),
        _ => format!("{filename}{suffix}"),
    }
}

/// §26.2 collision policy: if `dest/name` exists (or is claimed in
/// `claimed` — one export run is its own collision domain), append
/// ` (n)` before the extension with the smallest free `n ≥ 2` — unless
/// `overwrite`. Returns the resolved path and records the claim.
pub fn resolve_collision(
    dest: &Path,
    filename: &str,
    overwrite: bool,
    claimed: &mut BTreeMap<String, ()>,
) -> PathBuf {
    let free = |name: &str, claimed: &BTreeMap<String, ()>| {
        !claimed.contains_key(name) && !dest.join(name).exists()
    };
    if overwrite || free(filename, claimed) {
        claimed.insert(filename.to_string(), ());
        return dest.join(filename);
    }
    let (stem, ext) = match filename.rfind('.') {
        Some(dot) if dot > 0 => (&filename[..dot], &filename[dot..]),
        _ => (filename, ""),
    };
    let mut n = 2u64;
    loop {
        let candidate = format!("{stem} ({n}){ext}");
        if free(&candidate, claimed) {
            claimed.insert(candidate.clone(), ());
            return dest.join(candidate);
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> TemplateVars {
        TemplateVars {
            name: "hero".into(),
            ext: "png".into(),
            slice: Some("navbar".into()),
            seq: Some(7),
            w: Some(1200),
            h: Some(96),
            date_ymd: Some((2026, 8, 17)),
        }
    }

    #[test]
    fn default_slice_template_substitutes() {
        assert_eq!(
            substitute(DEFAULT_SLICE_TEMPLATE, &vars()).unwrap(),
            "hero-navbar.png"
        );
    }

    #[test]
    fn seq_pads_and_date_formats_ascii() {
        assert_eq!(substitute("{seq:4}", &vars()).unwrap(), "0007");
        assert_eq!(substitute("{seq}", &vars()).unwrap(), "7");
        assert_eq!(
            substitute("{date:yyyy-MM-dd}", &vars()).unwrap(),
            "2026-08-17"
        );
    }

    #[test]
    fn unknown_variable_is_a_validation_error() {
        let e = substitute("{name}-{nope}.{ext}", &vars()).unwrap_err();
        assert!(matches!(e, CodecError::OptionOutOfRange { .. }), "{e}");
    }

    #[test]
    fn literal_braces_escape() {
        assert_eq!(substitute("a{{b}}c", &vars()).unwrap(), "a{b}c");
    }

    #[test]
    fn traversal_guard_replaces_in_substituted_values() {
        let mut v = vars();
        v.slice = Some("../evil".into());
        let s = substitute(DEFAULT_SLICE_TEMPLATE, &v).unwrap();
        assert_eq!(s, "hero-__evil.png");
        assert!(!s.contains(".."));
        v.slice = Some("a/b\\c".into());
        assert_eq!(
            substitute(DEFAULT_SLICE_TEMPLATE, &v).unwrap(),
            "hero-a_b_c.png"
        );
    }

    #[test]
    fn w_h_are_post_processing_dimensions() {
        assert_eq!(substitute("{w}x{h}", &vars()).unwrap(), "1200x96");
    }

    #[test]
    fn scale_suffix_on_the_stem() {
        assert_eq!(apply_scale_suffix("a.png", 2.0), "a@2x.png");
        assert_eq!(apply_scale_suffix("a.png", 3.0), "a@3x.png");
        assert_eq!(apply_scale_suffix("a.png", 1.0), "a.png");
        assert_eq!(apply_scale_suffix("a.png", 1.5), "a@1.5x.png");
    }

    /// DEF-1: the containment check's three arms — absolute rejected,
    /// `..` SEGMENTS rejected (including composed ones the per-value
    /// guard cannot see), relative subdirs pass.
    #[test]
    fn check_contained_rejects_escapes_keeps_subdirs() {
        // absolute forms
        for bad in ["/tmp/x.png", "\\tmp\\x.png", "/x"] {
            assert!(check_contained(bad).is_err(), "{bad}");
        }
        // `..` segments, anywhere
        for bad in ["../x.png", "a/../x.png", "web/..", "..\\x.png"] {
            assert!(check_contained(bad).is_err(), "{bad}");
        }
        // legitimate names, including subdirs and dotty-but-not-`..`
        for ok in ["x.png", "web/x.png", "a/b/c.png", "hero-...png", "./x.png"] {
            assert!(check_contained(ok).is_ok(), "{ok}");
        }
        assert!(check_contained("").is_err(), "empty name");
    }

    /// DEF-1: the composed-dot escape at the naming layer — each `.`
    /// passes the per-value guard, the COMPOSED result carries the
    /// traversal, and only `check_contained` catches it.
    #[test]
    fn composed_values_can_escape_only_the_per_value_guard() {
        let mut v = vars();
        v.slice = Some(".".into());
        let composed = substitute("{slice}{slice}/escaped.{ext}", &v).unwrap();
        assert_eq!(composed, "../escaped.png", "the guard alone missed this");
        assert!(check_contained(&composed).is_err(), "layer 2 catches it");
    }

    #[test]
    fn collision_appends_smallest_free_n() {
        let dir = std::env::temp_dir().join(format!("appname-naming-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.png"), b"1").unwrap();
        let mut claimed = BTreeMap::new();
        let p1 = resolve_collision(&dir, "x.png", false, &mut claimed);
        assert!(p1.file_name().unwrap().to_str().unwrap() == "x (2).png");
        // in-run claim: the next identical name advances even though the
        // file is not yet written
        let p2 = resolve_collision(&dir, "x.png", false, &mut claimed);
        assert!(p2.file_name().unwrap().to_str().unwrap() == "x (3).png");
        // overwrite bypasses
        let p3 = resolve_collision(&dir, "x.png", true, &mut claimed);
        assert!(p3.file_name().unwrap().to_str().unwrap() == "x.png");
        std::fs::remove_dir_all(&dir).ok();
    }
}
