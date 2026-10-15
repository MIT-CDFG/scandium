// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! w19_psb_smoke — workflow gate W19 (PSB streaming open) at the plan's
//! CP-1.0 SMOKE scoping ("PSB streaming — smoke", plan §2.5 workflow
//! row; the 150 MP nightly and 600 MP release variants are tier work,
//! §41.8). Run by tests/workflows/w19_psb_streaming_smoke.sh.
//!
//! Spec: W19-psb-streaming-open.yaml (§41.5/§8.5). Fixtures are class
//! (c) — generated at test time by O-IO's proven byte-level emitter
//! (verbatim `#[path]` include, the w07/w18 precedent) with `psb: true`
//! (version 2, u64 length widenings, u32 RLE counts).
//!
//! Smoke legs, each an artifact step at reduced width:
//!  1. sniff + header: version 2 accepted; PSB dims cap (300 000) and
//!     DecodeOpts::max_* both enforced typed.
//!  2. import a layered RLE PSB (the widened-u32-RLE-count addressing
//!     leg): decode -> journaled landing -> composite.
//!  3. RLE row addressing: decoded pixels equal the emitter's source
//!     pattern at scattered spot points (16 spots).
//!  4. export PNG: byte-identical across two independent decode+land+
//!     export runs (determinism in place of a frozen golden).
//!
//! RSS/first-view/pan budgets and the 150/600 MP fixtures are the
//! recorded tier residue — the wrapper ledgers them.

#[path = "../../../../crates/appname-io/tests/psd_common/mod.rs"]
mod psd_common;

use appname_core::geom::IRect;
use appname_io::{CodecRegistry, DecodeOpts};
use psd_common::{LayerSpec, PsdBuilder};

const W: u32 = 640;
const H: u32 = 480;

fn srgb_bytes(w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            out.extend_from_slice(&[
                (x * 41 % 256) as u8,
                (y * 59 % 256) as u8,
                ((x * 17 + y * 31) % 256) as u8,
                255,
            ]);
        }
    }
    out
}

fn build_psb() -> Vec<u8> {
    let rgba = srgb_bytes(W as usize, H as usize);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    // Two layers, RLE-coded channels (the widened count leg), full-canvas.
    let mut l1 = LayerSpec::basic((0, 0, H as i32, W as i32), "deep", &rgba);
    for ch in &mut l1.channels {
        ch.2 = true; // RLE
    }
    // Quadrant-sized layer: its own pattern slice (same formula at the
    // same global coords — offset (0,0) — so the composite everywhere
    // equals the one pattern).
    let rgba_q = srgb_bytes((W / 2) as usize, (H / 2) as usize);
    let mut l2 = LayerSpec::basic((0, 0, (H / 2) as i32, (W / 2) as i32), "upper", &rgba_q);
    for ch in &mut l2.channels {
        ch.2 = true;
    }
    let mut b = PsdBuilder::rgb8(W, H);
    b.psb = true;
    b.layers = vec![l1, l2];
    b.merged_from_rgb8(&rgb, 3);
    b.merged_rle = true;
    b.build()
}

fn engine() -> appname_core::Engine {
    appname_assembly::production_assembly().build()
}

fn land_and_export(psb_bytes: &[u8]) -> (Vec<u8>, (u32, u32)) {
    let reg = CodecRegistry::builtin();
    let psd = reg.by_id("psd").expect("psd codec registered");
    let imported = psd
        .decode(psb_bytes, &DecodeOpts::default())
        .expect("PSB decodes");
    let size = imported.size;
    let mut eng = engine();
    eng.apply_json(&format!(
        "{{\"schema\":\"appname.command.v1\",\"id\":\"19000000-0000-4000-8000-000000000001\",\"kind\":\"doc.new\",\"params\":{{\"width\":{},\"height\":{},\"background\":\"transparent\"}}}}",
        size.0, size.1
    ))
    .expect("doc.new");
    appname_io::import_into_engine(&mut eng, "psb", imported).expect("journaled landing");
    let canvas = IRect::new(0, 0, size.0 as i32, size.1 as i32);
    let doc = eng.document().unwrap();
    let comp = appname_render::composite_document(doc, canvas).expect("composite");
    let png = appname_core::encode_composite_png(size.0, size.1, &comp.region_rgba_f32(canvas))
        .expect("png");
    (png, size)
}

