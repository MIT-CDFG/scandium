// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: plan P-ROUND P1 (design-token dark theme, WCAG-AA evidence) —
// reviewer-added invariant pin (P1 slice review, 2026-08-13). Theme.qml's
// header commits every visible-text pairing to WCAG-AA (>= 4.5:1 normal
// text) and the disabled/indicator tokens to the >= 3:1 UI bar. This test
// parses the hex tokens OUT OF Theme.qml mechanically (no duplicated
// values — editing a token re-checks automatically) and recomputes the
// WCAG 2.x relative-luminance contrast for every pairing the shell
// actually renders, so a future token edit cannot silently drop below the
// documented bars. Pairings and bars mirror the token comments plus the
// in-context uses read during review (panel grounds, raised rows, active
// row, window chrome).

#include <QFile>
#include <QRegularExpression>
#include <QTest>

#include <cmath>

namespace {

double channelLin(double c8)
{
    const double c = c8 / 255.0;
    return (c <= 0.04045) ? c / 12.92 : std::pow((c + 0.055) / 1.055, 2.4);
}

// WCAG 2.x relative luminance from an #rrggbb string.
double relativeLuminance(const QString &hex)
{
    const int r = hex.mid(1, 2).toInt(nullptr, 16);
    const int g = hex.mid(3, 2).toInt(nullptr, 16);
    const int b = hex.mid(5, 2).toInt(nullptr, 16);
    return 0.2126 * channelLin(r) + 0.7152 * channelLin(g) + 0.0722 * channelLin(b);
}

double contrastRatio(const QString &fg, const QString &bg)
{
    const double l1 = relativeLuminance(fg);
    const double l2 = relativeLuminance(bg);
    const double hi = std::max(l1, l2);
    const double lo = std::min(l1, l2);
    return (hi + 0.05) / (lo + 0.05);
}

} // namespace

class TstThemeContrast : public QObject
{
    Q_OBJECT

private slots:
    void initTestCase();
    void tokensParsed();
    void normalTextPairsHoldAa_data();
    void normalTextPairsHoldAa();
    void uiBarPairsHoldThreeToOne_data();
    void uiBarPairsHoldThreeToOne();
    void eyeTintUsageHoldsUiBar();
    void canvasGroundValueMatchesTheme();
    void layerDelegateHostsNoDisableableControl();
    void noDocPlaceholderHoldsAaOnCanvasGround();
    void historyRowStateTokensHoldAa();

private:
    QHash<QString, QString> m_tokens; // name -> "#rrggbb"

    QString tok(const char *name)
    {
        const QString v = m_tokens.value(QLatin1String(name));
        if (v.isEmpty())
            QTest::qFail(qPrintable(QStringLiteral("token missing from Theme.qml: %1")
                                        .arg(QLatin1String(name))),
                         __FILE__, __LINE__);
        return v;
    }
};

void TstThemeContrast::initTestCase()
{
    // Path injected by the build: the checked-in QML source, not a copy.
    QFile f(QStringLiteral(APPNAME_THEME_QML_PATH));
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             qPrintable(QStringLiteral("cannot open %1").arg(f.fileName())));
    const QString source = QString::fromUtf8(f.readAll());

    // Mechanical extraction: every color token declared as a quoted hex.
    static const QRegularExpression re(QStringLiteral(
        "readonly\\s+property\\s+color\\s+(\\w+)\\s*:\\s*\"(#[0-9a-fA-F]{6})\""));
    auto it = re.globalMatch(source);
    while (it.hasNext()) {
        const auto m = it.next();
        m_tokens.insert(m.captured(1), m.captured(2).toLower());
    }
}

void TstThemeContrast::tokensParsed()
{
    // The full palette roster this slice ships; a rename or a non-hex
    // rewrite (e.g. Qt.rgba) must surface here, not silently skip checks.
    const char *roster[] = {"bgWindow",    "bgSurface",     "bgRaised", "bgActive",
                            "bgCanvas",    "textPrimary",   "textSecondary",
                            "textDisabled", "accent",       "warnText", "badge",
                            "border",      "borderLight",   "borderInput",
                            "focusRing"};
    for (const char *name : roster)
        QVERIFY2(m_tokens.contains(QLatin1String(name)),
                 qPrintable(QStringLiteral("token missing or not plain #rrggbb: %1")
                                .arg(QLatin1String(name))));
}

