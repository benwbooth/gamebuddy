// Opt-in integration test against a private Qt WebEngine debug port and the
// guide_library_fixture database. Never point this at a user's live session.
import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
import {DatabaseSync} from 'node:sqlite';
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
  await click('.reader-gallery img');assert(await evaluate(`!!document.querySelector('.reader-lightbox img')`));await button('Close picture');
  await button('All maps');await until(()=>evaluate(`document.querySelector('[data-reader-map]')?.src.startsWith('data:image/jpeg') && document.querySelector('[data-reader-map]').naturalWidth>0`),'Cached map bridge failed');
  assert(await evaluate(`document.querySelector('.reader-content').textContent.includes('Generated fixture author')`));await screenshot('reader-map');
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
  assert.deepEqual(errors,[],'JavaScript runtime errors');
  console.log('PASS: Qt reader images/maps, source history, browsing, persistent bookmarks/goals/completion, minimize/restore/dismiss/reopen; live cursor preserved.');
} finally {socket.close();db.close()}
