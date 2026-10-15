// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: see header.

#include "font_meta.h"

#include <QStringDecoder>

namespace appname::shell {

namespace {

quint16 u16(const QByteArray &b, qsizetype off)
{
    return (quint16(quint8(b.at(off))) << 8) | quint8(b.at(off + 1));
}

quint32 u32(const QByteArray &b, qsizetype off)
{
    return (quint32(u16(b, off)) << 16) | u16(b, off + 2);
}

bool inBounds(const QByteArray &b, qsizetype off, qsizetype len)
{
    return off >= 0 && len >= 0 && off + len <= b.size();
}

} // namespace

bool parseFontNames(const QByteArray &b, QString *family, QString *style,
                    QString *postscriptName)
{
    // SFNT offset table: sfntVersion(4) numTables(2) …(6). 0x00010000
    // (TrueType) or 'OTTO' (CFF). 'ttcf' collections are out of scope.
    if (!inBounds(b, 0, 12))
        return false;
    const quint32 sfnt = u32(b, 0);
    if (sfnt != 0x00010000 && sfnt != 0x4F54544F) // 'OTTO'
        return false;
    const quint16 numTables = u16(b, 4);
    if (!inBounds(b, 12, qsizetype(numTables) * 16))
        return false;

    qsizetype nameOff = -1;
    qsizetype nameLen = -1;
    for (quint16 i = 0; i < numTables; ++i) {
        const qsizetype rec = 12 + qsizetype(i) * 16;
        if (u32(b, rec) == 0x6E616D65) { // 'name'
            nameOff = qsizetype(u32(b, rec + 8));
            nameLen = qsizetype(u32(b, rec + 12));
            break;
        }
    }
    if (nameOff < 0 || !inBounds(b, nameOff, nameLen) || nameLen < 6)
        return false;

    const quint16 count = u16(b, nameOff + 2);
    const qsizetype strOff = nameOff + u16(b, nameOff + 4);
    if (!inBounds(b, nameOff + 6, qsizetype(count) * 12))
        return false;

    // Per nameID keep the best candidate: Windows (3) UTF-16BE beats
    // Macintosh (1) Roman; within a platform the first record wins
    // (deterministic — records are stored sorted per the spec).
    struct Slot {
        int quality = 0; // 0 none, 1 mac, 2 windows
        QString value;
    };
    Slot best[3]; // family, style, postscript

    for (quint16 i = 0; i < count; ++i) {
        const qsizetype rec = nameOff + 6 + qsizetype(i) * 12;
        const quint16 platform = u16(b, rec);
        const quint16 nameId = u16(b, rec + 6);
        int slot = -1;
        if (nameId == 1)
            slot = 0;
        else if (nameId == 2)
            slot = 1;
        else if (nameId == 6)
            slot = 2;
        if (slot < 0)
            continue;
        const qsizetype len = u16(b, rec + 8);
        const qsizetype off = strOff + u16(b, rec + 10);
        if (!inBounds(b, off, len) || len == 0)
            continue;
        QString value;
        int quality = 0;
        if (platform == 3) { // Windows: UTF-16BE
            auto dec = QStringDecoder(QStringDecoder::Utf16BE);
            value = dec.decode(QByteArrayView(b.constData() + off, len));
            quality = 2;
        } else if (platform == 1) {
            // Macintosh Roman via Latin-1: an ASCII-compatible
            // APPROXIMATION (high bytes 0x80+ diverge and may mojibake
            // on Mac-only records; psName is ASCII by the OpenType
            // spec, and Mac values are used only when no Windows
            // record exists — review D2 precision fix).
            value = QString::fromLatin1(b.constData() + off, len);
            quality = 1;
        } else {
            continue;
        }
        value = value.trimmed();
        if (!value.isEmpty() && quality > best[slot].quality) {
            best[slot].quality = quality;
            best[slot].value = value;
        }
    }

    if (best[0].quality == 0 || best[2].quality == 0)
        return false; // family + postscript are the door's load-bearing pair
    if (family)
        *family = best[0].value;
    if (style)
        *style = best[1].quality ? best[1].value : QStringLiteral("Regular");
    if (postscriptName)
        *postscriptName = best[2].value;
    return true;
}

} // namespace appname::shell
