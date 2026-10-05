// Controlled playback delay in the isolated native voice regression only.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
assert(process.env.GAMEBUDDY_TEST_WORKING);
assert(process.env.GAMEBUDDY_TEST_DATA?.startsWith('/tmp/gamebuddy-realtime-test-'));
const endpoint = 'http://' + process.env.QTWEBENGINE_REMOTE_DEBUGGING;
assert.equal(new URL(endpoint).hostname, '127.0.0.1');
const target = (await (await fetch(endpoint + '/json/list')).json()).find(t => t.url.endsWith('/voice'));
assert(target, 'Missing voice page');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, {once:true}); socket.addEventListener('error', reject, {once:true}); });
let timeout;
try {
  const response = new Promise((resolve, reject) => {
    socket.addEventListener('message', e => {const m = JSON.parse(e.data); if(m.id === 1) m.error ? reject(m.error) : resolve(m.result);});
  });
  const noiseDelay = process.env.GAMEBUDDY_TEST_NOISE ? 1000 + 1000 * Number(execFileSync('ffprobe', ['-v','error','-show_entries','format=duration','-of','csv=p=0',process.env.GAMEBUDDY_TEST_VOICE_WAV], {encoding:'utf8'})) : null;
  const expression = `(() => {
    const original = gamebuddyVoice.queueSpeech;
    gamebuddyVoice.queueSpeech = text => {
      gamebuddyVoice.queueSpeech = original;
      console.info('GAMEBUDDY_VOICE ' + JSON.stringify({type:'test-answer-held',value:true,at:Date.now()}));
      setTimeout(() => {
        console.info('GAMEBUDDY_VOICE ' + JSON.stringify({type:'test-answer-released',value:true,at:Date.now()}));
        original(text);
      }, 35000);
    };
    if(${JSON.stringify(noiseDelay)}!==null)setTimeout(() => {
      const timer = setInterval(() => {
        gamebuddyVoice.playerSpeech(true);
        setTimeout(() => gamebuddyVoice.playerSpeech(false), 220);
      }, 1800);
      setTimeout(() => clearInterval(timer), 45000);
      console.info('GAMEBUDDY_VOICE ' + JSON.stringify({type:'test-unrecognized-noise',value:true,at:Date.now()}));
    }, ${JSON.stringify(noiseDelay)});
    return true;
  })()`;
  socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression,returnByValue:true}}));
  const result = await Promise.race([response, new Promise((_, reject) => { timeout=setTimeout(() => reject(new Error('Voice test injection timed out')), 8000); })]);
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  assert.equal(result.result.value, true);
} finally {clearTimeout(timeout);socket.close();setTimeout(() => process.exit(process.exitCode || 0), 250).unref();}
