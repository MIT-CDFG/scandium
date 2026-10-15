// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Spec: ES §2.6 (`LayerNode` field semantics, `LockFlags`, `ColorLabel`),
//! §4.5 (the 27 blend modes — enum identity only here); DD ⑧; schema
//! `contracts/schemas/document.v1.json` (`blend`, `kind`, `locks`,
//! `colorLabel` vocabularies).
//!
//! CP-0.1 carries the node data model: raster layers and the group skeleton.
//! Compositing semantics (§4.4–§4.7), effects (§11.3), masks-on-layers
//! (§11.2), and the derived per-node `content_hash` (§4.6 folding) belong to
//! later rows; the fields and vocabularies they need are shaped here so the
//! schema bridge (slice 3) and the render lane consume a stable model.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::buffer::Buffer;
use crate::geom::Mat3;
use crate::graph::arena::HasId;
use crate::graph::id::{LayerId, MaskId};

/// The 27 blend modes (ES §4.5). [NORMATIVE] `repr(u32)` discriminants
/// `0..=26` in the §4.5 order — the same values and order the
/// `document.v1.json` `blend` enum publishes (camelCase strings, §2.8
/// precedence note). Formulas/WGSL are the render lane's (CP-0.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[repr(u32)]
pub enum BlendMode {
    Normal = 0,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    /// All 27, in discriminant order (schema array order).
    pub const ALL: [BlendMode; 27] = [
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];
}

/// Lock bits (ES §2.6). [NORMATIVE] exact bits; serialized as an array of
/// snake_case strings; "lock all" is the union — there is no distinct ALL
/// bit. Enforcement happens at command validation (slice 2+).
///
/// Hand-rolled (the `bitflags` crate has no deps/ record); the bit layout is
/// the §2.6 contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LockFlags(u32);

impl LockFlags {
    /// Paint may not change alpha coverage (§15).
    pub const TRANSPARENCY: LockFlags = LockFlags(1 << 0);
    /// No pixel/mask edits (paint, fill, filters).
    pub const PIXELS: LockFlags = LockFlags(1 << 1);
    /// No move/transform.
    pub const POSITION: LockFlags = LockFlags(1 << 2);
    /// The union (ES §2.6: no distinct ALL bit).
    pub const ALL: LockFlags = LockFlags(Self::TRANSPARENCY.0 | Self::PIXELS.0 | Self::POSITION.0);

    pub const fn empty() -> LockFlags {
        LockFlags(0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: LockFlags) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn union(self, other: LockFlags) -> LockFlags {
        LockFlags(self.0 | other.0)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl Serialize for LockFlags {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Bit-order emission of set flags (schema `locks` array,
        // uniqueItems). Bits 3..31 are reserved zero at v1.
        let mut names: Vec<&str> = Vec::new();
        for (flag, name) in [
            (LockFlags::TRANSPARENCY, "transparency"),
            (LockFlags::PIXELS, "pixels"),
            (LockFlags::POSITION, "position"),
        ] {
            if self.contains(flag) {
                names.push(name);
            }
        }
        names.serialize(s)
    }
}

impl<'de> Deserialize<'de> for LockFlags {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(d)?;
        let mut flags = LockFlags::empty();
        for n in &names {
            flags = flags.union(match n.as_str() {
                "transparency" => LockFlags::TRANSPARENCY,
                "pixels" => LockFlags::PIXELS,
                "position" => LockFlags::POSITION,
                // Schema shorthand: "all" is the union.
                "all" => LockFlags::ALL,
                // Unknown flag strings round-trip via the §2.8 wrapper at
                // the document-reader layer (slice 3); this direct decoder
                // is strict.
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "unknown lock flag: {other}"
                    )))
                }
            });
        }
        Ok(flags)
    }
}

/// Organizational label (ES §2.6); the unlabeled state is `Option::None`.
/// Never affects rendering.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorLabel {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Violet,
    Gray,
}

