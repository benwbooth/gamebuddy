import QtQuick
import QtQuick.Controls

Button {
    id: control
    property bool accent: false
    implicitHeight: 40
    implicitWidth: Math.max(80, label.implicitWidth + 28)
    leftPadding: 14
    rightPadding: 14
    background: Rectangle {
        radius: 7
        color: control.accent ? (control.down ? "#57b5a8" : "#86decb") : (control.hovered ? "#293746" : "#1c2938")
        opacity: control.enabled ? 1 : 0.4
        border.color: control.activeFocus ? "#86decb" : "#354458"
    }
    contentItem: Text {
        id: label
        text: control.text
        color: control.accent ? "#102b29" : "#dce6ee"
        opacity: control.enabled ? 1 : 0.4
        font.pixelSize: 14
        font.weight: Font.DemiBold
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
}
