// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 11e — the tool letter-key chord dispatcher. Header carries
// the design record; this file carries THE RULED TABLE and the event
// path. Every letter row cites Appendix C's own keymap line (the SPEC is
// the letter authority — RULE 8) and the palette model row its member
// wire ids were copied from (never retyped from catalog id names: the
// recorded false-name trap is ui.tool.polygon = the LASSO member vs
// ui.tool.shape_polygon = the shape).

#include "tool_key_dispatcher.h"

#include "canvas_view.h"

#include <QCoreApplication>
#include <QGuiApplication>
#include <QKeyEvent>
#include <QKeySequence>
#include <QQuickItem>
#include <QQuickWindow>

namespace appname::shell {

const QVector<ToolRing> &ToolKeyDispatcher::ringTable()
{
    // Ring order = the palette group model's DECLARED order
    // (ToolPalette.qml groups) — pinned against that model by
    // tst_palette_pins so this table can never drift from it silently.
    // Cycle semantics (desk ruling (c)): a bare letter activates the
    // ring's current member; bare-repeat AND Shift+letter both step
    // FORWARD with wrap, identically. The spec never says reverse.
    static const QVector<ToolRing> table = {
        // V — Appendix C tool.move = V (ES:18397). Model row: Move slot.
        {Qt::Key_V, "V", {QStringLiteral("move")}},
        // M — tool.marquee = M (ES:18398); the :18516 example cycles
        // "the marquee shapes". Members: the merged selection slot's
        // first two (item 17 census order).
        {Qt::Key_M, "M",
         {QStringLiteral("select.rect"), QStringLiteral("select.ellipse")}},
        // L — tool.lasso = L (ES:18399); :18516 "the lasso family".
        {Qt::Key_L, "L",
         {QStringLiteral("select.lasso"), QStringLiteral("select.polygon")}},
        // W — tool.quick_select = W (ES:18400, "magic wand / quick
        // select group"). 1.0 final (corrigendum (l)): Quick Selection is
        // removed, so the ring is the wand alone.
        {Qt::Key_W, "W", {QStringLiteral("select.wand")}},
        // C — tool.crop = C (ES:18401). Desk ruling (b): crop and slice
        // SHARE C letter-locally, per §32.3 :10576's own Crop/Slice
        // group fold. The ring therefore CROSSES two palette slots;
        // sharing a cycle letter is orthogonal to the shell's split
        // slots.
        // 1.0 final (corrigendum (l)): the slice tools are removed, so C
        // is Crop alone.
        {Qt::Key_C, "C", {QStringLiteral("crop")}},
        // B — tool.brush = B (ES:18403). :18516's example names
        // "brush/pencil/mixer"; pencil and mixer DO NOT EXIST in this
        // shell, so the ring is one member today and grows only if they
        // ever land. Ruling (b): invent nothing.
        {Qt::Key_B, "B", {QStringLiteral("brush.stroke")}},
        // E — tool.eraser = E (ES:18407). Ring of one.
        {Qt::Key_E, "E", {QStringLiteral("eraser.stroke")}},
        // J — tool.heal = J (ES:18406). RING OF ONE, and content_aware_
        // fill is a second DEVIATION-BY-SILENCE record beside S's.
        // ALIGNED at the 11e fix round (§5 minor-2), and the argument is
        // the desk's own, a fortiori: it ruled that :18516's NAMED
        // example "Shift+S the clone group" is too weak to enumerate a
        // ring. Appendix C says nothing whatever about content-aware
        // fill — total silence is weaker evidence than a named example,
        // so if the named one cannot carry a ring, silence certainly
        // cannot. Shipping CAF inside J while denying S its named family
        // would be two different readings of the same silence in one
        // table. The cost is real and is NOT hidden: CAF loses its
        // letter reach and joins the narrowed-but-OPEN D2 set recorded
        // in ToolPalette.qml. OQ-1 accordingly becomes a spec-side
        // question — does CAF join the J ring? — and a fold-in
        // candidate, never a shell election.
        {Qt::Key_J, "J", {QStringLiteral("heal.stroke")}},
        // S — tool.clone_stamp = S (ES:18405). RING OF ONE, and that is
        // a DESK RULING, not an oversight: Appendix C :18516's
        // "Shift+S the clone group" is an EXAMPLE CITATION, not a
        // membership enumeration. Reading a ring out of it would invent
        // exactly what ruling (b) forbids and would risk J and S
        // overlapping on the retouch family. Recorded as a
        // DEVIATION-BY-SILENCE with its citation — the same footing as
        // blur/sharpen/smudge taking no letter at all. Ringing S is a
        // spec-side election and a fold-in candidate, never a shell one.
        {Qt::Key_S, "S", {QStringLiteral("clone.stroke")}},
        // G — 1.0 final (corrigendum (l)): the paint bucket, G's only
        // member, is removed, so G is unbound until a working member lands.
        // O — tool.dodge_burn = O (ES:18409); :18516's example is an
        // exact three-member match (dodge/burn/sponge).
        {Qt::Key_O, "O",
         {QStringLiteral("dodge_burn.dodge"), QStringLiteral("dodge_burn.burn"),
          QStringLiteral("dodge_burn.sponge")}},
        // P — tool.pen = P (ES:18410). Ring of one.
        {Qt::Key_P, "P", {QStringLiteral("vector.pen")}},
        // T — 1.0 final (corrigendum (l)): the Text tool is removed, so T
        // is unbound; transform keeps Ctrl/Cmd+T (edit.free_transform).
        // U — tool.shape = U (ES:18413); :18516 "the shape kinds".
        // 1.0 final (corrigendum (l)): line, polygon and rounded rectangle
        // are removed; the ring is the two live shapes.
        {Qt::Key_U, "U",
         {QStringLiteral("shape.rect"), QStringLiteral("shape.ellipse")}},
    };
    return table;
}

// X — Appendix C tool.swap_colors = X (ES:18418); §32.3 :10578 puts the
// swap at the palette foot. ACTION chord, no ring, no spring arm.
// Shift+X stays deliberately UNBOUND (OQ-4).
//
// D (tool.default_colors = D, ES:18417) is DEFERRED this wave and is
// deliberately ABSENT from this file: there is no cataloged
// reset/default-colors control name, so shipping D would either mint an
// id (barred by ruling (e)'s zero-new-ids) or ship a keyboard-only
// affordance with no named visible control. Blocked on mint, not
// forgotten; ES:16452 already names D so the guard needs no rework when
// it arrives.
Qt::Key ToolKeyDispatcher::swapColorsKey()
{
    return Qt::Key_X;
}

QStringList ToolKeyDispatcher::ruledSequences()
{
    // The binding-sweep set of record: every sequence this dispatcher
    // claims. Shift rides EVERY ring letter including the ring-of-one
    // ones (V included — a ring-of-one Shift is a re-select; the entry's
    // sweep list omitted Shift+V while including Shift for E/P/T/B/S/G,
    // which was inconsistent by inspection). X takes no Shift.
    QStringList out;
    for (const ToolRing &ring : ringTable()) {
        out << QString::fromLatin1(ring.letter);
        out << (QStringLiteral("Shift+") + QString::fromLatin1(ring.letter));
    }
    out << QStringLiteral("X");
    return out;
}

ToolKeyDispatcher::ToolKeyDispatcher(QObject *parent)
    : QObject(parent)
{
    m_ringCurrent.fill(0, ringTable().size());
    m_springClock.start();
}

void ToolKeyDispatcher::attach(QQuickWindow *window)
{
    if (!window)
        return;
    m_window = window;
    m_canvasItem = window->findChild<QQuickItem *>(QStringLiteral("canvasView"));
    m_canvas = qobject_cast<CanvasView *>(m_canvasItem.data());
    if (m_canvas) {
        // Ring pointers follow the tool through EVERY source — palette
        // click, flyout item, menu, chord (:18516 "each group remembers
        // its last-used member for the session"). Same pattern the slot
        // face uses.
        connect(m_canvas, &CanvasView::toolChanged, this,
                &ToolKeyDispatcher::syncRings);
        syncRings();
    }
    window->installEventFilter(this);
}

QStringList ToolKeyDispatcher::ringMembers(const QString &letter) const
{
    for (const ToolRing &ring : ringTable())
        if (letter == QLatin1String(ring.letter))
            return ring.members;
    return {};
}

QString ToolKeyDispatcher::ringCurrent(const QString &letter) const
{
    const QVector<ToolRing> &table = ringTable();
    for (int i = 0; i < table.size(); ++i)
        if (letter == QLatin1String(table.at(i).letter))
            return table.at(i).members.value(m_ringCurrent.value(i));
    return {};
}

void ToolKeyDispatcher::syncRings()
{
    const QString tool = canvasTool();
    if (tool.isEmpty())
        return;
    const QVector<ToolRing> &table = ringTable();
    for (int i = 0; i < table.size(); ++i) {
        const int at = table.at(i).members.indexOf(tool);
        if (at >= 0)
            m_ringCurrent[i] = at;
    }
}

QString ToolKeyDispatcher::canvasTool() const
{
    return m_canvasItem ? m_canvasItem->property("tool").toString() : QString();
}

bool ToolKeyDispatcher::isRuledKey(const QKeyEvent *ke, int *ringIndex,
                                   bool *isSwap) const
{
    *ringIndex = -1;
    *isSwap = false;
    // Bare letters and Shift+letters ONLY — never a sequence carrying
    // Ctrl/Cmd/Alt/Meta. That is what keeps Ctrl+T, Cmd+D and the whole
    // §30.2 modifier table out of this path (they also resolve ahead of
    // it in the shortcut map; this is the belt to that braces).
    const Qt::KeyboardModifiers blockers = Qt::ControlModifier
                                           | Qt::AltModifier
                                           | Qt::MetaModifier
                                           | Qt::GroupSwitchModifier;
    if (ke->modifiers() & blockers)
        return false;
    const int key = ke->key();
    if (key == swapColorsKey()) {
        // Shift+X is unbound: decline it so nothing is consumed.
        if (ke->modifiers().testFlag(Qt::ShiftModifier))
            return false;
        *isSwap = true;
        return true;
    }
    const QVector<ToolRing> &table = ringTable();
    for (int i = 0; i < table.size(); ++i) {
        if (table.at(i).key == key) {
            *ringIndex = i;
            return true;
        }
    }
    return false;
}

bool ToolKeyDispatcher::focusClaimsKey(const QKeyEvent *ke) const
{
    // Qt's OWN arbitration, re-run: before the shortcut map fires, Qt
    // sends a ShortcutOverride to the focus object and yields if it is
    // accepted. Item delivery happens AFTER this window filter, so
    // without this probe the dispatcher would out-race focused items
    // that legitimately claim plain letters (an open ComboBox popup's
    // type-ahead is the measured case). Probing is side-effect free:
    // ShortcutOverride handlers only accept or ignore.
    QObject *focus = QGuiApplication::focusObject();
    if (!focus)
        return false;
    QKeyEvent probe(QEvent::ShortcutOverride, ke->key(), ke->modifiers(),
                    ke->text());
    probe.setAccepted(false);
    QCoreApplication::sendEvent(focus, &probe);
    return probe.isAccepted();
}

bool ToolKeyDispatcher::textEditingGuardEngaged() const
{
    // ES:16452 [NORMATIVE]: while a text/numeric field has focus (or a
    // Type-tool editing session is live on canvas) single-key and
    // Shift+letter shortcuts are suppressed — "they insert text". The
    // guard predicate stays defined ONCE, in QML (textEditingFocus,
    // duck-typed on inputMethodComposing), and is consulted here FIRST.
    // On engagement the dispatcher MUST NOT CONSUME: it declines, so the
    // key still reaches the focused editor and inserts. That is the
    // semantic the landed "a disabled Shortcut leaves the shortcut map"
    // mechanism had, carried through the migration deliberately — an
    // activation no-op would have consumed the key and typed nothing.
    return m_window && m_window->property("textEditingFocus").toBool();
}

bool ToolKeyDispatcher::captureInFlight() const
{
    // R3, ES:16446's closing rule [NORMATIVE]: "during pointer capture,
    // key state routes to the capturing tool and never switches tools".
    // Without this gate a letter mid-drag reaches CanvasView::setTool,
    // whose non-same-tool arm calls cancelGesture() and the gesture dies
    // — the live pre-existing violation this migration fixes.
    //
    // MEASURED SCOPE LIMIT, recorded deliberately (not discovered
    // later): gestureInFlight() is BUTTON-HELD state and deliberately
    // EXCLUDES click-SESSIONS (polygon lasso, pen) — they hold no
    // button. So a letter pressed mid-polygon or mid-pen still routes
    // into setTool and still cancels that session. NOT widened here, on
    // three grounds: (i) :16446's rule is worded "during pointer
    // capture", which a click-session is not, so widening would invent
    // scope the spec does not give; (ii) gestureInFlight() is the
    // context-menu's refuse-mid-gesture predicate, and widening IT would
    // silently change that unrelated contract; (iii) click-sessions
    // persist until Enter/Esc, so suppressing every tool letter for
    // their unbounded duration is itself a keyboard-reach cost. The
    // measured behaviour is PINNED by the net so it cannot drift
    // unnoticed, and the scope question is routed to the desk.
    return m_canvas && m_canvas->gestureInFlight();
}

void ToolKeyDispatcher::writeTool(const QString &toolId)
{
    // dispatchToolChord is the SOLE keymap tool writer, which is what
    // makes ruling (e)'s input-source exemption expressible: the funnel
    // marks the write chord-driven, so the announcement fires even
    // while a tool pane holds focus. The POINTER and MENU paths bypass
    // it deliberately (the palette slot click, the flyout item, the
    // Edit > Puppet Warp item) — they are not keymap dispatch and are
    // outside the exemption.
    // CORRECTION OF RECORD (11e fix round, §5 MAJOR-2 — logged, never
    // overwritten): this comment previously listed Ctrl+T among the
    // paths that "bypass" the funnel, and argued the exemption FROM
    // that list. Ctrl+T is a KEYMAP chord, so a bypassing Ctrl+T made
    // the sole-writer claim false and left ruling (e) half discharged —
    // measured: with focus inside the palette pane, Ctrl+T changed the
    // tool and announced nothing. Ctrl+T now routes through this same
    // funnel (Main.qml), which makes the claim above true rather than
    // merely stated.
    if (!m_window)
        return;
    QMetaObject::invokeMethod(m_window, "dispatchToolChord",
                              Q_ARG(QVariant, QVariant(toolId)));
}

void ToolKeyDispatcher::dispatchRing(int ringIndex)
{
    const ToolRing &ring = ringTable().at(ringIndex);
    int current = m_ringCurrent.value(ringIndex);
    if (current < 0 || current >= ring.members.size())
        current = 0;
    // §32.3 :10578 bare-repeat + Appendix C :18516 Shift+letter, one
    // motion: on the ring's current member, STEP forward (wrapping);
    // otherwise ACTIVATE the ring's remembered member.
    if (canvasTool() == ring.members.at(current))
        current = (current + 1) % ring.members.size();
    m_ringCurrent[ringIndex] = current;
    writeTool(ring.members.at(current));
}

void ToolKeyDispatcher::dispatchSwap()
{
    if (m_window)
        QMetaObject::invokeMethod(m_window, "dispatchColorSwap");
}

void ToolKeyDispatcher::handleRelease(const QKeyEvent *ke)
{
    const qint64 held = m_springClock.elapsed();
    const QString prior = m_spring.priorTool;
    m_spring.armed = false;
    m_spring.priorTool.clear();
    // ES:16446 [NORMATIVE]: "holding a tool letter >= 300 ms temporarily
    // switches (revert on release); a tap < 300 ms latches the tool."
    if (held < SpringHoldMs)
        return; // tap: latch, nothing to undo
    if (prior.isEmpty() || prior == canvasTool())
        return;
    writeTool(prior); // spring: restore, through the same single writer
}

bool ToolKeyDispatcher::eventFilter(QObject *watched, QEvent *event)
{
    if (!m_window || watched != m_window)
        return QObject::eventFilter(watched, event);
    const QEvent::Type type = event->type();
    if (type != QEvent::KeyPress && type != QEvent::KeyRelease)
        return QObject::eventFilter(watched, event);
    auto *ke = static_cast<QKeyEvent *>(event);
    int ringIndex = -1;
    bool isSwap = false;
    if (!isRuledKey(ke, &ringIndex, &isSwap))
        return false;

    if (type == QEvent::KeyRelease) {
        // Only ever consume the release of a press this dispatcher
        // owned — never swallow a release belonging to a focused editor
        // or a popup.
        if (!m_spring.armed || m_spring.key != ke->key())
            return false;
        if (ke->isAutoRepeat())
            return true; // repeat pairs: swallowed, arm stays live
        handleRelease(ke);
        return true;
    }

    if (focusClaimsKey(ke))
        return false; // the focused item claims plain letters — yield
    if (textEditingGuardEngaged())
        return false; // ES:16452 — decline WITHOUT consuming, so it inserts
    if (ke->isAutoRepeat())
        return true; // ES:16453: repeat is for nudges/steppers, not rings
    if (captureInFlight())
        return true; // R3: routed to the capturing tool, no tool switch

    if (isSwap) {
        dispatchSwap(); // action chord: no ring, no spring arm
        return true;
    }
    // Arm the spring BEFORE the write so priorTool is the tool the user
    // springs back to.
    m_spring.armed = true;
    m_spring.key = ke->key();
    m_spring.priorTool = canvasTool();
    m_springClock.restart();
    dispatchRing(ringIndex);
    return true;
}

} // namespace appname::shell
