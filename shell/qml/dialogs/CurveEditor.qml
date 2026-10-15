// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// v1.0.1 item 4 class (c) — THE CURVE EDITOR.
//
// WHY THIS IS A CONTROL AND NOT A TABLE ENTRY, which is the whole reason
// the class-(c) estimate did not collapse the way (a) and (b) did.
// `$defs/pts` is an array of 2..16 points, each a 2-array of numbers in
// [0..1]. levels and hue_saturation were N groups of FIXED bounded
// scalars, so the grouping concept absorbed both; a curve is a
// VARIABLE-LENGTH ordered set of coordinate pairs where the COUNT IS
// PART OF THE DATA. No arrangement of spin rows expresses that.
//
// AND THE CHEAP VERSION WAS REFUSED ON PURPOSE. A 2-point curve is four
// numbers, which the scalar dialog could already render — `minItems: 2`
// would be satisfied and the engine would accept it. It would also be a
// control named Curves that CANNOT CURVE, meeting the acceptance row's
// letter while defeating its purpose.
//
// Bounds are the ENGINE's, transcribed here and pinned mechanically by
// tst_adjustment_table against ops.rs, exactly like every other row in
// this dialog.

import QtQuick
import AppnameShell   // Theme lives in the module, not in QtQuick
import QtQuick.Controls