void TstThemeContrast::normalTextPairsHoldAa_data()
{
    QTest::addColumn<QString>("fg");
    QTest::addColumn<QString>("bg");

    // Documented pairings (Theme.qml comments) + every ground the token
    // is actually rendered on (panels bgSurface, rows bgRaised, selected
    // row bgActive, window chrome bgWindow — per P1 review read).
    QTest::newRow("textPrimary/bgSurface") << "textPrimary" << "bgSurface";
    QTest::newRow("textPrimary/bgWindow") << "textPrimary" << "bgWindow";
    QTest::newRow("textPrimary/bgRaised") << "textPrimary" << "bgRaised";
    QTest::newRow("textPrimary/bgActive") << "textPrimary" << "bgActive";
    QTest::newRow("textSecondary/bgSurface") << "textSecondary" << "bgSurface";
    QTest::newRow("textSecondary/bgRaised") << "textSecondary" << "bgRaised";
    QTest::newRow("textSecondary/bgWindow") << "textSecondary" << "bgWindow";
    QTest::newRow("accent/bgSurface") << "accent" << "bgSurface";
    QTest::newRow("accent/bgWindow") << "accent" << "bgWindow";
    QTest::newRow("accent/bgRaised") << "accent" << "bgRaised";
    QTest::newRow("warnText/bgSurface") << "warnText" << "bgSurface";
    QTest::newRow("badge/bgRaised") << "badge" << "bgRaised";
    // A11Y-4 rendered-set additions: dialog error text on the dialog
    // ground (Basic Dialog paints palette.window), and the Basic
    // PRESSED-button fill (palette.mid = borderLight) under its
    // textPrimary label — the pairing that vetoed a global palette.mid
    // retint (lane log, A11Y-4 design).
    QTest::newRow("warnText/bgWindow") << "warnText" << "bgWindow";
    QTest::newRow("textPrimary/borderLight") << "textPrimary" << "borderLight";
}

void TstThemeContrast::normalTextPairsHoldAa()
{
    QFETCH(QString, fg);
    QFETCH(QString, bg);
    const double r = contrastRatio(tok(fg.toUtf8().constData()),
                                   tok(bg.toUtf8().constData()));
    QVERIFY2(r >= 4.5,
             qPrintable(QStringLiteral("%1 on %2: %3:1 < 4.5:1 (WCAG AA normal text)")
                            .arg(fg, bg)
                            .arg(r, 0, 'f', 2)));
}

void TstThemeContrast::uiBarPairsHoldThreeToOne_data()
{
    QTest::addColumn<QString>("fg");
    QTest::addColumn<QString>("bg");

    // Disabled text (WCAG-exempt, but the lane's stated bar) on every
    // ground it renders on — bgRaised is the Basic button ground, the
    // pairing the original token missed (review P1-D3) — and indicator
    // glyphs on the selected row.
    QTest::newRow("textDisabled/bgSurface") << "textDisabled" << "bgSurface";
    QTest::newRow("textDisabled/bgWindow") << "textDisabled" << "bgWindow";
    QTest::newRow("textDisabled/bgRaised") << "textDisabled" << "bgRaised";
    QTest::newRow("accent/bgActive") << "accent" << "bgActive";
    QTest::newRow("badge/bgActive") << "badge" << "bgActive";
    // Eye-off glyph on the ACTIVE layer row (review P2-D1): the hidden-eye
    // tint is an enabled state indicator, so it owes the UI bar on
    // bgActive — the ground textDisabled failed (2.18:1) and the retint
    // to textSecondary clears (3.36:1). Pinned so a token edit cannot
    // silently reintroduce the breach.
    QTest::newRow("textSecondary/bgActive") << "textSecondary" << "bgActive";
    // §30.4 focus.ring (A11Y-2): the ring draws on every pane ground —
    // all four pinned at the UI bar.
    QTest::newRow("focusRing/bgWindow") << "focusRing" << "bgWindow";
    QTest::newRow("focusRing/bgSurface") << "focusRing" << "bgSurface";
    QTest::newRow("focusRing/bgRaised") << "focusRing" << "bgRaised";
    QTest::newRow("focusRing/bgActive") << "focusRing" << "bgActive";
    // A11Y-4: borderInput is the 1.4.11 input-boundary token — ≥3:1 on
    // the input fill (bgSurface) and every ground inputs render over.
    QTest::newRow("borderInput/bgSurface") << "borderInput" << "bgSurface";
    QTest::newRow("borderInput/bgWindow") << "borderInput" << "bgWindow";
    QTest::newRow("borderInput/bgRaised") << "borderInput" << "bgRaised";
}

