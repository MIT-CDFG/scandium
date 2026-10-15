// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// Spec: ES §31.5 UI-change PR gate, item 1 — "zero unlabeled interactive
// items" — run mechanically over the checked-in shell/qml (the
// tst_filter_table pattern: parse the source of truth, not a mirror).
// Every interactive control block must expose a `text:` or an
// `Accessible.name:` at its own depth — a LITERAL-EMPTY label
// (`text: ""` / `Accessible.name: ""`) does not count (review D4).
// The A11Y-3 wave-B holds ride an EXPLICIT expected-gap roster (their
// ids are queued for O-I18N in the lane log — naming them from
// unrelated keys would be the P2-D2 abuse): the roster is TWO-SIDED —
// an unlabeled control missing from the roster reds (new debt), and a
// roster row matching no unlabeled control reds too (stale roster:
// shrink it, that is wave B's exit assertion). Roster rows anchor to
// the control's `id:` (review D4: positional anchors drift silently
// when a new control of the same type lands earlier in the file).
// Parse-knockout guards (the 516b538 lesson): minimum file and control
// counts pin the sweep itself; a single-quote/backtick guard forces a
// conscious stripper extension before those string forms enter the QML
// (the stripper is double-quote-aware only, review D4).

#include <QDirIterator>
#include <QFile>
#include <QRegularExpression>
#include <QtTest>

namespace {

QString readAll(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly))
        return QString();
    return QString::fromUtf8(f.readAll());
}

// Strip // and /* */ comments, string-aware: "image://lucide/…" style
// literals must survive (a naive stripper truncates them mid-string).
QString stripComments(const QString &in)
{
    QString out;
    out.reserve(in.size());
    bool inString = false;
    bool inLine = false;
    bool inBlock = false;
    for (int i = 0; i < in.size(); ++i) {
        const QChar c = in.at(i);
        const QChar n = i + 1 < in.size() ? in.at(i + 1) : QChar();
        if (inLine) {
            if (c == QLatin1Char('\n')) {
                inLine = false;
                out.append(c);
            }
            continue;
        }
        if (inBlock) {
            if (c == QLatin1Char('*') && n == QLatin1Char('/')) {
                inBlock = false;
                ++i;
            } else if (c == QLatin1Char('\n')) {
                out.append(c); // keep line structure for anchors
            }
            continue;
        }
        if (inString) {
            out.append(c);
            if (c == QLatin1Char('\\') && !n.isNull()) {
                out.append(n);
                ++i;
            } else if (c == QLatin1Char('"')) {
                inString = false;
            }
            continue;
        }
        if (c == QLatin1Char('"')) {
            inString = true;
            out.append(c);
            continue;
        }
        if (c == QLatin1Char('/') && n == QLatin1Char('/')) {
            inLine = true;
            ++i;
            continue;
        }
        if (c == QLatin1Char('/') && n == QLatin1Char('*')) {
            inBlock = true;
            ++i;
            continue;
        }
        out.append(c);
    }
    return out;
}

// The control block starting at the brace `open`: return its body text
// that sits at depth 1 (immediate properties only — a nested child's
// `text:` must not satisfy the parent).
QString depthOneBody(const QString &src, int open, int *closeOut)
{
    QString body;
    int depth = 0;
    bool inString = false;
    for (int i = open; i < src.size(); ++i) {
        const QChar c = src.at(i);
        if (inString) {
            if (c == QLatin1Char('\\')) {
                ++i;
            } else if (c == QLatin1Char('"')) {
                inString = false;
            }
            if (depth == 1)
                body.append(c);
            continue;
        }
        if (c == QLatin1Char('"'))
            inString = true;
        if (c == QLatin1Char('{')) {
            ++depth;
            continue;
        }
        if (c == QLatin1Char('}')) {
            --depth;
            if (depth == 0) {
                if (closeOut)
                    *closeOut = i;
                return body;
            }
            continue;
        }
        if (depth == 1)
            body.append(c);
    }
    if (closeOut)
        *closeOut = src.size();
    return body;
}

