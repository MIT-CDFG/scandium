// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! The in-crate byte-level PSD/PSB emitter — the fixture source for the
//! `psd-corpus.yaml` executable subset (the yaml's own "byte-level-emitter"
//! generator class; the checked-in matrix files do not exist in the spec
//! repo). Emits §8.5-shaped bytes: header, color-mode data, resources,
//! layer-and-mask info (records + channel data, optional Lr16 carriage,
//! optional PSB widenings), merged image data (raw or RLE).

#![allow(dead_code)]

/// Mask block spec: (rect (top, left, bottom, right), default_color, flags).
pub type MaskSpec = ((i32, i32, i32, i32), u8, u8);

pub struct LayerSpec {
    /// top,left,bottom,right
    pub rect: (i32, i32, i32, i32),
    pub name: &'static str,
    pub blend: [u8; 4],
    pub opacity: u8,
    pub clipping: u8,
    pub flags: u8,
    /// (channel id, RAW decoded bytes at file depth, rle?)
    pub channels: Vec<(i16, Vec<u8>, bool)>,
    /// additional-info keys appended verbatim (key, payload)
    pub keys: Vec<([u8; 4], Vec<u8>)>,
    /// mask block: (rect, default_color, flags) — 20-byte form
    pub mask: Option<MaskSpec>,
    /// Force every channel block's compression code with the RAW payload
    /// kept as-is (adversarial cases: e.g. code 2 over a non-zlib
    /// payload, which the decoder must refuse as corrupt).
    pub code_override: Option<u16>,
    /// Emit every channel legitimately ZIP-compressed: `Some(2)` = one
    /// zlib stream of the raw rows, `Some(3)` = per-row delta prediction
    /// (§8.5 depth specifics) then zlib.
    pub zip: Option<u16>,
}

impl LayerSpec {
    pub fn basic(rect: (i32, i32, i32, i32), name: &'static str, rgba: &[u8]) -> LayerSpec {
        let (t, l, b, r) = rect;
        let n = ((r - l) * (b - t)) as usize;
        assert_eq!(rgba.len(), n * 4);
        let plane = |c: usize| -> Vec<u8> { rgba.chunks_exact(4).map(|p| p[c]).collect() };
        LayerSpec {
            rect,
            name,
            blend: *b"norm",
            opacity: 255,
            clipping: 0,
            flags: 0,
            channels: vec![
                (-1, plane(3), false),
                (0, plane(0), false),
                (1, plane(1), false),
                (2, plane(2), false),
            ],
            keys: Vec::new(),
            mask: None,
            code_override: None,
            zip: None,
        }
    }
}

pub struct PsdBuilder {
    pub psb: bool,
    pub mode: u16,
    pub depth: u16,
    pub channels: u16,
    pub w: u32,
    pub h: u32,
    pub color_mode_data: Vec<u8>,
    /// (resource id, payload)
    pub resources: Vec<(u16, Vec<u8>)>,
    pub layers: Vec<LayerSpec>,
    /// negative layer count (merged alpha) flag
    pub merged_has_alpha: bool,
    /// planar RAW merged channels at file depth
    pub merged: Vec<Vec<u8>>,
    pub merged_rle: bool,
    /// Merged-data section legitimately ZIP-compressed (`Some(2)`/`Some(3)`,
    /// same §8.5 shapes as [`LayerSpec::zip`]); one zlib stream of all
    /// planes in mode order. Wins over `merged_rle`.
    pub merged_zip: Option<u16>,
    /// wrap the layer info under a global Lr16 key (16-bit convention)
    pub via_lr16: bool,
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// Simple valid PackBits encoding (literal chunks of ≤128).
pub fn packbits_encode(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in row.chunks(128) {
        out.push((chunk.len() - 1) as u8);
        out.extend_from_slice(chunk);
    }
    out
}

/// One zlib stream over `raw` (§8.5 codes 2/3 payload shape).
pub fn zlib_compress(raw: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(raw).unwrap();
    enc.finish().unwrap()
}

/// FORWARD §8.5 code-3 prediction (the exact inverse of the decoder's
/// per-row un-prediction): 8-bit byte delta; 16-bit big-endian `u16`
/// delta; 32-bit shuffle-then-delta — interleave byte planes per row
/// (`row'[k·width + j]` = sample `j` byte `k`), then byte delta.
pub fn predict_rows(raw: &[u8], width: usize, depth: u16) -> Vec<u8> {
    let bps = usize::from(depth.max(8)) / 8;
    let row_bytes = bps * width;
    let mut out = Vec::with_capacity(raw.len());
    for row in raw.chunks_exact(row_bytes) {
        let mut row = row.to_vec();
        if depth == 32 {
            let mut shuffled = vec![0u8; row_bytes];
            for j in 0..width {
                for k in 0..4 {
                    shuffled[k * width + j] = row[4 * j + k];
                }
            }
            row = shuffled;
        }
        match depth {
            16 => {
                let mut prev = 0u16;
                for i in 0..width {
                    let v = u16::from_be_bytes([row[2 * i], row[2 * i + 1]]);
                    let d = v.wrapping_sub(prev);
                    row[2 * i..2 * i + 2].copy_from_slice(&d.to_be_bytes());
                    prev = v;
                }
            }
            _ => {
                // 8-bit (and the 32-bit post-shuffle byte stream): byte
                // delta, back to front so each subtraction sees the
                // ORIGINAL predecessor.
                for i in (1..row.len()).rev() {
                    row[i] = row[i].wrapping_sub(row[i - 1]);
                }
            }
        }
        out.extend_from_slice(&row);
    }
    out
}

impl PsdBuilder {
    pub fn rgb8(w: u32, h: u32) -> PsdBuilder {
        PsdBuilder {
            psb: false,
            mode: 3,
            depth: 8,
            channels: 3,
            w,
            h,
            color_mode_data: Vec::new(),
            resources: Vec::new(),
            layers: Vec::new(),
            merged_has_alpha: false,
            merged: Vec::new(),
            merged_rle: false,
            merged_zip: None,
            via_lr16: false,
        }
    }

