// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// THE EXPORT-FILTER HONESTY NET (O-UI lane/ui-exportfilter — the same
// unbound-dialog defect the import-filter slice fixed one dialog over).
// File→Export used to open with NO name filter. An unbound Qt FileDialog
// does not show "no filter": Qt substitutes its OWN row, the untranslated
// English literal "All Files (*)". That is two defects at once — the
// dialog advertises every path as a valid export target, and it ships a
// visible English-only string the i18n audit cannot see because Qt, not
// the shell, emits it.
//
// The dialog now offers the formats the ENCODER ACTUALLY WRITES, and this
// suite pins the property that makes the rows worth having.
//
// EXPORT IS A SAVE DIALOG, so the honest set is what the shell can WRITE.
// That set is engine truth, not catalog convenience: the door is
// DocumentController::exportDocument -> AbiDoc::exportComposite ->
// appname_doc_export -> export_encoded_bytes, which resolves the format id
// in CodecRegistry::builtin(), refuses `!capabilities().write` with
// error.codec.write_unsupported, and refuses an unknown id with
// error.codec.unrecognized. This suite does not TAKE that on the source's
// word — case E0 drives the real door once per built-in registry id and
// reports which ones return bytes.
//
// THE HARNESS BOOTS A NON-`en` CATALOG, AND THAT IS LOAD-BEARING.
// Round-1 review MAJOR-1 / MAJOR-28-against-main: with the harness booted
// on `en`, replacing all seven AppI18n.tr(...) lookups in Main.qml with
// hardcoded English literals — the exact untranslated-in-every-locale
// defect this slice exists to remove — left this suite 1/1 GREEN and
// i18n/tools/audit.py green. Every text assert compared the live rows
// against the SAME `en` values those literals were copied from, so a
// literal and a correct lookup were the same string.
// The four-locale sweep in case V is NOT a substitute and never was: it
// LOADS the four catalogs directly and inspects the catalog FILES. That
// proves the files consistent; the DIALOG is a separate question, and it
// was only ever booted under `en`. The fix is to re-boot the thing under
// test, not to read more files alongside it.
// So the harness boots `pseudo` (kHarnessLang): its values are
// mechanically transformed ([!!! ... ~~~ !!!] plus accented glyphs) so no
// English literal can coincide with one; it is machine-GENERATED from en
// (i18n/tools/pseudo.py, audit RED-6 keeps it in sync) so it cannot drift
// into equalling en the way a hand translation could; and it preserves
// the parenthesized glob verbatim, which the honesty cases parse. Case D
// is the standing guard, and E2/E6 additionally assert the live rows
// DIFFER from the `en` values — so reverting the boot to `en`, or losing
// the pseudo catalog, REDs instead of going quietly vacuous again.
//
// Cases (declaration order is run order):
//   D  HARNESS-LOCALE GUARD (runs first): the catalog this harness wires
//      into QML as AppI18n is not `en` — every consumed id resolves to
//      something other than its English value and still carries its glob.
//      Without this, one edit restores the round-1 vacuity in silence.
//   E0 SERVED-SET PROBE (runs next): every built-in registry id is
//      exported for real into a QTemporaryDir. The ids that succeed are
//      the served set every later case is measured against; psd and a
//      bogus id must FAIL, which is what arms E3 rather than leaving it
//      vacuously true.
//   V  VACUITY GUARD (RULE 9): the loaded chain is non-empty, every
//      consumed id resolves in ALL FOUR checked-in locales without
//      key-echo, and each locale keeps its parenthesized glob verbatim
//      (Qt parses the glob out of the string — a locale that "translated"
//      *.png would ship a dead row).
//   E1 LIVE PROPERTY + CATCH-ALL GUARD: the REAL Main.qml booted
//      offscreen exposes a NON-EMPTY exportFileDialog.nameFilters AND not
//      one row is a catch-all. The catch-all half is the load-bearing
//      half: the import reviewer proved a naive non-empty assert PASSES
//      against Qt's substituted "All Files (*)", which arrives as a
//      size-1 NON-EMPTY row. The discriminator is therefore a NAMED
//      predicate (isCatchAllRow) carrying its own instrument check, not
//      an incidental consequence of the extension parse.
//   E2 CATALOG-RESOLVED, NOT LITERAL: the offered rows equal the
//      catalog's values, read back through the same catalog the app uses.
//      A key-echo REDs here — that half of the round-1 claim was always
//      true (`value != id` covers it). A hardcoded English literal that
//      merely LOOKS right REDs here TOO, but ONLY because the catalog the
//      app was booted on is `pseudo` (case D) and the rows are held
//      against the `en` values as well. Round 1 asserted that half
//      without having it; it is mutation-verified in the fix round.
//   E3 HONESTY: every extension the rows advertise is mapped to a format
//      id by the DIALOG'S OWN handler helper (exportFileDialog
//      .formatForSuffix — not a duplicated copy of the alias table) and
//      that format must be one E0 proved the encoder writes. A row for a
//      format the encoder refuses REDs here.
//   E4 DEFAULT SUFFIX: defaultSuffix must itself name a served format and
//      be one the rows advertise — a default the encoder cannot keep
//      would be the same dishonesty by another door.
//   E5 TYPED-ANSWER PRESERVED: formatForSuffix passes an unknown suffix
//      through unchanged, so the engine still answers typed
//      (error.codec.unrecognized) instead of the shell silently
//      substituting a format. The filter rows must never become the
//      authority over the user's typed name.
//   E6 PATTERN PARITY: Open/Save still consume their own cataloged filter
//      id the same way, so E2's precedent is this window's file-dialog
//      rule rather than a one-off — and their row is held to the same
//      differs-from-English test, so the literal defect cannot hide in
//      the two dialogs this slice did not touch.

