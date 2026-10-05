// Only called by compositor_probe inside its private Gamescope Xwayland.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const endpoint='http://'+process.env.QTWEBENGINE_REMOTE_DEBUGGING;
assert(new URL(endpoint).hostname==='127.0.0.1');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
let target;
for(let i=0;i<100&&!target;i++){
  target=(await(await fetch(endpoint+'/json/list')).json()).find(t=>t.title==='GameBuddy widgets');
  if(!target)await pause(100);
}
assert(target,'Missing widget page');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})});
let sequence=0;const pending=new Map();
socket.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){const p=pending.get(m.id);pending.delete(m.id);m.error?p.reject(new Error(JSON.stringify(m.error))):p.resolve(m.result)}});
function call(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});socket.send(JSON.stringify({id,method,params}))})}
async function evaluate(expression){const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!r.exceptionDetails,JSON.stringify(r.exceptionDetails));return r.result.value}
const input=(action,x,y,button=1)=>execFileSync(fileURLToPath(new URL('../target/debug/examples/input_probe',import.meta.url)),[action,String(Math.round(x)),String(Math.round(y)),String(button)],{env:{...process.env,...(process.env.GAMEBUDDY_TEST_OUTER_DISPLAY?{DISPLAY:process.env.GAMEBUDDY_TEST_OUTER_DISPLAY}:{})}});
async function move(x,y){for(let i=0;i<10;i++){await input('move',x+i%2,y);await pause(50)}}
async function wheel(button,count=5){for(let i=0;i<count;i++){await input('down',0,0,button);await input('up',0,0,button);await pause(25)}}
async function until(check,message){const t=Date.now();while(!await check()){assert(Date.now()-t<5000,message);await pause(50)}}
try {
  await evaluate(`window.nativeWheels=[];window.nativeMoves=[];document.addEventListener('pointermove',e=>nativeMoves.push({x:e.clientX,y:e.clientY}));document.addEventListener('wheel',e=>nativeWheels.push({x:e.deltaX,y:e.deltaY,at:performance.now()}));window.realRender=gamebuddy.render;
    window.nativeState={widgets:[{id:'native-text',title:'Native wheel fixture',content:{kind:'text',body:Array.from({length:100},(_,i)=>'Line '+i+' -- scroll this text').join('\\n')}}],layouts:{'native-text':{x:700,y:50,width:450,height:400}},widget_visibility:{'native-text':true}};
    gamebuddy.render=state=>{if(state===nativeState)realRender(state)};realRender(nativeState)`);
  if(process.env.GAMEBUDDY_TEST_CURSOR){
    await pause(500);
    for(const [name,x,y] of [['text',850,200],['drag',850,70],['resize',1145,445],['companion',160,103]]){
      await move(x,y);
      const cursor=execFileSync(fileURLToPath(new URL('../target/debug/examples/input_probe',import.meta.url)),['cursor'],{encoding:'utf8'}).trim();
      console.log('NATIVE_CURSOR',name,cursor);
      if(process.env.GAMEBUDDY_TEST_CURSOR!=='measure'&&name==='resize')assert(cursor.includes(process.env.GAMEBUDDY_TEST_CURSOR),'Unexpected native cursor size');
    }
  } else if(process.env.GAMEBUDDY_TEST_POINTER_REPLAY==='pinch'){
    await evaluate(`nativeState.widgets=[{id:'native-image',title:'Pinch fixture',content:{kind:'image',url:'https://gamebuddy.invalid/native.png'}}];nativeState.layouts={'native-image':{x:700,y:50,width:450,height:500}};nativeState.widget_visibility={'native-image':true};realRender(nativeState);document.querySelector('.picture-viewport img').src='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="800" height="800"><rect width="800" height="800" fill="red"/></svg>');`);
    await pause(500);await move(850,250);
    const before=await evaluate(`({width:document.querySelector('.map-stage').getBoundingClientRect().width,page:visualViewport.scale,widget:document.querySelector('.widget').getBoundingClientRect().width,style:document.querySelector('.map-stage').style.width,innerWidth,ratio:devicePixelRatio})`);
    await wheel(5,5);await pause(300);
    const after=await evaluate(`({width:document.querySelector('.map-stage').getBoundingClientRect().width,page:visualViewport.scale,widget:document.querySelector('.widget').getBoundingClientRect().width,style:document.querySelector('.map-stage').style.width,innerWidth,ratio:devicePixelRatio,wheels:nativeWheels})`);
    console.log('NATIVE_PINCH',JSON.stringify({before,after}));
    assert(after.width>before.width*1.4,'Native pinch did not zoom the image');
    assert.equal(after.page,1,'Native pinch zoomed the whole overlay');assert.equal(after.widget,before.widget,'Native pinch resized the overlay controls');
    console.log('PASS native Wayland pinch -> image zoom');
  } else {
  await pause(500);
  await move(850,200);
  console.log(execFileSync(fileURLToPath(new URL('../target/debug/examples/input_probe',import.meta.url)),['cursor'],{encoding:'utf8'}).trim());
  await wheel(5);
  await pause(300);
  const result=await evaluate(`({scroll:document.querySelector('.body').scrollTop,wheels:nativeWheels,moves:nativeMoves})`);
  console.log('NATIVE_WHEEL',JSON.stringify(result));
  assert(result.moves.length>0,'Gamescope did not deliver pointer movement');
  assert(result.wheels.length>0,'Gamescope did not deliver wheel events to Qt WebEngine');
  assert(result.scroll>50,'Native wheel did not scroll the widget');
  await move(850,200);await wheel(4);
  await until(()=>evaluate(`document.querySelector('.body').scrollTop<${result.scroll}`),'Up-wheel did not scroll');
  await evaluate(`document.querySelector('.body').scrollTop=0;nativeWheels=[]`);
  await move(850,200);
  // Xwayland primes its smooth-scroll valuator on the first axis packet
  // after pointer entry; begin the continuous gesture before updating content.
  await wheel(5,2);
  await until(()=>evaluate(`nativeWheels.length>0`),'Scroll gesture did not begin');
  for(let i=0;i<40;i++){
    const before=await evaluate(`nativeWheels.length`);
    await wheel(5,1);
    await until(()=>evaluate(`nativeWheels.length>${before}`),'Native wheel was not delivered at update '+i);
    await evaluate(`window.wheelingBody=document.querySelector('.body');nativeState.widgets[0].content.body+='\\nUpdate ${i}';realRender(nativeState)`);
    assert(await evaluate(`wheelingBody.isConnected`),'A live update replaced the viewport during wheel scrolling');
    await pause(300);
  }
  const sustained=await evaluate(`({scroll:document.querySelector('.body').scrollTop,wheels:nativeWheels.length})`);
  console.log('NATIVE_SUSTAINED_WHEEL',JSON.stringify(sustained));
  assert(sustained.wheels>=38 && sustained.scroll>900,'Stationary scrolling stopped after the cursor idle timeout');
  await until(()=>evaluate(`document.querySelector('.body').textContent.includes('Update 39')`),'Pending wheel updates did not settle');
  console.log('PASS Gamescope -> Qt -> Chromium wheel scrolling');
  }
} finally {socket.close()}
