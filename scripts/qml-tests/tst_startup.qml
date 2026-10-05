import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 360; height: 200
    UI.StartupStatus { id: status; width: 340 }
    TestCase {
        name: "StartupStatus"; when: windowShown
        function init() { status.progress = null }
        function test_elapsed_time_does_not_invent_progress() {
            const start = Date.now()
            status.progress = {status:"loading",message:"Loading the graphics runtime…",completed:0,total:6,started_at_ms:start,updated_at_ms:start}
            verify(status.visible)
            verify(findChild(status,"startupProgressBar").visible)
            wait(1100)
            compare(status.completed,0)
            verify(status.elapsed >= 1)
            verify(findChild(status,"startupElapsed").text.indexOf("Stage 1 of 6") >= 0)
            verify(findChild(status,"startupStage").text.indexOf("graphics runtime") >= 0)
            grabImage(status).save("/tmp/gamebuddy-startup-loading.png")
        }
        function test_completion_clears_after_showing_loaded() {
            const now = Date.now()
            status.progress = {status:"ready",message:"Tracker loaded · locating your game view…",completed:6,total:6,started_at_ms:now-12000,updated_at_ms:now}
            verify(status.visible)
            compare(status.elapsed,12)
            compare(findChild(status,"startupTitle").text,"Visual tracking loaded")
            verify(findChild(status,"startupElapsed").text.indexOf("finding your location") >= 0)
            tryCompare(status,"visible",false,3000)
        }
        function test_failure_stays_visible_without_a_completed_bar() {
            const now = Date.now()
            status.progress = {status:"failed",message:"Could not load <model>",completed:2,total:6,started_at_ms:now-1000,updated_at_ms:now}
            verify(status.visible)
            verify(!findChild(status,"startupProgressBar").visible)
            compare(findChild(status,"startupTitle").text,"Visual tracking unavailable")
            compare(findChild(status,"startupStage").textFormat,Text.PlainText)
            grabImage(status).save("/tmp/gamebuddy-startup-failed.png")
        }
    }
}
