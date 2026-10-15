// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.7 (arenas, handles, side-table lifecycle).
//!
//! [NORMATIVE, ES §2.7] Insert registers `id → value`; inserting a UUID that
//! is already registered is a logic error and MUST fail rather than silently
//! replace (duplicate ids indicate a corrupted command or file). Remove
//! returns the value; undo re-inserts the removed node **with its original
//! UUID** and every persistent reference re-resolves, because persistent
//! references are always UUIDs.
//!
//! Backing note: the ES sketches `slotmap` + a UUID side table for O(1)
//! integer-keyed access; `slotmap` has no deps/ record at CP-0.1, so this
//! arena is a UUID-keyed map with the same public contract (no in-memory
//! key type is exposed at all yet, which trivially satisfies "arena indices
//! are never serialized"). Swapping in the slotmap backing is an internal
//! change — recorded as an open question in the lane log.

use std::collections::HashMap;

use uuid::Uuid;

/// Values addressable by UUID identity.
pub trait HasId {
    fn uuid(&self) -> Uuid;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArenaError {
    /// The UUID is already registered (ES §2.7: MUST fail, never replace).
    DuplicateId(Uuid),
}

/// UUID-addressed object arena (ES §2.7 contract).
#[derive(Clone)]
pub struct Arena<T: HasId> {
    by_id: HashMap<Uuid, T>,
}

impl<T: HasId> Arena<T> {
    pub fn new() -> Arena<T> {
        Arena {
            by_id: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn contains(&self, id: impl Into<Uuid>) -> bool {
        self.by_id.contains_key(&id.into())
    }

    pub fn get(&self, id: impl Into<Uuid>) -> Option<&T> {
        self.by_id.get(&id.into())
    }

    /// Unordered iteration (like the tile map, consumers needing determinism
    /// order by their own structure — the tree's `children` lists).
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.by_id.values()
    }

    // Crate-internal mutation doors (plan rule 5).

    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn insert(&mut self, value: T) -> Result<(), ArenaError> {
        let id = value.uuid();
        if self.by_id.contains_key(&id) {
            debug_assert!(false, "Arena::insert duplicate id {id} (ES §2.7)");
            return Err(ArenaError::DuplicateId(id));
        }
        self.by_id.insert(id, value);
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn get_mut(&mut self, id: impl Into<Uuid>) -> Option<&mut T> {
        self.by_id.get_mut(&id.into())
    }

    /// Removes both directions; after removal `get(id)` is `None` until the
    /// *same object* is re-inserted (ES §2.7 undo re-insertion).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn remove(&mut self, id: impl Into<Uuid>) -> Option<T> {
        self.by_id.remove(&id.into())
    }
}

impl<T: HasId> Default for Arena<T> {
    fn default() -> Arena<T> {
        Arena::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Node {
        id: Uuid,
        tag: u32,
    }

    impl HasId for Node {
        fn uuid(&self) -> Uuid {
            self.id
        }
    }

    #[test]
    fn insert_get_remove_lifecycle() {
        let mut a: Arena<Node> = Arena::new();
        let id = Uuid::new_v4();
        a.insert(Node { id, tag: 1 }).unwrap();
        assert_eq!(a.get(id).unwrap().tag, 1);
        assert_eq!(a.len(), 1);

        let removed = a.remove(id).unwrap();
        assert_eq!(removed.tag, 1);
        assert!(a.get(id).is_none());
        assert!(a.is_empty());
    }

    #[test]
    fn duplicate_insert_fails_without_replacing() {
        let mut a: Arena<Node> = Arena::new();
        let id = Uuid::new_v4();
        a.insert(Node { id, tag: 1 }).unwrap();
        // debug_assert would fire under cargo test; exercise the release
        // contract through catch_unwind-free logic: check the Err path via
        // std::panic hook suppression is overkill — assert the error value
        // in a release-style call.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            a.insert(Node { id, tag: 2 })
        }));
        match result {
            // Debug build: the debug_assert fired (panic) — the map is
            // untouched either way.
            Err(_) => {}
            // Release build: explicit error.
            Ok(r) => assert_eq!(r, Err(ArenaError::DuplicateId(id))),
        }
        assert_eq!(a.get(id).unwrap().tag, 1, "original value must survive");
    }

    #[test]
    fn undo_reinsertion_with_original_uuid() {
        let mut a: Arena<Node> = Arena::new();
        let id = Uuid::new_v4();
        a.insert(Node { id, tag: 7 }).unwrap();
        let removed = a.remove(id).unwrap();
        assert!(a.get(id).is_none());
        // The same object comes back under the same UUID (ES §2.7).
        a.insert(removed).unwrap();
        assert_eq!(a.get(id).unwrap().tag, 7);
    }
}
