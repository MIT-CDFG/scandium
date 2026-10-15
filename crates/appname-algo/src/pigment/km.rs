// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Ported from spectral.js (https://github.com/rvanwijnen/spectral.js,
// commit bb2b05c9d1e65ae824d47e3b1cc17ea32c8ee68f, vendored at
// appname-proto/vendor/spectraljs/spectral.js): the Kubelka-Munk mixing
// math (KS/KM transforms, concentration-weighted mix), the seven base
// reflectance spectra, the D65-weighted CMF table, and the sRGB matrices.
// MIT License, Copyright (c) Ronald van Wijnen. License text carried in
// THIRD-PARTY-NOTICES.md and vendor/spectraljs/LICENSE.

//! Spec: ALGORITHMS.md §23 `paint_engine` — `pigment: {primary:
//! kubelka_munk_spectral, basis: [jakob19, spectraljs]}`; ES §22.3 pigment
//! mode; INTEGRATION-NOTES §6 order 3b (`spectraljs` cross-check).
//!
//! Kubelka–Munk two-flux pigment mixing, ported as the CROSS-CHECK
//! reference for the engine's own rgb2spec-based spectral path (the census
//! row's role: "K/S mixing math shape" + constants cross-check vs the
//! pigment-mixing dossier). The paint engine's shipping spectra come from
//! the rgb2spec LUT bake (order 3a); this module provides an independently
//! derived mixing result to test against, plus the K-M mix operator shape
//! itself (KS -> weighted average by concentration -> KM back-transform).
//!
//! Rights note: the contract's `paint_engine` area carries a banned
//! CC-BY-NC-4.0 pigment-mixing library (see the `banned:` row in
//! contracts/algorithms/algorithms.yaml): nothing here derives from it —
//! spectral.js is the sanctioned MIT reference the contract basis names.

// Ported-constant fidelity: the printed precision is attribution-faithful to
// the upstream source tables and the parsed bits are pinned by determinism
// goldens; truncating the literals would falsify both.
#![allow(clippy::excessive_precision)]

/// Number of reflectance samples (380..730 nm at 10 nm) per spectral.js.
pub const SIZE: usize = 38;

/// Base reflectance: white.
pub const BASE_W: [f64; SIZE] = [
    1.0011607271876399,
    1.00116065159728,
    1.0011603192274701,
    1.0011586727078901,
    1.0011525984455201,
    1.0011325252899801,
    1.0010850066332699,
    1.00099687889453,
    1.0008652515227401,
    1.0006962900094001,
    1.0005049611488801,
    1.0003080818799199,
    1.0001196660201299,
    0.99995276596840699,
    0.99982183689929705,
    0.99973860955759297,
    0.99970955163961195,
    0.99973193021062701,
    0.99979943634619495,
    0.99990033031667103,
    1.00002040652611,
    1.0001447879365799,
    1.00025997903412,
    1.0003557969708901,
    1.0004275378026899,
    1.0004762334488799,
    1.00050720967508,
    1.0005251915637301,
    1.0005350960689601,
    1.00054022097482,
    1.0005427281678401,
    1.0005438956908701,
    1.0005444821215099,
    1.0005447695999199,
    1.00054489887762,
    1.0005449625468901,
    1.00054498927058,
    1.000544996993,
];

/// Base reflectance: cyan.
pub const BASE_C: [f64; SIZE] = [
    0.97058500132296199,
    0.97059249814342496,
    0.97062534872989104,
    0.97078680611901702,
    0.97136867322824805,
    0.97316323062125198,
    0.97674022315876496,
    0.98158760549137702,
    0.98628026565294902,
    0.989949147689134,
    0.99249270153841995,
    0.99414568040525597,
    0.99518397503321199,
    0.99575675011081799,
    0.99591281828670997,
    0.99560615783452799,
    0.99459760096185401,
    0.99221571549236998,
    0.98623645278324901,
    0.96794333726454096,
    0.89128500424494295,
    0.536202477862053,
    0.15410811900187801,
    0.057457509322892897,
    0.031534987310700702,
    0.0222633920086335,
    0.018202284149243901,
    0.016299055973264,
    0.0153656239334613,
    0.014911156873397601,
    0.0146954339898235,
    0.014596414671771901,
    0.0145470156699655,
    0.014522877189949501,
    0.014512034111896501,
    0.0145066940939832,
    0.014504450731447901,
    0.014503800946463899,
];