void TstThemeContrast::uiBarPairsHoldThreeToOne()
{
    QFETCH(QString, fg);
    QFETCH(QString, bg);
    const double r = contrastRatio(tok(fg.toUtf8().constData()),
                                   tok(bg.toUtf8().constData()));
    QVERIFY2(r >= 3.0,
             qPrintable(QStringLiteral("%1 on %2: %3:1 < 3:1 (UI bar)")
                            .arg(fg, bg)
                            .arg(r, 0, 'f', 2)));
}

void TstThemeContrast::eyeTintUsageHoldsUiBar()
{
    // USAGE pin (reviewer, P2 re-review): the token-pair rows above parse
    // Theme.qml only, so a QML retint is invisible to them — reverting the
    // eye's hidden tint to textDisabled would pass every token row while
    // breaching the bar on the active row (2.18:1, review P2-D1). Read the
    // eye's icon.color ternary out of LayersPanel.qml mechanically and
    // hold BOTH branch tokens to >= 3:1 on BOTH row grounds the delegate
    // renders (bgRaised inactive, bgActive selected). Any compliant retint
    // passes; a breaching one fails with the measured ratio.
    QString path = QStringLiteral(APPNAME_THEME_QML_PATH);
    path.replace(QStringLiteral("Theme.qml"), QStringLiteral("LayersPanel.qml"));
    QFile f(path);
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             qPrintable(QStringLiteral("cannot open %1").arg(path)));
    const QString source = QString::fromUtf8(f.readAll());

    static const QRegularExpression re(QStringLiteral(
        "icon\\.color:\\s*row\\.modelData\\.visible\\s*\\?\\s*Theme\\.(\\w+)"
        "\\s*:\\s*Theme\\.(\\w+)"));
    const auto m = re.match(source);
    QVERIFY2(m.hasMatch(),
             "eye icon.color visible-ternary not found in LayersPanel.qml "
             "(tint expression changed shape — update this usage pin, do not delete it)");

    const char *grounds[] = {"bgRaised", "bgActive"};
    for (int cap = 1; cap <= 2; ++cap) {
        const QString fgName = m.captured(cap);
        const QString fg = m_tokens.value(fgName);
        QVERIFY2(!fg.isEmpty(),
                 qPrintable(QStringLiteral("eye tint token not a plain Theme hex token: %1")
                                .arg(fgName)));
        for (const char *bgName : grounds) {
            const double r = contrastRatio(fg, tok(bgName));
            QVERIFY2(r >= 3.0,
                     qPrintable(QStringLiteral("eye tint %1 on %2: %3:1 < 3:1 (UI bar; review P2-D1)")
                                    .arg(fgName, QLatin1String(bgName))
                                    .arg(r, 0, 'f', 2)));
        }
    }
}

void TstThemeContrast::canvasGroundValueMatchesTheme()
{
    // A11Y-4: Theme.qml's header claims the C++ canvas constants match
    // the tokens — make the claim mechanical for the workspace ground
    // (the audit's bgCanvas rows are computed against THIS value).
    QString path = QStringLiteral(APPNAME_THEME_QML_PATH);
    path.replace(QStringLiteral("qml/Theme.qml"),
                 QStringLiteral("src/canvas_view.cpp"));
    QFile f(path);
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             qPrintable(QStringLiteral("cannot open %1").arg(path)));
    const QString source = QString::fromUtf8(f.readAll());
    static const QRegularExpression re(QStringLiteral(
        "fillRect\\([^;]*QColor\\(0x([0-9a-fA-F]{2}),\\s*0x([0-9a-fA-F]{2}),"
        "\\s*0x([0-9a-fA-F]{2})\\)\\)"));
    const auto m = re.match(source);
    QVERIFY2(m.hasMatch(),
             "workspace-ground fillRect(QColor(0x..,0x..,0x..)) not found "
             "in canvas_view.cpp — update this value-match pin, do not "
             "delete it");
    const QString cpp = QStringLiteral("#%1%2%3")
                            .arg(m.captured(1), m.captured(2), m.captured(3))
                            .toLower();
    QCOMPARE(cpp, tok("bgCanvas"));
}

