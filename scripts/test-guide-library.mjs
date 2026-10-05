// Opt-in integration test against a private Qt WebEngine debug port and the
// guide_library_fixture database. Never point this at a user's live session.
import assert from 'node:assert/strict';
import {existsSync,readFileSync,writeFileSync} from 'node:fs';
import {DatabaseSync} from 'node:sqlite';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {imageInputs,widgetInputs} from './test-widget-gestures.mjs';
const [endpoint,root]=process.argv.slice(2);
assert(endpoint&&root,'Usage: node scripts/test-guide-library.mjs http://127.0.0.1:PORT TEMP_DATA_DIR');
assert(new URL(endpoint).hostname==='127.0.0.1','Only a private local test port is supported');
const db=new DatabaseSync(root+'/gamebuddy.sqlite');
const doc=key=>{const row=db.prepare('SELECT body FROM documents WHERE key=?').get(key);return row?JSON.parse(row.body):null};
const id=doc('active_guide');
const guide=()=>JSON.parse(db.prepare('SELECT body FROM guides WHERE id=?').get(id).body);
assert.equal(guide().title,'Reader integration fixture','Refusing to modify non-fixture data');
const progress=()=>doc('progress:'+id+':'+guide().revision);
const targets=await(await fetch(endpoint+'/json/list')).json();
const target=targets.find(t=>t.title==='GameBuddy widgets');assert(target,'Qt widget page missing');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})});
let sequence=0;const pending=new Map(),errors=[];
socket.addEventListener('message',event=>{const message=JSON.parse(event.data);if(message.id){const task=pending.get(message.id);pending.delete(message.id);if(message.error)task.reject(new Error(JSON.stringify(message.error)));else task.resolve(message.result)}else if(message.method==='Runtime.exceptionThrown')errors.push(message.params)});
function call(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});socket.send(JSON.stringify({id,method,params}))})}
async function evaluate(expression){const result=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(result.exceptionDetails)throw new Error(JSON.stringify(result.exceptionDetails));return result.result.value}
async function until(check,message){const start=Date.now();while(!await check()){assert(Date.now()-start<15000,message);await new Promise(r=>setTimeout(r,100))}}
async function click(selector){const rect=await evaluate(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;n.scrollIntoView({block:'nearest'});const r=n.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()`);assert(rect,'Missing '+selector);await call('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',clickCount:1,...rect});await call('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...rect})}
async function button(label){const selector=await evaluate(`(()=>{const n=[...document.querySelectorAll('.library button')].find(n=>n.textContent===${JSON.stringify(label)});if(!n)return null;n.dataset.testTarget='yes';return '[data-test-target="yes"]'})()`);assert(selector,'Missing button '+label);await click(selector);await evaluate(`document.querySelector('[data-test-target="yes"]')?.removeAttribute('data-test-target')`)}
async function screenshot(name){const result=await call('Page.captureScreenshot',{format:'png'});writeFileSync(root+'/'+name+'.png',Buffer.from(result.data,'base64'))}
try {
  await call('Runtime.enable');
  await until(()=>evaluate(`document.querySelector('.library')?.textContent.includes('12 pictures')`),'Manifest did not arrive through Qt');
  await until(()=>evaluate(`document.querySelectorAll('.reader-gallery img').length>0 && [...document.querySelectorAll('.reader-gallery img')].every(n=>n.complete&&n.naturalWidth>0)`),'Guide pictures did not load');
  const position=progress().current_step;
  await button('Shop');
  assert.equal(progress().current_step,position,'Browsing changed live position');
  await button('☆ Bookmark');await until(()=>doc('reader:'+id)?.bookmarks.includes('step-3'),'Bookmark not persisted');
  await click('.reader-detail input[type=checkbox]');await until(()=>progress().completed.includes('step-3'),'Checkbox not persisted');
  await until(()=>evaluate(`!document.querySelector('.reader-detail input').disabled`),'Checkbox remained disabled');
  await click('.reader-detail input[type=checkbox]');await until(()=>!progress().completed.includes('step-3'),'Uncheck not persisted');
  await until(()=>evaluate(`!document.querySelector('.reader-detail input').disabled`),'Checkbox remained disabled');
  await button('Make this a goal');await until(()=>guide().goals.some(g=>g.step_ids.includes('step-3')),'Goal missing from shared guide');
  await until(()=>evaluate(`document.querySelector('.reader-detail')?.textContent.includes('Remove my goal')`),'Goal revision did not refresh');
  await button('My goals');assert(await evaluate(`document.querySelector('.reader-content').textContent.includes('Buy the key') || document.querySelector('.reader-content').textContent.includes('Shop')`));
  await button('All pictures');await until(()=>evaluate(`document.querySelectorAll('.reader-gallery img').length===8 && [...document.querySelectorAll('.reader-gallery img')].every(n=>n.complete&&n.naturalWidth>0)`),'First picture page missing');
  await button('Next pictures');await until(()=>evaluate(`document.querySelectorAll('.reader-gallery img').length===4 && [...document.querySelectorAll('.reader-gallery img')].every(n=>n.complete&&n.naturalWidth>0)`),'Second picture page missing');
  await click('.reader-gallery img');assert(await evaluate(`!!document.querySelector('.reader-lightbox img')`));
  await imageInputs({call,evaluate,until},'.reader-lightbox .picture-viewport');await screenshot('picture-gestures');await button('Close picture');
  await button('All maps');await until(()=>evaluate(`document.querySelector('[data-reader-map]')?.src.startsWith('data:image/jpeg') && document.querySelector('[data-reader-map]').naturalWidth>0`),'Cached map bridge failed');
  assert(await evaluate(`document.querySelector('.reader-content').textContent.includes('Generated fixture author')`));await screenshot('reader-map');
  await imageInputs({call,evaluate,until},'.reader-map-view');await screenshot('map-gestures');
  await button('Sources');assert(await evaluate(`document.querySelector('.reader-content').textContent.includes('Reader integration fixture')`));
  await button('How it was built');assert(await evaluate(`document.querySelector('.reader-content').textContent.includes('Fixture source review')`));await screenshot('reader-history');
  await button('Current step');assert.equal(progress().current_step,position);await screenshot('reader-outline');
  await click('[aria-label="Minimize Reader fixture · Guide library"]');await until(()=>doc('widget-layout')['guide-library'].collapsed,'Minimize not saved');
  assert.equal(await evaluate(`Math.round(document.querySelector('[data-widget-id="guide-library"]').getBoundingClientRect().height)`),64);
  await click('[aria-label="Restore Reader fixture · Guide library"]');await until(()=>!doc('widget-layout')['guide-library'].collapsed,'Restore not saved');
  await click('[aria-label="Dismiss Reader fixture · Guide library"]');await until(()=>doc('widget-visibility:'+id)['guide-library']===false,'Dismiss not saved');
  await until(()=>evaluate(`!document.querySelector('.library')`),'Dismissed widget still visible');
  // Equivalent persisted preference to the companion tray checkbox; the native
  // tray and game input routing are exercised by scripts/test-gamescope.
  const prefs=doc('widget-visibility:'+id);prefs['guide-library']=true;db.prepare('UPDATE documents SET body=? WHERE key=?').run(JSON.stringify(prefs),'widget-visibility:'+id);
  await until(()=>evaluate(`!!document.querySelector('.library')`),'Widget did not return');
  await button('Bookmarks');assert(await evaluate(`document.querySelector('.reader-tree').textContent.includes('Shop')`));
  // Exercise the same harness mutation used by speech/chat, then observe the
  // persisted result through the actual Qt polling bridge.
  const beforeAdvice=progress();
  const advice='Speak to the shopkeeper and buy the key before leaving town.';
  execFileSync(fileURLToPath(new URL('../target/debug/gamebuddy-cli',import.meta.url)),[
    '--data-dir',root,'tool','set_next_action',JSON.stringify({expected_revision:guide().revision,epoch:beforeAdvice.epoch,step_id:'step-3',instructions:advice})
  ]);
  assert.deepEqual(progress(),beforeAdvice,'Advice must not claim progress');
  // Keep subsequent controlled render checks isolated from the app refresh.
  await evaluate(`window.testRender=gamebuddy.render;gamebuddy.render=state=>{window.testState=state;window.testRender(state)}`);
  await until(()=>evaluate(`window.testState?.navigation.guidance.now.instructions===${JSON.stringify(advice)}`),'Published advice did not reach the Qt widget state');
  await evaluate(`gamebuddy.render=()=>{};testState.widget_visibility={'walkthrough-outline':true,'location-map':true,'current-step':false,'guide-library':false};testRender(testState)`);
  assert(await evaluate(`document.querySelector('.outline-next')?.textContent.includes(testState.navigation.guidance.now.instructions)`),'Next instructions are not pinned');
  assert(await evaluate(`document.querySelector('[aria-current=step]').dataset.stepId==='step-3' && document.querySelector('.outline-next').textContent.includes(${JSON.stringify(advice)})`),'Outline did not follow conversation advice');
  await screenshot('shared-next-action');
  await evaluate(`const current=document.querySelector('[data-widget-id="walkthrough-outline"] [aria-current=step]');current.click();let parent=current.closest('details');while(parent){parent.open=false;parent=parent.parentElement.closest('details')}`);
  await until(()=>evaluate(`[...document.querySelectorAll('[data-widget-id="walkthrough-outline"] details')].filter(d=>d.querySelector('[aria-current=step]')).every(d=>d.open)`),'Current path can be collapsed');
  assert(await evaluate(`document.querySelector('[data-widget-id="walkthrough-outline"] [aria-current=step]').parentElement.querySelector('.instructions')?.textContent===testState.navigation.guidance.now.instructions`),'Current action click hid its instructions');
  // Reproduce frequent live observations while holding the same atlas image.
  await evaluate(`testState.navigation.map.current_place=testState.navigation.map.places[0];testState.navigation.map.selected_map_id='town';testState.navigation.map.geometric_marker={map_id:'town',region:{x:.1,y:.1,width:.2,height:.5},precision:'camera_footprint'};testRender(testState)`);
  await until(()=>evaluate(`document.querySelector('.map-viewport img')?.src.startsWith('data:image/jpeg') && document.querySelector('.map-viewport img').naturalWidth>0`),'Fixture map missing');
  await evaluate(`window.stableMap=document.querySelector('[data-widget-id="location-map"]');window.stableImage=stableMap.querySelector('.map-viewport img');window.mapSrcChanges=0;window.mapObserverTest=new MutationObserver(changes=>mapSrcChanges+=changes.filter(c=>c.attributeName==='src').length);mapObserverTest.observe(stableImage,{attributes:true});stableMap.querySelector('.spatial-panel').open=true`);
  await click('[aria-label="Location map"]');
  for(let tick=0;tick<6;tick++){
    await evaluate(`testState.navigation.location.detail='Observation '+${tick};testState.navigation.map.spatial={preparation:{status:'ready'},observation:{tracker:{status:'candidate',observed_at_ms:${tick}}},emulator:{age_ms:${tick*1000}}};testState.navigation.map.geometric_marker.region.x=${.1+tick*.02};testRender(testState)`);
    assert(await evaluate(`stableMap===document.querySelector('[data-widget-id="location-map"]') && stableImage===document.querySelector('.map-viewport img')`),'Live refresh replaced the map or image DOM');
    await new Promise(resolve=>setTimeout(resolve,100));
  }
  assert.equal(await evaluate('mapSrcChanges'),0,'Live updates reloaded the map image');
  assert(await evaluate(`stableMap.querySelector('.map-marker').style.left==='20%' && stableMap.querySelector('.spatial-panel').open && stableMap.querySelector('.picker.open')!==null`),'Marker updates or open controls were lost');
  await click('[aria-label="Location map"]');
  await click('[aria-label="Zoom in"]');
  await evaluate(`{const viewport=stableMap.querySelector('.map-viewport');viewport.scrollLeft=24;viewport.scrollTop=31}`);
  await new Promise(resolve=>setTimeout(resolve,100));
  await evaluate(`{const viewport=stableMap.querySelector('.map-viewport');window.savedMapPan={x:viewport.scrollLeft,y:viewport.scrollTop,width:stableImage.parentElement.style.width};testState.navigation.map.spatial.emulator={age_ms:9999};testRender(testState)}`);
  assert(await evaluate(`(()=>{const viewport=stableMap.querySelector('.map-viewport');return (savedMapPan.x>0||savedMapPan.y>0)&&viewport.scrollLeft===savedMapPan.x&&viewport.scrollTop===savedMapPan.y&&stableImage.parentElement.style.width===savedMapPan.width})()`),'Metadata-only refresh reset map pan or zoom');
  await evaluate(`testState.navigation.map.geometric_marker=null;testState.navigation.map.current_place=null;testState.navigation.map.selected_map_id=null;testRender(testState)`);
  assert(await evaluate(`stableMap.querySelector('.map-marker').hidden && stableImage===document.querySelector('.map-viewport img')`),'Stale location must hide its marker without flashing the image');
  await click('[aria-label="Minimize Reader fixture · Location map"]');
  await evaluate('testRender(testState)');
  await click('[aria-label="Restore Reader fixture · Location map"]');
  await evaluate('testRender(testState);mapObserverTest.disconnect()');
  assert(await evaluate(`stableImage.parentElement.style.width!=='' && !stableImage.parentElement.style.width.includes('NaN')`),'Hidden refresh corrupted the map zoom');
  // Optional production state obtained by the isolated saved-frame pipeline test.
  const fixtureState=await evaluate('testState');
  if(process.env.GAMEBUDDY_TEST_SPATIAL_OUTPUT){
    const replay=JSON.parse(readFileSync(process.env.GAMEBUDDY_TEST_SPATIAL_OUTPUT,'utf8'));
    const preview=replay.map_preview;delete replay.map_preview;
    replay.widget_visibility={'walkthrough-outline':true,'location-map':true,'current-step':false,'guide-library':false};
    await evaluate(`window.testState=${JSON.stringify(replay)};testRender(testState)`);
    await evaluate(`gamebuddy.mapPreview(${JSON.stringify(replay.navigation.map.selected_map_id)},${JSON.stringify(preview)})`);
    await until(()=>evaluate(`document.querySelector('.map-stage img')?.naturalWidth>0`),'Replay map image missing');
    assert(await evaluate(`document.querySelector('.map-status').textContent.includes(testState.navigation.map.current_place.label)`),'Named player location missing');
    assert(await evaluate(`!document.querySelector('.map-marker').hidden`),'Player area is not highlighted');
    assert.equal(await evaluate(`document.querySelector('.outline-next strong').textContent`),replay.navigation.guidance.now.title);
    assert(await evaluate(`![...document.querySelectorAll('[data-step-id]')].some(n=>/ · \\d+:\\d+/.test(n.textContent))`),'Source intervals leaked into player tree');
    const changed={...replay.navigation.guidance.now,parent_id:replay.navigation.player_steps.find(s=>s.id===replay.navigation.guidance.now.id).parent_id,id:'ui-next-action',title:'Next action after progress update',instructions:'Continue to the next door.'};
    await evaluate(`testState.navigation.player_steps.push(${JSON.stringify(changed)});testState.navigation.guidance.now=${JSON.stringify(changed)};testRender(testState)`);
    assert(await evaluate(`document.querySelector('[aria-current=step]').dataset.stepId==='ui-next-action' && document.querySelector('.outline-next').textContent.includes('Continue to the next door.')`),'Outline did not follow changed next action');
    await evaluate(`testState.navigation.guidance.now=${JSON.stringify(replay.navigation.guidance.now)};testRender(testState);gamebuddy.mapPreview(${JSON.stringify(replay.navigation.map.selected_map_id)},${JSON.stringify(preview)})`);
    await click('[aria-label="Location map"]');
    assert.equal(await evaluate(`document.querySelector('[aria-label="Location map choices"]').children.length`),replay.navigation.map.locations.length);
    const alternative=replay.navigation.map.locations.find(p=>p.map_id&&p.map_id!==replay.navigation.map.selected_map_id);
    await click(`[aria-label="Location map choices"] [data-value="${alternative.id}"]`);
    assert(await evaluate(`document.querySelector('[aria-label="Location map"]').textContent.includes(${JSON.stringify(alternative.label)})`),'Separate location map selection failed');
    await evaluate(`document.querySelector('.map-toolbar button[aria-pressed]').click();gamebuddy.mapPreview(${JSON.stringify(replay.navigation.map.selected_map_id)},${JSON.stringify(preview)})`);
    await until(()=>evaluate(`(()=>{const marker=document.querySelector('.map-marker'),viewport=document.querySelector('.map-viewport');if(!marker||marker.hidden)return false;const m=marker.getBoundingClientRect(),v=viewport.getBoundingClientRect();return m.x+m.width/2>v.x&&m.x+m.width/2<v.right&&m.y+m.height/2>v.y&&m.y+m.height/2<v.bottom})()`),'Follow player did not scroll the highlighted area into view');
    await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:5,y:5});
    await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
    await until(()=>evaluate(`document.querySelector('.provenance').hidden`),'Source tooltip covered the highlight');
    const updatesPath=process.env.GAMEBUDDY_TEST_SPATIAL_OUTPUT+'.updates.json';
    const updates=existsSync(updatesPath)?JSON.parse(readFileSync(updatesPath,'utf8')).map(({map,location})=>({map,location})):[];
    const continuity=await evaluate(`new Promise((resolve,reject)=>{
      const updates=${JSON.stringify(updates)};
      const widget=document.querySelector('[data-widget-id="location-map"]'),marker=widget.querySelector('.map-marker'),image=widget.querySelector('.map-stage img');
      let ticks=0,frames=0,hidden=0,offscreen=0,replaced=0,occluded=0,done=false;
      function inspect(){if(done)return;frames++;const m=marker.getBoundingClientRect(),v=widget.querySelector('.map-viewport').getBoundingClientRect();
        if(marker.hidden||!m.width||!m.height||!image.naturalWidth)hidden++;
        if(m.right<=v.x||m.x>=v.right||m.bottom<=v.y||m.y>=v.bottom)offscreen++;
        const front=document.elementFromPoint(m.x+m.width/2,m.y+m.height/2);if(front!==marker&&!marker.contains(front))occluded++;
        if(marker!==document.querySelector('.map-marker'))replaced++;
        requestAnimationFrame(inspect)}
      requestAnimationFrame(inspect);
      const timeout=setTimeout(()=>{clearInterval(timer);done=true;reject(Error('Camera-area audit timed out'))},20000);
      const timer=setInterval(()=>{try{if(ticks<updates.length)Object.assign(testState.navigation,updates[ticks]);else{testState.navigation.map.spatial.message=ticks%2?'':'Starting visual tracking…';if(ticks%4===3)testState.navigation.map.geometric_marker.region.x+=.00001}testRender(testState);
        if(++ticks===updates.length+40){clearInterval(timer);clearTimeout(timeout);done=true;resolve({ticks,frames,hidden,offscreen,replaced,occluded})}}catch(error){clearInterval(timer);clearTimeout(timeout);done=true;reject(error)}},100)
    })`);
    assert(continuity.frames>40&&continuity.hidden===0&&continuity.offscreen===0&&continuity.replaced===0&&continuity.occluded===0,'Camera highlight flickered in Qt: '+JSON.stringify(continuity));
    console.log('PASS Qt camera-area visibility:',continuity);
    await screenshot('faxanadu-player-navigation');
    await evaluate(`testState.progress.completed=[testState.guide.steps.find(s=>s.id===testState.navigation.guidance.now.id).parent_id];testState.widget_visibility={'guide-library':true,'walkthrough-outline':false,'location-map':false,'current-step':false};testRender(testState)`);
    assert(await evaluate(`document.querySelector('.reader-detail input[type=checkbox]').checked`),'Flattening source sections lost inherited completion');
    await evaluate(`window.testState=${JSON.stringify(fixtureState)};testRender(testState)`);
  }
  await widgetInputs({call,evaluate,until});
  assert.deepEqual(errors,[],'JavaScript runtime errors');
  console.log('PASS: Qt reader persistence, sources and widget controls; pinned next action; stable map/image across live updates, moving marker, open controls and stale location; optional captured-frame map and follow-player.');
} finally {socket.close();db.close()}
