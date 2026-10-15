// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! Conformance stub — a fake LibRaw served through the SAME [`Api`]
//! vtable the real module fills, so `appname-io`'s wrapper tests can
//! exercise every boundary arm (open rejection, unpack failure, bomb
//! gates, CFA classification, degenerate levels — the bound advisory)
//! without any dylib on the machine and without touching the real
//! LibRaw (the record's clean-room posture).
//!
//! Design: NO globals. The stub's behavior is driven entirely by a
//! self-describing byte format ([`stub_bytes`]) carried IN the "file"
//! the test hands to `open_buffer` — the handle state is parsed from
//! the buffer, so parallel tests never interfere. Bytes without the
//! stub magic are rejected at open (code [`STUB_UNIDENTIFIED`]),
//! modeling LibRaw's cannot-identify arm.
//!
//! Test-support only: behind the `conformance-stub` feature, enabled by
//! dev-dependencies exclusively (never a shipped configuration).

use std::ffi::{c_char, c_int, c_uint, c_void};

use super::{Api, Handle, RawLib};

/// Stub magic, searched in the first 48 bytes so tests may prefix
/// real vendor magic (e.g. a CR2 header) for registry-level routing.
pub const MAGIC: &[u8; 16] = b"APPNAME-STUBRAW1";

/// The stub's cannot-identify open code (mirrors the DOCUMENTED name
/// `LIBRAW_FILE_UNSUPPORTED`, whose numeric value is undocumented — the
/// wrapper must branch on call site, never on numeric codes).
pub const STUB_UNIDENTIFIED: i32 = -2;

/// What the fake module should serve. Field order mirrors the wire
/// layout in [`stub_bytes`].
#[derive(Debug, Clone, PartialEq)]
pub struct StubSpec {
    /// `libraw_open_buffer` result (0 = identified).
    pub open_result: i32,
    /// `libraw_unpack` result (0 = decoded).
    pub unpack_result: i32,
    /// Active-area size (`iwidth`/`iheight`).
    pub width: u32,
    pub height: u32,
    /// Full sensor size.
    pub raw_width: u32,
    pub raw_height: u32,
    /// As-shot multipliers `(r, g1, b, g2)`.
    pub cam_mul: [f32; 4],
    /// Saturation/white level.
    pub color_maximum: i32,
    /// Camera→sRGB 3×3.
    pub rgb_cam: [[f32; 3]; 3],
    /// CFA served through `libraw_COLOR`.
    pub cfa: StubCfa,
}

/// CFA pattern classes the stub can serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubCfa {
    /// 2×2 Bayer tile of LibRaw color codes for positions
    /// `(0,0), (0,1), (1,0), (1,1)` (0 = R, 1 = G, 2 = B, 3 = G2).
    Bayer([u8; 4]),
    /// A fixed 6×6 X-Trans-class pattern (non-Bayer classification arm).
    XTrans,
}

/// The fixed 6×6 X-Trans-style pattern (0 = R, 1 = G, 2 = B).
const XTRANS_6X6: [[u8; 6]; 6] = [
    [1, 2, 1, 1, 0, 1],
    [0, 1, 0, 2, 1, 2],
    [1, 2, 1, 1, 0, 1],
    [1, 0, 1, 1, 2, 1],
    [2, 1, 2, 0, 1, 0],
    [1, 0, 1, 1, 2, 1],
];

impl StubSpec {
    /// A plausible healthy RGGB shot: 14-bit-ish white level, mild warm
    /// as-shot multipliers, identity-adjacent camera matrix.
    pub fn bayer_rggb(width: u32, height: u32) -> StubSpec {
        StubSpec {
            open_result: 0,
            unpack_result: 0,
            width,
            height,
            raw_width: width + 8,
            raw_height: height + 8,
            cam_mul: [2.0, 1.0, 1.5, 1.0],
            color_maximum: 15871,
            rgb_cam: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            cfa: StubCfa::Bayer([0, 1, 3, 2]),
        }
    }
}

