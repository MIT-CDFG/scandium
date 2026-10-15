// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: the ABI 1.6 font door (appname_doc_provide_face) takes
// HOST-DERIVED family/style/postscript_name strings — this is the
// derivation: a bounds-checked SFNT 'name'-table read (nameIDs 1/2/6)
// from raw TTF/OTF bytes. PURE metadata extraction for the door's
// marshalling — the ENGINE remains the validator (HarfBuzz+FreeType
// accept or refuse the bytes) and the resolver (§44.1); nothing here
// touches glyphs or layout. Windows-platform UTF-16BE names preferred,
// Mac-Roman fallback. TTC collections are OUT of v1 scope (recorded in
// the lane log; the door takes single faces).

#pragma once

#include <QByteArray>
#include <QString>

namespace appname::shell {

// Parse family (nameID 1), style (2), and PostScript name (6) from a
// single-face TTF/OTF. Returns false (outputs untouched) when the bytes
// are not a parseable single-face SFNT or the name table is
// absent/truncated — callers SKIP such files (the engine would refuse
// them anyway; this parser never guesses).
bool parseFontNames(const QByteArray &fontBytes, QString *family,
                    QString *style, QString *postscriptName);

} // namespace appname::shell
