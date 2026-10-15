// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! §26.2 output-template substitution grammar [NORMATIVE].
//!
//! `{variable}` tokens substitute; everything else is literal. Literal
//! braces escape by doubling (`{{`, `}}`). Unknown variables are a
//! VALIDATION error (CLI exit 2), not silent passthrough. Path-traversal
//! guard [NORMATIVE §35.2], two layers: (a) `/`, `\`, and `..` PRODUCED
//! BY SUBSTITUTION are replaced with `_` within each substituted chunk;
//! (b) a post-substitution containment pass over the RESOLVED path
//! rewrites to `_` any path component equal to `..` that carries ANY
//! substituted content — `..` can COMPOSE across the substitution
//! boundary even when no single chunk contains it (input stem `.` +
//! template `comp/{name}./esc.{ext}` would otherwise resolve to
//! `comp/../esc.png`). Templates never create paths outside the
//! destination directory. Purely literal components — including a
//! literal `..` — are the caller's own path and pass through.
//!
//! Tokens at this checkpoint: `{name}`, `{ext}`, `{seq}`/`{seq:N}`,
//! `{date:PATTERN}`, `{w}`, `{h}`. `{slice}` is §29.2-context-only and is
//! a validation error here (per its table row). The `{date:…}` PATTERN
//! supports the ICU subset `yyyy MM dd HH mm ss` with all other pattern
//! letters rejected as validation errors (full ICU formatting is a typed
//! gap in the lane log; digits are locale-independent ASCII by
//! construction, as §26.2 requires).

use appname_core::{EngineError, ErrorCode};

/// Per-file values feeding one substitution.
#[derive(Debug, Clone)]
pub struct TemplateVars<'a> {
    /// Original filename stem (no directory, no extension).
    pub name: &'a str,
    /// Output format's canonical extension: lowercase, no dot.
    pub ext: &'a str,
    /// 1-based position in the batch.
    pub seq: usize,
    /// POST-processing pixel dimensions (after the action ran).
    pub w: u32,
    pub h: u32,
    /// Job start time as a civil UTC timestamp `(year, month, day, hour,
    /// minute, second)` — the ICU pattern formats from this.
    pub started: (i64, u32, u32, u32, u32, u32),
}

fn invalid(msg: String) -> EngineError {
    EngineError::new(ErrorCode::InvalidArg, msg)
}

/// Substitute `template` with `vars`. Errors are validation errors
/// (unknown variable, malformed token, unsupported date pattern letter).
/// The returned path has passed BOTH §35.2 guard layers (module docs):
/// per-chunk sanitize and the whole-path containment pass — callers may
/// `create_dir_all`/write on it without re-checking.
pub fn substitute(template: &str, vars: &TemplateVars<'_>) -> Result<String, EngineError> {
    // Built alongside `out`: one flag per pushed `char`, true when the
    // char came from substitution (feeds the containment pass).
    let mut out = String::with_capacity(template.len() + 16);
    let mut taint: Vec<bool> = Vec::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push('{');
                    taint.push(false);
                    continue;
                }
                let mut token = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(t) => token.push(t),
                        None => {
                            return Err(invalid(format!(
                                "unterminated {{token}} in output template (at {{{token}"
                            )))
                        }
                    }
                }
                let chunk = sanitize(&expand(&token, vars)?);
                for sc in chunk.chars() {
                    out.push(sc);
                    taint.push(true);
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    out.push('}');
                    taint.push(false);
                } else {
                    return Err(invalid(
                        "stray '}' in output template (escape literal braces as '}}')".into(),
                    ));
                }
            }
            other => {
                out.push(other);
                taint.push(false);
            }
        }
    }
    Ok(contain(&out, &taint))
}

