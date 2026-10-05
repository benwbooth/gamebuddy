// Runs only inside the private Qt reader fixture using Chromium's input path.
import assert from 'node:assert/strict';
export async function imageInputs({call,evaluate,until},selector){
  const metrics=()=>evaluate(`(()=>{const v=document.querySelector(${JSON.stringify(selector)}),s=v.querySelector('.map-stage'),r=v.getBoundingClientRect(),b=s.getBoundingClientRect();return {x:r.x+v.clientWidth*.48,y:r.y+Math.min(v.clientHeight,180)*.55,width:b.width,height:b.height,left:b.left,top:b.top,scrollX:v.scrollLeft,scrollY:v.scrollTop}})()`);
  await until(()=>evaluate(`document.querySelector(${JSON.stringify(selector)})?.querySelector('img')?.naturalWidth>0`),'Image not loaded');
  // Center the viewport on screen before injecting native events.
  await evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center'})`);
  await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
  const before=await metrics();
  await call('Input.dispatchMouseEvent',{type:'mouseWheel',x:before.x,y:before.y,deltaX:0,deltaY:-110,modifiers:2});
  await until(async()=> (await metrics()).width>before.width*1.5,'Ctrl+wheel did not zoom image');
  const zoomed=await metrics();
  assert(Math.abs((before.x-before.left)/before.width-(before.x-zoomed.left)/zoomed.width)<.02,'Zoom moved the point under the pointer: '+JSON.stringify({before,zoomed}));
  await call('Input.dispatchMouseEvent',{type:'mouseWheel',x:zoomed.x,y:zoomed.y,deltaX:73,deltaY:59});
  await until(async()=>{const m=await metrics();return m.scrollX>zoomed.scrollX+20&&m.scrollY>zoomed.scrollY+20},'Diagonal pixel scrolling lost an axis');
  const scrolled=await metrics();
  await call('Input.dispatchMouseEvent',{type:'mousePressed',x:scrolled.x,y:scrolled.y,button:'left',buttons:1,clickCount:1});
  await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:scrolled.x-45,y:scrolled.y-35,button:'left',buttons:1});
  await call('Input.dispatchMouseEvent',{type:'mouseReleased',x:scrolled.x-45,y:scrolled.y-35,button:'left',clickCount:1});
  await until(async()=>{const m=await metrics();return m.scrollX>scrolled.scrollX+30&&m.scrollY>scrolled.scrollY+20},'Mouse image pan did not retain both axes');
  // Qt's CDP synthesizePinchGesture dereferences its unimplemented synthetic
  // gesture target. Feed Chromium's Ctrl+wheel pinch path directly instead.
  const pinch=await metrics();
  await call('Input.dispatchMouseEvent',{type:'mouseWheel',x:pinch.x,y:pinch.y,deltaX:0,deltaY:-30,modifiers:2});
  await until(async()=> (await metrics()).width>pinch.width*1.1,'Pinch wheel path did not zoom the image');
  assert.equal(await evaluate('visualViewport.scale'),1,'Pinch magnified the whole overlay');
  assert(await evaluate(`!document.querySelector(${JSON.stringify(selector)}).classList.contains('panning')`),'Pointer capture remained stuck');
  console.log('PASS image gestures:',selector);
}