    /// Planar merged channels from interleaved RGB(A) 8-bit data.
    pub fn merged_from_rgb8(&mut self, rgb: &[u8], comps: usize) {
        self.merged = (0..comps)
            .map(|c| rgb.chunks_exact(comps).map(|p| p[c]).collect())
            .collect();
    }

    fn emit_channel_data(
        &self,
        raw: &[u8],
        rle: bool,
        rows: usize,
        row_bytes: usize,
        code_override: Option<u16>,
        zip: Option<u16>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(code) = code_override {
            out.extend_from_slice(&be16(code));
            out.extend_from_slice(raw);
        } else if let Some(code) = zip {
            assert!(code == 2 || code == 3, "zip codes are 2/3");
            let bps = usize::from(self.depth.max(8)) / 8;
            out.extend_from_slice(&be16(code));
            let payload = if code == 3 {
                zlib_compress(&predict_rows(raw, row_bytes / bps, self.depth))
            } else {
                zlib_compress(raw)
            };
            out.extend_from_slice(&payload);
        } else if rle {
            out.extend_from_slice(&be16(1));
            let packed: Vec<Vec<u8>> = raw.chunks(row_bytes).map(packbits_encode).collect();
            for p in &packed {
                if self.psb {
                    out.extend_from_slice(&be32(p.len() as u32));
                } else {
                    out.extend_from_slice(&be16(p.len() as u16));
                }
            }
            for p in &packed {
                out.extend_from_slice(p);
            }
            assert_eq!(packed.len(), rows);
        } else {
            out.extend_from_slice(&be16(0));
            out.extend_from_slice(raw);
        }
        out
    }

    fn emit_record(&self, l: &LayerSpec, data_blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut r = Vec::new();
        for v in [l.rect.0, l.rect.1, l.rect.2, l.rect.3] {
            r.extend_from_slice(&v.to_be_bytes());
        }
        r.extend_from_slice(&be16(l.channels.len() as u16));
        for ((id, _, _), block) in l.channels.iter().zip(data_blocks) {
            r.extend_from_slice(&id.to_be_bytes());
            if self.psb {
                r.extend_from_slice(&(block.len() as u64).to_be_bytes());
            } else {
                r.extend_from_slice(&be32(block.len() as u32));
            }
        }
        r.extend_from_slice(b"8BIM");
        r.extend_from_slice(&l.blend);
        r.push(l.opacity);
        r.push(l.clipping);
        r.push(l.flags);
        r.push(0); // filler
        let mut extra = Vec::new();
        // mask data
        match &l.mask {
            None => extra.extend_from_slice(&be32(0)),
            Some((rect, default_color, flags)) => {
                extra.extend_from_slice(&be32(20));
                for v in [rect.0, rect.1, rect.2, rect.3] {
                    extra.extend_from_slice(&v.to_be_bytes());
                }
                extra.push(*default_color);
                extra.push(*flags);
                extra.extend_from_slice(&[0, 0]); // pad to 20
            }
        }
        extra.extend_from_slice(&be32(0)); // blending ranges: empty
                                           // Pascal name, padded to multiple of 4.
        let name = l.name.as_bytes();
        extra.push(name.len() as u8);
        extra.extend_from_slice(name);
        let pad = (4 - ((name.len() + 1) % 4)) % 4;
        extra.extend(std::iter::repeat_n(0u8, pad));
        for (key, payload) in &l.keys {
            extra.extend_from_slice(b"8BIM");
            extra.extend_from_slice(key);
            extra.extend_from_slice(&be32(payload.len() as u32));
            extra.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                extra.push(0);
            }
        }
        r.extend_from_slice(&be32(extra.len() as u32));
        r.extend_from_slice(&extra);
        r
    }