void TstThemeContrast::layerDelegateHostsNoDisableableControl()
{
    // A11Y-4 (the O-I18N audit ask, finding 1): textDisabled on bgActive
    // is 2.2:1 — below every bar — and the window palette paints ALL
    // disabled control text with textDisabled. The layer delegate is the
    // only surface whose ground is bgActive, so the mechanical guard is:
    // no control inside the delegate block may carry an `enabled:`
    // binding (a disableable control there would render textDisabled on
    // bgActive the moment it disables). If one ever must, it needs an
    // explicit disabled tint meeting 3:1 on bgActive AND an update to
    // this pin naming it.
    QString path = QStringLiteral(APPNAME_THEME_QML_PATH);
    path.replace(QStringLiteral("Theme.qml"),
                 QStringLiteral("LayersPanel.qml"));
    QFile f(path);
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             qPrintable(QStringLiteral("cannot open %1").arg(path)));
    const QString source = QString::fromUtf8(f.readAll());
    const int start = source.indexOf(QLatin1String("delegate: Rectangle"));
    QVERIFY2(start >= 0, "layer delegate block not found — update this pin");
    // Brace-match the delegate block — ON A COPY WITH STRING LITERALS
    // BLANKED (A31, C-754). LayersPanel carries literals containing braces
    // (": row.modelData.layerId}, {", ": t}, ") INSIDE this very block, and
    // an unblanked walk can close it early or late without any symptom.
    // MEASURED at the time of writing: blanked and unblanked agree here, so
    // this changes no verdict today — it removes a dependency on the real
    // braces in the surrounding code happening to compensate.
    QString scan = source;
    {
        bool inString = false;
        for (int i = 0; i < scan.size(); ++i) {
            const QChar c = scan.at(i);
            if (inString) {
                if (c == QLatin1Char('\\') && i + 1 < scan.size()) { scan[i + 1] = QLatin1Char(' '); ++i; continue; }
                if (c == QLatin1Char('"')) { inString = false; continue; }
                scan[i] = QLatin1Char(' ');
            } else if (c == QLatin1Char('"')) {
                inString = true;
            }
        }
    }
    int depth = 0;
    int end = start;
    for (int i = scan.indexOf(QLatin1Char('{'), start); i < scan.size();
         ++i) {
        if (scan.at(i) == QLatin1Char('{'))
            ++depth;
        else if (scan.at(i) == QLatin1Char('}')) {
            if (--depth == 0) {
                end = i;
                break;
            }
        }
    }
    QVERIFY2(end > start,
             "delegate braces never rebalanced — parse broken, fix the "
             "matcher (review D5: a vacuous pass is worse than none)");
    const QString block = source.mid(start, end - start);
    // Review D5: the `;` anchor covers the house single-line style
    // (`id: x; enabled: false`), and a DIRECT textDisabled binding is
    // the palette-free route to the same breach.
    static const QRegularExpression enabledRe(
        QStringLiteral("(^|\\n|;)\\s*enabled\\s*:"));
    QVERIFY2(!enabledRe.match(block).hasMatch(),
             "a control inside the layer delegate carries `enabled:` — "
             "disabled text there renders textDisabled on bgActive "
             "(2.2:1, below every bar; O-I18N audit finding 1)");
    QVERIFY2(!block.contains(QLatin1String("Theme.textDisabled")),
             "Theme.textDisabled bound inside the layer delegate — "
             "2.2:1 on the bgActive row ground (O-I18N audit finding 1)");
}

