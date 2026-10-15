// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The per-plugin SESSION STORE — the only filesystem a sandboxed
//! plugin sees without an explicit grant.
//!
//! Spec: ES §25.3 ("WASI with no preopened dirs by default") + plan §2.5
//! row pin "fs = session store + grants": v1 gives every sandboxed
//! plugin one private directory (`<store root>/<plugin id>/`) for
//! presets, caches, and scratch, brokered by path — the plugin
//! addresses it with RELATIVE paths only, and no resolution can leave
//! it. Everything else on disk needs a per-directory grant (§25.3
//! table, `fs.read:`/`fs.write:` rows) checked by the same resolver.
//!
//! # Confinement algorithm (both layers, template.rs precedent)
//!
//! 1. LEXICAL: split the petition into components; refuse absolute
//!    petitions into the store, refuse any `..` that would escape the
//!    root (components are resolved left-to-right against a depth
//!    counter — a `..` at depth 0 refuses), refuse NUL and empty
//!    components.
//! 2. PHYSICAL: the confining root is canonicalized at construction;
//!    after lexical resolution the DEEPEST EXISTING ancestor of the
//!    resolved path is canonicalized and must still start with the
//!    root — a symlink planted inside the store (or a granted dir)
//!    pointing outside is refused at petition time, not followed.

use std::borrow::Cow;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// A canonicalized directory that confines resolutions into itself.
#[derive(Debug, Clone)]
pub struct BrokeredDir {
    root: PathBuf,
}

/// Typed refusal from path brokering (feeds
/// [`crate::plugin::PermissionError`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRefusal {
    /// The petition tried to leave the confining root (traversal or
    /// absolute-into-relative).
    Escape(String),
    /// A component was malformed (empty, NUL, a Windows prefix, …).
    Malformed(String),
    /// A symlink inside the confined tree resolves outside it.
    SymlinkEscape(String),
    /// The confining root itself could not be established.
    RootUnavailable(String),
}

impl fmt::Display for PathRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Escape(p) => write!(f, "path {p:?} escapes the confined root"),
            Self::Malformed(p) => write!(f, "path {p:?} is malformed"),
            Self::SymlinkEscape(p) => {
                write!(
                    f,
                    "path {p:?} resolves through a symlink outside the confined root"
                )
            }
            Self::RootUnavailable(e) => write!(f, "confined root unavailable: {e}"),
        }
    }
}

impl BrokeredDir {
    /// Establish a confining root. The directory is created if absent
    /// and canonicalized (symlink-free base for the physical check).
    pub fn establish(root: &Path) -> Result<Self, PathRefusal> {
        std::fs::create_dir_all(root)
            .map_err(|e| PathRefusal::RootUnavailable(format!("{}: {e}", root.display())))?;
        let root = root
            .canonicalize()
            .map_err(|e| PathRefusal::RootUnavailable(format!("{}: {e}", root.display())))?;
        Ok(Self { root })
    }

