import QtQuick

Rectangle {
    id: navigation
    property var cue: null
    property double now: Date.now()
    property bool trace: false
    property string lastTrace: ""
    property var occupiedRects: []
    readonly property var placement: {
        const w=parent ? parent.width : 1280, h=parent ? parent.height : 720
        const widths=[Math.min(460,w-40),Math.min(220,w-40)]
        for (const width of widths) {
            const top=Math.max(24,h*0.1), bottom=Math.max(24,h-128)
            const candidates=[[(w-width)/2,top],[(w-width)/2,bottom],[20,bottom],[w-width-20,bottom],[20,top],[w-width-20,top]]
            for (const p of candidates) {
                const clear=occupiedRects.every(r=>p[0]+width+8<=r[0] || p[0]>=r[0]+r[2]+8 || p[1]+104+8<=r[1] || p[1]>=r[1]+r[3]+8)
                if (clear) return {x:p[0],y:p[1],width:width}
            }
        }
        return {x:(w-widths[0])/2,y:Math.max(24,h*0.1),width:widths[0]}
    }
    readonly property var directions: ({
        up: [0, "Up"], up_right: [45, "Up and right"], right: [90, "Right"],
        down_right: [135, "Down and right"], down: [180, "Down"],
        down_left: [225, "Down and left"], left: [270, "Left"], up_left: [315, "Up and left"]
    })
    readonly property var heading: cue ? directions[cue.direction] : null
    visible: enabled && !!heading && Number(cue?.valid_until_ms || 0) > now
    x: placement.x; y: placement.y
    width: placement.width
    height: Math.max(88, actionLabel.implicitHeight + 42)
    radius: 14
    color: "#ed101f2a"
    border.color: "#86decb"
    Accessible.role: Accessible.StaticText
    Accessible.name: visible ? "Next movement: " + heading[1] + ". " + cue.label : ""
    function record() {
        if (!trace) return
        const state = JSON.stringify({visible:visible,direction:cue?.direction||null,step_id:cue?.step_id||null})
        if (state !== lastTrace) { lastTrace=state; console.warn("GAMEBUDDY_NAVIGATION_ARROW "+state) }
    }
    onCueChanged: { now=Date.now();Qt.callLater(record) }
    onVisibleChanged: record()
    Timer { interval: 200; repeat: true; running: navigation.enabled && !!navigation.cue; onTriggered: navigation.now = Date.now() }
    Canvas {
        objectName: "navigationArrowIcon"
        width: 64; height: 64
        anchors { left: parent.left; leftMargin: 12; verticalCenter: parent.verticalCenter }
        rotation: navigation.heading ? navigation.heading[0] : 0
        onPaint: {
            const c = getContext("2d")
            c.clearRect(0, 0, width, height)
            c.beginPath(); c.moveTo(32, 5); c.lineTo(58, 31); c.lineTo(42, 31)
            c.lineTo(42, 58); c.lineTo(22, 58); c.lineTo(22, 31); c.lineTo(6, 31); c.closePath()
            c.fillStyle = "#b5ffe9"; c.fill()
        }
    }
    Text {
        id: directionLabel
        objectName: "navigationArrowDirection"
        anchors { left: parent.left; leftMargin: 88; top: parent.top; topMargin: 12; right: parent.right; rightMargin: 14 }
        text: navigation.heading ? (navigation.width < 360 ? "" : (navigation.cue?.source === "geometry" ? "On the map · " : "Next movement · ")) + navigation.heading[1] + (navigation.cue?.source === "geometry" && navigation.cue?.target && navigation.width >= 360 ? " toward " + navigation.cue.target : "") : ""
        textFormat: Text.PlainText
        color: "#86decb"; font.pixelSize: 15; font.weight: Font.DemiBold
        elide: Text.ElideRight
    }
    Text {
        id: actionLabel
        objectName: "navigationArrowAction"
        anchors { left: directionLabel.left; right: directionLabel.right; top: directionLabel.bottom; topMargin: 3 }
        text: navigation.cue?.label || ""
        textFormat: Text.PlainText
        color: "#f1f7fa"; font.pixelSize: 20; font.weight: Font.Medium
        wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
    }
    // No pointer handlers: the cue is display-only and never steals game input.
}
