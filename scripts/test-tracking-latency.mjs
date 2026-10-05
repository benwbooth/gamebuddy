// Observe the actual Qt WebEngine marker while tracking_ui_probe changes frames.
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync} from 'node:fs';
import {DatabaseSync} from 'node:sqlite';
const [root,endpoint]=process.argv.slice(2);
assert(root.startsWith('/tmp/'),'Use a private replay store');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
let target;
for(let n=0;n<300&&!target;n++){
  try{target=(await(await fetch(endpoint+'/json/list')).json()).find(t=>t.type==='page')}catch{}
  if(!target)await sleep(100);
}
assert(target,'Qt WebEngine debug target did not start');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})});
let sequence=0;const pending=new Map();
socket.addEventListener('message',event=>{const value=JSON.parse(event.data);if(pending.has(value.id)){pending.get(value.id)(value);pending.delete(value.id)}});
function call(method,params){return new Promise(resolve=>{const id=++sequence;pending.set(id,resolve);socket.send(JSON.stringify({id,method,params}))})}
async function marker(){
  const response=await call('Runtime.evaluate',{expression:`new Promise(resolve=>requestAnimationFrame(()=>{const m=document.querySelector('[data-widget-id="location-map"] .map-marker');resolve(m&&!m.hidden&&!m.classList.contains('updating')&&m.textContent.includes('Visible camera area')?{x:parseFloat(m.style.left)/100,y:parseFloat(m.style.top)/100,width:parseFloat(m.style.width)/100,height:parseFloat(m.style.height)/100,at:Date.now()}:null)}))`,returnByValue:true,awaitPromise:true});
  return response.result?.result?.value;
}
const db=new DatabaseSync(root+'/gamebuddy.sqlite',{readOnly:true});
const id=JSON.parse(db.prepare('SELECT body FROM documents WHERE key=?').get('active_guide').body);
const observation=()=>{const row=db.prepare('SELECT body FROM documents WHERE key=?').get('world-observation:'+id);return row?JSON.parse(row.body):null};
let lastRegion;const results=[];
for(let index=0;index<6;index++){
  const deadline=Date.now()+45000;let result;
  while(Date.now()<deadline){
    let frame;try{frame=JSON.parse(readFileSync(root+'/frame.json','utf8'))}catch{}
    if(frame?.index===index){
      const m=await marker(),o=observation(),r=o?.map_registration?.region;
      if(m&&r&&o.tracker.observed_at_ms>=frame.displayed_at_ms
        &&['x','y','width','height'].every(k=>Math.abs(m[k]-r[k])<0.002)
        &&(!lastRegion||Math.abs(m.y-lastRegion.y)>0.05||Math.abs(m.x-lastRegion.x)>0.05)){
        result={...frame,marker:m,map_id:o.map_registration.map_id,room_id:o.tracker.room_id,
          display_to_marker_ms:m.at-frame.displayed_at_ms,timing:o.timing};break;
      }
    }
    await sleep(20);
  }
  assert(result,`Frame ${index} never produced a new rendered geometric marker`);
  results.push(result);lastRegion=result.marker;
  writeFileSync(root+'/render-latency.json',JSON.stringify(results,null,2));
  console.log(JSON.stringify(result));
  writeFileSync(root+'/advance',String(index));
}
socket.close();db.close();
