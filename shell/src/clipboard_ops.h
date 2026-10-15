// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §26.1 rule 1 + §40.5 (edit.cut) — Cut's Copy half is an
// APP-SIDE step by contract ("Copy / Copy Merged are NOT commands — they
// read via appname_doc_read_region ... and record as app steps"). The
// faithful pairing scales the COPIED alpha by the selection's fractional
// §13.1 coverage exactly as edit.cut scales the REMOVED alpha, so
// cut-then-paste round-trips. This alpha shaping is the app step the
// spec assigns to the host — OS-clipboard transport shaping under the
// CP-0.5 PNG-serialization precedent, not engine composite math.

#pragma once

#include <QImage>

namespace appname::shell {

// Multiply rgba's straight alpha by the selection coverage mask
// (Grayscale8, same dimensions; 255 = keep, 0 = drop, fractional
// scales with round-half-up). Color channels stay verbatim. Returns
// false (rgba untouched) on null/format/dimension mismatch.
bool applySelectionCoverage(QImage *rgba, const QImage &mask);

} // namespace appname::shell