#include "announcer.h"
#include "document_controller.h"
#include "i18n_catalog.h"
#include "units.h"
#include "icon_provider.h"
#include "layer_thumb_provider.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlExpression>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QtTest>

using namespace appname::shell;

namespace {

// The catalog this harness boots AppI18n on. NOT `en`: see the
// HARNESS-LOCALE paragraph in the header and case D. Changing this to
// "en" does not quietly weaken the suite — D, E2 and E6 all RED.
constexpr const char *kHarnessLang = "pseudo";

// The rows File→Export is required to offer, in §8.1 normative registry
// order minus the non-writing slots. Each id is written as a LITERAL so
// this file's assertions name it plainly and a grep for the id finds the
// pin.
// RECORD CORRECTION (fix round): the note that stood here said the
// literals let "the i18n audit see the emission". They do not.
// i18n/tools/audit.py derives its emitted set in emitted_ids() and skips
// every path containing "/tests/", so nothing in this file has ever been
// part of that set — the audit's view of these seven ids comes from
// shell/qml/Main.qml alone. (Which is also why the round-1 mutation left
// the audit green: it is the QML, not this file, that the audit reads.)
const QStringList kExpectedFilterIds = {
    QStringLiteral("ui.dialog.file.filter_png"),
    QStringLiteral("ui.dialog.file.filter_jpeg"),
    QStringLiteral("ui.dialog.file.filter_tiff"),
    QStringLiteral("ui.dialog.file.filter_webp"),
    QStringLiteral("ui.dialog.file.filter_gif"),
    QStringLiteral("ui.dialog.file.filter_bmp"),
    QStringLiteral("ui.dialog.file.filter_ora"),
};

// P1-1: globs live in CODE, not in the catalog. Main.qml's fileGlobs map is the
// production source; this mirrors it so the test pins the composition rather
// than re-deriving it from the row it is checking.
const QStringList kExpectedFilterGlobs = {
    QStringLiteral("*.png"),
    QStringLiteral("*.jpg *.jpeg"),
    QStringLiteral("*.tif *.tiff"),
    QStringLiteral("*.webp"),
    QStringLiteral("*.gif"),
    QStringLiteral("*.bmp"),
    QStringLiteral("*.ora"),
};

// Every built-in registry id (crates/appname-io/src/registry.rs
// CodecRegistry::builtin), plus one id that is in no registry at all.
// E0 asks the ENGINE which of these it will write; nothing here is a
// claim about the answer.
const QStringList kProbeFormats = {
    QStringLiteral("png"),  QStringLiteral("jpeg"), QStringLiteral("tiff"),
    QStringLiteral("webp"), QStringLiteral("gif"),  QStringLiteral("bmp"),
    QStringLiteral("psd"),  QStringLiteral("ora"),  QStringLiteral("tga"),
};

// Ids the encoder must NOT write, so E0's probe is proven discriminating
// rather than trivially all-green: psd is §8.5 import-only
// (CodecCaps::write false) and tga is in no registry.
const QStringList kMustNotServe = {QStringLiteral("psd"),
                                   QStringLiteral("tga")};

// Pull the advertised extensions out of a Qt name-filter row:
// "PNG image (*.png)" -> {"png"}. Empty when there is no parenthesized
// glob group at all (which the cases below treat as a failure).
QSet<QString> advertisedExtensions(const QString &filterRow)
{
    const int open = filterRow.lastIndexOf(QLatin1Char('('));
    const int close = filterRow.lastIndexOf(QLatin1Char(')'));
    if (open < 0 || close <= open)
        return {};
    const QString globs = filterRow.mid(open + 1, close - open - 1);
    QSet<QString> out;
    const QStringList parts =
        globs.split(QRegularExpression(QStringLiteral("[\\s;]+")),
                    Qt::SkipEmptyParts);
    for (const QString &raw : parts) {
        QString g = raw.trimmed();
        if (g.startsWith(QLatin1String("*.")))
            g = g.mid(2);
        else if (g.startsWith(QLatin1Char('.')))
            g = g.mid(1);
        if (!g.isEmpty())
            out.insert(g.toLower());
    }
    return out;
}

// A CATCH-ALL ROW, NAMED. An unbound Qt FileDialog does not report an
// empty nameFilters — it substitutes exactly one row, the untranslated
// English "All Files (*)" — and the round-1 reviewer MEASURED that it
// arrives as a size-1, NON-EMPTY list, so a naive non-empty assertion
// passes against it. This trap has been sprung twice in this dialog
// family and caught twice, so the discriminator gets a name and an
// instrument check of its own rather than riding along as a side effect
// of the extension parse. It rejects the SHAPE, not the provenance: a
// shell-bound ["All Files (*)"] is refused exactly like Qt's own.
bool isCatchAllRow(const QString &filterRow)
{
    return advertisedExtensions(filterRow).contains(QStringLiteral("*"))
           || filterRow.contains(QLatin1String("(*)"));
}

QString joined(const QSet<QString> &s)
{
    QStringList l(s.begin(), s.end());
    l.sort();
    return l.join(QStringLiteral(", "));
}

} // namespace