/// Layer content (ES §2.6). [NORMATIVE §2.6] every layer type lives in one
/// enum so the tree architecture never needs a rewrite; CP-0.1 ships the
/// `Raster` + `Group` rows and the enum is `#[non_exhaustive]` — the
/// remaining §2.6 variants (`Adjustment`, `Fill`, `Text`, `Shape`,
/// `SmartObject`) land with their payload types (plan CP-0.5/CP-1.0 rows).
#[non_exhaustive]
#[derive(Clone)]
pub enum LayerKind {
    Raster(Buffer),
    Group {
        /// Pass-through compositing when `false` (ES §4.7).
        isolated: bool,
    },
    /// §2.6/§12.1: a parametric adjustment layer — the referenced
    /// [`crate::graph::op_node::OpNode`] is the n = 1 op chain; the layer
    /// itself carries NO pixels (parameters, never baked pixels).
    Adjustment(crate::graph::id::OpId),
    /// §22 fill layer: parametric `solid` / `gradient` / `pattern` /
    /// `none` content, carries NO pixels. The payload is the frozen
    /// `document.v1.json` `$defs/fill` object, decoded by
    /// [`crate::engine::fill_reg`].
    Fill(FillLayerData),
    /// §11.5 smart object (v1 minimal: embedded raster sources).
    SmartObject(SmartObjectData),
    /// §20.2 shape layer (CP-1.0 vector doors): parametric vector content
    /// — a doc-path reference plus style; carries NO pixels (rasterization
    /// is the render row's §20.2 exact-coverage fill via the geometry
    /// service).
    Shape(ShapeLayerData),
    /// §23.1 text layer (CP-1.0 text doors, TEXTVEC Door 7): live,
    /// re-editable parametric content — the `appname.text.v1` sidecar
    /// carried OPAQUE (runs ARE the layer's string, §23.1 invariant; no
    /// second backing store) plus its content hash and LAID-OUT bounds.
    /// Carries NO pixels: rasterized glyph tiles are cache, never source
    /// of truth (§23.1); the render row composes via the TextLayout
    /// service (Door 6, parametric re-render — the approved D-48 posture).
    Text(TextLayerData),
}

/// §22 fill-layer payload: the decoded `$defs/fill` content plus the
/// wire object it came from.
///
/// Both halves are kept deliberately. The DECODED content is what the
/// compositor and the gradient engine consume, so neither re-parses JSON
/// per tile; the RAW object is what `document.v1.json` round-trips, and
/// keeping it verbatim preserves the §7.7 unknown fields that a v5.6
/// reader may care about and this v1 decoder deliberately ignores.
#[derive(Clone, Debug, PartialEq)]
pub struct FillLayerData {
    pub(crate) content: crate::engine::fill_reg::FillContent,
    pub(crate) wire: serde_json::Value,
}

impl FillLayerData {
    pub fn new(
        content: crate::engine::fill_reg::FillContent,
        wire: serde_json::Value,
    ) -> FillLayerData {
        FillLayerData { content, wire }
    }

    /// The decoded §22 content.
    pub fn content(&self) -> &crate::engine::fill_reg::FillContent {
        &self.content
    }

    /// The verbatim `$defs/fill` object this layer was created from.
    pub fn wire(&self) -> &serde_json::Value {
        &self.wire
    }
}

/// §23.1 `TextLayer` payload — core-side, text-blind: the sidecar bytes
/// are the `text/{layerId}.json` body (`appname.text.v1`, §7.5 worked
/// example), validated/normalized by the [`crate::engine::text_ops::
/// TextLayout`] service at every mutation; `bounds` is the LAID-OUT ink
/// hull the §40.7 family dirty rule reads ("D = laid-out bounds before ∪
/// after"); `content_hash` lives in the §7.4 identity space (render-DAG
/// dedup, cache keys).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayerData {
    pub(crate) sidecar: std::sync::Arc<Vec<u8>>,
    pub(crate) content_hash: uuid::Uuid,
    pub(crate) bounds: crate::geom::IRect,
}

impl TextLayerData {
    pub fn new(
        sidecar: std::sync::Arc<Vec<u8>>,
        content_hash: uuid::Uuid,
        bounds: crate::geom::IRect,
    ) -> TextLayerData {
        TextLayerData {
            sidecar,
            content_hash,
            bounds,
        }
    }

