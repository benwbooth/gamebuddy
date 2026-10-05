// Controlled fault in the isolated native voice regression, never a real game.
import assert from 'node:assert/strict';
assert(process.env.GAMEBUDDY_TEST_PROGRESS);
assert(process.env.GAMEBUDDY_TEST_DATA?.startsWith('/tmp/gamebuddy-realtime-test-'));
const endpoint = 'http://' + process.env.QTWEBENGINE_REMOTE_DEBUGGING;
assert.equal(new URL(endpoint).hostname, '127.0.0.1');
const target = (await (await fetch(endpoint + '/json/list')).json()).find(t => t.url.endsWith('/voice'));
assert(target, 'Missing voice page');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, {once:true}); socket.addEventListener('error', reject, {once:true}); });
try {
  const response = new Promise((resolve, reject) => {
    socket.addEventListener('message', e => {const m = JSON.parse(e.data); if(m.id === 1) m.error ? reject(m.error) : resolve(m.result);});
  });
  socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:`(() => {
    const original = gamebuddyVoice.queueSpeech;
    gamebuddyVoice.queueSpeech = text => {
      gamebuddyVoice.queueSpeech = original;
      console.info('GAMEBUDDY_VOICE ' + JSON.stringify({type:'cue-timeout',value:true,at:Date.now()}));
    };
    return true;
  })()`,returnByValue:true}}));
  const result = await response;
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  assert.equal(result.result.value, true);
} finally {socket.close();}