class TstExportFilter : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();

    void harnessLocaleIsNotEnglish();
    void servedSetProbe();
    void vacuityGuardAcrossLocales();
    void exportDialogExposesNonCatchAllNameFilters();
    void offeredRowsResolveThroughTheCatalog();
    void everyAdvertisedExtensionIsWritable();
    void defaultSuffixNamesAServedFormat();
    void unknownSuffixStillAnswersTyped();
    void openAndSaveKeepTheSameCatalogPattern();

private:
    CatalogQml *m_i18n = nullptr;
    // The ENGLISH GROUND, loaded explicitly and never wired into QML. The
    // booted catalog (m_i18n) is kHarnessLang; this is what the live rows
    // must NOT equal.
    Catalog m_en;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;
    QTemporaryDir m_tmp;
    // Filled by E0 from the REAL export door; read by E3/E4.
    QSet<QString> m_served;

    QVariant evalJs(const QString &code) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "evalJs failed:" << expr.error().toString();
        return undefined ? QVariant() : v;
    }
    // The dialog's live properties, read off the REAL booted object by its
    // Main.qml id (the tst_canvas_context_menu id-lookup pattern — these
    // FileDialogs carry no objectName).
    QStringList liveFilters(const char *dialogId) const
    {
        return evalJs(QStringLiteral("%1.nameFilters")
                          .arg(QLatin1String(dialogId)))
            .toStringList();
    }
    // The dialog's OWN suffix->format mapping, invoked live. Using the
    // production helper instead of a copy is the point: a divergence
    // between this suite's idea of the alias table and the handler's
    // cannot hide here.
    QString formatForSuffix(const QString &suffix) const
    {
        return evalJs(QStringLiteral("exportFileDialog.formatForSuffix(\"%1\")")
                          .arg(suffix))
            .toString();
    }
};