    pub fn sidecar(&self) -> &std::sync::Arc<Vec<u8>> {
        &self.sidecar
    }

    pub fn content_hash(&self) -> uuid::Uuid {
        self.content_hash
    }

    /// Laid-out bounds (document px, integer ink hull).
    pub fn bounds(&self) -> crate::geom::IRect {
        self.bounds
    }
}

/// §20.3 `ShapeLayer` payload — core-side, geometry-blind: `fill`/`stroke`
/// are the contract's §20.2 payload objects carried OPAQUE (validated at
/// the kind arms per contract; the render row interprets them), `bool_ops`
/// the non-destructive boolean stack (§20.3: sources stay editable;
/// composite path = left fold over `path` via the VectorGeometry seam).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeLayerData {
    pub(crate) path: crate::graph::id::PathId,
    pub(crate) fill: serde_json::Value,
    pub(crate) stroke: serde_json::Value,
    pub(crate) bool_ops: Vec<(
        crate::engine::vector_ops::VectorBoolOp,
        crate::graph::id::PathId,
    )>,
}

impl ShapeLayerData {
    pub fn path(&self) -> crate::graph::id::PathId {
        self.path
    }

    pub fn fill(&self) -> &serde_json::Value {
        &self.fill
    }

    pub fn stroke(&self) -> &serde_json::Value {
        &self.stroke
    }

    pub fn bool_ops(
        &self,
    ) -> &[(
        crate::engine::vector_ops::VectorBoolOp,
        crate::graph::id::PathId,
    )] {
        &self.bool_ops
    }
}

/// §11.5 smart-object payload. v1 minimal scope: `Embedded` sources are
/// content-addressed RASTER assets (the ingested-asset store — nested
/// `.appname` documents need the render-side nested composite and answer
/// typed until that row); `Linked` sources are typed-pending (drift
/// detection + the §8.1 threat surface are shell-side).
#[derive(Clone)]
pub struct SmartObjectData {
    pub(crate) source: SmartSource,
    /// The per-instance post-filter render cache (§11.5): persisted in the
    /// package, invalidated when source/params change. v1: the decoded
    /// embedded raster (filters compose at the render row).
    pub(crate) cached_composite: Buffer,
    /// §12.2 smart-filter chain (ordered, individually maskable).
    pub(crate) smart_filters: Vec<crate::graph::id::OpId>,
    pub(crate) place_transform: Mat3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmartSource {
    /// Content-addressed asset id in the engine store (§7.4 dedup).
    Embedded(uuid::Uuid),
}

impl SmartObjectData {
    pub fn source(&self) -> &SmartSource {
        &self.source
    }

    pub fn cached_composite(&self) -> &Buffer {
        &self.cached_composite
    }

    pub fn smart_filters(&self) -> &[crate::graph::id::OpId] {
        &self.smart_filters
    }

    pub fn place_transform(&self) -> &Mat3 {
        &self.place_transform
    }
}

impl LayerKind {
    /// Schema `kind` discriminator string (`document.v1.json`).
    pub fn kind_str(&self) -> &'static str {
        #[allow(clippy::match_like_matches_macro)]
        match self {
            LayerKind::Raster(_) => "raster",
            LayerKind::Group { .. } => "group",
            LayerKind::Adjustment(_) => "adjustment",
            LayerKind::Fill(_) => "fill", // document.v1 layer-kind enum row
            LayerKind::SmartObject(_) => "smartObject", // schema enum spelling (§2.8 precedence)
            LayerKind::Shape(_) => "shape",
            LayerKind::Text(_) => "text", // document.v1 layer-kind enum row
        }
    }
}

