// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Capabilities and grants (ES §25.3).
//!
//! A [`Capability`] is what a manifest may REQUEST (the closed §25.2
//! `permissions` enum). A [`Grant`] is what the user (or the silent
//! install rule for registration capabilities) actually GRANTED —
//! fs grants are per-directory, so a grant carries data a capability
//! does not. Grants persist as the `granted` strings of
//! `appname.pluginstate.v1` (§25.6): `"filter.register"`,
//! `"fs.read:/home/a/textures"`, `"net"`, …

use std::fmt;
use std::path::{Path, PathBuf};

/// The closed §25.2 manifest permission enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    FilterRegister,
    ToolRegister,
    CodecRegister,
    PanelRegister,
    MenuAction,
    FsRead,
    FsWrite,
    Net,
    Clipboard,
}

impl Capability {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "filter.register" => Self::FilterRegister,
            "tool.register" => Self::ToolRegister,
            "codec.register" => Self::CodecRegister,
            "panel.register" => Self::PanelRegister,
            "menu.action" => Self::MenuAction,
            "fs.read" => Self::FsRead,
            "fs.write" => Self::FsWrite,
            "net" => Self::Net,
            "clipboard" => Self::Clipboard,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FilterRegister => "filter.register",
            Self::ToolRegister => "tool.register",
            Self::CodecRegister => "codec.register",
            Self::PanelRegister => "panel.register",
            Self::MenuAction => "menu.action",
            Self::FsRead => "fs.read",
            Self::FsWrite => "fs.write",
            Self::Net => "net",
            Self::Clipboard => "clipboard",
        }
    }

    /// §25.3 capability table row 1: registration capabilities are
    /// granted silently at install (registration is inert until invoked);
    /// everything else needs an explicit user grant.
    pub fn granted_silently(self) -> bool {
        matches!(
            self,
            Self::FilterRegister
                | Self::ToolRegister
                | Self::CodecRegister
                | Self::PanelRegister
                | Self::MenuAction
        )
    }
}

/// One persisted grant (§25.6 `granted` string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// A registration capability (silent-grant class), e.g.
    /// `"filter.register"`.
    Register(Capability),
    /// Per-directory read grant: `"fs.read:<abs dir>"`.
    FsRead(PathBuf),
    /// Per-directory write grant: `"fs.write:<abs dir>"`. Write implies
    /// nothing about read (tested; the two are independent scopes).
    FsWrite(PathBuf),
    /// All-or-nothing per plugin (§25.3). Parsed and persisted, but the
    /// v1 sandboxed tier DENIES net regardless — see [`crate::plugin::NetPolicy`].
    Net,
    /// Parsed and persisted; the v1 sandboxed tier has no clipboard
    /// host call (spec table: sandboxed clipboard = off).
    Clipboard,
}

impl Grant {
    /// Parse a `appname.pluginstate.v1` grant string. Strict: unknown
    /// forms and relative fs paths refuse (`None`).
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(cap) = Capability::parse(s) {
            return match cap {
                Capability::Net => Some(Self::Net),
                Capability::Clipboard => Some(Self::Clipboard),
                c if c.granted_silently() => Some(Self::Register(c)),
                // Bare "fs.read"/"fs.write" without a directory is not a
                // valid GRANT (the grant is per-directory, §25.3).
                _ => None,
            };
        }
        #[cfg(not(windows))]
        let dir_grant = |p: &str| -> Option<PathBuf> {
            let path = Path::new(p);
            (path.is_absolute() && !p.is_empty()).then(|| path.to_path_buf())
        };
        #[cfg(windows)]
        let dir_grant = windows_dir_grant;
        if let Some(p) = s.strip_prefix("fs.read:") {
            return dir_grant(p).map(Self::FsRead);
        }
        if let Some(p) = s.strip_prefix("fs.write:") {
            return dir_grant(p).map(Self::FsWrite);
        }
        None
    }

    /// The persisted string form.
    pub fn to_grant_string(&self) -> String {
        match self {
            Self::Register(c) => c.as_str().to_string(),
            Self::FsRead(p) => format!("fs.read:{}", p.display()),
            Self::FsWrite(p) => format!("fs.write:{}", p.display()),
            Self::Net => "net".to_string(),
            Self::Clipboard => "clipboard".to_string(),
        }
    }

    /// The capability this grant instantiates.
    pub fn capability(&self) -> Capability {
        match self {
            Self::Register(c) => *c,
            Self::FsRead(_) => Capability::FsRead,
            Self::FsWrite(_) => Capability::FsWrite,
            Self::Net => Capability::Net,
            Self::Clipboard => Capability::Clipboard,
        }
    }
}