/// Base reflectance: magenta.
pub const BASE_M: [f64; SIZE] = [
    0.99067355731998796,
    0.99067152496197897,
    0.99066258235342097,
    0.99061810764479497,
    0.99045148087871004,
    0.98987108140020397,
    0.98828660875964003,
    0.98429069279750403,
    0.97393490562530605,
    0.941817838460145,
    0.81739032619515595,
    0.43247280506572899,
    0.13845397825886999,
    0.053734721694003298,
    0.0292174996673231,
    0.021313651750858999,
    0.020134953018113601,
    0.024132309628066201,
    0.037223614522362697,
    0.0760506552706601,
    0.205375471942399,
    0.54126890346043899,
    0.81584168508648602,
    0.912817704123976,
    0.94633983016696199,
    0.95992769633199104,
    0.96626059523031205,
    0.96932597005842402,
    0.97085453672139899,
    0.97160506652812795,
    0.97196276975739204,
    0.97212727227450901,
    0.97220941774581204,
    0.97224957767842402,
    0.97226762199874195,
    0.97227650946215005,
    0.97228024330687401,
    0.97228132482656004,
];

/// Base reflectance: yellow.
pub const BASE_Y: [f64; SIZE] = [
    0.0210523371789306,
    0.0210564627517414,
    0.021074617869503799,
    0.021164905844875299,
    0.0215027957272504,
    0.022673879904156102,
    0.025823564969362899,
    0.033487938563985097,
    0.051906966374030701,
    0.100749014833473,
    0.239129899706847,
    0.53480431227274805,
    0.79780757864302998,
    0.91144989406738397,
    0.95379796300450703,
    0.97124161546542898,
    0.97930312380758799,
    0.98338011950757498,
    0.98546124656775502,
    0.986435046976605,
    0.98673825067014098,
    0.98661788244503201,
    0.98627777675864303,
    0.98586059244405599,
    0.98547492767621003,
    0.98517693476555801,
    0.98497157401418101,
    0.984846303415712,
    0.98477535181119902,
    0.98473806662526497,
    0.98471964831176495,
    0.98471102339193906,
    0.98470668330067601,
    0.98470455439309101,
    0.98470359630936999,
    0.98470312407755201,
    0.98470292561508999,
    0.98470286812279495,
];

/// Base reflectance: red.
pub const BASE_R: [f64; SIZE] = [
    0.031560573777720702,
    0.031552071833014901,
    0.031514821551365799,
    0.031331804498270201,
    0.030672985772552699,
    0.028648047698960701,
    0.024645040704570902,
    0.019296075366365101,
    0.0142066612220556,
    0.010294260887860901,
    0.0076191460521811,
    0.0058980410835420003,
    0.0048233247781713003,
    0.0042298748350633004,
    0.0040599171299340996,
    0.0043533695594676,
    0.0053434425970201004,
    0.0076917201010462998,
    0.013596979573653601,
    0.031697544266111502,
    0.107861196355249,
    0.46381260316870399,
    0.84705540527201095,
    0.94318540939391804,
    0.96886215069655801,
    0.97803066747360301,
    0.98204364385430598,
    0.98392362371870701,
    0.984845484154382,
    0.98529427581459605,
    0.98550729521982505,
    0.98560507153983701,
    0.98565384993357796,
    0.98567768503388298,
    0.985688391806122,
    0.98569366469003095,
    0.98569587984820495,
    0.98569652146376197,
];

/// Base reflectance: green.
pub const BASE_G: [f64; SIZE] = [
    0.0095560747554212008,
    0.0095581580120850992,
    0.0095673245444587995,
    0.0096129126297348991,
    0.0097837090401843001,
    0.010378622705870999,
    0.012002645237856701,
    0.016097772147392199,
    0.026706190223167999,
    0.059555544018588098,
    0.186039826532826,
    0.57057982011615904,
    0.86146776840029204,
    0.945879089767658,
    0.97046548647430497,
    0.97841363028445005,
    0.979589031411224,
    0.97553353690863198,
    0.962288755397813,
    0.92312157451312005,
    0.79343401894311105,
    0.45927013590242899,
    0.185574103666303,
    0.088177495995537203,
    0.054363022876669997,
    0.0406288447060719,
    0.034221520431697003,
    0.0311185790956966,
    0.0295708898336134,
    0.028810873934892799,
    0.0284486271324597,
    0.028282030172473101,
    0.0281988376490237,
    0.028158165534203701,
    0.028139891021638601,
    0.028130890166581098,
    0.028127108680581599,
    0.028126013361209599,
];

