// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.1 (ID types), §0.3 (UUIDs).
//!
//! [NORMATIVE, ES §2.1] ID types are newtyped to prevent mixing; on-disk
//! identity is the UUID (arena indices are never serialized); UUIDs are
//! never reused — undo re-inserts the *same* node id, creating a new object
//! always draws a fresh UUID.
//!
//! [ES §0.3] UUIDs are UUIDv4 (random) for new objects. The round-1 proto
//! additionally derives engine-created ids as UUIDv5 over the creating
//! envelope's id (`derive_id`), so journal replay reproduces them — that
//! convention (and the uuid crate's `v5` feature) arrives with the command
//! engine in slice 2 (ES §9.6 determinism rule).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Fresh random identity (UUIDv4, ES §0.3).
            pub fn new() -> $name {
                $name(Uuid::new_v4())
            }

            pub const fn nil() -> $name {
                $name(Uuid::nil())
            }

            pub const fn from_uuid(u: Uuid) -> $name {
                $name(u)
            }

            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $name::new()
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                core::fmt::Display::fmt(&self.0, f)
            }
        }

        impl From<$name> for Uuid {
            fn from(v: $name) -> Uuid {
                v.0
            }
        }
    };
}

id_type!(DocumentId);
id_type!(LayerId);
id_type!(MaskId);
id_type!(OpId);
id_type!(ChannelId);
id_type!(PathId);
id_type!(AssetId);
id_type!(ProfileId);
id_type!(CommandId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ids_are_v4_and_distinct() {
        let a = LayerId::new();
        let b = LayerId::new();
        assert_ne!(a, b);
        assert_eq!(a.as_uuid().get_version_num(), 4);
    }

    #[test]
    fn ids_serialize_as_uuid_strings() {
        let id =
            LayerId::from_uuid(Uuid::parse_str("9b2f6c1e-4d3a-4f0e-9a51-2c7d8e5b0a44").unwrap());
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            "\"9b2f6c1e-4d3a-4f0e-9a51-2c7d8e5b0a44\""
        );
        let back: LayerId =
            serde_json::from_str("\"9b2f6c1e-4d3a-4f0e-9a51-2c7d8e5b0a44\"").unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn newtypes_do_not_mix() {
        // Compile-time property; assert the runtime shells still compare by
        // value within one type.
        let u = Uuid::new_v4();
        assert_eq!(LayerId::from_uuid(u), LayerId::from_uuid(u));
    }
}
