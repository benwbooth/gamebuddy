import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

const script=readFileSync(new URL('../crates/gamebuddy-app/assets/gemini.html',import.meta.url),'utf8').match(/<script>([\s\S]*)<\/script>/)[1];
const events=[],sent=[],sockets=[],sources=[],timeouts=[];
const context={
  window:{},console:{info(line){events.push(JSON.parse(line.slice(17)))}},
  Uint8Array,Int16Array,Float32Array,Map,Set,Date,JSON,Math,Error,
  atob:s=>Buffer.from(s,'base64').toString('binary'),btoa:s=>Buffer.from(s,'binary').toString('base64'),
  setTimeout(fn){timeouts.push(fn)},setInterval(){},clearInterval(){},
  WebSocket:class {
    static OPEN=1;readyState=1;
    constructor(){sockets.push(this)}
    send(data){sent.push(JSON.parse(data))}close(){}
  },
  AudioContext:class {
    currentTime=0;destination={};async resume(){}async close(){}
    createBuffer(){return {duration:.01,copyToChannel(){}}}
    createBufferSource(){const s={connect(){},start(){s.started=true},stop(){s.stopped=true}};sources.push(s);return s}
  }
};
runInNewContext(script,context);
const voice=context.window.gamebuddyGemini;
const last=type=>events.filter(e=>e.type===type).at(-1)?.value;
const wire=async value=>sockets.at(-1).onmessage({data:JSON.stringify(value)});
const audio={inlineData:{mimeType:'audio/pcm;rate=24000',data:Buffer.from([0,32,0,32]).toString('base64')}};
async function start(model){
  await voice.start({key:'fixture',setup:{model:'models/'+model,generationConfig:{thinkingConfig:{thinkingLevel:'high'}}}},'',false);
  sockets.at(-1).onopen();await wire({setupComplete:{}});
}
await start('gemini-3.8-live-extended-thinking');
voice.cue('Where should I go?');
assert.equal(last('work-pending'),true);
await wire({serverContent:{outputTranscription:{text:'Let me check.'},modelTurn:{parts:[audio]},turnComplete:true,interactionStatus:'IN_PROGRESS'}});
assert.equal(last('work-pending'),true,'a spoken acknowledgment cannot finish background reasoning');
assert.equal(last('output-final'),'Let me check.');
sources.at(-1).onended();
const before=sent.length;voice.bookkeep();voice.watch();
assert.equal(sent.length,before,'background cues cannot interrupt an unfinished answer');
await wire({interactionStatus:'IN_PROGRESS',toolCall:{functionCalls:[{id:'map',name:'get_location_map',args:{}}]},serverContent:{outputTranscription:{text:'I have the map.'},modelTurn:{parts:[audio]},turnComplete:true}});
assert.equal(last('output-final'),'I have the map.','combined messages process both tools and content');
voice.toolResult('map',{image_scope:'whole_map'},['reference-image']);
assert.equal(sent.at(-2).clientContent.turnComplete,false,'reference images cannot interrupt reasoning');
assert.equal(sent.at(-2).clientContent.turns[0].parts[1].inlineData.data,'reference-image');
assert.equal(sent.at(-1).toolResponse.functionResponses[0].id,'map');
assert.equal(last('work-pending'),true,'a tool result alone does not complete the answer');
await wire({serverContent:{outputTranscription:{text:'Use the ladder beside the door.'},turnComplete:true,interactionStatus:'IDLE'}});
assert.equal(last('work-pending'),false);
sources.at(-1).onended();

// A silent observation can span several spoken turns and tool calls.
voice.bookkeep();
const played=sources.length;
await wire({serverContent:{outputTranscription:{text:'Checking inventory.'},modelTurn:{parts:[audio]},turnComplete:true,interactionStatus:'IN_PROGRESS'}});
assert.equal(sources.length,played);
assert.equal(last('work-pending'),true);
await wire({interaction_status:'IN_PROGRESS',toolCall:{functionCalls:[{id:'state',name:'report_world_state',args:{}}]}});
voice.toolResult('state',{ok:true});
await wire({serverContent:{outputTranscription:{text:'The key is recorded.'},modelTurn:{parts:[audio]},turnComplete:true}});
assert.equal(sources.length,played,'silence lasts beyond the first utterance and tool result');
assert.equal(last('work-pending'),true);
await wire({interactionStatus:'IDLE'});
assert.equal(last('work-pending'),false);
assert.equal(last('bookkeeping-done'),true);
assert.match(last('muted-reply'),/key is recorded/);

// Player speech takes over a silent check and interruption clears playback.
voice.bookkeep();
await wire({serverContent:{inputTranscription:{text:'Wait, I lost the key.'},interrupted:true}});
await wire({serverContent:{outputTranscription:{text:'Okay, you need another key.'},modelTurn:{parts:[audio]},turnComplete:true,interactionStatus:'IDLE'}});
assert.equal(sources.length,played+1);
await wire({serverContent:{interrupted:true}});
assert(sources.at(-1).stopped);

// Cancellation drops late results, and a rejected thinking configuration
// must not silently degrade the trial to a different reasoning mode.
await wire({toolCall:{functionCalls:[{id:'cancelled',name:'search_guide',args:{}}]}});
await wire({toolCallCancellation:{ids:['cancelled']},interactionStatus:'IDLE'});
const count=sent.length;voice.toolResult('cancelled',{text:'stale'});
assert.equal(sent.length,count);
sockets.at(-1).onclose({code:1008,reason:'Invalid thinking configuration'});
assert.match(last('error'),/thinking/);
assert.equal(timeouts.length,0);

await start('gemini-3.8-live');
voice.cue('Hello');
await wire({serverContent:{outputTranscription:{text:'Hey.'},turnComplete:true}});
assert.equal(last('work-pending'),false,'regular Live still uses turn completion');
voice.stop();
console.log('GEMINI_LIVE_PROTOCOL_PASS async_reasoning=true images=true bookkeeping=true interruption=true');