fn main() {
    let mut failures = 0usize;
    let t0 = std::time::Instant::now();
    let psb_bytes = build_psb();
    println!(
        "fixture: {}x{} v2 PSB, 2 RLE layers, {} bytes (class (c), emitter-generated)",
        W,
        H,
        psb_bytes.len()
    );

    // --- leg 1: sniff + header ----------------------------------------
    let reg = CodecRegistry::builtin();
    let psd = reg.by_id("psd").expect("psd codec registered");
    // version-2 acceptance is proven by the decode below; the CAP legs:
    let tight = DecodeOpts {
        max_dimension: 100,
        ..DecodeOpts::default()
    };
    match psd.decode(&psb_bytes, &tight) {
        Err(e) => println!("bomb-gate PASS (max_dimension 100 refuses {W}px typed: {e:?})"),
        Ok(_) => {
            println!("bomb-gate FAIL (DecodeOpts::max_dimension not enforced)");
            failures += 1;
        }
    }
    // PSB dims cap (300 000): header-forge a huge width and expect the
    // typed refusal (no allocation — the process surviving IS the leg).
    let mut forged = psb_bytes.clone();
    // header: sig(4) ver(2) reserved(6) channels(2) height(4) width(4)
    forged[18..22].copy_from_slice(&400_000u32.to_be_bytes());
    match psd.decode(&forged, &DecodeOpts::default()) {
        Err(e) => println!("psb-cap PASS (width 400000 > 300000 refused typed: {e:?})"),
        Ok(_) => {
            println!("psb-cap FAIL (PSB dim cap not enforced)");
            failures += 1;
        }
    }

    // --- legs 2+3: import, land, RLE spot truth ------------------------
    let (png1, size) = land_and_export(&psb_bytes);
    println!(
        "import PASS (v2 accepted, {}x{} landed + composited)",
        size.0, size.1
    );

    // Decode the export back and spot-check 16 scattered points against
    // the emitter's source pattern (upper layer covers the top-left
    // quadrant; both layers carry the same pattern at the same offsets,
    // so the composite equals the pattern everywhere).
    let dec = png::Decoder::new(std::io::BufReader::new(std::io::Cursor::new(&png1)));
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let out = reader.next_frame(&mut buf).unwrap();
    let px = &buf[..out.buffer_size()];
    let mut spot_bad = 0usize;
    for i in 0..16u32 {
        let x = (i * 157 + 13) % W;
        let y = (i * 211 + 7) % H;
        let o = ((y * W + x) * 4) as usize;
        let want = [
            (x as usize * 41 % 256) as u8,
            (y as usize * 59 % 256) as u8,
            ((x as usize * 17 + y as usize * 31) % 256) as u8,
        ];
        if px[o..o + 3] != want {
            spot_bad += 1;
        }
    }
    if spot_bad == 0 {
        println!("rle-spots PASS (16 scattered points equal the source pattern — widened RLE row addressing holds)");
    } else {
        println!("rle-spots FAIL ({spot_bad}/16 points diverge)");
        failures += 1;
    }

    // --- leg 4: determinism across independent runs --------------------
    let (png2, _) = land_and_export(&psb_bytes);
    if png1 == png2 {
        println!("export-determinism PASS (PNG byte-identical across two independent decode+land+export runs)");
    } else {
        println!("export-determinism FAIL");
        failures += 1;
    }

    println!(
        "smoke wall: {:.2}s (budget legs — 150 MP <= 25 s nightly, 600 MP <= 90 s release, RSS <= ram_budget+2 GiB, first-view, pan — are tier residue, recorded by the wrapper)",
        t0.elapsed().as_secs_f64()
    );
    if failures > 0 {
        println!("w19_psb_smoke: FAIL ({failures} leg[s])");
        std::process::exit(1);
    }
    println!("w19_psb_smoke: PASS");
}