struct Violation {
    QString key; // "<file>:<Type>#<occurrence>"
};

// ---- C-754: the SECOND property, and the one the first cannot see ----
//
// §31.5 item 1 asks whether a control is LABELLED. It says nothing about
// whether the control can be OPERATED, and those come apart: CurveEditor
// carried a correct Accessible.role/name and was entirely mouse-only, so
// this lint PASSED a control a screen-reader user could hear announced and
// then not use (C-753). Three defects of this class landed in three ticks
// -- C-751 a list, C-752 the layer stack, C-753 the curve editor -- before
// anything mechanical looked for it.
//
// BLANKING STRINGS IS NOT HYGIENE, IT IS CORRECTNESS. Brace-walking QML
// that contains "{name}, {hex}" (ToolPalette) or "…layerId}, {}, …"
// (LayersPanel) mis-nests without it -- measured: 7 files carry a literal
// with a brace, and LayersPanel is the file this check exists because of.
// The scratch sweep that FOUND these three did not blank, so its nesting
// was unsound even though its two-sided controls fired; that is why this
// re-measures rather than inheriting its count.
QString blankStrings(const QString &in)
{
    QString out = in;
    bool inString = false;
    for (int i = 0; i < out.size(); ++i) {
        const QChar c = out.at(i);
        if (inString) {
            if (c == QLatin1Char('\\') && i + 1 < out.size()) {
                out[i + 1] = QLatin1Char(' ');
                ++i;
                continue;
            }
            if (c == QLatin1Char('"')) { inString = false; continue; }
            out[i] = QLatin1Char(' ');
        } else if (c == QLatin1Char('"')) {
            inString = true;
        }
    }
    return out;
}

struct QmlBlock { QString type; int brace; int close; };

QVector<QmlBlock> collectBlocks(const QString &blanked)
{
    QMap<int, QString> typeAt;
    static const QRegularExpression typeRe(QStringLiteral("\\b([A-Z]\\w*)\\s*\\{"));
    auto mi = typeRe.globalMatch(blanked);
    while (mi.hasNext()) {
        const auto m = mi.next();
        typeAt.insert(m.capturedEnd(0) - 1, m.captured(1));
    }
    QVector<QmlBlock> blocks;
    QVector<QPair<QString, int>> stack;
    for (int i = 0; i < blanked.size(); ++i) {
        const QChar c = blanked.at(i);
        if (c == QLatin1Char('{')) {
            stack.append({typeAt.value(i), i});
        } else if (c == QLatin1Char('}')) {
            if (stack.isEmpty()) continue;
            const auto f = stack.takeLast();
            if (!f.first.isEmpty()) blocks.append({f.first, f.second, i});
        }
    }
    return blocks;
}

// QtQuick.Controls types handle keys themselves; a MouseArea inside one is
// decoration on an already-operable control.
bool isControlType(const QString &t)
{
    static const QSet<QString> kControls = {
        QStringLiteral("Button"), QStringLiteral("ToolButton"),
        QStringLiteral("ItemDelegate"), QStringLiteral("MenuItem"),
        QStringLiteral("CheckBox"), QStringLiteral("RadioButton"),
        QStringLiteral("Switch"), QStringLiteral("Slider"),
        QStringLiteral("SpinBox"), QStringLiteral("ComboBox"),
        QStringLiteral("TextField"), QStringLiteral("TextArea"),
        QStringLiteral("TabButton"), QStringLiteral("RoundButton"),
        QStringLiteral("DelayButton"), QStringLiteral("Dial")};
    return kControls.contains(t);
}

