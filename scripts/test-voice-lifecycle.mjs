// No browser or microphone access: exercise delayed permission/device results.
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import assert from 'node:assert/strict';
const html=readFileSync(new URL('../crates/gamebuddy-app/assets/voice.html',import.meta.url),'utf8');
const script=html.match(/<script>([\s\S]*?)<\/script>/)[1];
const pending=[],streams=[],messages=[],channels=[],peers=[],activity=[],analysers=[],processors=[],receiverElement={srcObject:null,muted:true,play:async()=>{}},timers=new Set();
const node=()=>({connect(){},disconnect(){}});
let now=1000;const localBuffers=[],fetches=[];
const tick=(ms=100)=>{for(let i=0;i<ms/20;i++){now+=20;for(const fn of timers)fn()}};
const stream=()=>{const track={stopped:false,stop(){this.stopped=true}};const value={getTracks:()=>[track],getAudioTracks:()=>[track],track};streams.push(value);return value};
const context={window:{},Date:class extends Date{static now(){return now}},document:{getElementById:()=>receiverElement},console:{info(s){activity.push(JSON.parse(s.slice(16)))}},setTimeout,clearTimeout,TextEncoder,
  setInterval(fn){timers.add(fn);return fn},clearInterval(fn){timers.delete(fn)},
  navigator:{mediaDevices:{getUserMedia:()=>new Promise(resolve=>pending.push(resolve)),enumerateDevices:async()=>[{kind:'audioinput',label:'Test mic',deviceId:'test'}]}},
  fetch:url=>new Promise(resolve=>fetches.push({url,resolve})),
  AudioContext:class{async decodeAudioData(){return {duration:2}}createBufferSource(){const source={...node(),started:false,stopped:false,start(){this.started=true},stop(){this.stopped=true}};localBuffers.push(source);return source}get currentTime(){return now/1000}outputLatency=0.04;sampleRate=48000;destination={};createMediaStreamSource(){return node()}createScriptProcessor(){const p={...node(),onaudioprocess:null};processors.push(p);return p}createGain(){return {...node(),gain:{value:1}}}createAnalyser(){const a={...node(),level:0,getFloatTimeDomainData(data){data.fill(this.level)}};analysers.push(a);return a}createMediaStreamDestination(){return {stream:{getAudioTracks:()=>[{stop(){}}]}}}createConstantSource(){return {offset:{value:0},connect(){},start(){}}}async resume(){}async close(){}},
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
for(let i=0;i<3;i++){render([.25,.5]);tick(1000)}
assert(!activity.some(e=>e.type==='cue-timeout'),'a short old turn can drain before recovery');
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
// Waiting is visual only. No status cue may occupy the voice connection,
// restart a request, or delay the answer, even after a minute of tool work.
voice.muteOutput(false);channels.at(-1).readyState='open';wire({type:'session.started'});
const spoken=()=>messages.filter(m=>m.channel==='speakable').length;
const pendingWork=()=>activity.filter(e=>e.type==='work-pending').at(-1)?.value===true;
const spokenBefore=spoken();
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'[tongue click]'}});wire({type:'turn.done',turn:{role:'user',transcript:'[tongue click]'}});voice.playerSpeech(false);
assert(!pendingWork(),'nonverbal annotations cannot start work');
voice.setBackendBusy(true);tick(30000);
assert(!pendingWork(),'background work is not a player request');
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'Please check the guide.'}});
assert(pendingWork(),'recognized work immediately signals the visual status');
voice.playerSpeech(false);tick(60000);
assert.equal(spoken(),spokenBefore,'long waits must never inject spoken filler');
assert(muted(),'waiting never grants playback permission');
voice.queueSpeech('The verified answer.');voice.setBackendBusy(false);
assert(!pendingWork(),'the answer clears the work indicator');
tick(600);
assert.equal(activity.filter(e=>e.type==='speech-ready').at(-1).value,'The verified answer.','answer does not wait for a progress announcement');
wire({type:'turn.created',turn:{id:'final-without-progress',role:'assistant'}});
output.level=.1;tick(3000);
wire({type:'turn.done',turn:{id:'final-without-progress',role:'assistant'}});output.level=0;tick(600);
wire({type:'input_transcript.added',item:{text:'Late words from the answered request.'}});tick(20000);
assert(!pendingWork(),'late words cannot restart completed work');
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'Please check again.'}});voice.playerSpeech(false);
assert(pendingWork());
voice.playerSpeech(true);wire({type:'turn.done',turn:{role:'user',transcript:'Stop.'}});voice.playerSpeech(false);tick(20000);
assert(!pendingWork(),'stop clears pending feedback');
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'Stop'}});wire({type:'input_transcript.added',item:{text:' speaking.'}});voice.playerSpeech(false);tick(6000);
assert(!pendingWork(),'multi-fragment stop phrases stay quiet');
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'Another request.'}});voice.playerSpeech(false);voice.muteOutput(true);tick(25000);
assert(pendingWork(),'visual feedback is available when spoken replies are muted');
voice.requestFailed('Please try again.');voice.muteOutput(false);tick(20000);
assert(!pendingWork(),'failure clears pending feedback');
voice.playerSpeech(true);wire({type:'input_transcript.added',item:{text:'Open the guide.'}});voice.playerSpeech(false);
assert(pendingWork());voice.finishLocalRequest();tick(1000);
assert(!pendingWork(),'completed local commands clear the pending status');
assert.equal(activity.filter(e=>e.type==='reply-pending').at(-1).value,false);