/// Serialize a [`StubSpec`] into the stub wire format the fake module
/// parses at `open_buffer`.
pub fn stub_bytes(spec: &StubSpec) -> Vec<u8> {
    let mut v = Vec::with_capacity(MAGIC.len() + 96);
    v.extend_from_slice(MAGIC);
    v.extend_from_slice(&spec.open_result.to_le_bytes());
    v.extend_from_slice(&spec.unpack_result.to_le_bytes());
    v.extend_from_slice(&spec.width.to_le_bytes());
    v.extend_from_slice(&spec.height.to_le_bytes());
    v.extend_from_slice(&spec.raw_width.to_le_bytes());
    v.extend_from_slice(&spec.raw_height.to_le_bytes());
    for m in spec.cam_mul {
        v.extend_from_slice(&m.to_le_bytes());
    }
    v.extend_from_slice(&spec.color_maximum.to_le_bytes());
    for row in spec.rgb_cam {
        for c in row {
            v.extend_from_slice(&c.to_le_bytes());
        }
    }
    match spec.cfa {
        StubCfa::Bayer(tile) => {
            v.push(0);
            v.extend_from_slice(&tile);
        }
        StubCfa::XTrans => {
            v.push(1);
            v.extend_from_slice(&[0; 4]);
        }
    }
    v
}

/// Prefix `bytes` with `prefix` (e.g. CR2 magic) so registry sniffing
/// routes to the RAW codec while the stub still finds its own magic.
pub fn with_prefix(prefix: &[u8], bytes: &[u8]) -> Vec<u8> {
    assert!(
        prefix.len() + MAGIC.len() <= 48,
        "stub magic must stay inside the scanned window"
    );
    let mut v = prefix.to_vec();
    v.extend_from_slice(bytes);
    v
}

/// Per-handle stub state (heap box behind the opaque handle).
struct State {
    spec: Option<ParsedSpec>,
}

struct ParsedSpec {
    unpack_result: i32,
    width: u32,
    height: u32,
    raw_width: u32,
    raw_height: u32,
    cam_mul: [f32; 4],
    color_maximum: i32,
    rgb_cam: [[f32; 3]; 3],
    cfa_mode: u8,
    tile: [u8; 4],
}

fn parse(buffer: &[u8]) -> Option<(i32, ParsedSpec)> {
    let window = &buffer[..buffer.len().min(48)];
    let start = window
        .windows(MAGIC.len())
        .position(|w| w == MAGIC.as_slice())?;
    let mut off = start + MAGIC.len();
    let mut i32_at = |data: &[u8]| -> Option<i32> {
        let v = i32::from_le_bytes(data.get(off..off + 4)?.try_into().ok()?);
        off += 4;
        Some(v)
    };
    // Manual cursor over the fixed layout (see stub_bytes).
    let open_result = i32_at(buffer)?;
    let unpack_result = i32_at(buffer)?;
    let width = i32_at(buffer)? as u32;
    let height = i32_at(buffer)? as u32;
    let raw_width = i32_at(buffer)? as u32;
    let raw_height = i32_at(buffer)? as u32;
    let mut cam_mul = [0.0f32; 4];
    for m in &mut cam_mul {
        *m = f32::from_bits(i32_at(buffer)? as u32);
    }
    let color_maximum = i32_at(buffer)?;
    let mut rgb_cam = [[0.0f32; 3]; 3];
    for row in &mut rgb_cam {
        for c in row.iter_mut() {
            *c = f32::from_bits(i32_at(buffer)? as u32);
        }
    }
    let cfa_mode = *buffer.get(off)?;
    off += 1;
    let tile: [u8; 4] = buffer.get(off..off + 4)?.try_into().ok()?;
    Some((
        open_result,
        ParsedSpec {
            unpack_result,
            width,
            height,
            raw_width,
            raw_height,
            cam_mul,
            color_maximum,
            rgb_cam,
            cfa_mode,
            tile,
        },
    ))
}

// SAFETY note for every stub extern fn: `h` is either null or a pointer
// produced by `stub_init` (Box<State>); the vtable contract mirrors the
// real module's (init → open_buffer → accessors → close, one thread per
// handle), so the exclusive/shared borrows below are sound.

unsafe extern "C" fn stub_init(_flags: c_uint) -> Handle {
    Box::into_raw(Box::new(State { spec: None })).cast()
}

unsafe extern "C" fn stub_close(h: Handle) {
    if !h.is_null() {
        // SAFETY: reclaiming the Box produced by stub_init, exactly once.
        drop(unsafe { Box::from_raw(h.cast::<State>()) });
    }
}

