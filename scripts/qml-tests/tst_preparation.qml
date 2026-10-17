import QtQuick
import QtTest
import "../../crates/gamebuddy-app/qml" as UI

Item {
    width: 520; height: 760
    UI.PreparationStatus { id: status; width: 500 }
    SignalSpy { id: retry; target: status; signalName: "retryRequested" }
    SignalSpy { id: hidden; target: status; signalName: "dismissed" }
    function stage(id, label, status, extra) { return Object.assign({id:id,label:label,status:status,detail:"",done:null,total:null,started_at_ms:null,finished_at_ms:null}, extra || {}) }
    function report(state, stages, extra) {
        const now = Date.now()
        return Object.assign({game:"Faxanadu",status:state,started_at_ms:now-95000,updated_at_ms:now,stages:stages,events:[],error:null}, extra || {})
    }
    TestCase {
        name: "PreparationStatus"; when: windowShown
        function init() { status.report = null; status.compact = false; status.welcome = false; status.showAllEvents = false; retry.clear() }
        function test_running_shows_every_stage_counts_and_activity() {
            const now = Date.now()
            const events = []
            for (let i = 0; i < 8; i++) events.push({at_ms:now-60000+i*1000,stage:"sections",level:i===7?"warning":"info",text:"Building walkthrough: section "+(i+1)+" of 40"})
            status.report = report("running", [
                stage("source","Choose a walkthrough","done",{detail:"Faxanadu Full Walkthrough · NES Guides · 2:31:04",started_at_ms:now-95000,finished_at_ms:now-80000}),
                stage("download","Download video and captions","done",{detail:"Video and English captions downloaded"}),
                stage("references","Sample visual references","done",{done:812,total:812}),
                stage("spatial","Map the game world","running",{detail:"Verifying revisits: 3/9 views",started_at_ms:now-40000}),
                stage("sections","Write walkthrough sections","running",{detail:"Building walkthrough: section 8 of 40 · Eolis",done:7,total:40,started_at_ms:now-30000}),
                stage("atlas","Prepare location atlas","pending"),
                stage("maps","Register map images","pending")], {events:events})
            verify(status.visible)
            compare(status.current.id, "sections")
            const current = findChild(status, "preparationCurrent").text
            verify(current.indexOf("7 / 40") >= 0, current)
            verify(current.indexOf("Eolis") >= 0, current)
            compare(findChild(status, "preparationTitle").text, "Preparing Faxanadu's walkthrough")
            verify(findChild(status, "preparationElapsed").text.indexOf("1:3") === 0)
            compare(status.events.length, 8)
            verify(findChild(status, "preparationStages").visible)
            waitForRendering(status)
            grabImage(status).save("/tmp/gamebuddy-preparation-running.png")
        }
        function test_new_game_welcome_explains_and_lists_what_was_gathered() {
            const now = Date.now()
            status.compact = true
            status.welcome = true
            status.report = report("running", [
                stage("source","Find a walkthrough for this game","done",{detail:"Faxanadu NES - ULTIMATE GUIDE · 1:36:40"}),
                stage("download","Download the walkthrough video and captions","done",{detail:"Video and English captions downloaded"}),
                stage("references","Sample screens for recognizing where you are","done",{done:580,total:580}),
                stage("spatial","Map the game world","running",{detail:"Identifying game viewports, HUD masks, and camera type…",started_at_ms:now-40000}),
                stage("sections","Write the walkthrough steps","running",{detail:"Building walkthrough: section 8 of 40 · Eolis",done:7,total:40,started_at_ms:now-30000}),
                stage("charts","Find charts, tables and data cards","pending"),
                stage("atlas","Build the location atlas","pending"),
                stage("maps","Register maps and diagrams","pending")], {new_game:true})
            verify(status.visible)
            compare(findChild(status, "preparationTitle").text, "Starting a new game: Faxanadu")
            verify(findChild(status, "preparationIntro").visible)
            verify(findChild(status, "preparationStages").visible)
            waitForRendering(status)
            grabImage(status).save("/tmp/gamebuddy-preparation-welcome.png")
            status.report = report("ready", status.report.stages.map(s => Object.assign({}, s, {status:"done"})), {new_game:true, updated_at_ms:now-60000,
                gathered:["Walkthrough: Faxanadu NES - ULTIMATE GUIDE","412 walkthrough steps","45 data cards and 1 chart","118 places in the location atlas"]})
            verify(status.visible, "A welcome stays up after finishing until closed")
            compare(findChild(status, "preparationTitle").text, "Faxanadu is ready to play")
            verify(!findChild(status, "preparationIntro").visible)
            verify(findChild(status, "preparationGathered").visible)
            waitForRendering(status)
            grabImage(status).save("/tmp/gamebuddy-preparation-welcome-ready.png")
            hidden.clear()
            mouseClick(findChild(status, "preparationHide"))
            compare(hidden.count, 1)
        }
        function test_compact_shows_only_the_current_stage() {
            status.compact = true
            status.report = report("running", [stage("source","Choose a walkthrough","running",{detail:"Asking your reasoning model to assess 12 candidates…"})])
            verify(status.visible)
            verify(!findChild(status, "preparationStages").visible)
            verify(!findChild(status, "preparationEvents").visible)
            verify(findChild(status, "preparationCurrent").text.indexOf("12 candidates") >= 0)
        }
        function test_failure_explains_and_offers_retry() {
            status.report = report("failed", [stage("source","Choose a walkthrough","failed",{detail:"No suitable long-form walkthrough candidates were found"})], {error:"No suitable long-form walkthrough candidates were found"})
            verify(status.visible)
            compare(findChild(status, "preparationTitle").text, "Walkthrough setup stopped")
            verify(!findChild(status, "preparationError").visible, "Failed stage already shows the error")
            status.compact = true
            verify(findChild(status, "preparationError").visible)
            status.compact = false
            mouseClick(findChild(status, "preparationRetry"))
            compare(retry.count, 1)
            waitForRendering(status)
            grabImage(status).save("/tmp/gamebuddy-preparation-failed.png")
        }
        function test_ready_clears_but_partial_stays() {
            status.report = report("ready", [stage("source","Choose a walkthrough","done")])
            verify(status.visible)
            tryCompare(status, "visible", false, 6000)
            status.report = report("partial", [stage("atlas","Prepare location atlas","attention",{detail:"Location atlas preparation can be retried"})])
            verify(status.visible)
            verify(findChild(status, "preparationRetry").visible)
        }
    }
}
