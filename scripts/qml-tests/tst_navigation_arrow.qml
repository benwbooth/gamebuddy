import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 1280; height: 720
    Rectangle { anchors.fill: parent; color: "#223443" }
    UI.NavigationArrow { id: arrow }
    TestCase {
        name: "NavigationArrow"
        when: windowShown
        function init() { arrow.cue=null; arrow.enabled=true; arrow.occupiedRects=[] }
        function show(direction, label) {
            arrow.cue={direction:direction,label:label||"Head toward the next door",step_id:"leave-town",valid_until_ms:Date.now()+15000}
        }
        function test_screen_relative_directions_data() {
            return [{tag:"up",angle:0},{tag:"up_right",angle:45},{tag:"right",angle:90},{tag:"down_right",angle:135},{tag:"down",angle:180},{tag:"down_left",angle:225},{tag:"left",angle:270},{tag:"up_left",angle:315}]
        }
        function test_screen_relative_directions(data) {
            show(data.tag)
            verify(arrow.visible)
            compare(findChild(arrow,"navigationArrowIcon").rotation,data.angle)
            compare(findChild(arrow,"navigationArrowAction").text,"Head toward the next door")
        }
        function test_stale_unknown_and_hidden_cues_do_not_point() {
            show("right");arrow.enabled=false;verify(!arrow.visible)
            arrow.enabled=true;show("unknown");verify(!arrow.visible)
            arrow.cue=null;verify(!arrow.visible)
            arrow.cue={direction:"left",label:"Old route",valid_until_ms:Date.now()+100}
            tryCompare(arrow,"visible",false,1000)
        }
        function test_action_update_and_plain_text_rendering() {
            show("right");show("left","Take <the left door> & continue")
            compare(findChild(arrow,"navigationArrowIcon").rotation,270)
            compare(findChild(arrow,"navigationArrowAction").textFormat,Text.PlainText)
            show("right","Go through the next door")
            waitForRendering(arrow)
            grabImage(parent).save("/tmp/gamebuddy-navigation-arrow.png")
        }
        function test_moves_to_free_space_beside_open_widgets() {
            show("right","Next door")
            arrow.occupiedRects=[[250,20,600,670],[870,20,380,550],[36,90,80,80]]
            compare(arrow.width,220)
            verify(arrow.x+arrow.width<250)
            verify(arrow.y>170)
            verify(arrow.y+arrow.height<720)
            compare(findChild(arrow,"navigationArrowDirection").text,"Right")
            waitForRendering(arrow)
            grabImage(parent).save("/tmp/gamebuddy-navigation-arrow-compact.png")
        }
    }
}
