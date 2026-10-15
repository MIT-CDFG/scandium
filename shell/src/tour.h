// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §3.9 standing requirement — `--screenshot-tour` mode: a
// headless-friendly (QT_QPA_PLATFORM=offscreen) scripted run that
// launches, creates a document, sets zoom, attempts brush + eraser
// strokes, undoes, captures PNGs of the main-window states into a given
// output directory, and exits 0. Deterministic: fixed window size, fixed
// document, fixed stroke samples, seeded envelope ids/stroke seeds,
// software Qt Quick backend. This is the standing visual acceptance
// artifact from CP-0.3 on, extended per checkpoint.

#pragma once

#include <QString>

class QQuickWindow;

namespace appname::shell {

class DocumentController;

// Runs the scripted tour against the live window. Returns the process
// exit code (0 = every step ran and every PNG was written). Progress and
// the file inventory go to stdout as machine output (untranslated by
// design, like CLI --json).
int runScreenshotTour(QQuickWindow *window,
                      DocumentController *controller,
                      const QString &outputDir);

} // namespace appname::shell