/// W-22 (Windows arm of [`Grant::parse`]): a grant directory on Windows
/// is a drive-letter absolute path (`C:\…`, `\\?\C:\…`) or a UNC path
/// (`\\server\share\…`, `\\?\UNC\server\share\…`) — a disk or UNC
/// prefix followed by a root. Rooted-without-drive (`\foo`),
/// drive-relative (`C:foo`), the device namespace (`\\.\COM1`), other
/// verbatim forms and any `..` component refuse (an interior `.` is
/// dropped by component normalization; in a verbatim path it refuses).
/// The stored path is normalized to `\` separators (verbatim paths,
/// where `/` is not a separator, are kept as spelled), so `C:/Users/a`
/// and `C:\Users\a` are the same grant. Case is NOT folded here; the gate's prefix
/// comparison is case-insensitive (`session::grant_prefix_contains`).
#[cfg(windows)]
fn windows_dir_grant(p: &str) -> Option<PathBuf> {
    use std::path::{Component, Prefix};
    if p.is_empty() || p.contains('\0') {
        return None;
    }
    let verbatim = matches!(
        Path::new(p).components().next(),
        Some(Component::Prefix(pre)) if pre.kind().is_verbatim()
    );
    let spelled = if verbatim {
        p.to_string()
    } else {
        p.replace('/', "\\")
    };
    let path = Path::new(&spelled);
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Prefix(pre))
            if matches!(
                pre.kind(),
                Prefix::Disk(_)
                    | Prefix::VerbatimDisk(_)
                    | Prefix::UNC(..)
                    | Prefix::VerbatimUNC(..)
            ) => {}
        _ => return None,
    }
    if comps.next() != Some(Component::RootDir) {
        return None;
    }
    if comps.any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(path.components().collect())
}

impl fmt::Display for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_grant_string())
    }
}

/// A plugin's granted set, deduplicated, order-preserving.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantSet {
    grants: Vec<Grant>,
}

impl GrantSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Grant> {
        self.grants.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// Insert; duplicate grants are a no-op (returns false).
    pub fn insert(&mut self, g: Grant) -> bool {
        if self.grants.contains(&g) {
            return false;
        }
        self.grants.push(g);
        true
    }

    /// Remove an exact grant; returns whether it was present.
    pub fn remove(&mut self, g: &Grant) -> bool {
        let before = self.grants.len();
        self.grants.retain(|x| x != g);
        before != self.grants.len()
    }

    pub fn has_capability(&self, c: Capability) -> bool {
        self.grants.iter().any(|g| g.capability() == c)
    }

    /// The granted read directories (for the gate).
    pub fn fs_read_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.grants.iter().filter_map(|g| match g {
            Grant::FsRead(p) => Some(p),
            _ => None,
        })
    }

    /// The granted write directories (for the gate).
    pub fn fs_write_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.grants.iter().filter_map(|g| match g {
            Grant::FsWrite(p) => Some(p),
            _ => None,
        })
    }

    /// §25.6 load-order step "permission diff against stored grants":
    /// the manifest capabilities NOT yet covered here, silent-class
    /// first resolved (they auto-grant), so the returned list is exactly
    /// what needs a user re-prompt.
    pub fn ungranted_of(&self, requested: &[Capability]) -> Vec<Capability> {
        requested
            .iter()
            .copied()
            .filter(|c| !c.granted_silently() && !self.has_capability(*c))
            .collect()
    }

    /// Auto-grant the silent registration class for the given manifest
    /// request list (install-time rule, §25.3 row 1).
    pub fn grant_silent(&mut self, requested: &[Capability]) {
        for c in requested {
            if c.granted_silently() {
                self.insert(Grant::Register(*c));
            }
        }
    }
}