void TstExportFilter::initTestCase()
{
    QVERIFY2(m_tmp.isValid(), "could not create the probe temp dir");
    // Mirror shell/src/main.cpp's wiring (the tst_palette_pins harness),
    // with ONE deliberate divergence: the catalog wired into QML as
    // AppI18n is kHarnessLang, not `en`. That divergence IS the net — see
    // the HARNESS-LOCALE paragraph in the header and case D. The `en`
    // chain is loaded separately as the English ground the live rows are
    // held against; it is never given to the engine.
    m_en = Catalog::load(QStringLiteral("en"));
    m_i18n = new CatalogQml(Catalog::load(QLatin1String(kHarnessLang)));
    m_controller = new DocumentController(&m_i18n->catalog());
    m_announcer = new Announcer(&m_i18n->catalog());
    m_controller->setAnnouncer(m_announcer);

    m_engine = new QQmlApplicationEngine;
    m_engine->addImageProvider(QStringLiteral("lucide"),
                               new LucideIconProvider);
    m_engine->addImageProvider(QStringLiteral("layerthumb"),
                               new LayerThumbProvider(m_controller));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppI18n"),
                                                m_i18n);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppDoc"),
                                                m_controller);
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppA11y"),
                                                m_announcer);
    // AppUnits: main.cpp:132 registers it and this harness did not, so
    // NewDocumentDialog threw two ReferenceErrors and a TypeError on every
    // boot — this suite was exercising a tree the application never has.
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppUnits"),
                                                new UnitsQml(m_engine));
    m_engine->rootContext()->setContextProperty(QStringLiteral("AppKeys"),
                                                new QObject(m_engine));
    m_engine->loadFromModule("AppnameShell", "Main");
    QVERIFY2(!m_engine->rootObjects().isEmpty(), "Main.qml failed to load");
    m_window = qobject_cast<QQuickWindow *>(m_engine->rootObjects().first());
    QVERIFY(m_window);
    m_announcer->setSubject(m_window);
    QVERIFY(QTest::qWaitForWindowExposed(m_window));
}

// D — THE HARNESS-LOCALE GUARD. Round-1 MAJOR-1 reduced to one assert:
// if AppI18n is booted on `en`, an English literal frozen into Main.qml
// and a correct catalog lookup are THE SAME STRING, and no case in this
// file can tell them apart. Every row assert below is measured against
// the catalog this harness booted, so that catalog must be one whose
// values are not the English ones. If the boot locale is reverted, or the
// pseudo catalog fails to load and the chain falls back to en, this case
// REDs — which is the whole point of stating it separately instead of
// leaving it implicit in initTestCase.
void TstExportFilter::harnessLocaleIsNotEnglish()
{
    QVERIFY2(QLatin1String(kHarnessLang) != QLatin1String("en"),
             "the harness boot locale is `en` — a hardcoded English literal "
             "in Main.qml would be invisible to every case in this file "
             "(round-1 MAJOR-1)");
    QVERIFY2(m_i18n->catalog().hasEntries(),
             "the booted catalog chain is empty — key-echo would satisfy "
             "every text assert in this file");
    QVERIFY2(m_en.hasEntries(),
             "the en catalog chain is empty — the English ground this guard "
             "measures against is missing");

    QStringList guarded = kExpectedFilterIds;
    // Open/Save ride the same rule (E6), so their id is guarded here too.
    guarded << QStringLiteral("ui.dialog.file.filter_appname");
    for (const QString &id : guarded) {
        const QString booted = m_i18n->tr(id);
        const QString english = m_en.tr(id);
        QVERIFY2(booted != id,
                 qPrintable(QStringLiteral("%1 key-echoed in the booted "
                                           "catalog").arg(id)));
        QVERIFY2(english != id,
                 qPrintable(QStringLiteral("%1 key-echoed in en — the "
                                           "English ground is absent")
                                .arg(id)));
        QVERIFY2(booted != english,
                 qPrintable(QStringLiteral(
                                "the booted catalog (%1) returns the ENGLISH "
                                "value for %2 (%3): either the boot locale "
                                "was reverted to en or the %1 catalog did not "
                                "load. A hardcoded English literal in Main.qml "
                                "is invisible to this suite in that state "
                                "(round-1 MAJOR-1)")
                                .arg(QLatin1String(kHarnessLang), id,
                                     english)));
        // The honesty cases parse the glob out of the row, so the boot
        // locale has to carry it through.
        // P1-1 INVERTED THIS CONTRACT. It used to read "the glob must survive
        // translation"; globs are now CODE CONSTANTS and the catalog carries a
        // LABEL ONLY, because a lookup that key-echoes on a miss cannot be
        // trusted with a load-bearing match pattern. The honesty the old arm
        // protected is now asserted on the COMPOSED filter, not the catalog row.
        QVERIFY2(advertisedExtensions(booted).isEmpty(),
                 qPrintable(QStringLiteral("%1 %2 still carries a glob: %3")
                                .arg(QLatin1String(kHarnessLang), id, booted)));
    }
}

