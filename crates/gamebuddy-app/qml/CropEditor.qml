import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: editor
    required property var buddy
    property var preview: ({})
    property var selection: ({x:0,y:0,width:1,height:1})
    property bool drawNew: false
    property string error: ""
    property bool trace: false
    signal closed()
    function configure(value) {
        preview=value; selection=Object.assign({},value.ticket.crop)
        drawNew=selection.width===1 && selection.height===1; error=""
    }
    function clamp(value,min,max) { return Math.max(min,Math.min(max,value)) }
    function traceGeometry() {
        if(!trace || !visible || frame.status!==Image.Ready)return
        const p=plane.mapToItem(null,0,0),s=saveButton.mapToItem(null,saveButton.width/2,saveButton.height/2)
        const r=resetButton.mapToItem(null,resetButton.width/2,resetButton.height/2),c=cancelButton.mapToItem(null,cancelButton.width/2,cancelButton.height/2)
        console.info("GAMEBUDDY_CROP_EDITOR "+JSON.stringify({plane:[p.x,p.y,plane.width,plane.height],save:[s.x,s.y],reset:[r.x,r.y],cancel:[c.x,c.y]}))
    }
    onVisibleChanged: if(visible)Qt.callLater(traceGeometry)
    Rectangle { anchors.fill: parent; color: "#c0091019" }
    MouseArea { anchors.fill: parent } // modal: never send editing clicks into the game
    Rectangle {
        anchors.centerIn: parent
        width: Math.min(1160,parent.width-40); height: Math.min(760,parent.height-40)
        color: "#14202d"; radius: 14; border.color: "#4b6e75"
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 22; spacing: 14
            RowLayout {
                Text { text:"Crop game view";color:"#e6edf3";font.pixelSize:23;Layout.fillWidth:true }
                GbButton { id:cancelButton;text:"Cancel"; onClicked:editor.closed() }
            }
            Text {
                Layout.fillWidth:true;wrapMode:Text.WordWrap;color:"#a9c0cf";font.pixelSize:13
                text:"Select the play area inside the bezel. Drag inside to move it, or drag a corner to resize. Include the HUD if you want GameBuddy to read it."
            }
            RowLayout {
                Layout.fillWidth:true;Layout.fillHeight:true;spacing:18
                Rectangle {
                    Layout.fillWidth:true;Layout.minimumWidth:250;Layout.fillHeight:true;color:"#080f17";radius:8
                    Image {
                        id: frame
                        anchors.fill:parent;source:editor.preview.url || "";cache:false
                        fillMode:Image.PreserveAspectFit
                        onStatusChanged:if(status===Image.Ready)Qt.callLater(editor.traceGeometry)
                    }
                    Item {
                        id: plane
                        anchors.centerIn:parent;width:frame.paintedWidth;height:frame.paintedHeight
                        property real cropLeft: editor.selection.x*width
                        property real cropTop: editor.selection.y*height
                        property real cropRight: (editor.selection.x+editor.selection.width)*width
                        property real cropBottom: (editor.selection.y+editor.selection.height)*height
                        Rectangle { x:0;y:0;width:plane.width;height:plane.cropTop;color:"#99000000" }
                        Rectangle { x:0;y:plane.cropBottom;width:plane.width;height:plane.height-plane.cropBottom;color:"#99000000" }
                        Rectangle { x:0;y:plane.cropTop;width:plane.cropLeft;height:plane.cropBottom-plane.cropTop;color:"#99000000" }
                        Rectangle { x:plane.cropRight;y:plane.cropTop;width:plane.width-plane.cropRight;height:plane.cropBottom-plane.cropTop;color:"#99000000" }
                        Rectangle { x:plane.cropLeft;y:plane.cropTop;width:plane.cropRight-plane.cropLeft;height:plane.cropBottom-plane.cropTop;color:"transparent";border.color:"#86decb";border.width:2 }
                        Repeater {
                            model:[[0,0],[1,0],[0,1],[1,1]]
                            Rectangle {
                                required property var modelData
                                x:(modelData[0]?plane.cropRight:plane.cropLeft)-5;y:(modelData[1]?plane.cropBottom:plane.cropTop)-5
                                width:10;height:10;color:"#c7fff0";radius:2
                            }
                        }
                        MouseArea {
                            anchors.fill:parent;preventStealing:true;cursorShape:editor.drawNew?Qt.CrossCursor:Qt.SizeAllCursor
                            enabled:frame.status===Image.Ready
                            property var origin: ({})
                            property real startX:0
                            property real startY:0
                            property string mode:""
                            onPressed: mouse => {
                                startX=editor.clamp(mouse.x/width,0,1);startY=editor.clamp(mouse.y/height,0,1)
                                origin=Object.assign({},editor.selection)
                                const l=Math.abs(mouse.x-plane.cropLeft)<14,r=Math.abs(mouse.x-plane.cropRight)<14
                                const t=Math.abs(mouse.y-plane.cropTop)<14,b=Math.abs(mouse.y-plane.cropBottom)<14
                                mode=!editor.drawNew && (l||r) && (t||b)?(t?"n":"s")+(l?"w":"e"):
                                     !editor.drawNew && mouse.x>=plane.cropLeft && mouse.x<=plane.cropRight && mouse.y>=plane.cropTop && mouse.y<=plane.cropBottom?"move":"new"
                            }
                            onPositionChanged: mouse => {
                                if(!pressed)return
                                const x=editor.clamp(mouse.x/width,0,1),y=editor.clamp(mouse.y/height,0,1)
                                let l=origin.x,t=origin.y,r=l+origin.width,b=t+origin.height
                                if(mode==="move"){
                                    l=editor.clamp(origin.x+x-startX,0,1-origin.width);t=editor.clamp(origin.y+y-startY,0,1-origin.height)
                                    r=l+origin.width;b=t+origin.height
                                }else if(mode==="new"){
                                    l=Math.min(startX,x);t=Math.min(startY,y);r=Math.max(startX,x);b=Math.max(startY,y)
                                }else{
                                    if(mode.includes("w"))l=Math.min(x,r-0.01);else r=Math.max(x,l+0.01)
                                    if(mode.includes("n"))t=Math.min(y,b-0.01);else b=Math.max(y,t+0.01)
                                }
                                editor.selection={x:l,y:t,width:r-l,height:b-t}
                            }
                            onReleased:editor.drawNew=false
                        }
                    }
                }
                ColumnLayout {
                    Layout.minimumWidth:245;Layout.preferredWidth:245;Layout.maximumWidth:245;Layout.fillWidth:false;Layout.fillHeight:true;spacing:12
                    Text { text:"WHAT GAMEBUDDY WILL SEE";color:"#86decb";font.pixelSize:11 }
                    Rectangle {
                        Layout.fillWidth:true;Layout.fillHeight:true;color:"#080f17";radius:8
                        Image {
                            anchors.fill:parent;anchors.margins:4;source:editor.preview.url || "";cache:false
                            sourceClipRect:Qt.rect(editor.selection.x*(editor.preview.preview_width || 1),editor.selection.y*(editor.preview.preview_height || 1),Math.max(1,editor.selection.width*(editor.preview.preview_width || 1)),Math.max(1,editor.selection.height*(editor.preview.preview_height || 1)))
                            fillMode:Image.PreserveAspectFit
                        }
                    }
                    Text {
                        Layout.fillWidth:true;Layout.minimumWidth:0;color:"#a9c0cf";font.pixelSize:12;wrapMode:Text.WordWrap
                        text:Math.round(editor.selection.width*(editor.preview.width || 0))+" × "+Math.round(editor.selection.height*(editor.preview.height || 0))+" pixels selected\nSaved for "+(editor.preview.ticket?.game || "this game")+". Applies to screenshots and location matching."
                    }
                    GbButton { text:"New selection";Layout.fillWidth:true;onClicked:editor.drawNew=true }
                    GbButton { id:resetButton;text:"Reset to full frame";Layout.fillWidth:true;onClicked:{editor.selection={x:0,y:0,width:1,height:1};editor.drawNew=true} }
                }
            }
            Text { visible:editor.error.length>0;text:editor.error;color:"#ffb4a8";Layout.fillWidth:true;wrapMode:Text.WordWrap }
            RowLayout {
                Text { text:"Preview is frozen while you edit. Changes apply when saved.";color:"#91a3b6";font.pixelSize:12;Layout.fillWidth:true;wrapMode:Text.WordWrap }
                GbButton {
                    id:saveButton
                    text:"Save crop";accent:true
                    enabled:frame.status===Image.Ready && editor.selection.width>=0.01 && editor.selection.height>=0.01
                    onClicked:{editor.error=editor.buddy.save_game_crop(JSON.stringify({ticket:editor.preview.ticket,crop:editor.selection}));if(!editor.error.length)editor.closed()}
                }
            }
        }
    }
}