unsafe extern "C" fn stub_open_buffer(h: Handle, buf: *const c_void, len: usize) -> c_int {
    if h.is_null() || buf.is_null() {
        return STUB_UNIDENTIFIED;
    }
    // SAFETY: caller (RawLib::open_buffer) passes a live slice.
    let bytes = unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), len) };
    match parse(bytes) {
        None => STUB_UNIDENTIFIED,
        Some((open_result, spec)) => {
            if open_result != 0 {
                return open_result;
            }
            // SAFETY: exclusive access per the vtable contract.
            unsafe { (*h.cast::<State>()).spec = Some(spec) };
            0
        }
    }
}

fn with_spec<T>(h: Handle, default: T, f: impl FnOnce(&ParsedSpec) -> T) -> T {
    if h.is_null() {
        return default;
    }
    // SAFETY: shared read of the State behind a stub handle.
    let state = unsafe { &*h.cast::<State>() };
    match &state.spec {
        Some(s) => f(s),
        None => default,
    }
}

unsafe extern "C" fn stub_unpack(h: Handle) -> c_int {
    with_spec(h, STUB_UNIDENTIFIED, |s| s.unpack_result)
}

unsafe extern "C" fn stub_strerror(_code: c_int) -> *const c_char {
    static MSG: &[u8] = b"stub: simulated libraw error\0";
    MSG.as_ptr().cast()
}

unsafe extern "C" fn stub_version() -> *const c_char {
    static V: &[u8] = b"0.22.1\0";
    V.as_ptr().cast()
}

unsafe extern "C" fn stub_version_number() -> c_int {
    super::make_version(0, 22, 1)
}

unsafe extern "C" fn stub_get_iheight(h: Handle) -> c_int {
    with_spec(h, 0, |s| s.height as c_int)
}

unsafe extern "C" fn stub_get_iwidth(h: Handle) -> c_int {
    with_spec(h, 0, |s| s.width as c_int)
}

unsafe extern "C" fn stub_get_raw_height(h: Handle) -> c_int {
    with_spec(h, 0, |s| s.raw_height as c_int)
}

unsafe extern "C" fn stub_get_raw_width(h: Handle) -> c_int {
    with_spec(h, 0, |s| s.raw_width as c_int)
}

unsafe extern "C" fn stub_get_cam_mul(h: Handle, index: c_int) -> f32 {
    with_spec(h, 0.0, |s| {
        s.cam_mul.get(index as usize).copied().unwrap_or(0.0)
    })
}

unsafe extern "C" fn stub_get_rgb_cam(h: Handle, i: c_int, j: c_int) -> f32 {
    with_spec(h, 0.0, |s| {
        s.rgb_cam
            .get(i as usize)
            .and_then(|row| row.get(j as usize))
            .copied()
            .unwrap_or(0.0)
    })
}

unsafe extern "C" fn stub_get_color_maximum(h: Handle) -> c_int {
    with_spec(h, 0, |s| s.color_maximum)
}

unsafe extern "C" fn stub_color(h: Handle, row: c_int, col: c_int) -> c_int {
    with_spec(h, 0, |s| {
        let (r, c) = (row.rem_euclid(6) as usize, col.rem_euclid(6) as usize);
        match s.cfa_mode {
            0 => {
                let (r2, c2) = (row.rem_euclid(2) as usize, col.rem_euclid(2) as usize);
                c_int::from(s.tile[r2 * 2 + c2])
            }
            _ => c_int::from(XTRANS_6X6[r][c]),
        }
    })
}

/// A [`RawLib`] served entirely by the in-process stub (no dylib).
pub fn stub_lib() -> RawLib {
    RawLib {
        api: Api {
            init: stub_init,
            close: stub_close,
            open_buffer: stub_open_buffer,
            unpack: stub_unpack,
            strerror: stub_strerror,
            version: stub_version,
            version_number: stub_version_number,
            get_iheight: stub_get_iheight,
            get_iwidth: stub_get_iwidth,
            get_raw_height: stub_get_raw_height,
            get_raw_width: stub_get_raw_width,
            get_cam_mul: stub_get_cam_mul,
            get_rgb_cam: stub_get_rgb_cam,
            get_color_maximum: stub_get_color_maximum,
            color: stub_color,
        },
        lib: None,
    }
}
