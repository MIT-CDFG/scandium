// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan §2.2 "Qt/QML shell v1" row + §3.9 lane orders. Entry point:
// window bring-up, i18n catalog bootstrap (APPNAME_I18N_DIR ->
// exe-adjacent i18n/ -> key-echo), --screenshot-tour mode. Trade dress
// D-43: window titled with the "Appname" placeholder, generic layout and
// names throughout.

#include "abi_client.h"
#include "announcer.h"
#include "canvas_view.h"
#include "document_controller.h"
#include "icon_provider.h"
#include "i18n_catalog.h"
#include "layer_thumb_provider.h"
#include "tool_key_dispatcher.h"
#include "tour.h"
#include "units.h"

#include <QCommandLineParser>
#include "build_info.h"
#include "click_focus_parity.h"
#include "license_texts.h"

#include <QDir>
#include <QFont>

#include <QGuiApplication>
#include <QKeyEvent>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QStyleHints>

#include <cstdio>

using namespace appname::shell;

// A11Y-5 hook B (review delta): QQuickMenuBar's menu-to-menu switching
// state arms ONLY on a REAL key activation of a MenuBarItem — neither
// menu.open() nor highlighted=true arms it (reviewer-isolated, probes
// H1-H3/K1-K11). The access chord therefore injects a genuine Space
// key event after focusing item 0: menu 0 opens AND the bar arms, so
// F10 → Left/Right/Down/Up/Return is drivable end to end.
class MenuArmer : public QObject {
    Q_OBJECT
public:
    using QObject::QObject;
    Q_INVOKABLE void armMenuBar(QObject *window)
    {
        auto *w = qobject_cast<QQuickWindow *>(window);
        if (!w)
            return;
        QCoreApplication::postEvent(
            w, new QKeyEvent(QEvent::KeyPress, Qt::Key_Space, Qt::NoModifier));
        QCoreApplication::postEvent(
            w, new QKeyEvent(QEvent::KeyRelease, Qt::Key_Space, Qt::NoModifier));
    }
};