    /// The canonicalized root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a RELATIVE petition inside the root (session-store
    /// addressing mode). Absolute petitions refuse here — they are the
    /// grant resolver's job ([`Self::admit_absolute`]).
    pub fn resolve_relative(&self, petition: &str) -> Result<PathBuf, PathRefusal> {
        if petition.contains('\0') {
            return Err(PathRefusal::Malformed(petition.to_string()));
        }
        let p = Path::new(petition);
        if p.is_absolute() {
            return Err(PathRefusal::Escape(petition.to_string()));
        }
        // Layer 1 — lexical.
        let mut depth: usize = 0;
        let mut resolved = self.root.clone();
        for comp in p.components() {
            match comp {
                Component::Normal(c) => {
                    if c.is_empty() {
                        return Err(PathRefusal::Malformed(petition.to_string()));
                    }
                    resolved.push(c);
                    depth += 1;
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if depth == 0 {
                        return Err(PathRefusal::Escape(petition.to_string()));
                    }
                    resolved.pop();
                    depth -= 1;
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathRefusal::Escape(petition.to_string()));
                }
            }
        }
        // Layer 2 — physical.
        self.physical_check(&resolved, petition)?;
        Ok(resolved)
    }

    /// Admit an ABSOLUTE petition that must fall inside this (granted)
    /// directory. Lexical layer: refuse `..`/`.` components outright
    /// (a grant petition has no business traversing); physical layer as
    /// above.
    pub fn admit_absolute(&self, petition: &str) -> Result<PathBuf, PathRefusal> {
        if petition.contains('\0') {
            return Err(PathRefusal::Malformed(petition.to_string()));
        }
        let spelled = normalize_separators(petition);
        let p = Path::new(spelled.as_ref());
        if !p.is_absolute() {
            return Err(PathRefusal::Malformed(petition.to_string()));
        }
        if p.components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(PathRefusal::Escape(petition.to_string()));
        }
        // Prefix check in CANONICAL space: the petition may spell the
        // granted dir through an alias (macOS `/tmp` → `/private/tmp`),
        // so canonicalize the deepest existing ancestor, rejoin the
        // not-yet-existing remainder, and compare against the
        // canonicalized root. This is also the symlink defense: a link
        // inside the grant resolving outside lands outside the root.
        let canonical = self.canonicalize_deepest(p, petition)?;
        if !grant_prefix_contains(&self.root, &canonical) {
            return Err(PathRefusal::Escape(petition.to_string()));
        }
        Ok(canonical)
    }

    /// Canonicalize the deepest existing ancestor of `p` and rejoin the
    /// remaining (not-yet-existing) components.
    fn canonicalize_deepest(&self, p: &Path, petition: &str) -> Result<PathBuf, PathRefusal> {
        let mut probe: &Path = p;
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if probe.exists() {
                let canon = probe.canonicalize().map_err(|e| {
                    PathRefusal::RootUnavailable(format!("{}: {e}", probe.display()))
                })?;
                let mut joined = canon;
                for c in tail.iter().rev() {
                    joined.push(c);
                }
                return Ok(joined);
            }
            match (probe.parent(), probe.file_name()) {
                (Some(parent), Some(name)) => {
                    tail.push(name.to_os_string());
                    probe = parent;
                }
                _ => return Err(PathRefusal::Escape(petition.to_string())),
            }
        }
    }

    /// Canonicalize the deepest existing ancestor of `resolved` and
    /// require it to stay under the root — the symlink defense.
    fn physical_check(&self, resolved: &Path, petition: &str) -> Result<(), PathRefusal> {
        let mut probe: &Path = resolved;
        loop {
            if probe.exists() {
                let canon = probe.canonicalize().map_err(|e| {
                    PathRefusal::RootUnavailable(format!("{}: {e}", probe.display()))
                })?;
                if !canon.starts_with(&self.root) {
                    return Err(PathRefusal::SymlinkEscape(petition.to_string()));
                }
                return Ok(());
            }
            match probe.parent() {
                Some(parent) => probe = parent,
                // Ran past the filesystem root without finding anything
                // existing — the root itself exists, so this cannot
                // happen for in-root paths; refuse defensively.
                None => return Err(PathRefusal::Escape(petition.to_string())),
            }
        }
    }
}

/// Separator spelling of a petition before it is parsed.
///
/// Unix: unchanged.
#[cfg(not(windows))]
fn normalize_separators(petition: &str) -> Cow<'_, str> {
    Cow::Borrowed(petition)
}

/// Separator spelling of a petition before it is parsed.
///
/// Windows (W-33): inside a verbatim (`\\?\`) path `Path` splits on `\`
/// only, so `ROOT/a/../ok.txt` would parse as one `Normal` component and
/// the lexical `..` refusal would never see it. Every `/` becomes `\`
/// first; outside verbatim paths `/` is already a separator, so this
/// changes nothing there.
#[cfg(windows)]
fn normalize_separators(petition: &str) -> Cow<'_, str> {
    if petition.contains('/') {
        Cow::Owned(petition.replace('/', "\\"))
    } else {
        Cow::Borrowed(petition)
    }
}

/// Does granted root `root` contain `p` (path-component prefix)?
///
/// Unix: exactly [`Path::starts_with`].
#[cfg(not(windows))]
fn grant_prefix_contains(root: &Path, p: &Path) -> bool {
    p.starts_with(root)
}