/// §35.2 containment pass on the RESOLVED path (MAJOR-1): rewrite to `_`
/// any path component equal to `..` that carries substituted content.
/// Runs BEFORE any caller touches the filesystem — [`substitute`] is the
/// only producer of batch output paths. Purely literal `..` components
/// (no substituted chars) are the caller's own template path and pass
/// through, per the module-doc contract.
fn contain(resolved: &str, taint: &[bool]) -> String {
    debug_assert_eq!(resolved.chars().count(), taint.len());
    let mut out = String::with_capacity(resolved.len());
    let mut component = String::new();
    let mut component_tainted = false;
    let flush = |out: &mut String, comp: &mut String, tainted: &mut bool| {
        if comp == ".." && *tainted {
            out.push('_');
        } else {
            out.push_str(comp);
        }
        comp.clear();
        *tainted = false;
    };
    for (c, tainted) in resolved.chars().zip(taint.iter().copied()) {
        if c == '/' || c == '\\' {
            flush(&mut out, &mut component, &mut component_tainted);
            out.push(c);
        } else {
            component.push(c);
            component_tainted |= tainted;
        }
    }
    flush(&mut out, &mut component, &mut component_tainted);
    out
}

fn expand(token: &str, vars: &TemplateVars<'_>) -> Result<String, EngineError> {
    match token {
        "name" => return Ok(vars.name.to_string()),
        "ext" => return Ok(vars.ext.to_string()),
        "seq" => return Ok(vars.seq.to_string()),
        "w" => return Ok(vars.w.to_string()),
        "h" => return Ok(vars.h.to_string()),
        "slice" => {
            return Err(invalid(
                "{slice} is valid only in §29.2 slice-naming contexts, not batch output \
                 templates"
                    .into(),
            ))
        }
        _ => {}
    }
    if let Some(width) = token.strip_prefix("seq:") {
        let n: usize = width.parse().map_err(|_| {
            invalid(format!(
                "{{seq:N}} needs an integer pad width, got {{{token}}}"
            ))
        })?;
        if n == 0 || n > 32 {
            return Err(invalid(format!(
                "{{seq:N}} pad width must be 1..=32, got {n}"
            )));
        }
        return Ok(format!("{:0>width$}", vars.seq, width = n));
    }
    if let Some(pattern) = token.strip_prefix("date:") {
        return format_date(pattern, vars.started);
    }
    Err(invalid(format!(
        "unknown template variable {{{token}}} (§26.2: unknown variables are a validation \
         error, not passthrough)"
    )))
}

/// ICU-pattern date formatting, supported subset (module docs). Pattern
/// letters run in groups; literal text passes through; `'` quoting is not
/// supported at this checkpoint (rejected, typed).
fn format_date(
    pattern: &str,
    (year, month, day, hour, minute, second): (i64, u32, u32, u32, u32, u32),
) -> Result<String, EngineError> {
    let mut out = String::new();
    let bytes: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let mut run = 1;
        while i + run < bytes.len() && bytes[i + run] == c {
            run += 1;
        }
        match c {
            'y' => out.push_str(&format!("{:0>width$}", year, width = run.max(1))),
            'M' => out.push_str(&format!("{:0>width$}", month, width = run)),
            'd' => out.push_str(&format!("{:0>width$}", day, width = run)),
            'H' => out.push_str(&format!("{:0>width$}", hour, width = run)),
            'm' => out.push_str(&format!("{:0>width$}", minute, width = run)),
            's' => out.push_str(&format!("{:0>width$}", second, width = run)),
            'a'..='z' | 'A'..='Z' => {
                return Err(invalid(format!(
                    "unsupported ICU date pattern letter {c:?} in {{date:{pattern}}} \
                     (supported subset: y M d H m s)"
                )))
            }
            '\'' => {
                return Err(invalid(format!(
                    "ICU pattern quoting (') is not supported at this checkpoint in \
                     {{date:{pattern}}}"
                )))
            }
            other => {
                for _ in 0..run {
                    out.push(other);
                }
            }
        }
        i += run;
    }
    Ok(out)
}

/// §35.2 traversal guard on SUBSTITUTED content: `/`, `\`, `..` → `_`.
fn sanitize(value: &str) -> String {
    let mut s = value.replace(['/', '\\'], "_");
    while s.contains("..") {
        s = s.replace("..", "_");
    }
    s
}