/// Base reflectance: blue.
pub const BASE_B: [f64; SIZE] = [
    0.97940475250201398,
    0.97940070684312996,
    0.97938290347026102,
    0.97929436494559396,
    0.97896301460857005,
    0.97781446669404304,
    0.974724321133836,
    0.96719848234397299,
    0.94907965753057499,
    0.90085012894097705,
    0.76315044546223998,
    0.46592217164931898,
    0.201263280451005,
    0.087752441341962306,
    0.045717679329167898,
    0.028470605052184302,
    0.020527176756985,
    0.0165302792310211,
    0.0145135107212858,
    0.0136003508637687,
    0.013360425876957099,
    0.013548894314568001,
    0.0139594356366992,
    0.014443425575356999,
    0.014885444062140599,
    0.0152254296999746,
    0.0154592848180209,
    0.0156018026485961,
    0.0156824871281936,
    0.015724876436061502,
    0.015745810878412099,
    0.015755612335022501,
    0.015760544396491102,
    0.0157629637515278,
    0.015764052562910601,
    0.015764589232951001,
    0.0157648147772649,
    0.015764880114961601,
];

/// CIE CMF weighted by D65 (columns sum to the whitepoint).
pub const CMF: [[f64; SIZE]; 3] = [
    [
        6.4691998957599998e-05,
        0.0002194098998132,
        0.0011205743509342999,
        0.0037666134117110999,
        0.011880553603799001,
        0.0232864424191771,
        0.034559418196974702,
        0.037223790116200597,
        0.032418376109148599,
        0.021233205609380999,
        0.0104909907685421,
        0.0032958375797931001,
        0.0005070351633801,
        0.00094867420571409999,
        0.0062737180998318001,
        0.016864624189777501,
        0.028689649025981,
        0.042674812469173103,
        0.056254748131137698,
        0.069470397267715797,
        0.083053151699829103,
        0.086126096300225705,
        0.090466137684776904,
        0.085003865059127701,
        0.070906669107448805,
        0.050628891637364497,
        0.035473961885263999,
        0.021468210259706501,
        0.012516456761911701,
        0.0068045816390165,
        0.0034645657946526,
        0.0014976097506959,
        0.00076970048092800001,
        0.00040736805813150001,
        0.0001690104031614,
        9.5224515036500002e-05,
        4.9030987295800002e-05,
        1.9996149222199999e-05,
    ],
    [
        1.8442894439999999e-06,
        6.2053235864999997e-06,
        3.1009604679899997e-05,
        0.0001047483849269,
        0.00035364052995379998,
        0.00095147140564439997,
        0.0022822631748318002,
        0.0042073290434729996,
        0.0066887983719013996,
        0.0098883960193564997,
        0.0152494514496311,
        0.021418310944972301,
        0.033422930157506803,
        0.0513100134918512,
        0.070402083939948998,
        0.087838707260351701,
        0.094249053618408504,
        0.097956670271893101,
        0.094152185686260798,
        0.086781023748675301,
        0.078856533863201295,
        0.063526702620355494,
        0.053741416756820003,
        0.042646064357412,
        0.031617349279270797,
        0.020885205921390999,
        0.013860110136015199,
        0.0081026402038398993,
        0.0046301022588030004,
        0.0024913800051319002,
        0.0012593033677378001,
        0.00054164652216800004,
        0.00027795289200669999,
        0.00014710806738540001,
        6.1032747292699998e-05,
        3.4387322952300002e-05,
        1.7705986005300002e-05,
        7.2209749129999999e-06,
    ],
    [
        0.00030501714763800001,
        0.0010368066663574,
        0.0053131363323991997,
        0.017954392589953599,
        0.057077581534548498,
        0.113651618936287,
        0.17335872618355,
        0.196206575558657,
        0.18608237070629599,
        0.13995047538320701,
        0.089174529426864907,
        0.047896211351707497,
        0.028145625395795199,
        0.016137662295051401,
        0.0077591019215214,
        0.0042961483736617999,
        0.0020055092122156001,
        0.00086147110988020004,
        0.0003690387177652,
        0.0001914287288574,
        0.0001495555858975,
        9.2310928510399998e-05,
        6.8134918233699995e-05,
        2.8826365569600001e-05,
        1.5767182055300001e-05,
        3.9406041026999997e-06,
        1.584012587e-06,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
    ],
];

