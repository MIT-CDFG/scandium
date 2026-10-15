// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.1–§9.6 (C ABI contract; include/appname.h is the door);
// HLD "same door as everyone"; plan §2.2 "Qt/QML shell v1" row (pure C-ABI
// client — no Rust crate imports, no engine math).
//
// Thin RAII layer over the ABI (28 symbols at 1.2): owned string/error/buffer
// wrappers, one `CallResult` shape carrying the typed error surface
// (status + `error.*` code + developer message + wire JSON) so every
// rejection reaches the status bar typed, never as a crash or a blank.
// v1 ABI is synchronous; all calls happen on the UI thread (readback is
// viewport/dirty-rect sized — the §9.4 >1 Mpx off-thread guidance is a
// CP-0.5 follow-up alongside the shared-texture path).

#pragma once

#include "appname.h"

#include <QByteArray>
#include <QImage>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRect>
#include <QString>

#include <optional>

namespace appname::shell {

// Owned AppnameString -> QByteArray/QString, freed on destruction.
class AbiString {
public:
    AbiString() = default;
    explicit AbiString(AppnameString *s)
        : m_s(s)
    {
    }
    AbiString(const AbiString &) = delete;
    AbiString &operator=(const AbiString &) = delete;
    AbiString(AbiString &&other) noexcept
        : m_s(other.m_s)
    {
        other.m_s = nullptr;
    }
    AbiString &operator=(AbiString &&other) noexcept
    {
        if (this != &other) {
            free();
            m_s = other.m_s;
            other.m_s = nullptr;
        }
        return *this;
    }
    ~AbiString() { free(); }

    AppnameString **out() { return &m_s; }
    bool isNull() const { return m_s == nullptr; }
    QByteArray toBytes() const;

private:
    void free()
    {
        if (m_s) {
            appname_string_free(m_s);
            m_s = nullptr;
        }
    }
    AppnameString *m_s = nullptr;
};

// One call's outcome. `ok` mirrors APPNAME_OK; on rejection `code` is the
// engine's stable `error.*` taxonomy id (locale-independent), `message`
// the developer message, `json` whatever wire document the call produced
// (receipt or rejection document).
struct CallResult {
    bool ok = false;
    AppnameStatus status = APPNAME_OK;
    QString code;    // e.g. "error.command.not_implemented"; empty on ok
    QString message; // developer detail, untranslated by design
    QJsonObject json; // parsed out_json (receipt / rejection / info / ...)

    static CallResult success(QJsonObject payload = {});
    static CallResult fromError(AppnameStatus status, AppnameError *err); // frees err
};

// Owned document handle + the ABI verbs the shell uses. Pure client: every
// mutation is an appname.command.v1 envelope through apply().
class AbiDoc {
public:
    AbiDoc() = default;
    ~AbiDoc() { close(); }
    AbiDoc(const AbiDoc &) = delete;
    AbiDoc &operator=(const AbiDoc &) = delete;
    AbiDoc(AbiDoc &&other) noexcept
        : m_doc(other.m_doc)
    {
        other.m_doc = nullptr;
    }
    AbiDoc &operator=(AbiDoc &&other) noexcept
    {
        if (this != &other) {
            close();
            m_doc = other.m_doc;
            other.m_doc = nullptr;
        }
        return *this;
    }

    // appname_doc_new: fresh pre-init handle (only doc.new is accepted
    // until initialized — ES §9.6 single door).
    CallResult create();
    // appname_doc_open: load a .appname package (suffix-aligned history —
    // canUndo/canRedo false at the reopened base by design).
    CallResult open(const QString &path);
    // appname_doc_save: atomic temp+rename package save (journal travels).
    CallResult save(const QString &path);
    void close();
    bool isOpen() const { return m_doc != nullptr; }
    AppnameDoc *raw() const { return m_doc; }

    // THE single mutation entry (D-48): raw envelope bytes in, receipt or
    // typed rejection out.
    CallResult apply(const QByteArray &envelopeJson);

    // ABI 1.2: selection coverage mask over rect as Grayscale8 (0..255,
    // stride = w; the §13.1 f32 mask under 8-bit delivery — feathered
    // edges arrive as intermediate coverage). Verbatim byte copy.
    CallResult readSelection(const QRect &rect, QImage *outMask);