export async function widgetInputs(api){
  const {call,evaluate,until}=api;
  await evaluate(`testState.widget_visibility={'location-map':true,'walkthrough-outline':false,'guide-library':false,'current-step':false};testState.navigation.map.selected_map_id='town';testRender(testState);document.querySelector('[data-widget-id="location-map"]').style.cssText+=';left:20px;top:20px;width:650px;height:650px;';document.querySelector('.map-viewport').style.cssText='flex:none;height:200px'`);
  await imageInputs(api,'.map-viewport');
  const pan=await evaluate(`(()=>{const v=document.querySelector('.map-viewport');window.gestureMap=v;return [v.scrollLeft,v.scrollTop,v.querySelector('.map-stage').style.width]})()`);
  await evaluate(`testState.navigation.map.geometric_marker={map_id:'town',region:{x:.55,y:.3,width:.15,height:.4},precision:'camera_footprint'};testRender(testState)`);
  assert.deepEqual(await evaluate(`(()=>{const v=document.querySelector('.map-viewport');return [v.scrollLeft,v.scrollTop,v.querySelector('.map-stage').style.width]})()`),pan,'Live marker moved a manually browsed map');
  assert(await evaluate(`document.querySelector('.map-toolbar button[aria-pressed]').getAttribute('aria-pressed')==='false'`),'Manual map input kept following player');
  // Two direct contacts pinch and pan together on a touchscreen.
  const at=await evaluate(`(()=>{const r=gestureMap.getBoundingClientRect();return{x:r.x+180,y:r.y+80}})()`);
  const width=await evaluate(`gestureMap.querySelector('.map-stage').clientWidth`);
  const touch=(x,id)=>({x,y:at.y,id,radiusX:2,radiusY:2});
  await call('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[touch(at.x-30,1),touch(at.x+30,2)]});
  await call('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[touch(at.x-50,1),touch(at.x+50,2)]});
  await call('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await until(()=>evaluate(`gestureMap.querySelector('.map-stage').clientWidth>${width*1.25}`),'Two-contact pinch did not zoom');
  assert.equal(await evaluate('visualViewport.scale'),1);
  await evaluate(`gestureMap.focus()`);
  await call('Input.dispatchKeyEvent',{type:'keyDown',key:'0',code:'Digit0',windowsVirtualKeyCode:48});
  await call('Input.dispatchKeyEvent',{type:'keyUp',key:'0',code:'Digit0',windowsVirtualKeyCode:48});
  assert(await evaluate(`gestureMap.scrollLeft===0&&gestureMap.scrollTop===0`),'Fit did not reset pan');
  // Ordinary text and wide tables keep native scrolling and their controls.
  await evaluate(`testState.widget_visibility={'location-map':false,'walkthrough-outline':false,'guide-library':false,'current-step':false,'scroll-text':true,'scroll-table':true};testState.widgets=[{id:'scroll-text',title:'Scrollable text',content:{kind:'text',body:Array.from({length:60},(_,i)=>'Line '+i).join('\\n')}},{id:'scroll-table',title:'Wide table',content:{kind:'table',columns:Array.from({length:15},(_,i)=>'Column '+i),rows:Array.from({length:30},()=>Array.from({length:15},(_,i)=>'LongColumnValue'+i))}}];testRender(testState);for(const [i,n] of [...document.querySelectorAll('.widget')].entries())n.style.cssText='left:'+(20+i*380)+'px;top:20px;width:350px;height:300px'`);
  for(const id of ['scroll-text','scroll-table']){
    const pos=await evaluate(`(()=>{const v=document.querySelector('[data-widget-id="${id}"] .body'),r=v.getBoundingClientRect();return{x:r.x+100,y:r.y+70}})()`);
    await call('Input.dispatchMouseEvent',{type:'mouseWheel',...pos,deltaX:id==='scroll-table'?93:0,deltaY:83});
    await until(()=>evaluate(`(()=>{const v=document.querySelector('[data-widget-id="${id}"] .body');return v.scrollTop>20${id==='scroll-table'?'&&v.scrollLeft>20':''}})()`),'Native scrolling failed for '+id);
    if(id==='scroll-table'){
      const old=await evaluate(`document.querySelector('[data-widget-id="scroll-table"] .body').scrollLeft`);
      await call('Input.dispatchMouseEvent',{type:'mouseWheel',...pos,deltaX:0,deltaY:65,modifiers:8});
      await until(()=>evaluate(`document.querySelector('[data-widget-id="scroll-table"] .body').scrollLeft>${old+20}`),'Shift+wheel did not scroll horizontally');
    }
  }
  assert.equal(await evaluate('visualViewport.scale'),1);
  const tablePan=await evaluate(`(()=>{const b=document.querySelector('[data-widget-id="scroll-table"] .body');return[b.scrollLeft,b.scrollTop]})()`);
  await evaluate(`testState.widgets[1].content.rows[0][0]='UpdatedValue';testRender(testState)`);
  await until(()=>evaluate(`document.querySelector('[data-widget-id="scroll-table"] .body').scrollLeft===${tablePan[0]}`),'Refreshing content reset horizontal scroll');
  assert.deepEqual(await evaluate(`(()=>{const b=document.querySelector('[data-widget-id="scroll-table"] .body');return[b.scrollLeft,b.scrollTop]})()`),tablePan);
  console.log('PASS native two-axis text/table scrolling, Shift+wheel, touch pinch, keyboard fit and manual map continuity');
}