// E0 — ENGINE TRUTH, not the catalog's convenience and not a source
// comment: drive the real export door once per built-in registry id.
void TstExportFilter::servedSetProbe()
{
    QVERIFY2(m_controller->abiBlock12(),
             "ABI 1.2 export block absent — the probe below would report an "
             "empty served set for a shell reason, not an encoder one");
    QVERIFY2(m_controller->newDocument(24, 16, QStringLiteral("white")),
             "doc.new refused; the export probe needs an open document");

    for (const QString &format : kProbeFormats) {
        const QString path =
            QDir(m_tmp.path()).filePath(QStringLiteral("probe.%1").arg(format));
        const bool ok = m_controller->exportDocument(path, format, -1);
        if (ok) {
            // "Returned true" is not enough — bytes must exist on disk.
            QFileInfo fi(path);
            QVERIFY2(fi.exists() && fi.size() > 0,
                     qPrintable(QStringLiteral("export %1 reported ok but "
                                               "wrote no bytes")
                                    .arg(format)));
            m_served.insert(format);
        } else {
            qInfo("export door refused %s: %s", qPrintable(format),
                  qPrintable(m_controller->statusText()));
        }
    }
    qInfo("ENCODER SERVES: %s", qPrintable(joined(m_served)));

    // The probe must discriminate, or E3 is decoration.
    for (const QString &format : kMustNotServe) {
        QVERIFY2(!m_served.contains(format),
                 qPrintable(QStringLiteral("the export door WROTE %1 — the "
                                           "probe is not discriminating and "
                                           "every honesty assert below is "
                                           "vacuous")
                                .arg(format)));
    }
    QVERIFY2(m_served.size() >= 2,
             qPrintable(QStringLiteral("served set is %1 — implausibly small; "
                                       "the probe is measuring the harness, "
                                       "not the encoder")
                            .arg(joined(m_served))));
}