void TstThemeContrast::noDocPlaceholderHoldsAaOnCanvasGround()
{
    // A11Y-4 review D1: the placeholder's ground is bgCanvas (painted
    // unconditionally by canvas_view.cpp before its no-document
    // early-return) — the launch surface's only running text must hold
    // AA there. Parse the Label's color out of Main.qml.
    QString path = QStringLiteral(APPNAME_THEME_QML_PATH);
    path.replace(QStringLiteral("Theme.qml"), QStringLiteral("Main.qml"));
    QFile f(path);
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             qPrintable(QStringLiteral("cannot open %1").arg(path)));
    const QString source = QString::fromUtf8(f.readAll());
    const int anchor =
        source.indexOf(QLatin1String("ui.canvas.no_document"));
    QVERIFY2(anchor >= 0, "no-document placeholder not found — update pin");
    static const QRegularExpression re(
        QStringLiteral("color:\\s*Theme\\.(\\w+)"));
    const auto m = re.match(source, anchor);
    QVERIFY2(m.hasMatch(), "placeholder color binding not found");
    const double r = contrastRatio(m_tokens.value(m.captured(1)),
                                   tok("bgCanvas"));
    QVERIFY2(r >= 4.5,
             qPrintable(QStringLiteral(
                            "no-doc placeholder %1 on bgCanvas: %2:1 < 4.5")
                            .arg(m.captured(1))
                            .arg(r, 0, 'f', 2)));
}

void TstThemeContrast::historyRowStateTokensHoldAa()
{
    // A11Y-4 usage pin: history rows encode the redoable state as label
    // COLOR (the old 0.45 whole-row dim composited to ~3.2:1 effective).
    // Both ternary branches must hold AA normal text on the row ground.
    QString path = QStringLiteral(APPNAME_THEME_QML_PATH);
    path.replace(QStringLiteral("Theme.qml"),
                 QStringLiteral("HistoryPanel.qml"));
    QFile f(path);
    QVERIFY2(f.open(QIODevice::ReadOnly | QIODevice::Text),
             qPrintable(QStringLiteral("cannot open %1").arg(path)));
    const QString source = QString::fromUtf8(f.readAll());
    // Review D7: forbid ANY opacity expression keyed on the row state,
    // either polarity — not just the original exact pattern.
    static const QRegularExpression dimRe(QStringLiteral(
        "opacity\\s*:[^\\n]*modelData\\.active"));
    QVERIFY2(!dimRe.match(source).hasMatch(),
             "history rows regressed to state-keyed opacity dimming "
             "(composited effective contrast ~3.2:1 — A11Y-4)");
    // Review D3 (§31.4 color-alone): the redoable state must keep its
    // SHAPE cue alongside the color pair.
    QVERIFY2(source.contains(QLatin1String(
                 "font.italic: !modelData.active")),
             "history redoable state lost its non-color (italic) cue — "
             "color-alone state encoding (§31.4, §31.5 gate item 4)");
    static const QRegularExpression re(QStringLiteral(
        "color:\\s*modelData\\.active\\s*\\?\\s*Theme\\.(\\w+)"
        "\\s*:\\s*Theme\\.(\\w+)"));
    const auto m = re.match(source);
    QVERIFY2(m.hasMatch(),
             "history-row state ternary not found in HistoryPanel.qml "
             "(expression changed shape — update this usage pin, do not "
             "delete it)");
    for (int cap = 1; cap <= 2; ++cap) {
        const QString fgName = m.captured(cap);
        const QString fg = m_tokens.value(fgName);
        QVERIFY2(!fg.isEmpty(),
                 qPrintable(QStringLiteral(
                                "history label token not a plain Theme hex "
                                "token: %1")
                                .arg(fgName)));
        const double r = contrastRatio(fg, tok("bgRaised"));
        QVERIFY2(r >= 4.5,
                 qPrintable(QStringLiteral(
                                "history label %1 on bgRaised: %2:1 < 4.5:1")
                                .arg(fgName)
                                .arg(r, 0, 'f', 2)));
    }
}

QTEST_GUILESS_MAIN(TstThemeContrast)
#include "tst_theme_contrast.moc"