/// One node of the layer tree (ES §2.6, CP-0.1 field subset). Field defaults
/// and invariants follow the §2.6 table; the derived `content_hash` (§4.6)
/// and `effects` (§11.3) fields arrive with their owning lanes.
#[derive(Clone)]
pub struct LayerNode {
    pub(crate) id: LayerId,
    pub(crate) name: String,
    pub(crate) kind: LayerKind,
    pub(crate) blend: BlendMode,
    /// `0..=1`, whole-layer (clamped at command validation, slice 2).
    pub(crate) opacity: f32,
    /// `0..=1`, affects pixels not effects (§11.3).
    pub(crate) fill_opacity: f32,
    pub(crate) visible: bool,
    pub(crate) locks: LockFlags,
    /// Member of a clipping group (§11.2); meaningless on root.
    pub(crate) clip_to_below: bool,
    /// Applied in order (§11.2 stacking); resolution arrives with the mask
    /// arena (CP-0.5) — v1 trees keep this empty and `validate_tree` flags
    /// any entry as dangling.
    pub(crate) masks: Vec<MaskId>,
    /// Bottom-to-top z-order [NORMATIVE §2.6]: index 0 composites first.
    /// Non-empty only for `Group`.
    pub(crate) children: Vec<LayerId>,
    /// Layer-local → document (§18); identity until the transform commands
    /// land. Commands MUST reject singular transforms.
    pub(crate) transform: Mat3,
    pub(crate) color_label: Option<ColorLabel>,
}

