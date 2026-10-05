import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: panel
    property var progress: null
    property double now: Date.now()
    readonly property bool loading: progress?.status === "loading"
    readonly property bool failed: progress?.status === "failed"
    readonly property bool ready: progress?.status === "ready"
    readonly property bool shown: loading || failed || (ready && now - (progress?.updated_at_ms || 0) < 2000)
    readonly property int total: progress?.total || 0
    readonly property int completed: progress?.completed || 0
    readonly property int elapsed: progress ? Math.max(0, Math.floor(((loading ? now : progress.updated_at_ms) - progress.started_at_ms) / 1000)) : 0
    visible: shown
    implicitHeight: content.implicitHeight + 24
    color: "#f5162c35"; radius: 10
    border.color: failed ? "#bc796b" : "#547f91a1"
    onProgressChanged: now = Date.now()
    Timer { interval: 250; repeat: true; running: panel.loading || (panel.ready && panel.shown); onTriggered: panel.now = Date.now() }
    ColumnLayout {
        id: content
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 12 }
        spacing: 7
        Text {
            objectName: "startupTitle"
            text: panel.failed ? "Visual tracking unavailable" : panel.ready ? "Visual tracking loaded" : "Starting visual tracking"
            color: panel.failed ? "#ffc4b9" : "#e6f1f6"; font.pixelSize: 14; font.weight: Font.DemiBold
            Layout.fillWidth: true; wrapMode: Text.WordWrap
        }
        Text {
            objectName: "startupStage"
            text: panel.progress?.message || ""
            color: "#b8cfda"; font.pixelSize: 13; textFormat: Text.PlainText
            Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight
        }
        Item {
            objectName: "startupProgressBar"
            Layout.fillWidth: true; Layout.preferredHeight: 6
            visible: !panel.failed
            Accessible.role: Accessible.ProgressBar
            Accessible.name: "Visual tracking startup"
            Accessible.description: panel.ready ? "Loaded" : panel.total ? "Stage " + (panel.completed + 1) + " of " + panel.total : "Starting"
            Row {
                anchors.fill: parent; spacing: 3; visible: panel.total > 0
                Repeater {
                    model: panel.total
                    Rectangle {
                        id: segment
                        required property int index
                        width: (parent.width - (panel.total - 1) * parent.spacing) / panel.total
                        height: 6; radius: 3
                        color: index <= panel.completed ? "#86decb" : "#35515a"
                        SequentialAnimation on opacity {
                            running: panel.loading && index === panel.completed && panel.visible
                            loops: Animation.Infinite
                            NumberAnimation { to: 0.3; duration: 550 }
                            NumberAnimation { to: 1; duration: 550 }
                            onRunningChanged: if (!running) segment.opacity = 1
                        }
                    }
                }
            }
            ProgressBar {
                anchors.fill: parent; visible: panel.total === 0
                indeterminate: panel.loading
                value: panel.ready ? 1 : 0
                palette.highlight: "#86decb"
            }
        }
        Text {
            objectName: "startupElapsed"
            text: panel.failed ? "You can keep playing while tracking is unavailable." : panel.ready ? "Loaded in " + panel.elapsed + " s · finding your location" : (panel.total ? "Stage " + (panel.completed + 1) + " of " + panel.total + " · " : "") + panel.elapsed + " s elapsed"
            color: "#91a9b9"; font.pixelSize: 12
            Layout.fillWidth: true; wrapMode: Text.WordWrap
        }
    }
}