/// Linear sRGB -> XYZ (CSS Color 4 values).
pub const RGB_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.41239079926595934,
        0.35758433938387801,
        0.18048078840183429,
    ],
    [
        0.21263900587151027,
        0.71516867876775603,
        0.072192315360733714,
    ],
    [
        0.019330818715591822,
        0.11919477979462598,
        0.9505321522496607,
    ],
];

/// XYZ -> linear sRGB (CSS Color 4 values).
pub const XYZ_TO_RGB: [[f64; 3]; 3] = [
    [
        3.2409699419045226,
        -1.5373831775700939,
        -0.49861076029300339,
    ],
    [
        -0.96924363628087962,
        1.8759675015077202,
        0.041555057407175591,
    ],
    [
        0.055630079696993663,
        -0.20397695888897652,
        1.0569715142428786,
    ],
];

/// A pigment "color" in reflectance form (port of the spectral.js `Color`
/// essentials: reflectance + luminance + tinting strength).
#[derive(Clone, Debug)]
pub struct Pigment {
    /// Spectral reflectance, `SIZE` samples, each >= f64::EPSILON.
    pub reflectance: [f64; SIZE],
    /// Y tristimulus (luminance), floored at f64::EPSILON.
    pub luminance: f64,
    /// Mixing tinting strength (default 1).
    pub tinting_strength: f64,
}

/// Kubelka–Munk absorption/scattering ratio for reflectance `r`
/// (port of `KS`): `(1 - R)² / (2 R)`.
#[inline]
pub fn ks(r: f64) -> f64 {
    (1.0 - r) * (1.0 - r) / (2.0 * r)
}

/// Inverse transform back to reflectance (port of `KM`):
/// `1 + KS − sqrt(KS² + 2 KS)`.
#[inline]
pub fn km(ks: f64) -> f64 {
    1.0 + ks - (ks * ks + 2.0 * ks).sqrt()
}

/// Reflectance -> XYZ through the D65-weighted CMF (port of `R_to_XYZ`).
pub fn reflectance_to_xyz(r: &[f64; SIZE]) -> [f64; 3] {
    let mut xyz = [0f64; 3];
    for (row, out) in CMF.iter().zip(xyz.iter_mut()) {
        *out = row.iter().zip(r.iter()).map(|(a, b)| a * b).sum();
    }
    xyz
}

/// XYZ -> linear sRGB (port of `XYZ_to_lRGB`).
pub fn xyz_to_lrgb(xyz: [f64; 3]) -> [f64; 3] {
    let mut out = [0f64; 3];
    for (row, o) in XYZ_TO_RGB.iter().zip(out.iter_mut()) {
        *o = row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2];
    }
    out
}

/// Linear sRGB -> reflectance via the seven base spectra (port of
/// `lRGB_to_R`: W/C/M/Y/R/G/B decomposition of the color).
pub fn lrgb_to_reflectance(lrgb: [f64; 3]) -> [f64; SIZE] {
    let w = lrgb[0].min(lrgb[1]).min(lrgb[2]);
    let r0 = lrgb[0] - w;
    let g0 = lrgb[1] - w;
    let b0 = lrgb[2] - w;
    let c = g0.min(b0);
    let m = r0.min(b0);
    let y = r0.min(g0);
    let r = (r0 - b0).min(r0 - g0).max(0.0);
    let g = (g0 - b0).min(g0 - r0).max(0.0);
    let b = (b0 - g0).min(b0 - r0).max(0.0);
    let mut refl = [0f64; SIZE];
    for i in 0..SIZE {
        refl[i] = (w * BASE_W[i]
            + c * BASE_C[i]
            + m * BASE_M[i]
            + y * BASE_Y[i]
            + r * BASE_R[i]
            + g * BASE_G[i]
            + b * BASE_B[i])
            .max(f64::EPSILON);
    }
    refl
}

impl Pigment {
    /// Build a pigment from linear sRGB (tinting strength 1).
    pub fn from_lrgb(lrgb: [f64; 3]) -> Pigment {
        let reflectance = lrgb_to_reflectance(lrgb);
        Pigment::from_reflectance(reflectance)
    }

