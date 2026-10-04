import QtQuick
import QtQuick.Controls

TextField {
    color: "#e6edf3"
    placeholderTextColor: "#7e91a5"
    selectByMouse: true
    selectionColor: "#3e786e"
    font.pixelSize: 15
    implicitHeight: 40
    leftPadding: 12
    background: Rectangle {
        radius: 6
        color: "#0e1722"
        border.color: parent.activeFocus ? "#86decb" : "#344355"
    }
}