    // ABI 1.4: ONE layer's SELF-CONTAINED content over rect as RGBA8888
    // under the exact readRegion delivery convention. CONTENT, not
    // contribution — the layer's own blend/opacity are NOT applied
    // (header contract). Typed refusals per the header's service matrix
    // (adjustment layers, kinds without a composite path, ...) surface
    // in the CallResult; callers decide whether they are expected.
    CallResult readLayer(const QString &layerId, const QRect &rect,
                         QImage *outImage);

    // ABI 1.6: provide a HOST face to the document's layout FontDb
    // (content-addressed, idempotent per (asset, bytes); metadata
    // collisions and docEmbedded-as-source refuse typed — host bugs
    // surface, per the header). idOut receives the asset UUID.
    CallResult provideFace(const QByteArray &faceBytes,
                           const QString &source, const QString &family,
                           const QString &style,
                           const QString &postscriptName, QString *idOut);
    // ABI 1.6: the deterministic FontDb enumeration + fonts stamp
    // (appname.fontfaces.v1 JSON).
    CallResult fontFaces(QJsonObject *outJson);

    // ABI 1.2 recovery surface: bind/unbind the §7.8 per-command autosave
    // log (bind after the in-memory doc matches a disk base; a successful
    // save FOLDS the log engine-side).
    CallResult autosaveBegin(const QString &logPath);
    CallResult autosaveEnd();
    // Open BASE package + replay log into a fresh live handle.
    CallResult openRecovered(const QString &packagePath, const QString &logPath,
                             QJsonObject *reportOut);

    // ABI 1.2: engine-side export over Codec::encode (7 built-in formats;
    // optionsJson empty = §8.2 defaults, else e.g. {"quality":90}).
    CallResult exportComposite(const QString &path,
                               const QString &format,
                               const QByteArray &optionsJson = {});

    CallResult undo(); // json = reverted history row on ok
    CallResult redo();
    CallResult infoJson();    // json = {"docId","width","height","layers",...}
    CallResult historyJson(); // json = {"length","cursor","entries"}

    // Drained event array (ES §9.5 payloads); empty array when idle.
    CallResult pollEvents(QJsonArray *events);

    // Composite readback of `rect` (document space) as a straight-alpha
    // sRGB RGBA8888 QImage — a byte copy of what the engine delivered
    // (display what readback gives you; zero pixel math here).
    CallResult readRegion(const QRect &rect, QImage *outImage);

private:
    AppnameDoc *m_doc = nullptr;
};

// ABI + engine version for the About/status surface.
QString abiVersionString();

// ABI 1.2: scan an autosave log (torn-tail tolerant) — the W05 prompt's
// evidence surface. json = {docId, commands, assets, torn, lastCommandId,
// warnings}.
CallResult autosaveQuery(const QString &logPath, QJsonObject *reportOut);
// Delete an autosave log (idempotent; the prompt's Discard arm).
CallResult autosaveDiscard(const QString &logPath);

// N.5 rule 2 gate for the ABI 1.2 additive block (export, selection
// readback, recovery surface, relaxed raster ingress, info v2 fields,
// treeChanged). All 1.2 consumers check this, never symbol presence.
bool abiHasBlock12();

// N.5 rule 2 gate for the ABI 1.4 additive block (per-layer content
// readback, info v3 paths array, §12.1 parametric serving through the
// composite calls). All 1.4 consumers check this, never symbol
// presence. The block is OPEN until the next checkpoint tag (header
// note) — the version gate is exactly what protects consumers either
// way.
bool abiHasBlock14();

// N.5 rule 2 gate for the ABI 1.6 additive block (host font
// provisioning: provide_face + font_faces).
bool abiHasBlock16();
// v1.0.1 item 5: the crop_rect capability marker (desk ruling: the
// non-destructive crop arm is served from abi 1.7; the cabi bump is a
// follow-on slice — this reads false until it lands, then flips).
bool abiHasBlock17();

// Samples-asset ingress. The ES §9.2 ingress landed in the ABI as
// `appname_asset_create` (header-declared), taking the CANONICAL
// `appname.samples.v1` blob bytes (16-byte header + 48-byte records) and
// a byte length — NOT the record-array-plus-count shape of the §9.2 draft
// prototype this shell previously probed for via dlsym; the probe is
// retired in favor of the declared door. Returns the content-addressed
// assetId uuid the stroke envelope's `samples` field references (§9.6
// rule 2); on typed rejection returns std::nullopt with the engine's
// `error.*` code and developer message in the out-params.
std::optional<QString> ingestSamplesBlob(AppnameDoc *doc,
                                         const QByteArray &samplesBlobBytes,
                                         QString *codeOut,
                                         QString *messageOut);

} // namespace appname::shell