    pub fn build(&self) -> Vec<u8> {
        let mut out = Vec::new();
        // Header.
        out.extend_from_slice(b"8BPS");
        out.extend_from_slice(&be16(if self.psb { 2 } else { 1 }));
        out.extend_from_slice(&[0u8; 6]);
        out.extend_from_slice(&be16(self.channels));
        out.extend_from_slice(&be32(self.h));
        out.extend_from_slice(&be32(self.w));
        out.extend_from_slice(&be16(self.depth));
        out.extend_from_slice(&be16(self.mode));
        // Color-mode data.
        out.extend_from_slice(&be32(self.color_mode_data.len() as u32));
        out.extend_from_slice(&self.color_mode_data);
        // Resources.
        let mut res = Vec::new();
        for (id, payload) in &self.resources {
            res.extend_from_slice(b"8BIM");
            res.extend_from_slice(&be16(*id));
            res.extend_from_slice(&[0, 0]); // empty pascal name + pad
            res.extend_from_slice(&be32(payload.len() as u32));
            res.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                res.push(0);
            }
        }
        out.extend_from_slice(&be32(res.len() as u32));
        out.extend_from_slice(&res);
        // Layer-and-mask section.
        let bps = usize::from(self.depth.max(8)) / 8;
        let mut info = Vec::new();
        if !self.layers.is_empty() {
            let count = self.layers.len() as i16;
            let count = if self.merged_has_alpha { -count } else { count };
            info.extend_from_slice(&count.to_be_bytes());
            let mut all_blocks: Vec<Vec<Vec<u8>>> = Vec::new();
            for l in &self.layers {
                let (t, _, b, r) = (l.rect.0, l.rect.1, l.rect.2, l.rect.3);
                let lw = (l.rect.3 - l.rect.1).max(0) as usize;
                let lh = (b - t).max(0) as usize;
                let _ = r;
                let blocks: Vec<Vec<u8>> = l
                    .channels
                    .iter()
                    .map(|(id, raw, rle)| {
                        // Mask channels use the mask rect dims.
                        let (dw, dh) = if *id == -2 || *id == -3 {
                            l.mask
                                .as_ref()
                                .map(|(mr, _, _)| {
                                    ((mr.3 - mr.1).max(0) as usize, (mr.2 - mr.0).max(0) as usize)
                                })
                                .unwrap_or((0, 0))
                        } else {
                            (lw, lh)
                        };
                        self.emit_channel_data(raw, *rle, dh, bps * dw, l.code_override, l.zip)
                    })
                    .collect();
                all_blocks.push(blocks);
            }
            for (l, blocks) in self.layers.iter().zip(&all_blocks) {
                info.extend_from_slice(&self.emit_record(l, blocks));
            }
            for blocks in &all_blocks {
                for b in blocks {
                    info.extend_from_slice(b);
                }
            }
        }
        let mut section = Vec::new();
        if self.via_lr16 && !info.is_empty() {
            // Empty inline info + the real records under global Lr16.
            if self.psb {
                section.extend_from_slice(&0u64.to_be_bytes());
            } else {
                section.extend_from_slice(&be32(0));
            }
            section.extend_from_slice(&be32(0)); // global mask info
            section.extend_from_slice(b"8BIM");
            section.extend_from_slice(b"Lr16");
            if self.psb {
                section.extend_from_slice(&(info.len() as u64).to_be_bytes());
            } else {
                section.extend_from_slice(&be32(info.len() as u32));
            }
            section.extend_from_slice(&info);
            if info.len() % 2 == 1 {
                section.push(0);
            }
        } else {
            if self.psb {
                section.extend_from_slice(&(info.len() as u64).to_be_bytes());
            } else {
                section.extend_from_slice(&be32(info.len() as u32));
            }
            section.extend_from_slice(&info);
            section.extend_from_slice(&be32(0)); // global mask info
        }
        if self.psb {
            out.extend_from_slice(&(section.len() as u64).to_be_bytes());
        } else {
            out.extend_from_slice(&be32(section.len() as u32));
        }
        out.extend_from_slice(&section);
        // Merged image data.
        let row_bytes = match self.depth {
            1 => (self.w as usize).div_ceil(8),
            8 => self.w as usize,
            16 => 2 * self.w as usize,
            32 => 4 * self.w as usize,
            _ => unreachable!(),
        };
        if let Some(code) = self.merged_zip {
            assert!(code == 2 || code == 3, "zip codes are 2/3");
            out.extend_from_slice(&be16(code));
            let mut all = Vec::new();
            for plane in &self.merged {
                if code == 3 {
                    all.extend_from_slice(&predict_rows(plane, self.w as usize, self.depth));
                } else {
                    all.extend_from_slice(plane);
                }
            }
            out.extend_from_slice(&zlib_compress(&all));
        } else if self.merged_rle {
            out.extend_from_slice(&be16(1));
            let mut packed_all = Vec::new();
            for plane in &self.merged {
                for row in plane.chunks(row_bytes) {
                    packed_all.push(packbits_encode(row));
                }
            }
            for p in &packed_all {
                if self.psb {
                    out.extend_from_slice(&be32(p.len() as u32));
                } else {
                    out.extend_from_slice(&be16(p.len() as u16));
                }
            }
            for p in &packed_all {
                out.extend_from_slice(p);
            }
        } else {
            out.extend_from_slice(&be16(0));
            for plane in &self.merged {
                out.extend_from_slice(plane);
            }
        }
        out
    }
}
