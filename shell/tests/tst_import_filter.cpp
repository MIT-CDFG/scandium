// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// THE IMPORT-FILTER HONESTY NET (O-UI lane/ui-importfilter — the
// operator finding "any files that aren't PNGs can't be imported").
// File→Import used to open with NO name filter, inviting the user to
// pick files the engine then refuses. The dialog now offers a filter,
// and this suite pins the property that makes the filter WORTH having:
// it must MIRROR the ingress's accepted set, resolved THROUGH the
// catalog.
//
// The accepted set is not a shell opinion — it is a two-door engine
// fact (DocumentController::importImage): appname_asset_create sniffs
// `appname.samples.v1` first and otherwise takes the raster arm (PNG at
// CP-0.5, decode-validated: Engine::add_raster_asset ->
// decode_png_to_buffer); `edit.paste` then requires an ingested RASTER
// asset and refuses a samples asset by name (kinds.rs
// resolve_raster_asset). Reachable through THIS dialog: PNG alone.
//
// ⛔ THE TWO SENTENCES ABOVE ARE A DATED RECORD OF CP-0.5 AND THE RASTER
// ARM HAS SINCE WIDENED. Corrected 2026-09-03; the structure they describe
// (samples arm first, raster arm second) is unchanged and still correct.
// What changed is the raster arm: appname-cabi/src/doc.rs:438 falls back to
// `appname_io::decode_for_ingress`, and appname-io/src/ingress.rs:60 sets
// INGRESS_ALLOWED = [png, jpeg, tiff, webp, gif, bmp]. So "PNG alone" is
// false, and `decode_png_to_buffer` is no longer the door this dialog
// reaches. Operator ruling D5 wired it.
// ★★ THIS HEADER CONTRADICTED THIS SUITE'S OWN PASSING TEST, WHICH IS WHY IT
// IS WORTH CORRECTING RATHER THAN LEAVING AS HISTORY:
// everyAdvertisedExtensionIsIngestible asserts `advertised == accepted` and
// runs GREEN over the six-format set (run direct 2026-09-03: 9 passed, 0
// failed). A file's prose and its own assertion disagreeing is the shape a
// reader resolves in favour of the prose, because the prose is what they read.
//
// RECORD CORRECTION, MADE FROM ANOTHER LANE (O-UI lane/ui-exportfilter
// fix round). This suite is ALREADY MERGED on main. Its round-1 sibling
// tst_export_filter.cpp copied its structure faithfully, and an
// adversarial review of that copy found the structure has a hole plus a
// false claim about the hole — filed as MAJOR-28 against main, because
// this file carries the identical pair. Nothing about the shipped Import
// dialog changes: production reads the catalog and that was re-confirmed
// first-hand. What changes is that this net can now SEE the defect it
// says it catches, and the header says only what is true.
//
// THE HARNESS BOOTS A NON-`en` CATALOG, AND THAT IS LOAD-BEARING.
// With the harness booted on `en`, replacing the AppI18n.tr(...) lookup
// in Main.qml with the hardcoded English literal "PNG image (*.png)" —
// the exact untranslated-in-every-locale defect this suite exists to
// prevent — left the suite GREEN, because every text assert compared the
// live row against the SAME `en` value the literal was copied from.
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
// the parenthesized glob verbatim, which F3 parses. Case D is the
// standing guard, and F2/F4 additionally assert the live row DIFFERS from
// the `en` value — so reverting the boot to `en`, or losing the pseudo
// catalog, REDs instead of going quietly vacuous again.
//
// Cases:
//   D  HARNESS-LOCALE GUARD (runs first): the catalog this harness wires
//      into QML as AppI18n is not `en` — every consumed id resolves to
//      something other than its English value and still carries its glob.
//      Without this, one edit restores the old vacuity in silence.
//   V  VACUITY GUARD (RULE 9): the loaded chain is
//      non-empty, the consumed id resolves in ALL FOUR checked-in
//      locales without key-echo, and each locale keeps the parenthesized
//      glob verbatim (Qt parses the glob out of the filter string — a
//      locale that "translated" *.png would ship a dead filter). Without
//      this, every assert below could pass against an empty catalog that
//      echoes ids back.
//   F1 LIVE PROPERTY + CATCH-ALL GUARD: the REAL Main.qml booted
//      offscreen (the tst_palette_pins / item-7 harness) exposes a
//      NON-EMPTY importFileDialog.nameFilters AND not one row is a
//      catch-all. The catch-all half is the load-bearing half — Qt's
//      substituted "All Files (*)" arrives as a size-1 NON-EMPTY row, so
//      a naive non-empty assert PASSES against the unbound mutant. The
//      discriminator is therefore a NAMED predicate (isCatchAllRow) with
//      its own instrument check, not an incidental consequence of the
//      extension parse.
//   F2 CATALOG-RESOLVED, NOT LITERAL: the offered row equals the
//      catalog's value for `ui.dialog.file.filter_png` read back through
//      the same catalog the app uses. A key-echo REDs here — that half of
//      the original claim was always true (`value != id` covers it). A
//      hardcoded English literal that merely LOOKS right REDs here TOO,
//      but ONLY because the catalog the app was booted on is `pseudo`
//      (case D) and the row is held against the `en` value as well. The
//      original header asserted that half without having it; it is
//      mutation-verified in this fix round.
//   F3 HONESTY: every extension the filter advertises is one the
//      ingress accepts. The extension set is parsed mechanically OUT OF
//      the offered row (not a duplicated list), so swapping in the broad
//      `ui.dialog.file.filter_images` row — jpg/tiff/webp/gif/bmp, none
//      of which the CP-0.5 ingress takes — REDs here. This is the
//      finding itself, armed against regression.
//   F4 PATTERN PARITY: Open/Save still consume their own cataloged
//      filter id the same way, so F2's precedent is the file-dialog rule
//      in this window rather than a one-off — and their row is held to
//      the same differs-from-English test, so the literal defect cannot
//      hide in the two dialogs this suite does not otherwise cover.

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
#include <QtTest>
#include <QUrl>
#include <QImageReader>
#include <QTemporaryDir>

