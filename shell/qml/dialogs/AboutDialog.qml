// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 the Appname contributors
//
// B-09. The credit set is NOT a design choice and was not composed from
// the roadmap's summary line: it is the THREE obligations THIRD-PARTY-
// NOTICES.md records against THIS surface, derived by classifying all ten
// of its `additional surface` entries (C-699).
//
//   libmypaint  :390  "About-screen credit line" — ISC, notice retention
//   lucide      :432  "the shell About dialog's third-party credits"
//   lensfun     :464  "credits/About-screen attribution … shell About"
//
// THE SEVEN THAT ARE *NOT* HERE, each for a stated reason, because an
// attribution surface is judged by what it omits:
//   rgb2spec   :404  belongs to the LUT BAKE TOOL's help/About — a
//                    DIFFERENT BINARY. The roadmap's B-09 line names it;
//                    the notices record places it elsewhere, and the row's
//                    own acceptance clause ("matches THIRD-PARTY-NOTICES")
//                    makes the notices file the authority.
//   ACES       :377  per-file header retention, not a screen
//   PyMatting  :397  courtesy citation in a vendor ATTRIBUTION.md
//   lensfun    :463,:465,:466,:467  notices-half, share-alike workflow,
//                    addendum basis, data-dir posture — all non-screen
//                    halves of the one obligation already carried above
//
// AND cgats21 IS DELIBERATELY ABSENT THOUGH ITS CATALOG KEY EXISTS
// (`ui.dialog.about.credit.cgats21_profiles`). Its record sits under
// "Pending — vendor drop not landed yet"; it activates only when
// data/icc-profiles/cgats21 lands. Crediting it today would attest to a
// component that is not in the product. The key stays minted and unused
// until the record activates — that is a smaller error than a false
// attestation, and it is the one that reverses cleanly.
//
// VERSION IS ABSENT BY THE SAME RULE, NOT BY OVERSIGHT (C-708). The shell
// declares THREE versions — project 0.3.0, bundle 0.5.0, short 0.5 —
// against a v1.0.2 release, and none reaches the app. Binding any of them
// would render a confident falsehood. The row is therefore bound to a
// property that is empty until something authoritative supplies it, and
// an empty version simply does not render. That is canvas_view's
// `transformPreviewUnavailable` posture: when there is no truth to show,
// show nothing and say so — never a plausible placeholder.

import QtQuick
import AppnameShell
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    modal: true                 // item-7 F4: the dismissal press is CONSUMED
    title: AppI18n.tr("ui.dialog.about.title")
    anchors.centerIn: parent
    padding: 12

    // Set at the instantiation site (Main.qml) from the build's version, now
    // that the operator has worded it (WORD 36 (a)). The dialog's OWN
    // default stays empty: unwired, it renders nothing rather than invent
    // one -- see the header note.
    property string appVersion: ""

    // The build's own commit, set at the instantiation site from the
    // AppBuildCommit context property. Empty here for the same reason
    // appVersion is: a dialog that INVENTS its own provenance is the
    // failure this file's header is about. It carries a `-dirty` suffix
    // when the tree had uncommitted changes — a sha alone would say
    // "these bytes" about a build that was "these bytes plus something
    // unrecorded".
    property string buildCommit: ""

    ColumnLayout {
        spacing: 8

        Label {
            visible: root.appVersion !== ""
            text: AppI18n.tr("ui.dialog.about.version", {"version": root.appVersion})
            Layout.maximumWidth: 420
            wrapMode: Text.WordWrap
        }

        Label {
            objectName: "aboutBuildCommit"
            visible: root.buildCommit !== ""
            text: AppI18n.tr("ui.dialog.about.commit", {"commit": root.buildCommit})
            Layout.maximumWidth: 420
            wrapMode: Text.WordWrap
        }

        Label {
            text: AppI18n.tr("ui.dialog.about.license")
            Layout.maximumWidth: 420
            wrapMode: Text.WordWrap
        }
        Label {
            text: AppI18n.tr("ui.dialog.about.no_affiliation")
            Layout.maximumWidth: 420
            wrapMode: Text.WordWrap
        }
        Label {
            text: AppI18n.tr("ui.dialog.about.trademarks")
            Layout.maximumWidth: 420
            wrapMode: Text.WordWrap
        }

        Label {
            text: AppI18n.tr("ui.dialog.about.acknowledgments")
            font.bold: true
            Layout.topMargin: 4
        }

        // The three recorded obligations, in the order the notices file
        // records them. objectName per row so a test can assert the SET
        // rather than a rendered blob (B-09's acceptance is O-WATCH's read
        // against THIRD-PARTY-NOTICES, and a set is what that compares).
        Repeater {
            model: ["libmypaint", "lucide_icons", "lensfun_database"]
            Label {
                objectName: "aboutCredit_" + modelData
                text: AppI18n.tr("ui.dialog.about.credit." + modelData)
                Layout.maximumWidth: 420
                wrapMode: Text.WordWrap
            }
        }

        // Qt's obligation is NOT a credit line like the three above: it is a
        // LICENCE ELECTION plus a relink right. `deps/qt6.yaml` elects
        // LGPL-3.0-only with DYNAMIC LINKING ONLY, and the recorded duty is
        // to preserve relink-ability at packaging "or document the user's
        // path to relink" (LGPL-3.0 s4(d)).
        //
        // THE WORDING STATES THE RIGHT, NOT THE PACKAGING. Whether the dmg
        // actually ships Qt as replaceable frameworks is E-03's to verify
        // and O-BUILD's to do; a sentence here asserting it would be this
        // desk vouching for another's artifact. Stating the user's relink
        // right is true whatever E-03 lands and is what the obligation asks.
        Label {
            objectName: "aboutQtLgpl"
            text: AppI18n.tr("ui.dialog.about.qt_lgpl")
            Layout.maximumWidth: 420
            wrapMode: Text.WordWrap
        }

        // The label's ellipsis already promised a door; this is it. Kept as
        // a Button rather than a link so it carries an accessible name from
        // its text (C-697) and is reachable by keyboard like every other
        // control here.
        Button {
            objectName: "aboutOpenLicenses"
            text: AppI18n.tr("ui.dialog.about.third_party_notices")
            // NOT gated on haveTexts. The dialog behind this door carries
            // the explanation for the empty case AND the pointer to
            // Help > Open Licenses Folder; disabling the door makes that
            // explanation unreachable and tells the user nothing.
            onClicked: licensesDialog.open()
        }

        LicensesDialog {
            id: licensesDialog
            objectName: "aboutLicensesDialog"
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            // A text Button takes its accessible name from `text`
            // (C-697), so no explicit Accessible.name is needed or wanted
            // here — adding one would duplicate the label to a reader.
            Button {
                objectName: "aboutCloseButton"
                text: AppI18n.tr("ui.dialog.close")
                onClicked: root.close()
            }
        }
    }
}