impl LayerNode {
    /// A raster layer with the §2.6 defaults (localized default naming is
    /// the shell's job at creation, §27).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn new_raster(id: LayerId, name: impl Into<String>, pixels: Buffer) -> LayerNode {
        Self::with_kind(id, name, LayerKind::Raster(pixels))
    }

    /// A group node (`isolated = false` ⇒ pass-through, §4.7).
    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    pub(crate) fn new_group(id: LayerId, name: impl Into<String>, isolated: bool) -> LayerNode {
        Self::with_kind(id, name, LayerKind::Group { isolated })
    }

    /// §12.1 adjustment layer: references its single OpNode, carries no
    /// pixels.
    pub(crate) fn new_adjustment(
        id: LayerId,
        name: impl Into<String>,
        op: crate::graph::id::OpId,
    ) -> LayerNode {
        Self::with_kind(id, name, LayerKind::Adjustment(op))
    }

    /// §22 fill layer (parametric content, no pixels).
    pub(crate) fn new_fill(id: LayerId, name: impl Into<String>, data: FillLayerData) -> LayerNode {
        Self::with_kind(id, name, LayerKind::Fill(data))
    }

    /// §11.5 smart object (v1 embedded-raster minimal).
    pub(crate) fn new_smart_object(
        id: LayerId,
        name: impl Into<String>,
        data: SmartObjectData,
    ) -> LayerNode {
        Self::with_kind(id, name, LayerKind::SmartObject(data))
    }

    /// §20.2 shape layer (CP-1.0 vector doors).
    pub(crate) fn new_shape(
        id: LayerId,
        name: impl Into<String>,
        data: ShapeLayerData,
    ) -> LayerNode {
        Self::with_kind(id, name, LayerKind::Shape(data))
    }

    /// §23.1 text layer (CP-1.0 text doors, TEXTVEC Door 7).
    pub(crate) fn new_text(id: LayerId, name: impl Into<String>, data: TextLayerData) -> LayerNode {
        Self::with_kind(id, name, LayerKind::Text(data))
    }

    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    fn with_kind(id: LayerId, name: impl Into<String>, kind: LayerKind) -> LayerNode {
        LayerNode {
            id,
            name: name.into(),
            kind,
            blend: BlendMode::Normal,
            opacity: 1.0,
            fill_opacity: 1.0,
            visible: true,
            locks: LockFlags::empty(),
            clip_to_below: false,
            masks: Vec::new(),
            children: Vec::new(),
            transform: Mat3::IDENTITY,
            color_label: None,
        }
    }

    pub fn id(&self) -> LayerId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &LayerKind {
        &self.kind
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, LayerKind::Group { .. })
    }

    pub fn blend(&self) -> BlendMode {
        self.blend
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn fill_opacity(&self) -> f32 {
        self.fill_opacity
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn locks(&self) -> LockFlags {
        self.locks
    }

    pub fn clip_to_below(&self) -> bool {
        self.clip_to_below
    }

    /// Group isolation flag (§4.7); false on non-groups.
    pub fn isolated(&self) -> bool {
        matches!(self.kind, LayerKind::Group { isolated: true })
    }

    /// Apply the §2.6 shared prop block (value-delta restore + kinds).
    pub(crate) fn apply_props_block(&mut self, b: &crate::engine::delta::LayerPropsBlock) {
        self.name = b.name.clone();
        self.blend = b.blend;
        self.opacity = b.opacity;
        self.fill_opacity = b.fill_opacity;
        self.visible = b.visible;
        self.locks = b.locks;
        self.clip_to_below = b.clip_to_below;
        self.color_label = b.color_label;
        self.transform = b.transform;
        if let LayerKind::Group { isolated } = &mut self.kind {
            *isolated = b.isolated;
        }
    }

    /// The adjustment layer's op reference (§2.6), if that is this node's
    /// kind.
    pub fn adjustment_op(&self) -> Option<crate::graph::id::OpId> {
        match &self.kind {
            LayerKind::Adjustment(op) => Some(*op),
            _ => None,
        }
    }

    /// Fill-layer payload access (§22).
    pub fn fill_data(&self) -> Option<&FillLayerData> {
        match &self.kind {
            LayerKind::Fill(d) => Some(d),
            _ => None,
        }
    }

    /// Smart-object payload access (§11.5).
    pub fn smart_object(&self) -> Option<&SmartObjectData> {
        match &self.kind {
            LayerKind::SmartObject(d) => Some(d),
            _ => None,
        }
    }

    pub(crate) fn smart_object_mut(&mut self) -> Option<&mut SmartObjectData> {
        match &mut self.kind {
            LayerKind::SmartObject(d) => Some(d),
            _ => None,
        }
    }

    /// Shape-layer payload access (§20.2).
    pub fn shape_data(&self) -> Option<&ShapeLayerData> {
        match &self.kind {
            LayerKind::Shape(d) => Some(d),
            _ => None,
        }
    }

    pub(crate) fn shape_data_mut(&mut self) -> Option<&mut ShapeLayerData> {
        match &mut self.kind {
            LayerKind::Shape(d) => Some(d),
            _ => None,
        }
    }

    /// Text-layer payload access (§23.1, TEXTVEC Door 7).
    pub fn text_data(&self) -> Option<&TextLayerData> {
        match &self.kind {
            LayerKind::Text(d) => Some(d),
            _ => None,
        }
    }

    pub(crate) fn text_data_mut(&mut self) -> Option<&mut TextLayerData> {
        match &mut self.kind {
            LayerKind::Text(d) => Some(d),
            _ => None,
        }
    }

    /// `text.to_shape`'s recorded kind swap (§40.7 "SetProp kind +
    /// geometry payloads" — delta-invertible in BOTH directions; the
    /// [`crate::engine::delta::TreeOp::SetContentKind`] value door is the
    /// only caller).
    pub(crate) fn set_content_kind(&mut self, kind: LayerKind) {
        self.kind = kind;
    }

    pub fn masks(&self) -> &[MaskId] {
        &self.masks
    }

    /// Bottom-to-top z-order (ES §2.6): index 0 composites first; the Layers
    /// panel displays the reverse.
    pub fn children(&self) -> &[LayerId] {
        &self.children
    }

    pub fn transform(&self) -> &Mat3 {
        &self.transform
    }

    pub fn color_label(&self) -> Option<ColorLabel> {
        self.color_label
    }

    /// Raster pixels, if this is a raster layer.
    pub fn pixels(&self) -> Option<&Buffer> {
        match &self.kind {
            LayerKind::Raster(b) => Some(b),
            _ => None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // slice-2 command-engine door (in-crate tests exercise it today)
    /// Replace the raster payload wholesale (CheckpointOnly depth/resize
    /// kinds — snapshot-inverse steps, §10.4).
    pub(crate) fn set_pixels(&mut self, pixels: Buffer) -> bool {
        match &mut self.kind {
            LayerKind::Raster(b) => {
                *b = pixels;
                true
            }
            _ => false,
        }
    }

    /// §12.4 rasterize: the kind swaps to plain Raster wholesale
    /// (CheckpointOnly — the engine's snapshot is the inverse).
    pub(crate) fn replace_kind_with_raster(&mut self, pixels: Buffer) {
        self.kind = LayerKind::Raster(pixels);
    }

    pub(crate) fn pixels_mut(&mut self) -> Option<&mut Buffer> {
        match &mut self.kind {
            LayerKind::Raster(b) => Some(b),
            _ => None,
        }
    }
}

impl HasId for LayerNode {
    fn uuid(&self) -> Uuid {
        self.id.as_uuid()
    }
}

/// Summary Debug. [ES §1.6] user-authored strings (layer names) never reach
/// diagnostics — the name is reported as a length.
impl core::fmt::Debug for LayerNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LayerNode")
            .field("id", &self.id)
            .field("kind", &self.kind.kind_str())
            .field("name_len", &self.name.len())
            .field("children", &self.children.len())
            .field("visible", &self.visible)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_serialization_matches_schema_vocabulary_and_order() {
        // The exact 27 camelCase values of document.v1.json / §4.5, in
        // discriminant order.
        const SCHEMA: [&str; 27] = [
            "normal",
            "dissolve",
            "darken",
            "multiply",
            "colorBurn",
            "linearBurn",
            "darkerColor",
            "lighten",
            "screen",
            "colorDodge",
            "linearDodge",
            "lighterColor",
            "overlay",
            "softLight",
            "hardLight",
            "vividLight",
            "linearLight",
            "pinLight",
            "hardMix",
            "difference",
            "exclusion",
            "subtract",
            "divide",
            "hue",
            "saturation",
            "color",
            "luminosity",
        ];
        for (i, (mode, expect)) in BlendMode::ALL.iter().zip(SCHEMA).enumerate() {
            assert_eq!(*mode as u32, i as u32, "repr(u32) 0..=26 in §4.5 order");
            assert_eq!(
                serde_json::to_string(mode).unwrap(),
                format!("\"{expect}\"")
            );
            let back: BlendMode = serde_json::from_str(&format!("\"{expect}\"")).unwrap();
            assert_eq!(back, *mode);
        }
    }

    #[test]
    fn lock_flags_bits_and_all_union() {
        assert_eq!(LockFlags::TRANSPARENCY.bits(), 1);
        assert_eq!(LockFlags::PIXELS.bits(), 2);
        assert_eq!(LockFlags::POSITION.bits(), 4);
        assert_eq!(LockFlags::ALL.bits(), 7);
        assert!(LockFlags::ALL.contains(LockFlags::PIXELS));
        assert!(!LockFlags::PIXELS.contains(LockFlags::ALL));
    }

    #[test]
    fn lock_flags_serde_round_trip() {
        let f = LockFlags::TRANSPARENCY.union(LockFlags::POSITION);
        assert_eq!(
            serde_json::to_string(&f).unwrap(),
            "[\"transparency\",\"position\"]"
        );
        let back: LockFlags = serde_json::from_str("[\"position\",\"transparency\"]").unwrap();
        assert_eq!(back, f);
        // "all" shorthand reads as the union.
        let all: LockFlags = serde_json::from_str("[\"all\"]").unwrap();
        assert_eq!(all, LockFlags::ALL);
        // Unknown strings are refused at this layer.
        assert!(serde_json::from_str::<LockFlags>("[\"vibes\"]").is_err());
        // Empty set serializes to the empty array.
        assert_eq!(serde_json::to_string(&LockFlags::empty()).unwrap(), "[]");
    }

    #[test]
    fn color_label_snake_case() {
        assert_eq!(
            serde_json::to_string(&ColorLabel::Violet).unwrap(),
            "\"violet\""
        );
    }

    #[test]
    fn node_defaults_match_the_2_6_table() {
        let n = LayerNode::new_group(LayerId::new(), "G", false);
        assert_eq!(n.blend(), BlendMode::Normal);
        assert_eq!(n.opacity(), 1.0);
        assert_eq!(n.fill_opacity(), 1.0);
        assert!(n.visible());
        assert!(n.locks().is_empty());
        assert!(!n.clip_to_below());
        assert!(n.masks().is_empty());
        assert!(n.children().is_empty());
        assert!(n.transform().is_identity());
        assert!(n.color_label().is_none());
        assert!(n.is_group());
        assert_eq!(n.kind().kind_str(), "group");
    }
}