using namespace appname::shell;

namespace {

// The catalog this harness boots AppI18n on. NOT `en`: see the
// HARNESS-LOCALE paragraph in the header and case D. Changing this to
// "en" does not quietly weaken the suite — D, F2 and F4 all RED.
constexpr const char *kHarnessLang = "pseudo";

// The id the dialog is required to consume, and the id it must NOT
// (the broad row: honest only once the tiered import door lands).
constexpr const char *kPngFilterId = "ui.dialog.file.filter_png";
constexpr const char *kImagesFilterId = "ui.dialog.file.filter_images";

// Extensions the import path actually ingests. Sourced from the ENGINE,
// NOT from the catalog and NOT from the dialog this test is checking.
//
// SOURCE OF TRUTH: `INGRESS_ALLOWED` at crates/appname-io/src/ingress.rs:60
//   = ["png", "jpeg", "tiff", "webp", "gif", "bmp"]   -- six FORMAT ids.
// The eight EXTENSIONS below are those six plus the two alternate spellings
// the dialog advertises (`jpg` -> jpeg, `tif` -> tiff).
//
// ★ WHY THIS IS A HAND-KEPT LIST AND NOT A DERIVED ONE. `INGRESS_ALLOWED` is
// a Rust const with NO C-ABI door, so a C++ test cannot read it at runtime.
// There IS one derivable channel -- the engine names the set in its own
// refusal text ("no codec recognized these bytes (accepted: png, jpeg, ...)")
// -- and parsing that would make this list self-updating. Filed rather than
// built: it is a real improvement and it is not what retiring the pin needed.
//
// ★★ UNTIL THEN, THE DRIFT IS ONE GREP AWAY, WHICH IS THE WHOLE POINT OF
// NAMING THE CONSTANT ABOVE. This list was `{"png"}` from CP-0.5 until
// 2026-08-30 -- correct when written, and left standing for days after
// operator ruling D5 wired the decode path. It was the pin, not this list,
// that carried the staleness.
//
// MEASURED 2026-08-30 through the release engine, one file per extension,
// every one produced by `appname export` from the demo document:
//   png jpg jpeg tif tiff webp gif bmp  -> ALL EIGHT "Imported layer"
//   NEGATIVE CONTROL (fires): 4096 random bytes named `.png` ->
//     error.command.bad_target: not an ingestible raster image
//   ★ the FIRST control did not fire -- a PNG renamed `.zzq` imported fine,
//     because the ingress sniffs CONTENT, not extension. It tested nothing
//     and was replaced. A control that cannot fire is not evidence.
QSet<QString> acceptedExtensions()
{
    return {QStringLiteral("png"),  QStringLiteral("jpg"),
            QStringLiteral("jpeg"), QStringLiteral("tif"),
            QStringLiteral("tiff"), QStringLiteral("webp"),
            QStringLiteral("gif"),  QStringLiteral("bmp")};
}

// Pull the advertised extensions out of a Qt name-filter row:
// "PNG image (*.png)" -> {"png"}. Returns empty when there is no
// parenthesized glob group at all (which F3 treats as a failure).
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
// English "All Files (*)" — and that row is size-1 and NON-EMPTY, so a
// naive non-empty assertion passes against it (measured on THIS dialog's
// binding-removed mutant, then again on the export dialog's). This trap
// has been sprung twice in this dialog family and caught twice, so the
// discriminator gets a name and an instrument check of its own rather
// than riding along as a side effect of the extension parse. It rejects
// the SHAPE, not the provenance: a shell-bound ["All Files (*)"] is
// refused exactly like Qt's own.
bool isCatchAllRow(const QString &filterRow)
{
    return advertisedExtensions(filterRow).contains(QStringLiteral("*"))
           || filterRow.contains(QLatin1String("(*)"));
}

} // namespace