// V — the guard that arms every text assert in this file.
void TstExportFilter::vacuityGuardAcrossLocales()
{
    QVERIFY2(m_i18n->catalog().hasEntries(),
             "en catalog chain is empty — every text assert here would be "
             "comparing key-echo to key-echo");

    for (const char *lang : {"en", "es", "ja", "pseudo"}) {
        const Catalog cat = Catalog::load(QLatin1String(lang));
        QVERIFY2(cat.hasEntries(),
                 qPrintable(QStringLiteral("catalog chain empty for %1")
                                .arg(QLatin1String(lang))));
        for (const QString &id : kExpectedFilterIds) {
            QVERIFY2(cat.has(id),
                     qPrintable(QStringLiteral("%1 missing %2")
                                    .arg(QLatin1String(lang), id)));
            const QString value = cat.tr(id);
            QVERIFY2(!value.isEmpty(),
                     qPrintable(QStringLiteral("%1 %2 is empty")
                                    .arg(QLatin1String(lang), id)));
            QVERIFY2(value != id,
                     qPrintable(QStringLiteral("%1 key-echoes %2")
                                    .arg(QLatin1String(lang), id)));
            // P1-1: globs are code constants; the translation is a label.
            QVERIFY2(advertisedExtensions(value).isEmpty(),
                     qPrintable(QStringLiteral("%1 %2 still carries a glob: %3")
                                    .arg(QLatin1String(lang), id, value)));
        }
    }

    // Pin the checked-in en values (echo-proofing, the
    // tst_canvas_context_menu catalogGround pattern). Read from the
    // explicitly loaded `en` chain, NOT from the booted one: the harness
    // boots kHarnessLang (case D), so these literals are the ENGLISH
    // GROUND the live rows must DIFFER from — they are no longer what the
    // dialog is expected to show.
    // P1-1: these pins are LABELS now — the glob moved to Main.qml fileGlobs.
    QCOMPARE(m_en.tr(QStringLiteral("ui.dialog.file.filter_png")),
             QStringLiteral("PNG image"));
    QCOMPARE(m_en.tr(QStringLiteral("ui.dialog.file.filter_ora")),
             QStringLiteral("OpenRaster image"));
}

// E1 — the binding exists at runtime AND is the shell's own.
//
// MUTATION RECORD (why this case is shaped this way): on the import
// dialog a bare non-empty assert PASSED against the binding-removed
// mutant, because an unbound FileDialog does not report empty
// nameFilters — Qt substitutes the untranslated English literal
// "All Files (*)". The catch-all glob is therefore the discriminator: an
// explicit shell filter never offers `*`.
void TstExportFilter::exportDialogExposesNonCatchAllNameFilters()
{
    const QStringList filters = liveFilters("exportFileDialog");
    QVERIFY2(!filters.isEmpty(),
             "File→Export opens with no name filter — it advertises every "
             "path as a valid export target (the finding)");
    for (const QString &row : filters) {
        QVERIFY2(!row.trimmed().isEmpty(), "blank filter row offered");
        const QSet<QString> ext = advertisedExtensions(row);
        QVERIFY2(!ext.isEmpty(),
                 qPrintable(QStringLiteral("filter row advertises no glob: %1")
                                .arg(row)));
        QVERIFY2(!isCatchAllRow(row),
                 qPrintable(QStringLiteral(
                                "File→Export fell back to a catch-all filter "
                                "row (%1) — Qt's untranslated default, i.e. "
                                "no shell binding")
                                .arg(row)));
    }
    // Instrument check (RULE 9), on the NAMED predicate: it flags Qt's own
    // substituted row, so the guard above is armed rather than vacuously
    // true. Both shapes are pinned — the parsed `*` glob and the literal
    // "(*)" group — because the guard claims to reject either.
    QVERIFY2(isCatchAllRow(QStringLiteral("All Files (*)")),
             "vacuity: isCatchAllRow does not flag Qt's substituted default "
             "row — the catch-all guard above is decoration");
    QVERIFY2(isCatchAllRow(QStringLiteral("Anything (*.*)")),
             "vacuity: isCatchAllRow does not flag a *.* escape row");
    QVERIFY2(!isCatchAllRow(QStringLiteral("PNG image (*.png)")),
             "vacuity: isCatchAllRow flags an honest row — it would RED on "
             "a correct dialog");
}

