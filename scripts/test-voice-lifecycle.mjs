// No browser or microphone access: exercise delayed permission/device results.
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import assert from 'node:assert/strict';
const html=readFileSync(new URL('../crates/gamebuddy-app/assets/voice.html',import.meta.url),'utf8');
const script=html.match(/<script>([\s\S]*?)<\/script>/)[1];
const pending=[],streams=[],messages=[],channels=[],speaker={srcObject:null,play:async()=>{}};
const stream=()=>{const track={stopped:false,stop(){this.stopped=true}};const value={getTracks:()=>[track],getAudioTracks:()=>[track],track};streams.push(value);return value};
const context={window:{},document:{getElementById:()=>speaker},console:{info(){}},setTimeout,clearTimeout,TextEncoder,
  navigator:{mediaDevices:{getUserMedia:()=>new Promise(resolve=>pending.push(resolve)),enumerateDevices:async()=>[{kind:'audioinput',label:'Test mic',deviceId:'test'}]}},
  AudioContext:class{createMediaStreamDestination(){return {stream:{getAudioTracks:()=>[{stop(){}}]}}}createConstantSource(){return {offset:{value:0},connect(){},start(){}}}async resume(){}async close(){}},
  RTCPeerConnection:class{iceGatheringState='complete';addTransceiver(){return {sender:{replaceTrack:async()=>{}}}}createDataChannel(){const channel={readyState:'connecting',send(text){messages.push(JSON.parse(text))}};channels.push(channel);return channel}async createOffer(){return {type:'offer',sdp:'test'}}async setLocalDescription(v){this.localDescription=v}close(){}}
};
runInNewContext(script,context);
const flush=async()=>{await new Promise(resolve=>setImmediate(resolve))};
const voice=context.window.gamebuddyVoice;
await voice.start('Test mic',false);
assert.equal(pending.length,0,'spoken startup must never request a microphone');
voice.updateContext('old observation');voice.updateContext('latest observation');
assert.equal(messages.length,0,'wait for the session to connect');
channels[0].readyState='open';channels[0].onopen();
assert.equal(messages.length,1,'only the latest waiting observation should be delivered');
assert.equal(messages[0].channel,'commentary','observations must never use the speakable channel');
assert.equal(messages[0].content[0].text,'latest observation');
const unicode='🕹️'.repeat(400);messages.length=0;voice.updateContext(unicode);
assert.equal(messages.map(m=>m.content[0].text).join(''),unicode,'chunks preserve Unicode');
assert(messages.every(m=>Buffer.byteLength(m.content[0].text)<=500),'chunks stay within the protocol limit');
const first=voice.setMicrophone(true,'Test mic');pending.shift()(stream());await flush();
assert.equal(pending.length,1,'first device request is pending');
const oldDevice=pending.shift();
voice.stop();
const second=voice.start('Test mic',true);await flush();pending.shift()(stream());await flush();
const newDevice=pending.shift();
const late=stream();oldDevice(late);await first;
assert(late.track.stopped,'late permission result must release its own stream');
const current=stream();newDevice(current);await second;
assert(!current.track.stopped,'old result must not close the new microphone');
await voice.setMicrophone(false,'Test mic');
assert(current.track.stopped,'muting closes capture while voice output remains connected');
voice.stop();
assert(streams.every(s=>s.track.stopped),'mute must leave no microphone track open');
console.log('Voice lifecycle passed: stale capture released, new session preserved, mute closes every stream.');