class TstImportFilter : public QObject {
    Q_OBJECT
private slots:
    void initTestCase();

    void harnessLocaleIsNotEnglish();
    void vacuityGuardAcrossLocales();
    void importDialogExposesNonEmptyNameFilters();
    void offeredRowResolvesThroughTheCatalog();
    void dropRefusalIdResolvesThroughTheCatalog();
    void everyAdvertisedExtensionIsIngestible();
    void openAndSaveKeepTheSameCatalogPattern();
    void openAcceptsImagesAsANewDocumentOfTheirSize();

private:
    CatalogQml *m_i18n = nullptr;
    // The ENGLISH GROUND, loaded explicitly and never wired into QML. The
    // booted catalog (m_i18n) is kHarnessLang; this is what the live row
    // must NOT equal.
    Catalog m_en;
    DocumentController *m_controller = nullptr;
    Announcer *m_announcer = nullptr;
    QQmlApplicationEngine *m_engine = nullptr;
    QQuickWindow *m_window = nullptr;

    QVariant evalJs(const QString &code) const
    {
        QQmlExpression expr(qmlContext(m_window), m_window, code);
        bool undefined = false;
        const QVariant v = expr.evaluate(&undefined);
        if (expr.hasError())
            qWarning() << "evalJs failed:" << expr.error().toString();
        return undefined ? QVariant() : v;
    }
    // The dialog's live nameFilters, read off the REAL booted object by
    // its Main.qml id (the tst_canvas_context_menu id-lookup pattern —
    // these FileDialogs carry no objectName).
    QStringList liveFilters(const char *dialogId) const
    {
        return evalJs(QStringLiteral("%1.nameFilters")
                          .arg(QLatin1String(dialogId)))
            .toStringList();
    }
};