assert.equal(spoken(),spokenBefore,'no waiting path sends spoken status');
voice.setBackendBusy(false);
wire({type:'turn.created',turn:{id:'unwanted-long-answer',role:'assistant'}});
const blockedTimeouts=activity.filter(e=>e.type==='cue-timeout').length;
const blockedReady=activity.filter(e=>e.type==='speech-ready').length;
voice.queueSpeech('The saved answer is ready.');
for(let i=0;i<7;i++){render([.25,.5]);tick(1000)}
assert.equal(activity.filter(e=>e.type==='cue-timeout').length,blockedTimeouts+1,'continuous unapproved audio must trigger bounded speech recovery');
assert.equal(activity.filter(e=>e.type==='speech-ready').length,blockedReady,'unwanted audio cannot inherit the ready answer permission');
assert(!current.track.stopped,'status playback preserves the interruptible microphone');
await voice.setMicrophone(false,'Test mic');
assert(current.track.stopped,'muting closes capture while voice output remains connected');
voice.stop();
assert.equal(timers.size,0,'disconnect closes every activity timer');
assert(streams.every(s=>s.track.stopped),'mute must leave no microphone track open');
await voice.start('Test mic',false);
wire({type:'output_transcript.added',item:{text:'Unapproved greeting'},start_ms:100,end_ms:200});
voice.queueSpeech('The chosen answer.');tick(200);
assert(muted(),'text before turn.created must also block early playback');
wire({type:'turn.created',turn:{id:'late-created',role:'assistant'}});tick(600);
assert(muted(),'the unapproved turn must finish before the chosen answer');
wire({type:'turn.done',turn:{id:'late-created',role:'assistant'}});tick(600);
assert.equal(activity.filter(e=>e.type==='speech-ready').at(-1).value,'The chosen answer.');
voice.stop();
// The prepared path bypasses remote audio-drain delays while retaining exact
// approval, output activity, interruption and generation guards.
assert(html.includes("connect-src 'self'"),'same-origin WAV requests must pass CSP');
await voice.start('Test mic',false);voice.muteOutput(false);
const prepared=voice.queuePrepared('key-one','Enter the door.');
assert.equal(fetches.at(-1).url,'/speech/key-one.wav');
assert.equal(localBuffers.length,0,'no playback before the WAV arrives');
fetches.pop().resolve({ok:true,arrayBuffer:async()=>new ArrayBuffer(0)});await prepared;
assert(localBuffers.at(-1).started,'decoded prepared audio plays without a remote speech turn');
analysers.at(-1).level=.1;tick(100);
assert.equal(activity.filter(e=>e.type==='output-active').at(-1).value,true);
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'Enter the door.');
voice.interruptOutput();assert(localBuffers.at(-1).stopped,'urgent replacement stops actual local audio');
const obsolete=voice.queuePrepared('late','Old directions.');voice.interruptOutput();
const count=localBuffers.length;fetches.pop().resolve({ok:true,arrayBuffer:async()=>new ArrayBuffer(0)});await obsolete;
assert.equal(localBuffers.length,count,'late downloads cannot play cancelled words');
await voice.queuePrepared('key-one','Enter the door.');
assert(localBuffers.at(-1).started,'prepared audio is cached');
voice.muteOutput(true);assert(localBuffers.at(-1).stopped,'mute stops local audio');
voice.muteOutput(false);tick();assert.equal(localBuffers.length,count+1,'unmute cannot replay old audio');
// Cached later sentences can arrive before the first synthesis. Keep order,
// captions and completion tied to the audio that is actually playing.
const chunkCount=localBuffers.length;
const later=voice.queuePrepared('last-chunk','Then leave.',{request:'ordered',index:1,last:true});
fetches.pop().resolve({ok:true,arrayBuffer:async()=>new ArrayBuffer(0)});await later;
assert.equal(localBuffers.length,chunkCount,'later sentences wait for the first');
await voice.queuePrepared('key-one','Enter the door.',{request:'ordered',index:0,last:false,answer:'Enter the door. Then leave.'});
analysers.at(-1).level=.1;tick(100);
assert.equal(activity.filter(e=>e.type==='caption').at(-1).value,'Enter the door.','future sentences cannot appear early');
const firstChunk=localBuffers.at(-1);firstChunk.onended();
assert.equal(localBuffers.length,chunkCount+2,'the second sentence starts after the first ends');
assert.equal(activity.filter(e=>e.type==='reply-pending').at(-1).value,true,'a sentence boundary is not answer completion');
localBuffers.at(-1).onended();
assert.equal(activity.filter(e=>e.type==='reply-pending').at(-1).value,false,'only the final audio chunk completes the reply');
const partial=voice.queuePrepared('slow-part','Cancelled.',{request:'cancelled-parts',index:0,last:false});
voice.interruptOutput();
fetches.pop().resolve({ok:true,arrayBuffer:async()=>new ArrayBuffer(0)});await partial;
const afterCancel=localBuffers.length;
await voice.queuePrepared('key-one','Enter the door.',{request:'cancelled-parts',index:1,last:true});
assert.equal(localBuffers.length,afterCancel,'late chunks cannot resurrect an interrupted request');
voice.stop();
// Native conversation plays the realtime model's own audio. Acknowledgments
// and local VAD do not impose a text gate or buffer speech behind backchannels.
await voice.start('Test mic',false);voice.setConversationMode(true);
const nativeReceiver={jitterBufferTarget:null};
peers.at(-1).ontrack({streams:[{}],receiver:nativeReceiver});
assert.equal(nativeReceiver.jitterBufferTarget,null,'native playback uses its natural playout buffer');
channels.at(-1).readyState='open';
wire({type:'session.started'});
wire({type:'output_transcript.added',start_ms:100,end_ms:200,item:{text:'Hello there.'}});
wire({type:'turn.created',turn:{id:'native-hello',role:'assistant'}});
assert.deepEqual(render([.25,.5]),[.25,.5],'native audio needs no completed text answer');
voice.playerSpeech(true,true);
assert.deepEqual(render([.25,.5]),[.25,.5],'the native model decides how to handle a spoken backchannel');
voice.playerSpeech(false);
const delegated=activity.filter(e=>e.type==='delegation').length;
const handoff={type:'delegation.created',item:{id:'native-task',type:'delegation',target:'client',content:[{type:'input_text',text:'Inspect the rectangle.'}]}};
wire(handoff);wire(handoff);
assert.equal(activity.filter(e=>e.type==='delegation').length,delegated+1,'native requests reach the tool coordinator');
assert.equal(messages.at(-1).channel,'commentary','pending work is quiet and tied to its handoff');
voice.queueSpeech('The verified rectangle is green.','native-task');
assert.equal(messages.at(-1).type,'delegation.context.append');
assert.equal(messages.at(-1).delegation_item_id,'native-task','a result must resolve its actual delegation');
assert.equal(messages.at(-1).channel,'speakable');
voice.muteOutput(true);
assert.deepEqual(render([.25,.5]),[0,0],'mute also silences direct native audio');
voice.muteOutput(false);
voice.stop();
assert.equal(timers.size,0);
// Exercise the actual QML console handler: an interrupted caption must stop
// being active even when a delegated answer is still pending.
const panel=readFileSync(new URL('../crates/gamebuddy-app/qml/RealtimePanel.qml',import.meta.url),'utf8');
const handler=panel.match(/onJavaScriptConsoleMessage: \(level,message,line,source\) => \{([\s\S]*?)\n    \}\n    onEnabledBackendChanged/)[1];
const state={started:true,trace:false,captionText:'',captionStreaming:false,assistantSpeaking:false,waitingForReply:false,activeAdviceStep:'',pendingCue:'',outputAt:0,transport:{trace_event(){}}};
runInNewContext('function receive(message){'+handler+'}',state);
const panelEvent=(type,value)=>state.receive('GAMEBUDDY_VOICE '+JSON.stringify({type,value}));
panelEvent('reply-pending',true);
panelEvent('caption','Back at the green-stone passage,');
panelEvent('output-active',true);
assert(state.captionStreaming&&state.assistantSpeaking);
panelEvent('output-active',false);
assert(state.waitingForReply,'tool work remains pending');
assert(!state.captionStreaming&&!state.assistantSpeaking,'silence releases caption expiry while tools are pending');
assert.equal(state.captionText,'Back at the green-stone passage,','the last words remain briefly readable');
panelEvent('caption','Over twenty-two hundred experience.');
panelEvent('output-active',true);
assert(state.captionStreaming&&state.assistantSpeaking,'the next audible answer resumes captions');
console.log('Voice lifecycle passed: native speech, delegated tools, backchannels, prepared audio, captions and microphone cleanup.');