impl FromIterator<Grant> for GrantSet {
    fn from_iter<T: IntoIterator<Item = Grant>>(iter: T) -> Self {
        let mut s = Self::new();
        for g in iter {
            s.insert(g);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_strings_round_trip() {
        #[cfg(not(windows))]
        let (read, write) = ("fs.read:/home/a/textures", "fs.write:/tmp/out");
        #[cfg(windows)]
        let (read, write) = (r"fs.read:C:\Users\a\textures", r"fs.write:C:\Temp\out");
        for s in [
            "filter.register",
            "menu.action",
            read,
            write,
            "net",
            "clipboard",
        ] {
            let g = Grant::parse(s).unwrap_or_else(|| panic!("{s} must parse"));
            assert_eq!(g.to_grant_string(), s);
        }
    }

    #[test]
    fn hostile_grant_strings_refuse() {
        for s in [
            "fs.read", // per-directory grant needs a directory
            "fs.write",
            "fs.read:",           // empty path
            "fs.read:relative/p", // relative refused
            "fs.read:../..",
            "sudo",
            "",
            "fs.readx:/tmp",
        ] {
            assert!(Grant::parse(s).is_none(), "{s:?} must refuse");
        }
    }

    /// W-22: Windows grant paths are drive-letter or UNC absolute.
    #[cfg(windows)]
    #[test]
    fn windows_grant_paths_are_drive_or_unc() {
        let unc = Grant::parse(r"fs.read:\\server\share\dir").expect("UNC grant must parse");
        assert_eq!(unc.to_grant_string(), r"fs.read:\\server\share\dir");
        assert!(Grant::parse(r"fs.write:\\?\C:\Users\a").is_some());
        for s in [
            r"fs.read:\foo",            // rooted without a drive
            r"fs.read:C:foo",           // drive-relative
            r"fs.read:\\.\COM1",        // device namespace
            r"fs.read:\\?\Volume{x}",   // verbatim, neither disk nor UNC
            r"fs.read:C:\a\..\b",       // traversal
            "fs.read:/home/a/textures", // a Unix path is not absolute here
        ] {
            assert!(Grant::parse(s).is_none(), "{s:?} must refuse");
        }
    }

    /// W-22: separators normalize — both spellings are one grant, stored
    /// with backslashes.
    #[cfg(windows)]
    #[test]
    fn windows_grant_separators_normalize() {
        let fwd = Grant::parse("fs.read:C:/Users/a").unwrap();
        let back = Grant::parse(r"fs.read:C:\Users\a").unwrap();
        assert_eq!(fwd, back);
        assert_eq!(fwd.to_grant_string(), r"fs.read:C:\Users\a");
        let mut set = GrantSet::new();
        assert!(set.insert(fwd));
        assert!(!set.insert(back), "same grant, deduplicated");
    }

    #[test]
    fn silent_class_and_diff() {
        let req = [
            Capability::FilterRegister,
            Capability::FsRead,
            Capability::Net,
        ];
        let mut g = GrantSet::new();
        g.grant_silent(&req);
        assert!(g.has_capability(Capability::FilterRegister));
        // fs.read and net remain to prompt:
        assert_eq!(
            g.ungranted_of(&req),
            vec![Capability::FsRead, Capability::Net]
        );
        g.insert(Grant::FsRead(PathBuf::from("/tmp/tex")));
        assert_eq!(g.ungranted_of(&req), vec![Capability::Net]);
    }

    #[test]
    fn read_and_write_scopes_are_independent() {
        let g: GrantSet = [Grant::FsWrite(PathBuf::from("/tmp/out"))]
            .into_iter()
            .collect();
        assert!(g.has_capability(Capability::FsWrite));
        assert!(!g.has_capability(Capability::FsRead));
        assert_eq!(g.fs_read_dirs().count(), 0);
        assert_eq!(g.fs_write_dirs().count(), 1);
    }
}