void TstImportFilter::initTestCase()
{
    // Mirror shell/src/main.cpp's wiring (the tst_palette_pins harness),
    // with ONE deliberate divergence: the catalog wired into QML as
    // AppI18n is kHarnessLang, not `en`. That divergence IS the net — see
    // the HARNESS-LOCALE paragraph in the header and case D. The `en`
    // chain is loaded separately as the English ground the live row is
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

// D — THE HARNESS-LOCALE GUARD. The inherited hole reduced to one
// assert: if AppI18n is booted on `en`, an English literal frozen into
// Main.qml and a correct catalog lookup are THE SAME STRING, and no case
// in this file can tell them apart. Every row assert below is measured
// against the catalog this harness booted, so that catalog must be one
// whose values are not the English ones. If the boot locale is reverted,
// or the pseudo catalog fails to load and the chain falls back to en,
// this case REDs — which is the whole point of stating it separately
// instead of leaving it implicit in initTestCase.
void TstImportFilter::harnessLocaleIsNotEnglish()
{
    QVERIFY2(QLatin1String(kHarnessLang) != QLatin1String("en"),
             "the harness boot locale is `en` — a hardcoded English literal "
             "in Main.qml would be invisible to every case in this file");
    QVERIFY2(m_i18n->catalog().hasEntries(),
             "the booted catalog chain is empty — key-echo would satisfy "
             "every text assert in this file");
    QVERIFY2(m_en.hasEntries(),
             "the en catalog chain is empty — the English ground this guard "
             "measures against is missing");

    // filter_appname rides the same rule (F4), so its id is guarded here.
    const QStringList guarded = {QLatin1String(kPngFilterId),
                                 QStringLiteral("ui.dialog.file.filter_appname")};
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
                                "is invisible to this suite in that state")
                                .arg(QLatin1String(kHarnessLang), id,
                                     english)));
        // F3 parses the glob out of the row, so the boot locale has to
        // carry it through.
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

// V — the guard that arms every other case in this file.
void TstImportFilter::vacuityGuardAcrossLocales()
{
    const QString id = QLatin1String(kPngFilterId);

    // The booted app's own catalog is not empty (an empty chain
    // key-echoes and would make F2 self-satisfying).
    QVERIFY2(m_i18n->catalog().hasEntries(),
             "en catalog chain is empty — every text assert here would be "
             "comparing key-echo to key-echo");

    for (const char *lang : {"en", "es", "ja", "pseudo"}) {
        const Catalog cat = Catalog::load(QLatin1String(lang));
        QVERIFY2(cat.hasEntries(),
                 qPrintable(QStringLiteral("catalog chain empty for %1")
                                .arg(QLatin1String(lang))));
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
        // P1-1: the glob is a code constant, so the TRANSLATION must not carry
        // one. The extensions themselves are asserted against the ingress on
        // the composed filter row (everyAdvertisedExtensionIsIngestible).
        QVERIFY2(advertisedExtensions(value).isEmpty(),
                 qPrintable(QStringLiteral("%1 %2 still carries a glob: %3")
                                .arg(QLatin1String(lang), id, value)));
    }

    // Pin the checked-in en value (echo-proofing, the
    // tst_canvas_context_menu catalogGround pattern). Read from the
    // explicitly loaded `en` chain, NOT from the booted one: the harness
    // boots kHarnessLang (case D), so this literal is the ENGLISH GROUND
    // the live row must DIFFER from — it is no longer what the dialog is
    // expected to show.
    // P1-1: a LABEL now — the glob moved to Main.qml fileGlobs.
    QCOMPARE(m_en.tr(id), QStringLiteral("PNG image"));
}