/// Does granted root `root` contain `p` (path-component prefix)?
///
/// Windows (W-22): components compare case-insensitively (drive letter,
/// UNC server/share, and every normal component), and a disk or UNC
/// prefix equals its verbatim spelling (`C:` = `\\?\C:`,
/// `\\srv\sh` = `\\?\UNC\srv\sh`) because `canonicalize` returns
/// verbatim paths. Root markers are structural (both sides are absolute)
/// and are skipped, so verbatim UNC without a trailing root still
/// compares.
#[cfg(windows)]
fn grant_prefix_contains(root: &Path, p: &Path) -> bool {
    use std::path::Prefix;

    #[derive(PartialEq)]
    enum Key {
        Disk(u8),
        Unc(String, String),
        Other(String),
        Normal(String),
    }
    fn fold(s: &std::ffi::OsStr) -> String {
        s.to_string_lossy().to_lowercase()
    }
    fn key(c: Component<'_>) -> Option<Key> {
        Some(match c {
            Component::Prefix(pre) => match pre.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Key::Disk(d.to_ascii_uppercase()),
                Prefix::UNC(srv, sh) | Prefix::VerbatimUNC(srv, sh) => {
                    Key::Unc(fold(srv), fold(sh))
                }
                _ => Key::Other(fold(pre.as_os_str())),
            },
            Component::RootDir => return None,
            Component::Normal(n) => Key::Normal(fold(n)),
            Component::CurDir => Key::Other(".".into()),
            Component::ParentDir => Key::Other("..".into()),
        })
    }
    let mut rk = root.components().filter_map(key);
    let mut pk = p.components().filter_map(key);
    loop {
        match (rk.next(), pk.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) if a == b => {}
            _ => return false,
        }
    }
}

