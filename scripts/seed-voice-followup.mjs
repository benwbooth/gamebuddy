// Supply the same verified coaching cue that preceded the reported follow-up.
import assert from 'node:assert/strict';
assert(process.env.GAMEBUDDY_TEST_FOLLOWUPS);
assert(process.env.GAMEBUDDY_TEST_DATA?.startsWith('/tmp/gamebuddy-realtime-test-'));
const endpoint='http://'+process.env.QTWEBENGINE_REMOTE_DEBUGGING;
assert.equal(new URL(endpoint).hostname,'127.0.0.1');
const target=(await (await fetch(endpoint+'/json/list')).json()).find(t=>t.url.endsWith('/voice'));
assert(target,'Missing native voice page');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true})});
let timeout;
try {
  const response=new Promise((resolve,reject)=>socket.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id===1)m.error?reject(m.error):resolve(m.result)}));
  socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:`gamebuddyVoice.queueSpeech("Back at the green-stone passage, fight with your sword until you've got over twenty-two hundred experience."); true`,returnByValue:true}}));
  const result=await Promise.race([response,new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('Cue injection timed out')),8000)})]);
  assert(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));
  assert.equal(result.result.value,true);
} finally {clearTimeout(timeout);socket.close()}