// Keys at DEPTH ONE of the ancestor -- a route buried in a SIBLING child
// does not reach this handler, and counting it would be the same
// nested-child error depthOneBody exists to prevent. Booleans must read
// `true`: `keyNavigationEnabled: false` is what C-752 SET, deliberately,
// and it is not a keyboard route.
bool carriesKeyboardRoute(const QString &blanked, const QmlBlock &b)
{
    const QString body = depthOneBody(blanked, b.brace, nullptr);
    static const QRegularExpression keysRe(QStringLiteral("Keys\\.on\\w+\\s*:"));
    static const QRegularExpression tabRe(
        QStringLiteral("activeFocusOnTab\\s*:\\s*true"));
    static const QRegularExpression navRe(
        QStringLiteral("keyNavigationEnabled\\s*:\\s*true"));
    static const QRegularExpression scRe(QStringLiteral("\\bShortcut\\s*\\{"));
    return keysRe.match(body).hasMatch() || tabRe.match(body).hasMatch()
        || navRe.match(body).hasMatch() || scRe.match(body).hasMatch();
}

QStringList unroutedMouseHandlers(const QString &strippedSrc, const QString &file,
                                  int *acting)
{
    const QString blanked = blankStrings(strippedSrc);
    const QVector<QmlBlock> blocks = collectBlocks(blanked);
    static const QRegularExpression actRe(QStringLiteral(
        "(^|\\n|;)\\s*on(Clicked|Tapped|DoubleClicked|PressAndHold)\\s*:"));
    QStringList out;
    int n = 0;
    for (const QmlBlock &blk : blocks) {
        if (blk.type != QLatin1String("MouseArea")
            && blk.type != QLatin1String("TapHandler"))
            continue;
        const QString body = blanked.mid(blk.brace, blk.close - blk.brace + 1);
        if (!actRe.match(body).hasMatch())
            continue;
        ++n;
        bool routed = false;
        for (const QmlBlock &a : blocks) {
            if (a.brace >= blk.brace || a.close <= blk.close)
                continue;                       // strict ancestor only
            if (isControlType(a.type) || carriesKeyboardRoute(blanked, a)) {
                routed = true;
                break;
            }
        }
        if (!routed) {
            // ANCHOR BY objectName, NOT BY OCCURRENCE. Review D4 on the
            // labelling roster: positional anchors drift silently when a
            // new control of the same type lands earlier in the file, and
            // a roster row that drifts is worse than none.
            QString anchor;
            static const QRegularExpression onRe(
                QStringLiteral("(^|\\n|;)\\s*objectName\\s*:\\s*\"([^\"]+)\""));
            // THE HANDLER'S OWN NAME FIRST: curveArea names itself, and an
            // ancestor-only search threw that away for a positional "#1".
            {
                const auto self = onRe.match(depthOneBody(strippedSrc, blk.brace, nullptr));
                if (self.hasMatch())
                    anchor = self.captured(2);
            }
            for (const QmlBlock &a : blocks) {
                if (!anchor.isEmpty())
                    break;
                if (a.brace >= blk.brace || a.close <= blk.close)
                    continue;
                // READ THE UNBLANKED SOURCE HERE. blankStrings emptied every
                // literal to spaces -- which is exactly what the brace walk
                // needed and exactly what destroys the name being read. Same
                // offsets in both, and depthOneBody is string-aware itself.
                const auto m = onRe.match(depthOneBody(strippedSrc, a.brace, nullptr));
                if (m.hasMatch())
                    anchor = m.captured(2);     // nearest wins: inner-to-outer
            }
            out << QStringLiteral("%1:%2:%3")
                       .arg(file, anchor.isEmpty() ? QStringLiteral("#%1").arg(n) : anchor,
                            blk.type);
        }
    }
    if (acting)
        *acting += n;
    return out;
}

} // namespace

class TstA11yTreeLint : public QObject {
    Q_OBJECT
private slots:
    void everyInteractiveControlIsLabeled();
    void everyActingMouseHandlerHasAKeyboardRoute();
};

