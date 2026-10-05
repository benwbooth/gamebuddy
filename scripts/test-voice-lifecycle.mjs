// No browser or microphone access: exercise delayed permission/device results.
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import assert from 'node:assert/strict';
const html=readFileSync(new URL('../crates/gamebuddy-app/assets/voice.html',import.meta.url),'utf8');
const script=html.match(/<script>([\s\S]*?)<\/script>/)[1];
const pending=[],streams=[],messages=[],channels=[],peers=[],activity=[],analysers=[],processors=[],receiverElement={srcObject:null,muted:true,play:async()=>{}},timers=new Set();
const node=()=>({connect(){},disconnect(){}});
let now=1000;
const tick=(ms=100)=>{for(let i=0;i<ms/20;i++){now+=20;for(const fn of timers)fn()}};
const stream=()=>{const track={stopped:false,stop(){this.stopped=true}};const value={getTracks:()=>[track],getAudioTracks:()=>[track],track};streams.push(value);return value};
const context={window:{},Date:class extends Date{static now(){return now}},document:{getElementById:()=>receiverElement},console:{info(s){activity.push(JSON.parse(s.slice(16)))}},setTimeout,clearTimeout,TextEncoder,
  setInterval(fn){timers.add(fn);return fn},clearInterval(fn){timers.delete(fn)},
  navigator:{mediaDevices:{getUserMedia:()=>new Promise(resolve=>pending.push(resolve)),enumerateDevices:async()=>[{kind:'audioinput',label:'Test mic',deviceId:'test'}]}},
  AudioContext:class{get currentTime(){return now/1000}outputLatency=0.04;sampleRate=48000;destination={};createMediaStreamSource(){return node()}createScriptProcessor(){const p={...node(),onaudioprocess:null};processors.push(p);return p}createGain(){return {...node(),gain:{value:1}}}createAnalyser(){const a={...node(),level:0,getFloatTimeDomainData(data){data.fill(this.level)}};analysers.push(a);return a}createMediaStreamDestination(){return {stream:{getAudioTracks:()=>[{stop(){}}]}}}createConstantSource(){return {offset:{value:0},connect(){},start(){}}}async resume(){}async close(){}},
  RTCPeerConnection:class{constructor(){peers.push(this)}iceGatheringState='complete';async getStats(){return new Map()}addTransceiver(){return {sender:{replaceTrack:async()=>{}}}}createDataChannel(){const channel={readyState:'connecting',send(text){messages.push(JSON.parse(text))}};channels.push(channel);return channel}async createOffer(){return {type:'offer',sdp:'test'}}async setLocalDescription(v){this.localDescription=v}close(){}}
};
runInNewContext(script,context);
const flush=async()=>{await new Promise(resolve=>setImmediate(resolve))};
const voice=context.window.gamebuddyVoice;
const muted=()=>activity.filter(e=>e.type==='playback-allowed').at(-1)?.value!==true;
const wire=value=>channels.at(-1).onmessage({data:JSON.stringify(value)});
await voice.start('Test mic',false);
assert.equal(pending.length,0,'spoken startup must never request a microphone');
tick();assert.equal(activity.filter(e=>e.type==='input-pcm').length,0,'no microphone samples before activation');
const receiver={jitterBufferTarget:null};
peers[0].ontrack({streams:[{}],receiver});analysers.at(-1).level=0.1;tick();
assert.equal(receiver.jitterBufferTarget,300,'retain speech onset while local endpoint detection releases');
assert.equal(muted(),true,'remote audio cannot start speaking on its own');
voice.approveSpeech();tick();
assert.equal(activity.filter(e=>e.type==='output-active').at(-1).value,true,'audible assistant speech is detected');
voice.muteOutput(true);tick();
assert.equal(activity.filter(e=>e.type==='output-active').at(-1).value,false,'muted output cannot duck the game');
voice.muteOutput(false);analysers.at(-1).level=0;tick();
assert.equal(activity.filter(e=>e.type==='output-active').at(-1).value,false,'a connected silent output is not speech');
voice.updateContext('old observation');voice.updateContext('latest observation');
assert.equal(messages.length,0,'wait for the session to connect');
channels[0].readyState='open';
assert.equal(activity.filter(e=>e.type==='ready').length,0,'an open data channel alone is not a started voice session');
wire({type:'session.started'});
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
analysers.at(-1).level=0.25;tick();
const pcm=activity.filter(e=>e.type==='input-pcm').at(-1).value;
assert.equal(pcm.length,320);assert(pcm.every(v=>v===8192),'48 kHz audio is converted into 20 ms 16 kHz PCM for VAD');
// The backend owns speech. Voice-model acknowledgments and old gameplay
// directions must never play while the completed backend answer is pending.
peers.at(-1).ontrack({streams:[{}]});const output=analysers.at(-1);
const render=values=>{const input=new Float32Array(values),result=new Float32Array(input.length);processors.at(-1).onaudioprocess({playbackTime:now/1000+0.02,inputBuffer:{getChannelData:()=>input},outputBuffer:{getChannelData:()=>result}});return Array.from(result)};
assert(receiverElement.muted,'the native decoder must not double-play queued speech');
voice.playerSpeech(true);voice.playerSpeech(false);
wire({type:'turn.created',turn:{id:'wrong-advice',role:'assistant'}});
wire({type:'output_transcript.added',start_ms:10000,end_ms:10200,item:{text:'Keep walking right.'}});
assert.deepEqual(render([0.25,0.5]),[0,0],'unapproved old directions are discarded');
assert(muted(),'a question alone does not authorize model improvisation');
voice.setBackendBusy(true);tick(25000);
voice.setBackendBusy(false);tick(1000);
assert(muted(),'backend completion alone does not authorize audio');
assert(!activity.some(e=>e.type==='caption'&&e.value.includes('walking')),'unapproved speech has no captions');
voice.queueSpeech('Use a J key at the door.');tick(1000);
assert(muted(),'completed answer waits while the old generated turn is still active');
for(let i=0;i<15;i++){render([.25,.5]);tick(1000)}
assert(!activity.some(e=>e.type==='cue-timeout'),'an actively draining answer must not expire after six seconds');
wire({type:'turn.done',turn:{id:'wrong-advice',role:'assistant'}});
render([.25,.5]);tick(400);
assert(muted(),'drain the old RTP tail before approving the new words');
tick(120);
assert.equal(activity.filter(e=>e.type==='speech-ready').at(-1).value,'Use a J key at the door.');
wire({type:'turn.created',turn:{id:'approved',role:'assistant'}});
wire({type:'output_transcript.added',start_ms:40000,end_ms:40200,item:{text:'Use a J key'}});
wire({type:'output_transcript.added',start_ms:40400,end_ms:40600,item:{text:' at the door.'}});
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'','arriving text cannot precede audio');
// Caption timing uses the rendered samples and output-device latency, even
// when future transcript chunks arrive in a batch.
render(Array(4800).fill(.25));tick(40);
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'','wait until audio reaches the output device');
tick(40);
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'Use a J key');
assert(!activity.filter(e=>e.type==='caption').at(-1).value.includes('door'),'later words wait for their timestamp');
for(let i=0;i<4;i++){render(Array(4800).fill(.25));tick(100)}
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'Use a J key at the door.');
output.level=.1;tick(500);
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'Wait, stop.'}});
assert(muted(),'recognized interruption stops playback');
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'','interruption discards pending subtitles');
voice.playerSpeech(false);
assert.deepEqual(render([.5,.5]),[0,0],'old queued audio cannot return after interruption');
voice.approveSpeech();
wire({type:'turn.created',turn:{id:'replacement',role:'assistant'}});
assert.deepEqual(render([.75,.75]),[.75,.75],'the new approved reply starts cleanly');
voice.muteOutput(true);assert.deepEqual(render([.5,.5]),[0,0]);
voice.muteOutput(false);assert.deepEqual(render([0,0]),[0,0],'muted audio cannot replay on unmute');
output.level=.1;tick(35000);
assert(!muted(),'requested long speech can finish');
wire({type:'turn.done',turn:{id:'replacement',role:'assistant'}});
output.level=0;tick(600);assert(muted(),'completed speech returns to listening');
wire({type:'turn.created',turn:{id:'unsolicited-followup',role:'assistant'}});
assert.deepEqual(render([.5,.5]),[0,0],'another generated turn requires another completed answer');
voice.updateContext('screen changed');voice.setBackendBusy(true);voice.setBackendBusy(false);tick();
assert(muted(),'background context and backend activity cannot grant playback');
voice.approveSpeech();output.level=0;
const retries=activity.filter(e=>e.type==='cue-retry').length;
tick(6200);assert.equal(activity.filter(e=>e.type==='cue-retry').length,retries+1);
tick(6200);assert.equal(activity.filter(e=>e.type==='cue-retry').length,retries+1,'retry an unstarted cue only once');
assert(muted(),'an unstarted cue expires');
// Even a created assistant turn can fail to produce playable audio. Its
// existence must not disable the bounded text fallback or retry its tools.
voice.approveSpeech();
wire({type:'turn.created',turn:{id:'silent-approved',role:'assistant'}});
const failedBefore=activity.filter(e=>e.type==='cue-timeout').length;
tick(12200);
assert.equal(activity.filter(e=>e.type==='cue-timeout').length,failedBefore+1);
assert(muted(),'a turn with no audio returns to listening');
// Missing completion from an older unapproved turn cannot strand the next
// completed backend answer behind it forever.
voice.queueSpeech('The key opens the gate.');tick(6200);
assert.equal(activity.filter(e=>e.type==='cue-timeout').length,failedBefore+2);
assert.equal(activity.filter(e=>e.type==='reply-pending').at(-1).value,false);
wire({type:'turn.done',turn:{id:'silent-approved',role:'assistant'}});
// Native transcript completion must not flush subtitles before buffered audio.
voice.approveSpeech();
wire({type:'turn.created',turn:{id:'buffered',role:'assistant'}});
wire({type:'output_transcript.added',start_ms:80000,end_ms:80200,item:{text:'First words'}});
wire({type:'output_transcript.added',start_ms:81000,end_ms:81200,item:{text:' last words'}});
wire({type:'turn.done',turn:{id:'buffered',role:'assistant',transcript:'First words last words'}});
render(Array(4800).fill(.25));tick(100);
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'First words','completion cannot reveal unplayed words');
wire({type:'turn.done',turn:{role:'user',transcript:'Stop.'}});
assert.deepEqual(render([.5,.5]),[0,0],'an exact stop request immediately silences audio');
voice.muteOutput(true);voice.approveSpeech();
wire({type:'turn.created',turn:{id:'text-only',role:'assistant'}});
wire({type:'output_transcript.added',start_ms:90000,end_ms:90200,item:{text:'The key opens the gate.'}});
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'The key opens the gate.','muted replies still provide approved text');
wire({type:'turn.done',turn:{id:'text-only',role:'assistant'}});output.level=0;tick(600);
assert.equal(activity.filter(e=>e.type==='reply-pending').at(-1).value,false,'muted text replies do not wait for deliberately discarded audio');
voice.setBackendBusy(true);voice.queueSpeech('An old pending reply');voice.requestFailed('Please try again.');tick(1000);
assert.equal(activity.filter(e=>e.type==='request-error').at(-1).value,'Please try again.','error text follows the caption reset');
assert.equal(activity.filter(e=>e.type==='reply-pending').at(-1).value,false,'backend failure releases waiting');
assert(!current.track.stopped,'a failed request keeps the microphone available');
assert(muted(),'a failed request cannot release stale audio');
await voice.setMicrophone(false,'Test mic');
assert(current.track.stopped,'muting closes capture while voice output remains connected');
voice.stop();
assert.equal(timers.size,0,'disconnect closes every activity timer');
assert(streams.every(s=>s.track.stopped),'mute must leave no microphone track open');
console.log('Voice lifecycle passed: backend approval, playback-clock captions, future transcript buffering, interruption, long replies, quiet context and microphone cleanup.');