    /// Build a pigment from an arbitrary reflectance spectrum (tinting
    /// strength 1) — the entry point for spectra sampled from the rgb2spec
    /// sigmoid path.
    pub fn from_reflectance(reflectance: [f64; SIZE]) -> Pigment {
        let xyz = reflectance_to_xyz(&reflectance);
        Pigment {
            reflectance,
            luminance: xyz[1].max(f64::EPSILON),
            tinting_strength: 1.0,
        }
    }

    /// Linear sRGB of this pigment.
    pub fn to_lrgb(&self) -> [f64; 3] {
        xyz_to_lrgb(reflectance_to_xyz(&self.reflectance))
    }
}

/// Kubelka–Munk mix of weighted pigments (port of `mix`): per band, the
/// concentration-weighted KS average (concentration = factor² ×
/// tinting-strength² × luminance) is transformed back through KM.
pub fn mix(parts: &[(&Pigment, f64)]) -> Pigment {
    assert!(!parts.is_empty(), "km::mix: empty input");
    let mut refl = [0f64; SIZE];
    for (i, band) in refl.iter_mut().enumerate() {
        let mut ks_mix = 0.0;
        let mut total = 0.0;
        for (p, factor) in parts {
            let concentration =
                factor * factor * p.tinting_strength * p.tinting_strength * p.luminance;
            total += concentration;
            ks_mix += ks(p.reflectance[i]) * concentration;
        }
        *band = km(ks_mix / total);
    }
    let xyz = reflectance_to_xyz(&refl);
    Pigment {
        reflectance: refl,
        luminance: xyz[1].max(f64::EPSILON),
        tinting_strength: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::plane_hash_f64;

    #[test]
    fn ks_km_are_inverse_transforms() {
        for r in [0.001f64, 0.05, 0.3, 0.7, 0.999] {
            let back = km(ks(r));
            assert!((back - r).abs() < 1e-12, "R {r} -> {back}");
        }
    }

    #[test]
    fn white_reflectance_reproduces_white() {
        // The white base spectrum through CMF/D65 must land at lRGB ~ 1.
        let p = Pigment::from_lrgb([1.0, 1.0, 1.0]);
        let rgb = p.to_lrgb();
        for c in 0..3 {
            assert!((rgb[c] - 1.0).abs() < 5e-3, "white -> {rgb:?}");
        }
    }

    #[test]
    fn round_trip_of_primaries_is_close() {
        for rgb in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.2, 0.5, 0.8],
        ] {
            let back = Pigment::from_lrgb(rgb).to_lrgb();
            for c in 0..3 {
                assert!(
                    (back[c] - rgb[c]).abs() < 0.03,
                    "{rgb:?} -> {back:?} channel {c}"
                );
            }
        }
    }

    #[test]
    fn self_mix_is_identity() {
        let p = Pigment::from_lrgb([0.3, 0.6, 0.2]);
        let m = mix(&[(&p, 1.0), (&p, 1.0)]);
        for i in 0..SIZE {
            assert!((m.reflectance[i] - p.reflectance[i]).abs() < 1e-9);
        }
    }

    #[test]
    fn blue_plus_yellow_makes_green() {
        // The signature Kubelka-Munk behavior (and the reason the pigment
        // mode exists): subtractive blue + yellow mixing trends green, not
        // the gray that linear RGB averaging produces.
        let blue = Pigment::from_lrgb([0.0, 0.0, 1.0]);
        let yellow = Pigment::from_lrgb([1.0, 1.0, 0.0]);
        let m = mix(&[(&blue, 1.0), (&yellow, 1.0)]).to_lrgb();
        assert!(
            m[1] > m[0] && m[1] > m[2],
            "blue+yellow K-M mix should be green-dominant, got {m:?}"
        );
    }

    #[test]
    fn mix_is_deterministic() {
        let a = Pigment::from_lrgb([0.8, 0.1, 0.2]);
        let b = Pigment::from_lrgb([0.1, 0.3, 0.9]);
        let m1 = mix(&[(&a, 2.0), (&b, 1.0)]);
        let m2 = mix(&[(&a, 2.0), (&b, 1.0)]);
        assert_eq!(
            plane_hash_f64(&m1.reflectance),
            plane_hash_f64(&m2.reflectance)
        );
    }
}
