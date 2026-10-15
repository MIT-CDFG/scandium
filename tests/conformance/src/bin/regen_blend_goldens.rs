// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors

//! regen_blend_goldens — mechanical blend-golden (re)generator (O-QA).
//!
//! Spec: ES §4.4/§4.5 (formulas; the §4.4 worked example is spec-mandated
//! in this suite), §34.3 (golden suite; the CPU path is the permanent
//! parity oracle); plan §0.6 rule 6; goldens/README.md rules 1-5.
//!
//! Writes goldens/blend/: per case a 64x64 stride-4 window of the
//! composited working tile (.bin, little-endian f32 RGBA row-major), a
//! .meta sidecar (closed-form input definitions, the §4.4 formula, the
//! hand-computed oracle probes with their derivation inputs — no
//! timestamps), and MANIFEST.sha256. Every window is produced TWICE and
//! byte-compared before writing (rule 5). The oracle values themselves
//! live in appname_conformance::blend_goldens::PROBES — derived from the
//! spec formula text independently of appname-render.
//!
//! Run:  cargo run -p appname-conformance --bin regen_blend_goldens

use appname_conformance::blend_goldens::{
    cases, dissolve_coverage_mask, doc_walk_window, produce_window, PROBES, STRIDE, WIN,
};
use appname_conformance::paths;
use std::path::Path;
use std::process::Command;

fn sha256(path: &Path) -> String {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .expect("shasum available");
    assert!(out.status.success(), "shasum failed for {}", path.display());
    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn main() {
    let dir = paths::repo_root().join("goldens").join("blend");
    std::fs::create_dir_all(&dir).unwrap();
    let mut manifest = Vec::new();

    for case in cases() {
        let a = produce_window(&case);
        let b = produce_window(&case);
        assert_eq!(a, b, "{}: window nondeterminism", case.name);

        let bin = dir.join(format!("{}.bin", case.name));
        std::fs::write(&bin, &a).unwrap();
        let sha = sha256(&bin);
        manifest.push(format!("{sha}  {}.bin", case.name));

        let mut probe_lines = String::new();
        for (name, wx, wy, co, ao) in PROBES {
            if *name == case.name {
                probe_lines.push_str(&format!(
                    "oracle-probe: window ({wx},{wy}) = tile ({},{}) -> \
                     stored (Co, ao) = ({co}, {ao}) per channel R=G=B\n",
                    wx * STRIDE,
                    wy * STRIDE,
                ));
            }
        }
        let layers: Vec<String> = case
            .layers
            .iter()
            .map(|(m, o, t)| format!("{m:?} opacity {o} tile {t:?}"))
            .collect();
        let meta = format!(
            "golden: {name}\n\
             producer: appname-conformance regen_blend_goldens via \
             appname-render PUBLIC API (composite_tile); window = 64x64 \
             samples of the 256x256 working tile at stride 4, \
             little-endian f32 RGBA row-major\n\
             backdrop: {backdrop:?}\n\
             layers (bottom->top): {layers}\n\
             tile definitions (closed form, per pixel (x,y)): SrcRamp \
             Cs=x/255 sa=y/255; BackdropRamp Cb=y/255 ab=x/255; HdrSrc \
             Cs=4x/255 sa=y/255; HdrBackdrop Cb=2y/255 ab=x/255; Zero \
             all-0; SpecSrc stored (0.30,0.50); SpecBackdrop stored \
             (0.40,0.80) — stored values premultiplied\n\
             formula (ES 4.4): as=sa*opacity; Cs=premult/sa (sa>0 else \
             0); Cb=premult/ab (ab>0 else 0); Co = as*(1-ab)*Cs + \
             as*ab*B(Cb,Cs) + (1-as)*ab*Cb; ao = as + ab*(1-as)\n\
             oracle derivation: O-QA scalar evaluation of the formula \
             text (ES 4.4/4.5), independent of appname-render's code; \
             script recorded in reports/O-QA-log.md\n\
             {probe_lines}\
             sha256: {sha}\n\
             rule: goldens change only via O-QA review (plan section 0.6 \
             rule 6); regenerate with `cargo run -p appname-conformance \
             --bin regen_blend_goldens`\n",
            name = case.name,
            backdrop = case.backdrop,
            layers = layers.join("; "),
        );
        std::fs::write(dir.join(format!("{}.meta", case.name)), meta).unwrap();
        println!("golden blend/{}: {sha}", case.name);
    }

    // document-walk case (end-to-end public path)
    let a = doc_walk_window();
    let b = doc_walk_window();
    assert_eq!(a, b, "doc-walk window nondeterminism");
    let bin = dir.join("doc-walk-import.bin");
    std::fs::write(&bin, &a).unwrap();
    let sha = sha256(&bin);
    manifest.push(format!("{sha}  doc-walk-import.bin"));
    std::fs::write(
        dir.join("doc-walk-import.meta"),
        format!(
            "golden: doc-walk-import\n\
             producer: appname-conformance regen_blend_goldens via \
             composite_document over a real document: doc.new white 96x64 \
             (fixed envelope id b1e4d000-0000-4000-8000-000000000001) -> \
             export_png_bytes -> decode_png_to_buffer -> import_layer -> \
             composite_document ROI (0,0,{WIN},{WIN}); window bytes as \
             the tile cases\n\
             oracle: white over white composites to opaque white \
             (1,1,1,1) at every pixel — asserted in production and in the \
             verify test\n\
             note: both layers Normal; per-layer blend-mode variety in \
             document walks arrives with layer.set_props at its wave\n\
             sha256: {sha}\n\
             rule: goldens change only via O-QA review (plan section 0.6 \
             rule 6)\n"
        ),
    )
    .unwrap();
    println!("golden blend/doc-walk-import: {sha}");

    // Dissolve coverage mask (CP-0.5 render-companion standing item,
    // closed at CP-0.7 wave 2): the deterministic screen-door pattern
    // itself, stored as golden bytes beyond the membership property.
    let a = dissolve_coverage_mask();
    let b = dissolve_coverage_mask();
    assert_eq!(a, b, "dissolve coverage-mask nondeterminism");
    let selected: usize = a.iter().map(|&m| usize::from(m)).sum();
    let bin = dir.join("dissolve-coverage.mask.bin");
    std::fs::write(&bin, &a).unwrap();
    let sha = sha256(&bin);
    manifest.push(format!("{sha}  dissolve-coverage.mask.bin"));
    std::fs::write(
        dir.join("dissolve-coverage.mask.meta"),
        format!(
            "golden: dissolve-coverage.mask\n\
             producer: appname-conformance regen_blend_goldens via \
             appname_conformance::blend_goldens::dissolve_coverage_mask \
             (composite_tile Dissolve over SrcRamp/BackdropRamp — the \
             mode2-dissolve inputs — at FULL 256x256 tile resolution, one \
             byte per pixel row-major)\n\
             semantics: 1 = pixel took the HARDENED SOURCE \
             (un-premultiplied Cs at alpha 1, Appendix A); 0 = backdrop \
             (incl. the sa==0 hardening rule and exact source==backdrop \
             coincidences, deterministically classified 0)\n\
             selected: {selected} of 65536 pixels\n\
             sha256: {sha}\n\
             rule: goldens change only via O-QA review (plan section 0.6 \
             rule 6)\n"
        ),
    )
    .unwrap();
    println!("golden blend/dissolve-coverage.mask: {sha} ({selected}/65536 source-selected)");

    manifest.sort();
    std::fs::write(dir.join("MANIFEST.sha256"), manifest.join("\n") + "\n").unwrap();
    println!("blend goldens written under {}", dir.display());
}