// E2 — the rows come from the catalog, not from literals.
void TstExportFilter::offeredRowsResolveThroughTheCatalog()
{
    const QStringList filters = liveFilters("exportFileDialog");
    QStringList expected;
    QCOMPARE(kExpectedFilterGlobs.size(), kExpectedFilterIds.size());
    for (int i = 0; i < kExpectedFilterIds.size(); ++i) {
        const QString &id = kExpectedFilterIds.at(i);
        const QString value = m_i18n->tr(id);
        QVERIFY2(value != id,
                 qPrintable(QStringLiteral("vacuity: %1 key-echoed").arg(id)));
        // P1-1: label from the catalog, glob from code.
        QVERIFY2(advertisedExtensions(value).isEmpty(),
                 qPrintable(QStringLiteral("%1 still carries a glob").arg(id)));
        expected << value + QStringLiteral(" (") + kExpectedFilterGlobs.at(i)
                        + QStringLiteral(")");
    }
    QCOMPARE(filters, expected);
    // THE LITERAL HALF, asserted where the claim is made. The rows above
    // came back through the kHarnessLang catalog (case D), so a row that
    // still reads as its ENGLISH value is either a hardcoded literal in
    // Main.qml or a harness that stopped discriminating. This is the
    // assert the round-1 header claimed and did not have; the fix round
    // mutation-verified it by re-introducing exactly that defect.
    for (int i = 0; i < kExpectedFilterIds.size(); ++i) {
        const QString id = kExpectedFilterIds.at(i);
        const QString english = m_en.tr(id);
        QVERIFY2(filters.value(i) != english,
                 qPrintable(QStringLiteral(
                                "File→Export row %1 is the untranslated "
                                "ENGLISH value for %2 (%3) under a %4 boot "
                                "— a hardcoded literal in the QML, or a "
                                "harness no longer discriminating")
                                .arg(QString::number(i), id, english,
                                     QLatin1String(kHarnessLang))));
    }
    // The broad row is NOT what export offers: it names eight extensions
    // as one undifferentiated group, which a SaveFile dialog cannot turn
    // into a format, and MINTED.md assigns it to the import dialog.
    QVERIFY2(!filters.contains(
                 m_i18n->tr(QStringLiteral("ui.dialog.file.filter_images"))),
             "the broad image row is offered by the export dialog");
}

// E3 — THE FINDING: nothing is advertised that the encoder refuses.
void TstExportFilter::everyAdvertisedExtensionIsWritable()
{
    QVERIFY2(!m_served.isEmpty(), "E0 did not run / found nothing served");
    const QStringList filters = liveFilters("exportFileDialog");
    QVERIFY(!filters.isEmpty());

    QSet<QString> unwritable;
    QSet<QString> advertised;
    for (const QString &row : filters) {
        for (const QString &ext : advertisedExtensions(row)) {
            advertised.insert(ext);
            // Through the DIALOG'S OWN mapping, so the suffix a user
            // actually gets from picking this row is what is tested.
            const QString format = formatForSuffix(ext);
            QVERIFY2(!format.isEmpty(),
                     qPrintable(QStringLiteral("formatForSuffix(%1) returned "
                                               "nothing — the handler helper "
                                               "is missing or broken")
                                    .arg(ext)));
            if (!m_served.contains(format))
                unwritable.insert(QStringLiteral("%1->%2").arg(ext, format));
        }
    }
    QVERIFY2(unwritable.isEmpty(),
             qPrintable(QStringLiteral("File→Export advertises %1, which the "
                                       "encoder refuses to write")
                            .arg(joined(unwritable))));

    // And the honest set is actually offered: every served format must be
    // reachable through some advertised extension. A row set that quietly
    // shrank to one safe format would pass the assert above.
    for (const QString &format : m_served) {
        bool reachable = false;
        for (const QString &ext : advertised) {
            if (formatForSuffix(ext) == format) {
                reachable = true;
                break;
            }
        }
        QVERIFY2(reachable,
                 qPrintable(QStringLiteral("the encoder writes %1 but no "
                                           "offered row reaches it")
                                .arg(format)));
    }

    // Instrument check: a refused format IS detected by this machinery.
    QVERIFY2(!m_served.contains(formatForSuffix(QStringLiteral("psd"))),
             "vacuity: psd maps to a served format — the honesty assert "
             "above could not fail");
}

// E4 — the appended default must be a promise the encoder can keep.
void TstExportFilter::defaultSuffixNamesAServedFormat()
{
    const QString suffix =
        evalJs(QStringLiteral("exportFileDialog.defaultSuffix")).toString();
    QVERIFY2(!suffix.isEmpty(), "exportFileDialog lost its defaultSuffix");
    const QString format = formatForSuffix(suffix);
    QVERIFY2(m_served.contains(format),
             qPrintable(QStringLiteral("defaultSuffix %1 maps to %2, which the "
                                       "encoder does not write")
                            .arg(suffix, format)));
    // It must also be a suffix the dialog itself advertises, or the
    // default silently leaves the offered set.
    QSet<QString> advertised;
    for (const QString &row : liveFilters("exportFileDialog"))
        advertised.unite(advertisedExtensions(row));
    QVERIFY2(advertised.contains(suffix.toLower()),
             qPrintable(QStringLiteral("defaultSuffix %1 is not in the offered "
                                       "rows (%2)")
                            .arg(suffix, joined(advertised))));
}

