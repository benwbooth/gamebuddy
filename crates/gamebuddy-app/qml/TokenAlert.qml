import QtQuick
import QtQuick.Layouts

// Shown when an AI provider reports that the account is out of tokens,
// quota or credit, so a silent voice or failed answer has a visible reason.
Rectangle {
    id: alert
    property string message: ""
    readonly property bool tokens: message.startsWith("Out of tokens:")
    signal dismissed()
    visible: message.length > 0
    implicitHeight: content.implicitHeight + 20
    radius: 10; color: "#f23a1f1c"; border.color: "#e0806c"
    RowLayout {
        id: content
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 10 }
        spacing: 8
        Text { text: "⚠"; color: "#ffb4a3"; font.pixelSize: 18 }
        ColumnLayout {
            Layout.fillWidth: true; spacing: 2
            Text { objectName: "tokenAlertTitle"; text: alert.tokens ? "Out of tokens" : "Voice unavailable"; color: "#ffd6cc"; font.pixelSize: 14; font.weight: Font.DemiBold }
            Text {
                objectName: "tokenAlertMessage"
                text: alert.message.replace(/^(Out of tokens|Voice unavailable):\s*/, "")
                color: "#f3c3b8"; font.pixelSize: 12; textFormat: Text.PlainText
                wrapMode: Text.WordWrap; maximumLineCount: 4; elide: Text.ElideRight; Layout.fillWidth: true
            }
            Text { visible: alert.tokens; text: "Add credit or wait for the limit to reset, or switch providers in Models & speed."; color: "#c9a49b"; font.pixelSize: 11; wrapMode: Text.WordWrap; Layout.fillWidth: true }
        }
        Text {
            objectName: "tokenAlertDismiss"
            text: "✕"; color: "#ffd6cc"; font.pixelSize: 16
            MouseArea { anchors.fill: parent; anchors.margins: -6; cursorShape: Qt.PointingHandCursor; onClicked: alert.dismissed() }
        }
    }
}