void TstA11yTreeLint::everyInteractiveControlIsLabeled()
{
    // Wave B CLOSED the roster: every queued id landed (74fd5e3) and was
    // adopted — zero unlabeled interactive controls is now the standing
    // §31.5-item-1 state. Any future hold goes back through an explicit
    // id-anchored row here plus a lane-log queue entry, never silently.
    const QSet<QString> expectedGaps;

    const QRegularExpression controlRe(QStringLiteral(
        "\\b(ToolButton|TabButton|RadioButton|RoundButton|DelayButton|"
        "Button|CheckBox|Switch|Slider|Dial|Tumbler|SpinBox|TextField|"
        "TextArea|ComboBox|MenuItem|ItemDelegate)\\s*\\{"));
    // Anchored at line start OR after `;` — the house single-line style
    // (`Switch { id: x; text: ... }`) is a labeled control too. Property
    // names merely ENDING in "text" (errorText:) cannot match: the
    // anchor+\s* leaves no way to start mid-identifier.
    const QRegularExpression labelRe(QStringLiteral(
        "(^|\\n|;)\\s*(text|Accessible\\.name)\\s*:"));
    // A literal-empty (or whitespace-only — delta-review residual) label
    // is no label (review D4). Dynamic bindings are trusted — only
    // literals are refutable statically.
    const QRegularExpression emptyLabelRe(QStringLiteral(
        "(^|\\n|;)\\s*(text|Accessible\\.name)\\s*:\\s*\"\\s*\"\\s*($|\\n|;)"));
    const QRegularExpression idRe(QStringLiteral(
        "(^|\\n|;)\\s*id\\s*:\\s*([A-Za-z_][A-Za-z0-9_]*)"));

    int filesScanned = 0;
    int controlsScanned = 0;
    QSet<QString> found;
    QStringList detail;

    QDirIterator it(QStringLiteral(APPNAME_QML_DIR),
                    {QStringLiteral("*.qml")}, QDir::Files,
                    QDirIterator::Subdirectories);
    while (it.hasNext()) {
        const QString path = it.next();
        const QString raw = readAll(path);
        QVERIFY2(!raw.isEmpty(), qPrintable(path + " unreadable"));
        ++filesScanned;
        const QString src = stripComments(raw);
        const QString file = path.section(QLatin1Char('/'), -1);

        // Stripper-capability guard: blank out double-quoted string
        // CONTENTS, then any surviving single quote or backtick means a
        // string form the stripper does not understand has entered the
        // QML — red loudly instead of desyncing silently (review D4).
        {
            QString blanked = src;
            static const QRegularExpression dq(QStringLiteral(
                "\"([^\"\\\\]|\\\\.)*\""));
            blanked.replace(dq, QStringLiteral("\"\""));
            QVERIFY2(!blanked.contains(QLatin1Char('\''))
                         && !blanked.contains(QLatin1Char('`')),
                     qPrintable(file
                                + QStringLiteral(": single-quoted/template "
                                                 "string outside \"\" — extend "
                                                 "stripComments before using "
                                                 "these forms")));
        }

        QHash<QString, int> occurrence;
        auto m = controlRe.globalMatch(src);
        while (m.hasNext()) {
            const auto match = m.next();
            const QString type = match.captured(1);
            const int idx = ++occurrence[type];
            ++controlsScanned;
            int close = 0;
            const QString body =
                depthOneBody(src, match.capturedEnd() - 1, &close);
            // Labeled = at least one label line that is NOT the empty
            // literal.
            int labels = 0;
            int empties = 0;
            for (auto lm = labelRe.globalMatch(body); lm.hasNext(); lm.next())
                ++labels;
            for (auto em = emptyLabelRe.globalMatch(body); em.hasNext();
                 em.next())
                ++empties;
            if (labels <= empties) {
                const auto idm = idRe.match(body);
                const QString key =
                    idm.hasMatch()
                        ? QStringLiteral("%1:%2:%3").arg(file, type,
                                                         idm.captured(2))
                        : QStringLiteral("%1:%2#%3").arg(file, type).arg(idx);
                found.insert(key);
                detail << key;
            }
        }
    }

    // Parse-knockout guards: a regex or stripper regression must red
    // LOUDLY, never pass vacuously.
    QVERIFY2(filesScanned >= 15,
             qPrintable(QStringLiteral("only %1 qml files scanned")
                            .arg(filesScanned)));
    QVERIFY2(controlsScanned >= 25,
             qPrintable(QStringLiteral("only %1 controls scanned — sweep "
                                       "knocked out?")
                            .arg(controlsScanned)));

    const QSet<QString> unexpected = found - expectedGaps;
    const QSet<QString> stale = expectedGaps - found;
    QVERIFY2(unexpected.isEmpty(),
             qPrintable(QStringLiteral("unlabeled interactive controls "
                                       "(§31.5 item 1): %1")
                            .arg(QStringList(unexpected.values())
                                     .join(QStringLiteral(", ")))));
    QVERIFY2(stale.isEmpty(),
             qPrintable(QStringLiteral("stale expected-gap roster rows "
                                       "(shrink the roster): %1")
                            .arg(QStringList(stale.values())
                                     .join(QStringLiteral(", ")))));
}