int main(int argc, char *argv[])
{
    // Pre-application environment: the tour must be runnable headless with
    // no caller setup. Defaults only — explicit environment always wins.
    bool tourRequested = false;
    for (int i = 1; i < argc; ++i) {
        if (qstrcmp(argv[i], "--screenshot-tour") == 0)
            tourRequested = true;
    }
    if (tourRequested) {
        if (qgetenv("QT_QPA_PLATFORM").isEmpty())
            qputenv("QT_QPA_PLATFORM", "offscreen");
        if (qgetenv("QT_QUICK_BACKEND").isEmpty())
            qputenv("QT_QUICK_BACKEND", "software"); // deterministic raster path
#ifdef _WIN32
        // W-24 (a), operator WORD 31 (2): on Windows the offscreen platform
        // has no font database, so every tour glyph rendered as tofu. Point
        // Qt's FreeType database at the Windows Fonts folder; the family is
        // pinned to Segoe UI just after the application exists (below).
        if (qgetenv("QT_QPA_FONTDIR").isEmpty()) {
            QByteArray winDir = qgetenv("WINDIR");
            if (winDir.isEmpty())
                winDir = "C:\\Windows";
            qputenv("QT_QPA_FONTDIR", winDir + "\\Fonts");
        }
#endif
    }

    QGuiApplication app(argc, argv);
#ifdef _WIN32
    // W-24 (a): the Windows tour pins the system UI face, so its goldens
    // (artifacts/tour-chrome-win/, recorded by O-WIN) do not depend on which
    // family the FreeType database happens to pick first.
    if (tourRequested) {
        QFont uiFont = QGuiApplication::font();
        uiFont.setFamilies({QStringLiteral("Segoe UI")});
        QGuiApplication::setFont(uiFont);
    }
#endif
    QGuiApplication::setApplicationName(QStringLiteral("Appname")); // D-43 placeholder
    QGuiApplication::setOrganizationName(QStringLiteral("Appname contributors"));
    QQuickStyle::setStyle(QStringLiteral("Basic")); // generic controls, deterministic tour
    // §49.5 [NORMATIVE]: "long-press (600 ms) = right-click/context menu
    // on every surface, matching the tool-palette long-press flyout
    // (§32.3)". The palette flyout rides QQuickAbstractButton::
    // pressAndHold, whose timer reads this hint (Qt platform default
    // ~800 ms) — set app-wide per the clause's own every-surface scope,
    // before the engine loads any control. (v1.0.1 item 11b review D4;
    // QML cannot set it — QStyleHints exposes the property read-only
    // there, verified on this Qt build.)
    QGuiApplication::styleHints()->setMousePressAndHoldInterval(600);
    // OPERATOR WORD 14 (2)(b), 2026-09-17 02:01 ET — Q-D72, PATH 2, RULED: "Tab reaches
    // every control in the app regardless of macOS Full Keyboard Access." The app is a
    // keyboard-heavy professional tool and the macOS default (0x3, text and list controls
    // only, when Full Keyboard Access is off) takes every Button, ToolButton and CheckBox
    // off the Tab ring. The override is app-scoped; the USER-GUIDE keyboard section
    // documents it (O-UI WORD 8 (1)). Written here, not folded from 906e48c78.
    QGuiApplication::styleHints()->setTabFocusBehavior(Qt::TabFocusAllControls);
    // W-14 / W-15 / W-17, operator WORD 31 (5): a pointer click on a button
    // does not take keyboard focus — the macOS convention the a11y design was
    // recorded against. Installed on Windows only; see click_focus_parity.h.
    installClickFocusParity(app);

    QCommandLineParser parser;
    parser.setApplicationDescription(
        QStringLiteral("Appname Qt/QML shell v1 (CP-0.3) — pure C-ABI client"));
    parser.addHelpOption();
    const QCommandLineOption langOption(QStringLiteral("lang"),
                                        QStringLiteral("UI language code (default: APPNAME_LANG or en)"),
                                        QStringLiteral("code"));
    const QCommandLineOption tourOption(QStringLiteral("screenshot-tour"),
                                        QStringLiteral("Run the scripted screenshot tour into <dir> and exit"),
                                        QStringLiteral("dir"));
    parser.addOption(langOption);
    parser.addOption(tourOption);
    parser.process(app);

    const QString lang = Catalog::resolveLang(parser.value(langOption));
    Catalog catalog = Catalog::load(lang);
    CatalogQml catalogQml(std::move(catalog));

    DocumentController controller(&catalogQml.catalog());
    // v1.0.1 items 2+3: pure dialog-side unit math + the preset table.
    // Constructed BEFORE the engine (like the controller/announcer) so
    // it outlives every QML binding — a context property torn down
    // ahead of the engine nulls mid-teardown and bindings TypeError.
    UnitsQml units;
    // A11Y-3: the single polite live region (§31.3). Wired in every mode
    // — announcements are receipt-driven state truth, and the tour's
    // offscreen run exercises the same code path the AT bridge rides.
    Announcer announcer(&catalogQml.catalog());
    controller.setAnnouncer(&announcer);
    const bool tourMode = parser.isSet(tourOption);
    if (tourMode)
        controller.setDeterministicIds(0xA99AA9E5EEDULL); // fixed ids + stroke seeds

    std::printf("appname-shell: %s; asset ingress appname_asset_create (linked)\n",
                qPrintable(abiVersionString()));

    QQmlApplicationEngine engine;
    engine.addImageProvider(QStringLiteral("lucide"),
                            new LucideIconProvider); // engine takes ownership
    engine.addImageProvider(QStringLiteral("layerthumb"),
                            new LayerThumbProvider(&controller));
    // B-09: the build's own commit, captured fresh each build (see
    // cmake/write_build_info.cmake). A STRING, not a struct, because it has
    // exactly one consumer and a one-field object would be scaffolding for
    // a second field that is blocked on an operator word (the version).
    engine.rootContext()->setContextProperty(
        QStringLiteral("AppBuildCommit"),
        QString::fromLatin1(APPNAME_BUILD_COMMIT));
    // RC3 (operator WORD 36 (a)): the version the build carries
    // (APPNAME_VERSION from Cargo.toml, via CMake), worded for 1.0.2 by the
    // operator -- shown in About beside the build commit.
    engine.rootContext()->setContextProperty(
        QStringLiteral("AppVersion"),
        QString::fromLatin1(APPNAME_VERSION_STRING));

    // WORD 3's fallback item: the Help > Open Licenses Folder door. The
    // path is resolved HERE, not in QML, because only C++ can see
    // applicationDirPath — and it uses i18n_catalog's search-list idiom
    // rather than a new one: env override, then the packaged location, then
    // the repo tree for a dev run.
    //
    // ★ THE DESTINATION IS NOT INVENTED: scripts/build_dmg.sh:262 sets
    //   NOTICE_DEST="$APP/Contents/Resources/licenses" and copies the repo's
    //   LICENSES/ there, so this resolves the path PACKAGING ACTUALLY MAKES.
    //
    // ★ EMPTY IS AN HONEST ANSWER. If no candidate exists the property is
    //   empty and the menu item hides itself — the same posture as
    //   appVersion and buildCommit. A door that opens on nothing is worse
    //   than no door, and this desk has now shipped that mistake once.
    {
        QString licensesDir;
        QVector<QString> candidates;
        const QByteArray envDir = qgetenv("APPNAME_LICENSES_DIR");
        if (!envDir.isEmpty())
            candidates.append(QString::fromUtf8(envDir));
        const QString exeDir = QCoreApplication::applicationDirPath();
        candidates.append(QDir(exeDir).filePath(
            QStringLiteral("../Resources/licenses")));   // packaged .app
        candidates.append(QDir(exeDir).filePath(QStringLiteral("licenses")));
        candidates.append(QDir(exeDir).filePath(QStringLiteral("../../LICENSES")));
        for (const QString &c : candidates) {
            const QDir d(c);
            if (d.exists() && !d.entryList(QDir::Files).isEmpty()) {
                licensesDir = d.canonicalPath();
                break;
            }
        }
        engine.rootContext()->setContextProperty(
            QStringLiteral("AppLicensesDir"), licensesDir);

        // B-09's display half: the same resolved folder, as a model QML can
        // list and read on demand. Parented to the engine so its lifetime
        // matches the context it is exposed in.
        auto *licenseTexts = new LicenseTexts(licensesDir, &engine);
        engine.rootContext()->setContextProperty(
            QStringLiteral("AppLicenseTexts"), licenseTexts);
    }
    engine.rootContext()->setContextProperty(QStringLiteral("AppI18n"), &catalogQml);
    engine.rootContext()->setContextProperty(QStringLiteral("AppDoc"), &controller);
    engine.rootContext()->setContextProperty(QStringLiteral("AppA11y"), &announcer);
    MenuArmer menuArmer;
    engine.rootContext()->setContextProperty(QStringLiteral("AppKeys"), &menuArmer);
    engine.rootContext()->setContextProperty(QStringLiteral("AppUnits"), &units);
    engine.loadFromModule("AppnameShell", "Main");
    if (engine.rootObjects().isEmpty()) {
        std::fprintf(stderr, "appname-shell: QML load failed\n");
        return 1;
    }
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
    if (!window) {
        std::fprintf(stderr, "appname-shell: root object is not a window\n");
        return 1;
    }
    // A11Y-3 D1 fix: announcements dispatch on the window (resolves a
    // real QAccessibleInterface) — on a bare QObject every platform
    // bridge drops them silently.
    announcer.setSubject(window);

    // v1.0.1 item 11e: the tool letter-key chords. A real key-event
    // dispatcher, installed on the loaded window (the letters left the
    // shortcut map at the migration — see the record in Main.qml).
    // Constructed AFTER the engine load because it needs the window and
    // the CanvasView; it is QML-invisible on purpose (no context
    // property): it CALLS INTO QML through the dispatchToolChord funnel
    // and never the other way round, so there is no new QML surface to
    // keep coherent. Every Main.qml test harness mirrors this one call.
    ToolKeyDispatcher toolKeys;
    toolKeys.attach(window);

    if (tourMode)
        return runScreenshotTour(window, &controller, parser.value(tourOption));

    return app.exec();
}

#include "main.moc"