/// The session store: one [`BrokeredDir`] per plugin id under a common
/// root the embedder chooses (ES §35.2 scratch discipline applies to
/// the location; this module only confines).
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// `root` is the store root for ALL plugins (e.g.
    /// `<app state dir>/plugin-sessions`).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The confined per-plugin directory, created on first use. The
    /// plugin id has already passed the manifest `^[a-z0-9.]+$` pattern,
    /// so it is path-safe by construction (asserted anyway).
    pub fn for_plugin(&self, plugin_id: &str) -> Result<BrokeredDir, PathRefusal> {
        assert!(
            !plugin_id.is_empty()
                && plugin_id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.'),
            "plugin id must have passed manifest validation before reaching the store"
        );
        BrokeredDir::establish(&self.root.join(plugin_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "appname-session-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn relative_resolution_stays_inside() {
        let root = tmp("rel");
        let dir = BrokeredDir::establish(&root).unwrap();
        let p = dir.resolve_relative("presets/warm.json").unwrap();
        assert!(p.starts_with(dir.root()));
        // `.` and interior `..` that stay inside are fine:
        let q = dir.resolve_relative("a/./b/../c").unwrap();
        assert_eq!(q, dir.root().join("a/c"));
    }

    #[test]
    fn traversal_and_absolute_refuse_typed() {
        let root = tmp("esc");
        let dir = BrokeredDir::establish(&root).unwrap();
        assert!(matches!(
            dir.resolve_relative("../outside.txt"),
            Err(PathRefusal::Escape(_))
        ));
        assert!(matches!(
            dir.resolve_relative("a/../../outside.txt"),
            Err(PathRefusal::Escape(_))
        ));
        assert!(matches!(
            dir.resolve_relative("/etc/passwd"),
            Err(PathRefusal::Escape(_))
        ));
        assert!(matches!(
            dir.resolve_relative("a\0b"),
            Err(PathRefusal::Malformed(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_refused() {
        let root = tmp("sym");
        let outside = tmp("sym-outside");
        let dir = BrokeredDir::establish(&root).unwrap();
        std::os::unix::fs::symlink(&outside, dir.root().join("link")).unwrap();
        assert!(matches!(
            dir.resolve_relative("link/data.bin"),
            Err(PathRefusal::SymlinkEscape(_))
        ));
        // The symlink itself (deepest existing = the link target) refuses
        // too — reading "the link" IS reading outside.
        assert!(matches!(
            dir.resolve_relative("link"),
            Err(PathRefusal::SymlinkEscape(_))
        ));
    }

    #[test]
    fn absolute_admission_is_prefix_and_symlink_checked() {
        let root = tmp("adm");
        let dir = BrokeredDir::establish(&root).unwrap();
        let inside = dir.root().join("ok.txt");
        let admitted = dir.admit_absolute(inside.to_str().unwrap()).unwrap();
        assert_eq!(admitted, inside);
        #[cfg(not(windows))]
        let outside = "/etc/passwd";
        #[cfg(windows)]
        let outside = r"C:\Windows\win.ini";
        assert!(matches!(
            dir.admit_absolute(outside),
            Err(PathRefusal::Escape(_))
        ));
        // Traversal inside an absolute petition refuses outright:
        let sneaky = format!("{}/a/../ok.txt", dir.root().display());
        assert!(matches!(
            dir.admit_absolute(&sneaky),
            Err(PathRefusal::Escape(_))
        ));
        // Prefix trickery: /root-sibling ("/tmp/xyzXX" vs "/tmp/xyzXX2")
        let sibling = format!("{}2/f.txt", dir.root().display());
        assert!(matches!(
            dir.admit_absolute(&sibling),
            Err(PathRefusal::Escape(_))
        ));
    }

    /// W-33: forward-slash traversal inside a verbatim (`\\?\`) petition is
    /// refused like backslash traversal (O-WIN 0029's admitted spelling).
    #[cfg(windows)]
    #[test]
    fn windows_verbatim_forward_slash_traversal_refuses() {
        let root = tmp("w33");
        let dir = BrokeredDir::establish(&root).unwrap();
        let verbatim = dir.root().display().to_string();
        assert!(verbatim.starts_with(r"\\?\"), "canonical root is verbatim");
        for petition in [
            format!("{verbatim}/a/../ok.txt"),
            format!("{verbatim}/../outside.txt"),
            format!("{verbatim}\\a/..\\ok.txt"),
            format!("{verbatim}/./ok.txt"),
        ] {
            assert!(
                matches!(dir.admit_absolute(&petition), Err(PathRefusal::Escape(_))),
                "{petition} must refuse"
            );
        }
        // A plain in-scope forward-slash spelling still admits.
        let admitted = dir.admit_absolute(&format!("{verbatim}/ok.txt")).unwrap();
        assert_eq!(admitted, dir.root().join("ok.txt"));
    }

    /// W-22: the Windows grant prefix compare ignores case and separator
    /// spelling, and is still a component prefix (no sibling match).
    #[cfg(windows)]
    #[test]
    fn windows_admission_ignores_case_and_separators() {
        let root = tmp("win");
        let dir = BrokeredDir::establish(&root).unwrap();
        let spelled = format!("{}/ok.txt", root.display())
            .replace('\\', "/")
            .to_lowercase();
        let admitted = dir.admit_absolute(&spelled).unwrap();
        assert!(grant_prefix_contains(dir.root(), &admitted));
        let sibling = format!("{}2\\f.txt", root.display());
        assert!(matches!(
            dir.admit_absolute(&sibling),
            Err(PathRefusal::Escape(_))
        ));
        // Disk vs verbatim disk, any case, compare equal component-wise.
        assert!(grant_prefix_contains(
            Path::new(r"\\?\C:\Users\A"),
            Path::new(r"c:\users\a\f.txt")
        ));
        assert!(grant_prefix_contains(
            Path::new(r"\\?\UNC\Srv\Share\d"),
            Path::new(r"\\srv\share\D\f.txt")
        ));
        assert!(!grant_prefix_contains(
            Path::new(r"C:\Users\a"),
            Path::new(r"C:\Users\ab\f.txt")
        ));
        assert!(!grant_prefix_contains(
            Path::new(r"C:\Users\a"),
            Path::new(r"D:\Users\a\f.txt")
        ));
    }

    #[test]
    fn store_partitions_by_plugin_id() {
        let store = SessionStore::new(tmp("store"));
        let a = store.for_plugin("org.example.a").unwrap();
        let b = store.for_plugin("org.example.b").unwrap();
        assert_ne!(a.root(), b.root());
        // a cannot resolve into b:
        assert!(matches!(
            a.resolve_relative("../org.example.b/steal.txt"),
            Err(PathRefusal::Escape(_))
        ));
    }
}