// E5 — the typed name stays authoritative and unknown suffixes still
// reach the engine unaltered, so the §8.1 typed answer is preserved
// rather than papered over by the new rows.
void TstExportFilter::unknownSuffixStillAnswersTyped()
{
    QCOMPARE(formatForSuffix(QStringLiteral("xyzzy")),
             QStringLiteral("xyzzy"));
    // Case folding and the recorded aliases are the handler's, unchanged.
    QCOMPARE(formatForSuffix(QStringLiteral("PNG")), QStringLiteral("png"));
    QCOMPARE(formatForSuffix(QStringLiteral("jpg")), QStringLiteral("jpeg"));
    QCOMPARE(formatForSuffix(QStringLiteral("jpe")), QStringLiteral("jpeg"));
    QCOMPARE(formatForSuffix(QStringLiteral("tif")), QStringLiteral("tiff"));
    // And the engine really does answer typed for that unknown suffix —
    // the behaviour the rows must not have replaced.
    const QString path =
        QDir(m_tmp.path()).filePath(QStringLiteral("typed-answer.xyzzy"));
    QVERIFY2(!m_controller->exportDocument(
                 path, formatForSuffix(QStringLiteral("xyzzy")), -1),
             "an unknown suffix was accepted by the export door");
    QVERIFY2(!QFileInfo::exists(path),
             "a refused export still wrote a file");
}

// E6 — the pattern E2 relies on is this window's file-dialog rule.
void TstExportFilter::openAndSaveKeepTheSameCatalogPattern()
{
    const QString appnameRow =
        m_i18n->tr(QStringLiteral("ui.dialog.file.filter_appname"));
    QVERIFY2(appnameRow != QStringLiteral("ui.dialog.file.filter_appname"),
             "vacuity: filter_appname key-echoed");
    QVERIFY2(appnameRow
                 != m_en.tr(QStringLiteral("ui.dialog.file.filter_appname")),
             "the booted catalog returns the ENGLISH filter_appname value — "
             "see case D; the comparison below could not tell a hardcoded "
             "literal from a lookup");
    // P1-1: the LABEL is translated and the GLOB is a code constant, so the
    // catalog row must NOT carry a pattern. This arm is the regression guard
    // for the fix class itself — put a glob back into the catalog and it fires.
    QVERIFY2(!appnameRow.contains(QLatin1Char('*')),
             "P1-1: a translated label carries a file glob — the pattern must "
             "be a code constant, because a catalog miss key-echoes and a "
             "key-echo carrying no glob matches NOTHING (all files grey out)");
    const QString appnameRowComposed =
        appnameRow + QStringLiteral(" (*.appname)");
    for (const char *dialogId : {"openFileDialog", "saveFileDialog"}) {
        const QStringList filters = liveFilters(dialogId);
        QVERIFY2(!filters.isEmpty(),
                 qPrintable(QStringLiteral("%1 lost its name filter")
                                .arg(QLatin1String(dialogId))));
        // The package row is offered by both, in the catalog pattern (RC3
        // D-4: Open's FIRST row now covers packages AND images -- pinned in
        // tst_import_filter -- so only Save's first row is the package row).
        QVERIFY2(filters.contains(appnameRowComposed),
                 qPrintable(QStringLiteral("%1 lost the package row")
                                .arg(QLatin1String(dialogId))));
        // The glob survives independently of the lookup: this is the property
        // the fix buys, and it is asserted rather than assumed.
        QVERIFY2(filters.first().contains(QStringLiteral("*.appname")),
                 qPrintable(QStringLiteral("%1 offers no glob")
                                .arg(QLatin1String(dialogId))));
    }
    QCOMPARE(liveFilters("saveFileDialog").first(), appnameRowComposed);
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstExportFilter tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_export_filter.moc"
