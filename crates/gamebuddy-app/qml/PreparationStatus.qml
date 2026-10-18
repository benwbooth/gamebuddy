import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// First-launch walkthrough preparation: every stage, its counts, and the
// pipeline's own messages, so long model and download work is never silent.
Rectangle {
    id: panel
    property var report: null
    property bool compact: false
    property bool showAllEvents: false
    // The new-game welcome: an opening line, the full stage list, what was
    // gathered once finished, and a way to tuck it away.
    property bool welcome: false
    signal dismissed()
    property double now: Date.now()
    signal retryRequested()
    readonly property bool running: report?.status === "running"
    readonly property bool failed: report?.status === "failed"
    readonly property bool partial: report?.status === "partial"
    readonly property bool ready: report?.status === "ready"
    readonly property bool shown: running || failed || partial || (ready && (welcome || now - (report?.updated_at_ms || 0) < 4000))
    readonly property var stages: report?.stages || []
    readonly property var active: stages.filter(s => s.status === "running")
    readonly property var current: active.filter(s => s.id !== "spatial").slice(-1)[0] || active[0] || null
    readonly property int finishedCount: stages.filter(s => ["done", "skipped", "attention"].indexOf(s.status) >= 0).length
    readonly property real overall: stages.length === 0 ? 0 : (ready || partial) ? 1
        : Math.min(1, (finishedCount + active.reduce((sum, s) => sum + (s.total ? Math.min(1, (s.done || 0) / s.total) : 0), 0)) / stages.length)
    readonly property var events: (report?.events || []).slice().reverse()
    visible: shown
    implicitHeight: content.implicitHeight + 24
    color: "#f5162c35"; radius: 10
    border.color: failed ? "#bc796b" : partial ? "#c9a46a" : "#547f91a1"
    onReportChanged: now = Date.now()
    Timer { interval: 500; repeat: true; running: panel.visible; onTriggered: panel.now = Date.now() }

    function clock(ms) {
        const s = Math.max(0, Math.floor(ms / 1000))
        return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }
    function icon(status) {
        return {done: "✓", running: "●", skipped: "–", attention: "!", failed: "✕"}[status] || "○"
    }
    function tint(status) {
        return {done: "#86decb", running: "#efc783", attention: "#e6b36f", failed: "#f0a291"}[status] || "#61798a"
    }
    function counts(stage) {
        if (stage.total) return stage.done + " / " + stage.total
        if (stage.done) return String(stage.done)
        return ""
    }
    function stageTime(stage) {
        if (!stage.started_at_ms) return ""
        return clock((stage.finished_at_ms || now) - stage.started_at_ms)
    }

    ColumnLayout {
        id: content
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 12 }
        spacing: 7
        RowLayout {
            Layout.fillWidth: true
            Text {
                objectName: "preparationTitle"
                text: panel.failed ? "Walkthrough setup stopped"
                    : panel.welcome && (panel.ready || panel.partial) ? (panel.report?.game || "Your game") + " is ready to play"
                    : panel.partial ? "Walkthrough ready · some steps need attention"
                    : panel.ready ? "Walkthrough ready"
                    : panel.welcome ? "Starting a new game: " + (panel.report?.game || "your game")
                    : "Preparing " + (panel.report?.game || "your game") + "'s walkthrough"
                color: panel.failed ? "#ffc4b9" : "#e6f1f6"; font.pixelSize: 14; font.weight: Font.DemiBold
                Layout.fillWidth: true; wrapMode: Text.WordWrap
            }
            Text {
                objectName: "preparationElapsed"
                text: panel.report ? panel.clock((panel.running ? panel.now : panel.report.updated_at_ms) - panel.report.started_at_ms) : ""
                color: "#91a9b9"; font.pixelSize: 12; font.family: "monospace"
            }
            Text {
                objectName: "preparationHide"
                visible: panel.welcome
                text: panel.running ? "Hide" : "Close"
                color: "#86decb"; font.pixelSize: 12; font.underline: true
                MouseArea { anchors.fill: parent; anchors.margins: -6; cursorShape: Qt.PointingHandCursor; onClicked: panel.dismissed() }
            }
        }
        Text {
            objectName: "preparationIntro"
            visible: panel.welcome && panel.running
            text: "I don't have anything saved for this game yet, so let me gather walkthrough details and materials first: the walkthrough itself, charts and data cards, maps and diagrams, and the screens I use to tell where you are. You can start playing while this runs; I'll tell you when it's ready."
            color: "#d4e3ec"; font.pixelSize: 13; textFormat: Text.PlainText
            Layout.fillWidth: true; wrapMode: Text.WordWrap
        }
        Column {
            objectName: "preparationGathered"
            visible: panel.welcome && (panel.ready || panel.partial) && (panel.report?.gathered || []).length > 0
            Layout.fillWidth: true; spacing: 3
            Text { text: "GATHERED"; color: "#86decb"; font.pixelSize: 11 }
            Repeater {
                model: panel.report?.gathered || []
                Text {
                    required property var modelData
                    width: parent.width
                    text: "✓  " + modelData
                    color: "#e3edf5"; font.pixelSize: 13; textFormat: Text.PlainText; wrapMode: Text.WordWrap
                }
            }
        }
        Text {
            objectName: "preparationCurrent"
            // The welcome's stage list already shows the running stage.
            visible: panel.running && !!panel.current && !panel.welcome
            text: panel.current ? panel.current.label + (panel.counts(panel.current) ? " · " + panel.counts(panel.current) : "") + (panel.current.detail ? "\n" + panel.current.detail : "") : ""
            color: "#b8cfda"; font.pixelSize: 13; textFormat: Text.PlainText
            Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: panel.compact ? 3 : 4; elide: Text.ElideRight
        }
        // One bar for the whole preparation: finished stages plus the running
        // stage's own fraction when it reports counts.
        ColumnLayout {
            Layout.fillWidth: true; spacing: 3
            Rectangle {
                objectName: "preparationBar"
                Layout.fillWidth: true; Layout.preferredHeight: 6; radius: 3; color: "#35515a"
                Accessible.role: Accessible.ProgressBar
                Accessible.name: "Walkthrough preparation"
                Accessible.description: Math.round(panel.overall * 100) + "% · " + panel.finishedCount + " of " + panel.stages.length + " steps finished"
                Rectangle {
                    width: parent.width * panel.overall; height: parent.height; radius: 3
                    color: panel.failed ? "#f0a291" : "#86decb"
                }
            }
            Text {
                objectName: "preparationStep"
                visible: panel.running && !!panel.current
                text: "Step " + (panel.finishedCount + 1) + " of " + panel.stages.length
                    + (panel.current && panel.current.total ? " · " + panel.current.done + " / " + panel.current.total : "")
                color: "#91a9b9"; font.pixelSize: 11
            }
        }
        Column {
            objectName: "preparationStages"
            visible: !panel.compact || (panel.welcome && !panel.ready)
            Layout.fillWidth: true; spacing: 6
            Repeater {
                model: panel.stages
                ColumnLayout {
                    required property var modelData
                    width: parent.width; spacing: 2
                    RowLayout {
                        Layout.fillWidth: true; spacing: 8
                        Text { text: panel.icon(modelData.status); color: panel.tint(modelData.status); font.pixelSize: 13; Layout.preferredWidth: 14; horizontalAlignment: Text.AlignHCenter }
                        Text { text: modelData.label; color: modelData.status === "pending" || modelData.status === "skipped" ? "#7d93a2" : "#e3edf5"; font.pixelSize: 13; Layout.fillWidth: true; elide: Text.ElideRight }
                        Text { text: [panel.counts(modelData), panel.stageTime(modelData)].filter(t => t).join(" · "); color: "#91a9b9"; font.pixelSize: 12; font.family: "monospace" }
                    }
                    Rectangle {
                        visible: modelData.status === "running" && !!modelData.total
                        Layout.fillWidth: true; Layout.leftMargin: 22; Layout.preferredHeight: 4
                        radius: 2; color: "#35515a"
                        Rectangle { width: parent.width * Math.min(1, (modelData.done || 0) / (modelData.total || 1)); height: parent.height; radius: 2; color: "#86decb" }
                    }
                    Text {
                        visible: !!modelData.detail && modelData.status !== "pending"
                        text: modelData.detail; textFormat: Text.PlainText
                        color: modelData.status === "attention" || modelData.status === "failed" ? "#e6b9a8" : "#91a9b9"
                        font.pixelSize: 12; wrapMode: Text.WordWrap; maximumLineCount: modelData.status === "running" ? 3 : 2; elide: Text.ElideRight
                        Layout.fillWidth: true; Layout.leftMargin: 22
                    }
                }
            }
        }
        RowLayout {
            visible: !panel.compact && panel.events.length > 0
            Layout.fillWidth: true; Layout.topMargin: 4
            Text { text: "ACTIVITY"; color: "#86decb"; font.pixelSize: 11; Layout.fillWidth: true }
            Text {
                visible: panel.events.length > 5
                text: panel.showAllEvents ? "Show less" : "Show all " + panel.events.length
                color: "#86decb"; font.pixelSize: 11; font.underline: true
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: panel.showAllEvents = !panel.showAllEvents }
            }
        }
        Column {
            objectName: "preparationEvents"
            visible: !panel.compact
            Layout.fillWidth: true; spacing: 3
            Repeater {
                model: panel.showAllEvents ? panel.events : panel.events.slice(0, 5)
                Text {
                    required property var modelData
                    width: parent.width
                    text: panel.clock(modelData.at_ms - panel.report.started_at_ms) + "  " + modelData.text
                    textFormat: Text.PlainText; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight
                    color: modelData.level === "error" ? "#f0a291" : modelData.level === "warning" ? "#e6c38f" : "#a9c0cf"
                    font.pixelSize: 12
                }
            }
        }
        Text {
            objectName: "preparationError"
            // The failed stage already shows the error in the full view.
            visible: panel.failed && !!panel.report?.error && (panel.compact || !panel.stages.some(s => s.status === "failed"))
            text: panel.report?.error || ""
            color: "#ffc4b9"; font.pixelSize: 12; textFormat: Text.PlainText
            Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: panel.compact ? 2 : 6; elide: Text.ElideRight
        }
        RowLayout {
            visible: panel.failed || panel.partial
            Layout.fillWidth: true
            Text {
                text: panel.failed ? "Retry, or import a walkthrough from Options." : "You can play now. Retry to finish the rest."
                color: "#91a9b9"; font.pixelSize: 12; wrapMode: Text.WordWrap; Layout.fillWidth: true
            }
            GbButton { objectName: "preparationRetry"; text: "Retry"; onClicked: panel.retryRequested() }
        }
    }
}
