// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §9.1–§9.6; include/appname.h (ABI 1.x — 28 symbols at 1.2;
// every 1.2-block use is version-gated via abiHasBlock12).

#include "abi_client.h"

#include <QJsonArray>
#include <QJsonParseError>

#include <algorithm>
#include <cstring>

namespace appname::shell {

namespace {

QJsonObject parseObject(const QByteArray &bytes)
{
    QJsonParseError err;
    const QJsonDocument doc = QJsonDocument::fromJson(bytes, &err);
    if (err.error != QJsonParseError::NoError || !doc.isObject())
        return {};
    return doc.object();
}

CallResult resultFromCall(AppnameStatus status, AppnameError *err, const AbiString &outJson)
{
    if (status == APPNAME_OK) {
        CallResult r = CallResult::success(parseObject(outJson.toBytes()));
        return r;
    }
    CallResult r = CallResult::fromError(status, err);
    if (!outJson.isNull())
        r.json = parseObject(outJson.toBytes()); // rejection wire document, when it exists
    return r;
}

} // namespace

QByteArray AbiString::toBytes() const
{
    if (!m_s)
        return {};
    size_t len = 0;
    const char *data = appname_string_data(m_s, &len);
    if (!data)
        return {};
    // Portable consumers use the length, not the NUL (ES §9.3 contract).
    return QByteArray(data, static_cast<qsizetype>(len));
}

CallResult CallResult::success(QJsonObject payload)
{
    CallResult r;
    r.ok = true;
    r.status = APPNAME_OK;
    r.json = std::move(payload);
    return r;
}

CallResult CallResult::fromError(AppnameStatus status, AppnameError *err)
{
    CallResult r;
    r.ok = false;
    r.status = status;
    if (err) {
        size_t len = 0;
        const char *json = appname_error_json(err, &len);
        if (json) {
            const QJsonObject obj = parseObject(QByteArray(json, static_cast<qsizetype>(len)));
            r.code = obj.value(QLatin1String("code")).toString();
            r.message = obj.value(QLatin1String("message")).toString();
        }
        appname_error_free(err);
    }
    if (r.code.isEmpty())
        r.code = QStringLiteral("error.api.invalid_arg"); // boundary misuse without payload
    return r;
}

CallResult AbiDoc::create()
{
    close();
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_new(&m_doc, &err);
    if (status != APPNAME_OK) {
        m_doc = nullptr;
        return CallResult::fromError(status, err);
    }
    return CallResult::success();
}

CallResult AbiDoc::open(const QString &path)
{
    close();
    AppnameError *err = nullptr;
    const AppnameStatus status =
        appname_doc_open(path.toUtf8().constData(), &m_doc, &err);
    if (status != APPNAME_OK) {
        m_doc = nullptr;
        return CallResult::fromError(status, err);
    }
    return CallResult::success();
}

CallResult AbiDoc::save(const QString &path)
{
    AppnameError *err = nullptr;
    const AppnameStatus status =
        appname_doc_save(m_doc, path.toUtf8().constData(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    return CallResult::success();
}

void AbiDoc::close()
{
    if (m_doc) {
        appname_doc_close(m_doc);
        m_doc = nullptr;
    }
}

CallResult AbiDoc::apply(const QByteArray &envelopeJson)
{
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_apply_command(
        m_doc,
        reinterpret_cast<const uint8_t *>(envelopeJson.constData()),
        static_cast<size_t>(envelopeJson.size()),
        out.out(),
        &err);
    return resultFromCall(status, err, out);
}

CallResult AbiDoc::undo()
{
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_undo(m_doc, out.out(), &err);
    return resultFromCall(status, err, out);
}

CallResult AbiDoc::redo()
{
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_redo(m_doc, out.out(), &err);
    return resultFromCall(status, err, out);
}

CallResult AbiDoc::infoJson()
{
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_info_json(m_doc, out.out(), &err);
    return resultFromCall(status, err, out);
}

CallResult AbiDoc::historyJson()
{
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_history_json(m_doc, out.out(), &err);
    return resultFromCall(status, err, out);
}

CallResult AbiDoc::pollEvents(QJsonArray *events)
{
    *events = QJsonArray();
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_poll_events(m_doc, out.out(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    QJsonParseError perr;
    const QJsonDocument doc = QJsonDocument::fromJson(out.toBytes(), &perr);
    if (perr.error == QJsonParseError::NoError && doc.isArray())
        *events = doc.array();
    return CallResult::success();
}

CallResult AbiDoc::readRegion(const QRect &rect, QImage *outImage)
{
    AppnameRect r;
    r.x = rect.x();
    r.y = rect.y();
    r.w = static_cast<uint32_t>(std::max(rect.width(), 0));
    r.h = static_cast<uint32_t>(std::max(rect.height(), 0));

    AppnameImageBuffer *buf = nullptr;
    AppnameError *err = nullptr;
    AppnameStatus status = appname_doc_read_region(m_doc, r, &buf, &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);

    AppnameMapInfo map;
    std::memset(&map, 0, sizeof(map));
    status = appname_buffer_map(buf, &map);
    if (status != APPNAME_OK || !map.data) {
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = status;
        res.code = QStringLiteral("error.api.internal"); // unified (review D13)
        res.message = QStringLiteral("buffer map failed");
        return res;
    }

    // v1 delivery contract: pixel_format 0 (u8), color_model 3 (RGBA),
    // straight alpha, tight stride. Copy the bytes verbatim into a QImage —
    // presentation only, no pixel math (separation rule).
    QImage img(static_cast<int>(map.width), static_cast<int>(map.height),
               QImage::Format_RGBA8888);
    if (img.isNull()) {
        // Allocation failure (readback is legal up to 2^26 px): fail typed
        // like every other path in this wrapper, never crash (review R6).
        appname_buffer_unmap(buf);
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = APPNAME_ERR_INTERNAL;
        res.code = QStringLiteral("error.api.internal");
        res.message = QStringLiteral("image allocation failed for readback region");
        return res;
    }
    if (map.stride == map.width * 4 && img.bytesPerLine() == static_cast<qsizetype>(map.stride)) {
        // Generic wrapper: clamp to the destination even though the header
        // documents len == height*stride (review R6).
        std::memcpy(img.bits(), map.data,
                    std::min<size_t>(map.len, static_cast<size_t>(img.sizeInBytes())));
    } else {
        for (uint32_t row = 0; row < map.height; ++row)
            std::memcpy(img.scanLine(static_cast<int>(row)), map.data + row * map.stride,
                        static_cast<size_t>(map.width) * 4);
    }
    appname_buffer_unmap(buf);
    appname_buffer_free(buf);
    *outImage = std::move(img);
    return CallResult::success();
}

CallResult AbiDoc::readLayer(const QString &layerId, const QRect &rect,
                             QImage *outImage)
{
    AppnameRect r;
    r.x = rect.x();
    r.y = rect.y();
    r.w = static_cast<uint32_t>(std::max(rect.width(), 0));
    r.h = static_cast<uint32_t>(std::max(rect.height(), 0));

    const QByteArray idUtf8 = layerId.toUtf8();
    AppnameImageBuffer *buf = nullptr;
    AppnameError *err = nullptr;
    AppnameStatus status =
        appname_doc_read_layer(m_doc, idUtf8.constData(), r, &buf, &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);

    AppnameMapInfo map;
    std::memset(&map, 0, sizeof(map));
    status = appname_buffer_map(buf, &map);
    if (status != APPNAME_OK || !map.data) {
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = status;
        res.code = QStringLiteral("error.api.internal"); // unified (review D13)
        res.message = QStringLiteral("buffer map failed");
        return res;
    }

    // Same delivery contract as readRegion (header: "the exact
    // appname_doc_read_region delivery convention"): RGBA8, straight
    // alpha, tight stride. Verbatim byte copy — presentation only.
    QImage img(static_cast<int>(map.width), static_cast<int>(map.height),
               QImage::Format_RGBA8888);
    if (img.isNull()) {
        appname_buffer_unmap(buf);
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = APPNAME_ERR_INTERNAL;
        res.code = QStringLiteral("error.api.internal");
        res.message = QStringLiteral("image allocation failed for readback region");
        return res;
    }
    if (map.stride == map.width * 4
        && img.bytesPerLine() == static_cast<qsizetype>(map.stride)) {
        std::memcpy(img.bits(), map.data,
                    std::min<size_t>(map.len, static_cast<size_t>(img.sizeInBytes())));
    } else {
        for (uint32_t row = 0; row < map.height; ++row)
            std::memcpy(img.scanLine(static_cast<int>(row)), map.data + row * map.stride,
                        static_cast<size_t>(map.width) * 4);
    }
    appname_buffer_unmap(buf);
    appname_buffer_free(buf);
    *outImage = std::move(img);
    return CallResult::success();
}

CallResult AbiDoc::provideFace(const QByteArray &faceBytes,
                               const QString &source, const QString &family,
                               const QString &style,
                               const QString &postscriptName, QString *idOut)
{
    const QByteArray src = source.toUtf8();
    const QByteArray fam = family.toUtf8();
    const QByteArray sty = style.toUtf8();
    const QByteArray psn = postscriptName.toUtf8();
    AppnameString *id = nullptr;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_provide_face(
        m_doc, reinterpret_cast<const uint8_t *>(faceBytes.constData()),
        static_cast<size_t>(faceBytes.size()), src.constData(),
        fam.constData(), sty.constData(), psn.constData(),
        idOut ? &id : nullptr, &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    if (idOut && id) {
        size_t len = 0;
        const char *data = appname_string_data(id, &len);
        *idOut = QString::fromUtf8(data, qsizetype(len));
    }
    if (id)
        appname_string_free(id);
    return CallResult::success();
}

CallResult AbiDoc::fontFaces(QJsonObject *outJson)
{
    AppnameString *out = nullptr;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_font_faces(m_doc, &out, &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    if (outJson && out) {
        size_t len = 0;
        const char *data = appname_string_data(out, &len);
        const QJsonDocument doc =
            QJsonDocument::fromJson(QByteArray(data, qsizetype(len)));
        *outJson = doc.object();
    }
    if (out)
        appname_string_free(out);
    return CallResult::success();
}

CallResult AbiDoc::readSelection(const QRect &rect, QImage *outMask)
{
    AppnameRect r;
    r.x = rect.x();
    r.y = rect.y();
    r.w = static_cast<uint32_t>(std::max(rect.width(), 0));
    r.h = static_cast<uint32_t>(std::max(rect.height(), 0));

    AppnameImageBuffer *buf = nullptr;
    AppnameError *err = nullptr;
    AppnameStatus status = appname_doc_read_selection(m_doc, r, &buf, &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);

    AppnameMapInfo map;
    std::memset(&map, 0, sizeof(map));
    status = appname_buffer_map(buf, &map);
    if (status != APPNAME_OK || !map.data) {
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = status;
        res.code = QStringLiteral("error.api.internal");
        res.message = QStringLiteral("selection map failed");
        return res;
    }
    // §9.2: branch on the reported fields, never the producing symbol
    // (review D13) — selection delivery is pixel_format 0 / color_model 0.
    if (map.pixel_format != 0 || map.color_model != 0) {
        appname_buffer_unmap(buf);
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = APPNAME_ERR_INTERNAL;
        res.code = QStringLiteral("error.api.internal");
        res.message = QStringLiteral("unexpected selection delivery encoding");
        return res;
    }
    QImage img(static_cast<int>(map.width), static_cast<int>(map.height),
               QImage::Format_Grayscale8);
    if (img.isNull()) {
        appname_buffer_unmap(buf);
        appname_buffer_free(buf);
        CallResult res;
        res.ok = false;
        res.status = APPNAME_ERR_INTERNAL;
        res.code = QStringLiteral("error.api.internal");
        res.message = QStringLiteral("mask allocation failed");
        return res;
    }
    for (uint32_t row = 0; row < map.height; ++row)
        std::memcpy(img.scanLine(static_cast<int>(row)), map.data + row * map.stride,
                    static_cast<size_t>(map.width));
    appname_buffer_unmap(buf);
    appname_buffer_free(buf);
    *outMask = std::move(img);
    return CallResult::success();
}

CallResult AbiDoc::autosaveBegin(const QString &logPath)
{
    AppnameError *err = nullptr;
    const AppnameStatus status =
        appname_doc_autosave_begin(m_doc, logPath.toUtf8().constData(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    return CallResult::success();
}

CallResult AbiDoc::autosaveEnd()
{
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_autosave_end(m_doc, &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    return CallResult::success();
}

CallResult AbiDoc::openRecovered(const QString &packagePath, const QString &logPath,
                                 QJsonObject *reportOut)
{
    // Fresh-handle discipline (review D10, mirrors the R2 open() rule):
    // recover into a LOCAL handle and adopt only on success — a replay
    // failure must not destroy the caller's live document.
    AppnameDoc *recovered = nullptr;
    AbiString report;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_open_recovered(
        packagePath.toUtf8().constData(), logPath.toUtf8().constData(), &recovered,
        report.out(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    close();
    m_doc = recovered;
    if (reportOut) {
        QJsonParseError perr;
        const QJsonDocument doc = QJsonDocument::fromJson(report.toBytes(), &perr);
        *reportOut = (perr.error == QJsonParseError::NoError && doc.isObject())
            ? doc.object()
            : QJsonObject();
    }
    return CallResult::success();
}

CallResult autosaveQuery(const QString &logPath, QJsonObject *reportOut)
{
    AbiString out;
    AppnameError *err = nullptr;
    const AppnameStatus status =
        appname_autosave_query(logPath.toUtf8().constData(), out.out(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    if (reportOut) {
        QJsonParseError perr;
        const QJsonDocument doc = QJsonDocument::fromJson(out.toBytes(), &perr);
        *reportOut = (perr.error == QJsonParseError::NoError && doc.isObject())
            ? doc.object()
            : QJsonObject();
    }
    return CallResult::success();
}

CallResult autosaveDiscard(const QString &logPath)
{
    AppnameError *err = nullptr;
    const AppnameStatus status =
        appname_autosave_discard(logPath.toUtf8().constData(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    return CallResult::success();
}

QString abiVersionString()
{
    const AppnameVersion v = appname_version();
    return QStringLiteral("ABI %1.%2 / engine %3.%4")
        .arg(v.abi_major)
        .arg(v.abi_minor)
        .arg(v.engine_major)
        .arg(v.engine_minor);
}

bool abiHasBlock12()
{
    const AppnameVersion v = appname_version();
    return v.abi_major == 1 && v.abi_minor >= 2;
}

bool abiHasBlock14()
{
    const AppnameVersion v = appname_version();
    return v.abi_major == 1 && v.abi_minor >= 4;
}

bool abiHasBlock16()
{
    const AppnameVersion v = appname_version();
    return v.abi_major == 1 && v.abi_minor >= 6;
}

bool abiHasBlock17()
{
    const AppnameVersion v = appname_version();
    return v.abi_major == 1 && v.abi_minor >= 7;
}

CallResult AbiDoc::exportComposite(const QString &path,
                                   const QString &format,
                                   const QByteArray &optionsJson)
{
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_doc_export(
        m_doc, path.toUtf8().constData(), format.toUtf8().constData(),
        optionsJson.isEmpty() ? nullptr : optionsJson.constData(), &err);
    if (status != APPNAME_OK)
        return CallResult::fromError(status, err);
    return CallResult::success();
}

// ---- samples-asset ingress ------------------------------------------------
//
// appname_asset_create (header-declared): validates + copies the canonical
// appname.samples.v1 blob bytes before returning; ids are content-addressed
// (same bytes -> same id). See abi_client.h for why the old dlsym probe of
// the §9.2 draft prototype is retired.

std::optional<QString> ingestSamplesBlob(AppnameDoc *doc,
                                         const QByteArray &samplesBlobBytes,
                                         QString *codeOut,
                                         QString *messageOut)
{
    AbiString uuid;
    AppnameError *err = nullptr;
    const AppnameStatus status = appname_asset_create(
        doc,
        reinterpret_cast<const uint8_t *>(samplesBlobBytes.constData()),
        static_cast<size_t>(samplesBlobBytes.size()),
        uuid.out(),
        &err);
    if (status != APPNAME_OK) {
        const CallResult r = CallResult::fromError(status, err);
        if (codeOut)
            *codeOut = r.code;
        if (messageOut)
            *messageOut = r.message;
        return std::nullopt;
    }
    return QString::fromUtf8(uuid.toBytes());
}

} // namespace appname::shell