void TstA11yTreeLint::everyActingMouseHandlerHasAKeyboardRoute()
{
    // CONTROLS FIRST, ON LITERAL FIXTURES, so "zero unrouted in shell/qml"
    // below is a MEASUREMENT and not a property of a detector that cannot
    // fire. Each fixture isolates one way this check could be wrong.
    auto run = [](const char *qml) {
        int n = 0;
        return unroutedMouseHandlers(QString::fromUtf8(qml),
                                     QStringLiteral("fx"), &n);
    };

    // (1) POSITIVE: a bare MouseArea with no route anywhere above it.
    QCOMPARE(run("Item {\n"
                 "  Rectangle {\n"
                 "    MouseArea {\n"
                 "      onClicked: go()\n"
                 "    }\n"
                 "  }\n"
                 "}\n").size(), 1);

    // (2) NEGATIVE: an ancestor carrying the route.
    QVERIFY2(run("Item {\n"
                 "  activeFocusOnTab: true\n"
                 "  Keys.onPressed: function (e) {}\n"
                 "  Rectangle {\n"
                 "    MouseArea {\n"
                 "      onClicked: go()\n"
                 "    }\n"
                 "  }\n"
                 "}\n").isEmpty(),
             "a routed handler was flagged");

    // (3) NEGATIVE: inside a Control, which handles keys itself.
    QVERIFY2(run("Item {\n"
                 "  Button {\n"
                 "    MouseArea {\n"
                 "      onClicked: go()\n"
                 "    }\n"
                 "  }\n"
                 "}\n").isEmpty(),
             "a MouseArea inside a Control was flagged");

    // (4) `false` IS NOT A ROUTE. C-752 set keyNavigationEnabled: false
    // deliberately, so a substring test would have called that routed.
    QCOMPARE(run("Item {\n"
                 "  activeFocusOnTab: false\n"
                 "  keyNavigationEnabled: false\n"
                 "  Rectangle {\n"
                 "    MouseArea {\n"
                 "      onClicked: go()\n"
                 "    }\n"
                 "  }\n"
                 "}\n").size(), 1);

    // (5) A ROUTE IN A SIBLING DOES NOT REACH THIS HANDLER — the same
    // nested-child error depthOneBody exists to prevent, one level out.
    QCOMPARE(run("Item {\n"
                 "  Rectangle {\n"
                 "    activeFocusOnTab: true\n"
                 "  }\n"
                 "  Rectangle {\n"
                 "    MouseArea {\n"
                 "      onClicked: go()\n"
                 "    }\n"
                 "  }\n"
                 "}\n").size(), 1);

    // (6) A BRACE INSIDE A STRING LITERAL. Without blankStrings the walk
    // mis-nests and reports this ROUTED handler as unrouted. Seven files
    // under shell/qml carry such a literal, LayersPanel among them.
    QVERIFY2(run("Item {\n"
                 "  activeFocusOnTab: true\n"
                 "  Text {\n"
                 "    text: \"{name}, {hex}\"\n"
                 "  }\n"
                 "  Rectangle {\n"
                 "    MouseArea {\n"
                 "      onClicked: go()\n"
                 "    }\n"
                 "  }\n"
                 "}\n").isEmpty(),
             "a brace inside a string literal mis-nested the walk");

    // TWO-SIDED. C-751/752/753 fixed three instances; this row is the
    // fourth, and it is a HOLD rather than a defect:
    //
    //   refusalToast   the toast DISMISSES ITSELF on a 6 s timer, so the
    //                  click handler is an ACCELERATOR and not the only
    //                  route out. A keyboard user is never trapped by it,
    //                  and Escape is already spent three ways nearby
    //                  (canvas session cancel, dialog close, pane return).
    //
    // ★ THE ROW'S PREMISE IS PINNED BELOW, not just asserted here. A roster
    // row justified by a timer becomes a lie the moment the timer goes, and
    // nothing in a roster can notice that — so the timer is checked too.
    const QSet<QString> expectedGaps = {
        QStringLiteral("Main.qml:refusalToast:MouseArea")};

    int filesScanned = 0;
    int acting = 0;
    QSet<QString> found;

    QDirIterator it(QStringLiteral(APPNAME_QML_DIR),
                    {QStringLiteral("*.qml")}, QDir::Files,
                    QDirIterator::Subdirectories);
    while (it.hasNext()) {
        const QString path = it.next();
        const QString raw = readAll(path);
        QVERIFY2(!raw.isEmpty(), qPrintable(path + QStringLiteral(" unreadable")));
        ++filesScanned;
        const QString file = path.section(QLatin1Char('/'), -1);
        for (const QString &k : unroutedMouseHandlers(stripComments(raw), file, &acting))
            found.insert(k);
    }

    // Parse-knockout guards: a sweep that scanned nothing reports zero
    // defects and looks identical to a clean tree (the 516b538 lesson).
    QVERIFY2(filesScanned >= 15,
             qPrintable(QStringLiteral("only %1 qml files scanned").arg(filesScanned)));
    QVERIFY2(acting >= 4,
             qPrintable(QStringLiteral("only %1 acting mouse handlers found — "
                                       "sweep knocked out?").arg(acting)));

    // THE HELD ROW'S PREMISE, pinned: refusalToast must still dismiss
    // ITSELF. If the timer is removed or made endless, the only way out of
    // the toast becomes a mouse click and the roster row above silently
    // becomes false.
    {
        const QString main = stripComments(
            readAll(QStringLiteral(APPNAME_QML_DIR "/Main.qml")));
        QVERIFY2(!main.isEmpty(), "Main.qml unreadable");
        static const QRegularExpression timerRe(QStringLiteral(
            "id:\\s*refusalToastTimer[^}]*?interval:\\s*(\\d+)"));
        const auto m = timerRe.match(main);
        QVERIFY2(m.hasMatch(),
                 "refusalToast has no self-dismiss timer — the keyboard-route "
                 "roster row that holds it is now false");
        QVERIFY2(m.captured(1).toInt() > 0 && m.captured(1).toInt() <= 30000,
                 qPrintable(QStringLiteral("refusalToast self-dismiss interval is %1 ms")
                                .arg(m.captured(1))));
        QVERIFY2(main.contains(QLatin1String("onTriggered: refusalToast.visible = false")),
                 "refusalToastTimer no longer hides the toast");
    }

    const QSet<QString> unexpected = found - expectedGaps;
    const QSet<QString> stale = expectedGaps - found;
    QVERIFY2(unexpected.isEmpty(),
             qPrintable(QStringLiteral("mouse-only controls (no keyboard route in "
                                       "any ancestor): %1")
                            .arg(QStringList(unexpected.values())
                                     .join(QStringLiteral(", ")))));
    QVERIFY2(stale.isEmpty(),
             qPrintable(QStringLiteral("stale keyboard-route roster rows "
                                       "(shrink the roster): %1")
                            .arg(QStringList(stale.values())
                                     .join(QStringLiteral(", ")))));
}

QTEST_GUILESS_MAIN(TstA11yTreeLint)
#include "tst_a11y_tree_lint.moc"