// F1 — the binding exists at runtime AND is the shell's own.
//
// MUTATION RECORD (why this case is shaped this way): a bare non-empty
// assert here PASSED against the binding-removed mutant. An unbound
// FileDialog does not report an empty nameFilters — Qt substitutes its
// own default row, the untranslated English literal "All Files (*)".
// So the pre-fix dialog was not merely silent about types; it actively
// advertised EVERY file as importable, in one locale only. The catch-all
// glob is therefore the discriminator: an explicit shell filter never
// offers `*`.
void TstImportFilter::importDialogExposesNonEmptyNameFilters()
{
    const QStringList filters = liveFilters("importFileDialog");
    QVERIFY2(!filters.isEmpty(),
             "File→Import opens with no name filter — it invites the user "
             "to pick files the ingress refuses (the finding)");
    for (const QString &row : filters) {
        QVERIFY2(!row.trimmed().isEmpty(), "blank filter row offered");
        QVERIFY2(!advertisedExtensions(row).isEmpty(),
                 qPrintable(QStringLiteral("filter row advertises no glob: %1")
                                .arg(row)));
        QVERIFY2(!isCatchAllRow(row),
                 qPrintable(
                     QStringLiteral("File→Import fell back to a catch-all "
                                    "filter row (%1) — Qt's untranslated "
                                    "default, i.e. no shell binding")
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

// F2 — the row comes from the catalog, not from a literal.
void TstImportFilter::offeredRowResolvesThroughTheCatalog()
{
    // RETARGETED 2026-08-26 (operator ruling 4 on the parked set): the dialog
    // this case reads now offers ui.dialog.file.filter_images, not filter_png
    // (c1fd9b2a). The case was asserting the OLD row through the catalog and
    // reddened on the id, not on the invariant -- the invariant it exists for,
    // "the offered row came back THROUGH the catalog and is not a hardcoded
    // English literal", is unchanged and still asserted, now against the id
    // the dialog actually offers.
    const QStringList filters = liveFilters("importFileDialog");
    QVERIFY(!filters.isEmpty());
    const QString expectedLabel = m_i18n->tr(QLatin1String(kImagesFilterId));
    QVERIFY2(expectedLabel != QLatin1String(kImagesFilterId),
             "vacuity: the id key-echoed (see vacuityGuardAcrossLocales)");
    // P1-1: label from the catalog, glob from code. The catalog row must not
    // carry the pattern -- see openAndSaveKeepTheSameCatalogPattern.
    QVERIFY2(!expectedLabel.contains(QLatin1Char('*')),
             "P1-1: the images label carries a file glob");
    const QString expected =
        expectedLabel
        + QStringLiteral(" (*.png *.jpg *.jpeg *.tif *.tiff *.webp *.gif *.bmp)");
    QCOMPARE(filters.first(), expected);
    // THE LITERAL HALF, asserted where the claim is made. The row above
    // came back through the kHarnessLang catalog (case D), so a row that
    // still reads as its ENGLISH value is either a hardcoded literal in
    // Main.qml or a harness that stopped discriminating. This is the
    // assert the original header claimed and did not have; the fix round
    // mutation-verified it by re-introducing exactly that defect.
    QVERIFY2(filters.first() != m_en.tr(QLatin1String(kImagesFilterId)),
             qPrintable(QStringLiteral(
                            "File→Import offers the untranslated ENGLISH "
                            "value for %1 (%2) under a %3 boot — a hardcoded "
                            "literal in the QML, or a harness no longer "
                            "discriminating")
                            .arg(QLatin1String(kImagesFilterId),
                                 m_en.tr(QLatin1String(kImagesFilterId)),
                                 QLatin1String(kHarnessLang))));
    // A localized run must move this text; nothing may be frozen English
    // in the QML. Proven by construction above (the value is READ from
    // the catalog), re-armed by the English comparison just made, and
    // re-armed here against the broad row.
    // NOT retargeted: this assertion is about the row that must NOT be
    // offered. It stays on kPngFilterId deliberately — retargeting it made it
    // contradict the QCOMPARE above (offered == images AND offered != images)
    // and the case reddened on my own blanket replace, not on the code.
    QVERIFY2(filters.first() != m_i18n->tr(QLatin1String(kPngFilterId)),
             "the broad image row is offered — see F3");
}

// The drop surface answers a documentless drop with ui.error.no_document via
// AppDoc.statusPending (Main.qml DropArea). statusPending is CATALOG-GATED: an
// uncataloged id is REFUSED and the status line is left untouched -- which is
// the right policy and also means a rename of this id turns the message back
// OFF SILENTLY, restoring the exact D4 defect the change removed. Nothing else
// in the suite would notice, because the drop path has no functional test:
// QTest cannot synthesize a DropArea drop. This is the guard that can be had.
//
// ⛔ CORRECTED 2026-09-03 BY O-INT #8, AND THE CONCLUSION SURVIVES WHILE THE
// REASON DOES NOT. "QTest cannot synthesize a DropArea drop" packs two claims:
//   (a) QTest has no drop helper             -- TRUE. There is no QTest::drop().
//   (b) so a harness cannot drive the drop   -- FALSE, and it does not follow
//                                               from (a).
// Delivering a QDragEnterEvent, a QDragMoveEvent and a QDropEvent to the
// window is plain event delivery, not a QTest gesture. Measured:
// tst_gesture_chrome::dropGlueFitsOnOverflow gets
// `enterAccepted=true dropAccepted=true layers 1->2 zoom 400->132`, and it is
// mutation-verified (remove the handler's fitIfOverflow and it reds at the
// zoom arm while the import still lands). The DragEnter is load-bearing:
// QQuickDropArea tracks containsDrag, so a bare QDropEvent is ignored.
//
// ★★ THE GUARD BELOW IS STILL LOAD-BEARING AND SHOULD NOT BE RETIRED ON THE
// STRENGTH OF THAT TEST. The functional case drops with a document OPEN --
// tst_gesture_chrome's init() creates one and asserts it -- so it never takes
// the `!AppDoc.hasDocument || !AppDoc.abiBlock12` branch this case guards.
// The documentless drop, and therefore the catalog-gating of
// ui.error.no_document, remains covered ONLY here.
// ★ So the sentence above was right about what to build and wrong about why,
// and the correction narrows the gap rather than closing it: what is now
// covered is the HAPPY branch; what is still uncovered is the REFUSAL branch,
// and no functional test drives it yet.
void TstImportFilter::dropRefusalIdResolvesThroughTheCatalog()
{
    const QString id = QStringLiteral("ui.error.no_document");
    const QString shown = m_i18n->tr(id);
    QVERIFY2(shown != id,
             qPrintable(QStringLiteral("%1 KEY-ECHOED: statusPending would refuse "
                                       "it and the documentless drop goes silent "
                                       "again, with no other test to catch it")
                            .arg(id)));
    QVERIFY2(!shown.isEmpty(), "the drop-refusal id resolves to an empty string");
    // POSITIVE CONTROL: a deliberately absent id MUST key-echo, or this probe
    // cannot discriminate and the assertion above is decoration.
    const QString bogus = QStringLiteral("ui.error.no_document.NOT_A_REAL_ID");
    QCOMPARE(m_i18n->tr(bogus), bogus);
}

// F3 — THE FINDING: nothing is advertised that the ingress refuses.
void TstImportFilter::everyAdvertisedExtensionIsIngestible()
{
    const QStringList filters = liveFilters("importFileDialog");
    QVERIFY(!filters.isEmpty());

    QSet<QString> advertised;
    for (const QString &row : filters) {
        const QSet<QString> ext = advertisedExtensions(row);
        QVERIFY2(!ext.isEmpty(),
                 qPrintable(QStringLiteral("filter row advertises no glob: %1")
                                .arg(row)));
        advertised.unite(ext);
    }

    const QSet<QString> accepted = acceptedExtensions();
    QSet<QString> refused = advertised;
    refused.subtract(accepted);
    QVERIFY2(refused.isEmpty(),
             qPrintable(
                 QStringLiteral("File→Import advertises %1, which the CP-0.5 "
                                "ingress refuses (error.command.bad_target)")
                     .arg(QStringList(refused.begin(), refused.end())
                              .join(QStringLiteral(", ")))));
    // And the honest set is actually offered — an empty-but-harmless
    // filter must not pass as "nothing refused".
    QCOMPARE(advertised, accepted);

    // Instrument check (RULE 9): the same parse must still DISCRIMINATE, so
    // the assert above is armed rather than vacuously true.
    //
    // ★★★★ RE-ANCHORED 2026-08-30, AND THE OLD ANCHOR IS THE LESSON. This
    // guard used to take the BROAD `images` row, subtract `accepted`, and
    // require a non-empty remainder -- i.e. it proved the parse discriminates
    // BY RELYING ON SOME ADVERTISED EXTENSION BEING REFUSED. That was true
    // while the ingress took `png` alone. Operator ruling D5 wired the decode
    // path, every advertised extension now ingests, and the remainder went
    // EMPTY -- so a control built around the defect FAILED THE MOMENT THE
    // DEFECT WAS FIXED, turning a correct product into a red test.
    // ★★ The replacement anchors on something the corpus CANNOT make true: a
    // synthetic row carrying an extension no codec will ever claim. The parse
    // is proved discriminating without borrowing a real refusal.
    QSet<QString> impossible =
        advertisedExtensions(QStringLiteral("Synthetic (*.zzq)"));
    QVERIFY2(impossible.contains(QStringLiteral("zzq")),
             "vacuity: the extension parse did not read a known-good glob");
    impossible.subtract(accepted);
    QVERIFY2(!impossible.isEmpty(),
             "vacuity: an extension no codec accepts survived the subtract — "
             "the extension parse is not discriminating");
    // ★★★★★ AND THE ARM THAT USED TO SIT HERE IS DELETED, NOT REPAIRED —
    // BECAUSE IT HAD NEVER ONCE RUN. It read the broad row out of the CATALOG
    // (`m_i18n->tr(kImagesFilterId)`) and required a non-empty parse. That
    // catalog value is a LABEL: P1-1 moved the globs to Main.qml `fileGlobs`,
    // so it parses to {} and the arm CANNOT PASS.
    // ★★★ It never failed because it was never reached: the preserved
    // pre-change LastTest.log shows this case dying earlier every time, at
    // `refused.isEmpty()` (:495) — "File→Import advertises tiff, tif, bmp,
    // gif, webp, jpeg, jpg, which the CP-0.5 ingress refuses". FIXING THE
    // FIRST DEFECT IS WHAT EXECUTED THE SECOND ONE FOR THE FIRST TIME.
    // ★★ A guard that has never run is indistinguishable from one that works,
    // and a dead arm behind a live failure is invisible for exactly as long as
    // the failure lasts. Deleted rather than re-pointed at the live row: the
    // loop above already asserts every live row parses non-empty (:481), so
    // re-aiming it would only duplicate an assert that is already there.
}

// F4 — the pattern F2 relies on is the window's file-dialog rule.
void TstImportFilter::openAndSaveKeepTheSameCatalogPattern()
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
        // The glob survives independently of the lookup: this is the property
        // the fix buys, and it is asserted rather than assumed.
        QVERIFY2(filters.first().contains(QStringLiteral("*.appname")),
                 qPrintable(QStringLiteral("%1 offers no glob")
                                .arg(QLatin1String(dialogId))));
        // The package-only row is offered by both, in the catalog pattern.
        QVERIFY2(filters.contains(appnameRowComposed),
                 qPrintable(QStringLiteral("%1 lost the package row")
                                .arg(QLatin1String(dialogId))));
    }
    // Save writes packages only, so its first row is still the package row.
    QCOMPARE(liveFilters("saveFileDialog").first(), appnameRowComposed);
    // RC3 D-4: Open's FIRST row covers packages AND images (the macOS panel
    // greys out whatever the active filter does not match), labelled from the
    // two whole-literal catalog strings and carrying every advertised glob.
    const QString imagesRow =
        m_i18n->tr(QStringLiteral("ui.dialog.file.filter_images"));
    const QString openFirst = liveFilters("openFileDialog").first();
    QVERIFY2(openFirst.startsWith(appnameRow + QStringLiteral(", ") + imagesRow
                                  + QStringLiteral(" (")),
             qPrintable(QStringLiteral("Open's first row is not the combined "
                                       "package + images row: ") + openFirst));
    for (const char *glob : {"*.appname", "*.png", "*.jpg", "*.jpeg", "*.tif",
                             "*.tiff", "*.webp", "*.gif", "*.bmp"})
        QVERIFY2(openFirst.contains(QLatin1String(glob)),
                 qPrintable(QStringLiteral("Open's first row lacks %1: %2")
                                .arg(QLatin1String(glob), openFirst)));
    QVERIFY2(!openFirst.contains(QStringLiteral("*.ora")),
             ".ora is out of D-4's scope and must not be offered by Open");
}

// RC3 D-4 (operator WORD 35, sitting S-04): File > Open accepted ONLY the
// package -- every image (png/jpg/tiff/webp/bmp/gif) was greyed out in the
// panel. Open now offers images (first filter covers both, see the case
// above) and an accepted image becomes a NEW document of the image's pixel
// size with the image imported into it through the existing Import door.
// Driven through the REAL openFileDialog.onAccepted, per format. .ora stays
// out of scope and is refused, opening nothing.
void TstImportFilter::openAcceptsImagesAsANewDocumentOfTheirSize()
{
    QTemporaryDir tmp;
    QVERIFY(tmp.isValid());
    const QSize size(37, 23);
    QImage src(size, QImage::Format_ARGB32);
    src.fill(Qt::transparent);
    for (int y = 4; y < 19; ++y)
        for (int x = 6; x < 31; ++x)
            src.setPixelColor(x, y, QColor(200, 40, 40));
    QStringList files;
    for (const char *ext : {"png", "jpg", "tif", "webp", "bmp"}) {
        const QString f = tmp.filePath(QStringLiteral("d4.") + QLatin1String(ext));
        const QImage img = (QLatin1String(ext) == QLatin1String("jpg")
                            || QLatin1String(ext) == QLatin1String("bmp"))
                               ? src.convertToFormat(QImage::Format_RGB32)
                               : src;
        QVERIFY2(img.save(f), qPrintable(QStringLiteral("could not write ") + f));
        files << f;
    }
    // Qt has no GIF writer: a literal 37x23 GIF89a (generated once with PIL:
    // palette [black, (200,40,40)], index 0 transparent, the same red block).
    {
        QByteArray gif = QByteArray::fromHex(
            "47494638396125001700810000000000c8282800000000000021f90401000000"
            "002c00000000250017004008590001081c48b0a0c183080f0658c8b0a1c3870d"
            "130e8448b1620089022d6a8c8871a3478c20438a1c49b2a0c78d204f6a4ca9b2"
            "22cb96104bca9c49b3a64d833029beccc95122cf873b7f5eec289461d09f4779"
            "de5cca146140003b");
        const QString f = tmp.filePath(QStringLiteral("d4.gif"));
        QFile g(f);
        QVERIFY(g.open(QIODevice::WriteOnly));
        g.write(gif);
        g.close();
        QImageReader probe(f);
        QVERIFY2(probe.size() == size,
                 qPrintable(QStringLiteral("the GIF fixture is %1x%2, not 37x23")
                                .arg(probe.size().width()).arg(probe.size().height())));
        files << f;
    }
    for (const QString &f : files) {
        // A clean document first, so the discard guard has nothing to ask.
        QVERIFY(m_controller->newDocument(8, 8, QStringLiteral("white")));
        QVERIFY(!m_controller->documentHasUnsavedWork());
        evalJs(QStringLiteral("openFileDialog.selectedFile = \"%1\"; "
                              "openFileDialog.accepted()")
                   .arg(QUrl::fromLocalFile(f).toString()));
        QTRY_COMPARE(m_controller->docWidth(), size.width());
        QCOMPARE(m_controller->docHeight(), size.height());
        // ENGINE TRUTH, not a count: the image's red block is in the
        // composite (JPEG within its loss) -- a format whose import silently
        // failed would leave only the transparent background here.
        QImage px;
        QVERIFY(m_controller->readRegionImage(QRect(15, 10, 1, 1), &px));
        const QColor c = px.pixelColor(0, 0);
        QVERIFY2(c.red() > 150 && c.green() < 90 && c.blue() < 90,
                 qPrintable(QStringLiteral("%1 opened, but the composite at "
                                           "(15,10) is %2, not the image's red")
                                .arg(QFileInfo(f).fileName(), c.name())));
        qInfo().noquote() << QStringLiteral("D-4 %1: %2x%3, %4 layers, (15,10)=%5")
                                 .arg(QFileInfo(f).fileName())
                                 .arg(m_controller->docWidth())
                                 .arg(m_controller->docHeight())
                                 .arg(m_controller->layers().size())
                                 .arg(c.name());
    }
    // .ora: not an Open format; the door refuses and opens nothing.
    const QString ora = tmp.filePath(QStringLiteral("d4.ora"));
    {
        QFile o(ora);
        QVERIFY(o.open(QIODevice::WriteOnly));
        o.write("PK\x03\x04not-a-real-ora");
    }
    QVERIFY(m_controller->newDocument(8, 8, QStringLiteral("white")));
    QVERIFY(!m_controller->openImageAsDocument(QUrl::fromLocalFile(ora).toString()));
    QCOMPARE(m_controller->docWidth(), 8);
}

int main(int argc, char *argv[])
{
    if (qgetenv("QT_QPA_PLATFORM").isEmpty())
        qputenv("QT_QPA_PLATFORM", "offscreen");
    QQuickStyle::setStyle(QStringLiteral("Basic"));
    QGuiApplication app(argc, argv);
    TstImportFilter tc;
    QTEST_SET_MAIN_SOURCE_PATH
    return QTest::qExec(&tc, argc, argv);
}
#include "tst_import_filter.moc"
