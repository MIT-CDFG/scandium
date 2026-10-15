// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09's display half: the licence texts the bundle ships, as a model QML can
// list and read ON DEMAND.
//
// WHY NOT EAGER, MEASURED BEFORE CHOOSING: the eleven texts are 3 top-level
// files beside a LICENSES/ subdirectory, and THIRD-PARTY-NOTICES.md alone is
// 573,513 BYTES — the generated aggregate. Loading all of them at startup
// costs ~750 KB for a dialog most users never open, and pushing half a
// megabyte into a QML Text element stalls the UI. So `entries` is cheap
// metadata and `readText` is the only thing that touches bytes.
//
// WHY A WALK AND NOT AN entryList: the layout is NESTED. `build_dmg.sh`
// stages NOTICE_FILES at the top level and copies LICENSES/ in as a
// DIRECTORY, so a flat listing finds 3 of 11 and looks like it worked.
//
// READS ARE CAPPED, AND THE CAP IS HONEST RATHER THAN SILENT: a truncated
// view says so and points at the folder, which the Help > Open Licenses
// Folder item opens. The FULL text always ships in the bundle; this class
// never decides what is carried, only what is rendered at once.

#pragma once

#include <QObject>
#include <QString>
#include <QVariantList>

namespace appname::shell {

class LicenseTexts : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantList entries READ entries CONSTANT)
    Q_PROPERTY(bool available READ available CONSTANT)
public:
    explicit LicenseTexts(const QString &dir, QObject *parent = nullptr);

    QVariantList entries() const { return m_entries; }
    bool available() const { return !m_entries.isEmpty(); }

    // Returns the file's text, or an empty string if `rel` is not one of the
    // entries. `truncated` is set when the cap bit; the caller must SAY so.
    Q_INVOKABLE QString readText(const QString &rel, bool *truncated = nullptr) const;
    Q_INVOKABLE QVariantMap read(const QString &rel) const;   // {text, truncated}

    static constexpr qint64 kMaxRenderBytes = 256 * 1024;

private:
    QString m_dir;
    QVariantList m_entries;   // [{ name, rel, bytes }]
};

} // namespace appname::shell