Item {
    id: root
    implicitWidth: 240
    implicitHeight: 240

    // NOT caught by the §31.5 lint — its control list covers Buttons,
    // SpinBoxes and the like, and this is a bare Item wrapping a Canvas.
    // It is still an interactive control a screen reader would meet
    // unnamed, so it is named here rather than waiting for a lint that
    // happens to look for it. A guard's silence is not a clean bill.
    Accessible.role: Accessible.Canvas
    Accessible.name: AppI18n.tr("ui.dialog.adjust.curve_editor")

    // KEYBOARD (C-753). Every operation here was MOUSE-ONLY: add on press,
    // move on drag, remove on double-click. The a11y lint passed this file
    // because the lint checks LABELS and this control is properly labelled
    // -- so a screen reader announced "curve editor" and then nothing
    // responded, which is worse than an unlabelled control, not better.
    // B-03's own acceptance clause reads "keyboard-accessible controls".
    //
    //   Left / Right          select the previous / next point
    //   Up / Down             move the selected point's OUTPUT one step
    //   Shift + Up / Down     ... ten steps
    //   Shift + Left / Right  move the selected point's INPUT one step
    //   Delete / Backspace    remove the selected point (>= minPoints)
    //   + / =                 add midway to the next point (<= maxPoints)
    //
    // SELECTION IS UNMODIFIED AND MOVEMENT TAKES THE MODIFIER because
    // selection is the ENTRY gesture: a user who cannot select cannot do
    // anything else, so it must not require a chord. Output is the primary
    // edit on a tone curve, so it holds the bare vertical arrows.
    // LIMIT: coarse INPUT movement has no binding -- Shift is already spent
    // on the fine step there, and inventing a second modifier for the
    // rarest operation is not a call to make without a reason to.
    activeFocusOnTab: true
    Keys.onPressed: function (event) { root._onKey(event); }
    // The ring is conditioned on activeFocus, and a Canvas does not repaint
    // because a property it reads changed — it repaints when told to.
    onActiveFocusChanged: plot.requestPaint()

    // The engine's own limits (curves.v1.json $defs/pts).
    readonly property int minPoints: 2
    readonly property int maxPoints: 16

    // Points are [x, y] pairs in [0..1], kept sorted by x. y is drawn
    // INVERTED (0 at the bottom) because that is what every curve control
    // in this class of application does; the stored value is not flipped.
    property var points: [[0, 0], [1, 1]]
    signal pointsEdited()

    readonly property bool isIdentity: points.length === 2
                                       && points[0][0] === 0 && points[0][1] === 0
                                       && points[1][0] === 1 && points[1][1] === 1

    property int _dragIndex: -1
    readonly property real _hitPx: 10
    property int _selIndex: 0
    readonly property real _step: 1.0 / 256.0

    function _selectDelta(d) {
        if (root.points.length === 0)
            return;
        root._selIndex = Math.max(0, Math.min(root.points.length - 1,
                                              root._selIndex + d));
        plot.requestPaint();
    }
    function _moveSel(dx, dy) {
        if (root._selIndex < 0 || root._selIndex >= root.points.length)
            return;
        const pts = root.points.slice();
        const p = [Math.max(0, Math.min(1, pts[root._selIndex][0] + dx)),
                   Math.max(0, Math.min(1, pts[root._selIndex][1] + dy))];
        pts[root._selIndex] = p;
        root._commit(pts);
        // _commit SORTS BY X, so a point moved past a neighbour changes
        // index — the same hazard the drag path handles by re-deriving
        // _dragIndex from _nearest after every commit. Follow the point,
        // not the slot, or the next arrow press edits a different point.
        const at = root.points.indexOf(p);
        if (at >= 0)
            root._selIndex = at;
    }
    function _removeSel() {
        if (root.points.length <= root.minPoints)
            return;                       // the schema floor, agreed here
        const pts = root.points.slice();
        pts.splice(root._selIndex, 1);
        root._commit(pts);
        root._selIndex = Math.max(0, Math.min(root._selIndex, root.points.length - 1));
    }
    function _addAfterSel() {
        if (root.points.length >= root.maxPoints)
            return;                       // the schema ceiling
        const i = root._selIndex;
        if (i < 0 || i >= root.points.length - 1)
            return;                       // nothing to be midway TO
        const a = root.points[i], b = root.points[i + 1];
        const pts = root.points.slice();
        pts.push([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]);
        root._commit(pts);
        root._selIndex = i + 1;
    }
    function _onKey(event) {
        const shift = (event.modifiers & Qt.ShiftModifier) !== 0;
        const s = root._step;
        switch (event.key) {
        case Qt.Key_Left:
            if (shift) root._moveSel(-s, 0); else root._selectDelta(-1);
            break;
        case Qt.Key_Right:
            if (shift) root._moveSel(s, 0); else root._selectDelta(1);
            break;
        case Qt.Key_Up:    root._moveSel(0, shift ? s * 10 : s); break;
        case Qt.Key_Down:  root._moveSel(0, shift ? -s * 10 : -s); break;
        case Qt.Key_Delete:
        case Qt.Key_Backspace: root._removeSel(); break;
        case Qt.Key_Plus:
        case Qt.Key_Equal: root._addAfterSel(); break;
        default: return;                  // NOT accepted: let it propagate
        }
        event.accepted = true;
    }

    function _toView(p) { return Qt.point(p[0] * plot.width, (1 - p[1]) * plot.height); }

    readonly property real _ringR: 6

    // WHERE TO DRAW THE KEYBOARD CURSOR, which is NOT simply the point.
    // _toView([0,0]) is the plot's bottom-left CORNER and _selIndex defaults
    // to 0, so a ring centred on the point starts with three quarters of
    // itself outside the Canvas and clipped away — the cursor is least
    // visible exactly where it begins. Found by LOOKING at the committed
    // frame (C-758): no assertion in this tree could have said it, because
    // every test here checks values and the defect is geometry.
    //
    // Clamped, not centred: for a corner point the ring sits slightly inside
    // the point it marks. That is the standard trade and it is the right way
    // round — an indicator that is visible and 6px off beats one that is
    // exact and invisible.
    // Generalised from _ringCentre (C-770): the HANDLES have the same
    // corner problem the keyboard ring had — a dot centred on [0,0] or [1,1]
    // is half outside the Canvas and clipped. Each shape insets by its OWN
    // radius, so the ring and the dot it encircles stay concentric.
    function _insetCentre(vx, vy, r) {
        return [Math.max(r, Math.min(plot.width - r, vx)),
                Math.max(r, Math.min(plot.height - r, vy))];
    }
    function _ringCentre(vx, vy) {
        return root._insetCentre(vx, vy, root._ringR);
    }
    function _toDoc(x, y) {
        return [Math.max(0, Math.min(1, x / plot.width)),
                Math.max(0, Math.min(1, 1 - y / plot.height))];
    }
    function _nearest(x, y) {
        let best = -1, bestD = root._hitPx;
        for (let i = 0; i < points.length; ++i) {
            const v = _toView(points[i]);
            const d = Math.hypot(v.x - x, v.y - y);
            if (d <= bestD) { bestD = d; best = i; }
        }
        return best;
    }
    function _commit(pts) {
        pts.sort(function (a, b) { return a[0] - b[0]; });
        root.points = pts;
        plot.requestPaint();
        root.pointsEdited();
    }
    function resetToIdentity() {
        root.points = [[0, 0], [1, 1]];
        plot.requestPaint();
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.bgRaised
        border.color: Theme.borderInput
        border.width: 1

        Canvas {
            id: plot
            objectName: "curvePlot"
            anchors.fill: parent
            anchors.margins: 1
            onPaint: {
                const ctx = getContext("2d");
                ctx.reset();
                // quarter grid
                ctx.strokeStyle = Theme.border;
                ctx.lineWidth = 1;
                for (let i = 1; i < 4; ++i) {
                    const gx = width * i / 4, gy = height * i / 4;
                    ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, height); ctx.stroke();
                    ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(width, gy); ctx.stroke();
                }
                if (!root.points || root.points.length < 2)
                    return;
                // the curve: straight segments between control points.
                // NOT a spline — the engine owns interpolation, and a
                // shell that drew a smooth curve while sending the same
                // points would be showing the user a picture the engine
                // does not agree with.
                ctx.strokeStyle = Theme.accent;
                ctx.lineWidth = 2;
                ctx.beginPath();
                for (let i = 0; i < root.points.length; ++i) {
                    const v = root._toView(root.points[i]);
                    if (i === 0) ctx.moveTo(v.x, v.y); else ctx.lineTo(v.x, v.y);
                }
                ctx.stroke();
                // handles
                ctx.fillStyle = Theme.textPrimary;
                for (let j = 0; j < root.points.length; ++j) {
                    const h = root._toView(root.points[j]);
                    // Inset by the dot's own radius: an endpoint handle sits
                    // exactly on the plot corner, so a centred dot loses
                    // three quarters of itself to the clip. Observed in the
                    // committed frame at C-762 and deferred there; the
                    // keyboard ring got the same treatment at C-758.
                    const c = root._insetCentre(h.x, h.y, 3);
                    ctx.beginPath();
                    ctx.ellipse(c[0] - 3, c[1] - 3, 6, 6);
                    ctx.fill();
                }
                // The keyboard cursor, drawn ONLY while this control holds
                // focus: a ring visible to a mouse user would be a second
                // cursor that nothing moves.
                if (root.activeFocus && root._selIndex >= 0
                        && root._selIndex < root.points.length) {
                    const sv = root._toView(root.points[root._selIndex]);
                    const c = root._ringCentre(sv.x, sv.y);
                    const r = root._ringR;
                    ctx.strokeStyle = Theme.accent;
                    ctx.lineWidth = 2;
                    ctx.beginPath();
                    ctx.ellipse(c[0] - r, c[1] - r, 2 * r, 2 * r);
                    ctx.stroke();
                }
            }
        }

        MouseArea {
            id: area
            objectName: "curveArea"
            anchors.fill: plot
            acceptedButtons: Qt.LeftButton
            onPressed: function (mouse) {
                root._dragIndex = root._nearest(mouse.x, mouse.y);
                if (root._dragIndex < 0 && root.points.length < root.maxPoints) {
                    // ADD on empty space, capped at the engine's maxItems.
                    const pts = root.points.slice();
                    pts.push(root._toDoc(mouse.x, mouse.y));
                    root._commit(pts);
                    root._dragIndex = root._nearest(mouse.x, mouse.y);
                }
            }
            onPositionChanged: function (mouse) {
                if (root._dragIndex < 0)
                    return;
                const pts = root.points.slice();
                pts[root._dragIndex] = root._toDoc(mouse.x, mouse.y);
                root._commit(pts);
                root._dragIndex = root._nearest(mouse.x, mouse.y);
            }
            onReleased: root._dragIndex = -1
            // Keep the two cursors in agreement: a mouse edit moves the
            // keyboard selection with it, so tabbing back in resumes where
            // the user was rather than at point 0.
            onClicked: function (mouse) {
                const i = root._nearest(mouse.x, mouse.y);
                if (i >= 0) { root._selIndex = i; plot.requestPaint(); }
            }
            onDoubleClicked: function (mouse) {
                // REMOVE, floored at the engine's minItems. Refusing the
                // third removal is the shell agreeing with the schema
                // rather than sending a 1-point array to be rejected.
                const i = root._nearest(mouse.x, mouse.y);
                if (i < 0 || root.points.length <= root.minPoints)
                    return;
                const pts = root.points.slice();
                pts.splice(i, 1);
                root._commit(pts);
            }
        }
    }
}