/// §26.2 collision policy: if `path` exists, append ` (n)` before the
/// extension with the smallest free `n >= 2` — unless `overwrite`.
pub fn resolve_collision(path: &std::path::Path, overwrite: bool) -> std::path::PathBuf {
    if overwrite || !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for n in 2u32.. {
        let file = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dir.join(file);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("some n is free");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>() -> TemplateVars<'a> {
        TemplateVars {
            name: "shoot-a",
            ext: "png",
            seq: 7,
            w: 1200,
            h: 800,
            started: (2026, 8, 13, 9, 12, 3),
        }
    }

    #[test]
    fn the_spec_tokens_substitute() {
        let v = vars();
        assert_eq!(
            substitute("{name}-{date:yyyy-MM-dd}.{ext}", &v).unwrap(),
            "shoot-a-2026-08-13.png"
        );
        assert_eq!(substitute("{seq:4}", &v).unwrap(), "0007");
        assert_eq!(substitute("{seq}", &v).unwrap(), "7");
        assert_eq!(substitute("{w}x{h}", &v).unwrap(), "1200x800");
        assert_eq!(substitute("a{{b}}c", &v).unwrap(), "a{b}c");
    }

    #[test]
    fn unknown_variables_are_validation_errors() {
        for bad in [
            "{nope}",
            "{slice}",
            "{seq:x}",
            "{date:EEEE}",
            "{unclosed",
            "}",
        ] {
            let e = substitute(bad, &vars()).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidArg, "{bad}: {}", e.message);
        }
    }

    #[test]
    fn traversal_guard_rewrites_substituted_separators_only() {
        let mut v = vars();
        v.name = "../../etc/passwd";
        let out = substitute("out/{name}.{ext}", &v).unwrap();
        assert_eq!(out, "out/____etc_passwd.png");
        assert!(!out.contains(".."));
        // The literal template directory survives.
        assert!(out.starts_with("out/"));
    }

    /// MAJOR-1 regression: `..` composed ACROSS the substitution boundary
    /// — the reviewer's exact breach. Input file `..png` has stem `.`;
    /// template `comp/{name}./esc.{ext}` would resolve to
    /// `comp/../esc.png` (outside `comp/`) without the containment pass.
    #[test]
    fn composed_traversal_across_the_substitution_boundary_is_contained() {
        let mut v = vars();
        v.name = "."; // Path::file_stem of "..png"
        let out = substitute("comp/{name}./esc.{ext}", &v).unwrap();
        assert_eq!(out, "comp/_/esc.png");
        assert!(
            !std::path::Path::new(&out)
                .components()
                .any(|c| c == std::path::Component::ParentDir),
            "no `..` component may survive: {out}"
        );

        // Composition purely from substituted chunks (two `.` tokens).
        let out = substitute("comp/{name}{name}/esc.{ext}", &v).unwrap();
        assert_eq!(out, "comp/_/esc.png");

        // Literal `.` + substituted `.` on the other side composes too.
        let out = substitute("comp/.{name}/esc.{ext}", &v).unwrap();
        assert_eq!(out, "comp/_/esc.png");

        // A lone substituted `.` component is harmless and passes.
        assert_eq!(
            substitute("comp/{name}/x.{ext}", &v).unwrap(),
            "comp/./x.png"
        );
    }

    /// The module-doc contract's other half: a PURELY LITERAL `..` is the
    /// caller's own path and passes through untouched.
    #[test]
    fn literal_parent_components_are_the_callers_own_path() {
        let v = vars();
        assert_eq!(
            substitute("../exports/{name}.{ext}", &v).unwrap(),
            "../exports/shoot-a.png"
        );
    }

    #[test]
    fn collision_appends_the_smallest_free_n() {
        let dir =
            std::env::temp_dir().join(format!("appname-script-collide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.png");
        assert_eq!(resolve_collision(&p, false), p);
        std::fs::write(&p, b"a").unwrap();
        assert_eq!(resolve_collision(&p, true), p);
        assert_eq!(resolve_collision(&p, false), dir.join("x (2).png"));
        std::fs::write(dir.join("x (2).png"), b"a").unwrap();
        assert_eq!(resolve_collision(&p, false), dir.join("x (3).png"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
