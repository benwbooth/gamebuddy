import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';

const script=readFileSync(new URL('../crates/gamebuddy-app/assets/walkthrough.js',import.meta.url),'utf8'),sandbox={};
runInNewContext(script,sandbox);
const model=(content,source)=>JSON.parse(JSON.stringify(sandbox.walkthroughContext(content,source)));
const guide={id:'passive-route',game:'Route fixture',steps:[{id:'town',title:'Town',instructions:''},{id:'forest',title:'Forest',instructions:''}]};
for(let i=1;i<=14;i++)guide.steps.push({id:'action-'+i,parent_id:i<=7?'town':'forest',title:['Speak to the guru','Buy the key','Leave through the east gate','Cross the stone bridge','Defeat the armored enemy','Collect the experience','Enter the passage','Find the forest inn','Restore your magic','Take the upper path','Open the tower door','Climb to the next floor','Defeat the tower guardian','Return to the forest'][i-1],instructions:'Walk toward the marked doorway, then enter when the path is clear.'});
const step=id=>guide.steps.find(s=>s.id===id);
const content={guide,progress:{current_step:'action-6',completed:['action-4']},guidance:{status:'ready',now:{...step('action-6'),instructions:'Keep fighting until you have over 2,200 experience.'},next:[step('action-7'),step('action-8')]}};
assert.deepEqual(model(content).earlier.map(s=>s.id),['action-3','action-4','action-5']);
assert.deepEqual(model(content).later.map(s=>s.id),['action-7','action-8']);
assert.equal(model(content).earlier[1].done,true);
assert.equal(model(content).earlier[2].done,false,'Being earlier in the guide is not completion');
assert.equal(model(content).now.instructions,content.guidance.now.instructions,'Use the published advice');
assert.equal(model(content).later[1].path,'Forest','Keep context across chapter boundaries');
const rerouted={...content,guidance:{...content.guidance,next:[step('action-9'),step('action-8'),step('action-10'),step('action-11')]},planning:{not_needed:[{step_id:'action-8'}]},progress:{...content.progress,completed:['action-10']}};
assert.deepEqual(model(rerouted).later.map(s=>s.id),['action-9','action-11'],'Use route order and omit completed or unnecessary actions');
const inherited={...content,progress:{...content.progress,completed:['town']}};
assert(model(inherited).earlier.every(s=>s.done),'Parent completion applies to visible children');
const sourceSteps=guide.steps.map(s=>s.id==='action-4'?{...s,parent_id:'source-section'}:s).concat({id:'source-section',parent_id:'town'});
assert(model({...content,progress:{...content.progress,completed:['source-section']}},sourceSteps).earlier[1].done,'Flattening source sections must preserve inherited completion');
assert.deepEqual(model({...content,guidance:{status:'goals_complete'},progress:{current_step:null}}).later,[],'A completed goal must not invent another instruction');
console.log('PASS walkthrough context: published advice, neighboring chapters, route order and completion provenance');

// Exercise production DOM/layout/timers in the isolated Qt WebEngine fixture.
export async function walkthroughInputs({call,evaluate,until,screenshot}) {
  assert(await evaluate(`document.querySelector('script').textContent.includes(${JSON.stringify(script)})`),'Qt executable contains an older walkthrough implementation');
  const style=readFileSync(new URL('../crates/gamebuddy-app/assets/widgets.html',import.meta.url),'utf8').match(/<style>([\s\S]*?)<\/style>/)[1];
  assert.equal(await evaluate(`document.querySelector('style').textContent`),style,'Qt executable contains older widget styling');
  const saved=await evaluate('structuredClone(testState)');
  const fixture={...saved,game:guide.game,guide,progress:content.progress,navigation:{guidance:content.guidance,player_steps:guide.steps,planning:{}},widgets:[],widget_visibility:{'walkthrough-outline':true,'current-step':true,'guide-library':false,'location-map':false}};
  await evaluate(`window.testState=${JSON.stringify(fixture)};testRender(testState);window.walkthroughNode=document.querySelector('[data-widget-id="walkthrough-outline"]');walkthroughNode.style.cssText='left:20px;top:20px;width:400px;height:550px'`);
  const count=()=>evaluate(`[...document.querySelectorAll('.walkthrough-neighbor,.walkthrough-preview')].filter(n=>!n.hidden&&n.getBoundingClientRect().height>0).length`);
  const fits=()=>evaluate(`(()=>{
    const b=document.querySelector('.walkthrough'),r=b.getBoundingClientRect(),copy=b.querySelector('.walkthrough-copy');
    const rows=[...b.querySelectorAll('[data-step-id]')].filter(n=>n.getBoundingClientRect().height>0).map(n=>{
      const s=n.getBoundingClientRect(),word=[...n.childNodes].find(c=>c.nodeType===Node.TEXT_NODE&&c.textContent.trim());
      if(!word)return {id:n.dataset.stepId,visible:false};
      // Range bounds include font ascent/descent, which can overhang a line box by a pixel.
      const range=document.createRange();range.setStart(word,0);range.setEnd(word,Math.min(8,word.textContent.length));const ink=range.getBoundingClientRect();
      return {id:n.dataset.stepId,top:s.top,bottom:s.bottom,ink:[ink.top,ink.bottom],visible:s.top>=r.top&&s.bottom<=r.bottom&&ink.top>=s.top-2&&ink.bottom<=s.bottom+2};
    });
    return {outerOverflow:b.scrollHeight-b.clientHeight,copyHeight:copy.clientHeight,allVisible:rows.every(r=>r.visible),rows,
      manualScrollers:[b,...b.querySelectorAll('*')].filter(n=>/(auto|scroll)/.test(getComputedStyle(n).overflowY)&&n.scrollHeight>n.clientHeight+1).length,
      earlier:b.querySelectorAll('.earlier li:not([hidden])').length,later:b.querySelectorAll('.later li:not([hidden])').length};
  })()`);
  async function readable(){
    try{await until(async()=>{const f=await fits();return f.outerOverflow<=1&&f.copyHeight>=50&&f.allVisible&&f.manualScrollers===0&&f.earlier>0&&f.later>0},'Walkthrough layout did not settle')}
    catch(error){await screenshot('walkthrough-clipped');throw Error(error.message+': '+JSON.stringify(await fits()))}
  }
  await readable();
  assert.equal(await evaluate(`document.querySelectorAll('[data-widget-id="current-step"]').length`),0,'Duplicate next-action widget remains');
  assert.equal(await evaluate(`document.querySelectorAll('[aria-current=step]').length`),1,'Current instruction duplicated in the outline');
  assert.equal(await evaluate(`document.querySelector('.outline-next .instructions').textContent`),content.guidance.now.instructions);
  await screenshot('walkthrough-context');
  const compactCount=await count();
  await evaluate(`walkthroughNode.style.width='600px';walkthroughNode.style.height='690px'`);
  await until(async()=>await count()>compactCount,'Extra space did not expand neighboring context');
  await readable();await screenshot('walkthrough-expanded');
  await evaluate(`walkthroughNode.style.width='300px';walkthroughNode.style.height='360px'`);
  await readable();await screenshot('walkthrough-compact');
  // No click, key, scroll or expansion event is needed to cross a chapter.
  await evaluate(`walkthroughNode.style.width='400px';walkthroughNode.style.height='550px';testState.progress.current_step='action-8';testState.progress.completed.push('action-6');testState.navigation.guidance.now=testState.guide.steps.find(s=>s.id==='action-8');testState.navigation.guidance.next=['action-9','action-10','action-11','action-12'].map(id=>testState.guide.steps.find(s=>s.id===id));testRender(testState)`);
  await until(()=>evaluate(`document.querySelector('[aria-current=step]').dataset.stepId==='action-8'&&document.querySelector('.walkthrough-path').textContent==='Forest'`),'Walkthrough did not follow into the next chapter');
  assert(await evaluate(`walkthroughNode===document.querySelector('[data-widget-id="walkthrough-outline"]')`),'Progress replaced the mounted widget');
  await readable();await screenshot('walkthrough-next-chapter');
  // A long current instruction reads itself while earlier/later rows stay put.
  const longText=Array.from({length:14},(_,i)=>i===13?'Final instruction: enter the blue door.':`Instruction ${i+1}: cross the next platform.`).join('\n');
  await evaluate(`testState.navigation.guidance.now={...testState.navigation.guidance.now,instructions:${JSON.stringify(longText)}};testRender(testState);window.readingViewport=document.querySelector('.walkthrough-copy')`);
  await readable();
  const top=await evaluate(`readingViewport.scrollTop`);assert.equal(top,0);
  await until(()=>evaluate('readingViewport.scrollTop>24'),'Long instruction did not scroll automatically');
  let prior=await evaluate('readingViewport.scrollTop');
  const beforeUpdates=prior,samples=[];
  const metrics=()=>evaluate(`({at:performance.now(),top:readingViewport.scrollTop,height:readingViewport.clientHeight,extent:readingViewport.scrollHeight})`);
  samples.push(await metrics());
  for(let i=0;i<12;i++){
    await evaluate(`testState.navigation.location={status:'estimated',detail:'Background observation ${i}'};testState.estimate={detail:'Frame ${i}'};testState.preparation={message:'Preparing references · snapshot ${i}'};testRender(testState)`);
    await new Promise(resolve=>setTimeout(resolve,100));
    assert(await evaluate('readingViewport===document.querySelector(".walkthrough-copy")&&readingViewport.scrollTop>='+prior),'Live observations reset passive reading');
    prior=await evaluate('readingViewport.scrollTop');samples.push(await metrics());
  }
  assert(prior>beforeUpdates+8,'Preparation updates kept postponing automatic reading: '+JSON.stringify(samples));
  await until(()=>evaluate(`(()=>{const copy=readingViewport.querySelector('.instructions'),range=document.createRange();range.setStart(copy.firstChild,copy.textContent.lastIndexOf('Final instruction'));range.setEnd(copy.firstChild,copy.textContent.length);const r=range.getBoundingClientRect(),v=readingViewport.getBoundingClientRect();return r.top>=v.top&&r.bottom<=v.bottom})()`),'Final instruction never became visible automatically');
  await readable();await screenshot('walkthrough-long-instruction');
  await evaluate(`testState.progress.current_step='action-9';testState.navigation.guidance.now=testState.guide.steps.find(s=>s.id==='action-9');testRender(testState)`);
  await until(()=>evaluate(`document.querySelector('[aria-current=step]').dataset.stepId==='action-9'&&readingViewport.scrollTop===0`),'New action did not start at its first words');
  // An explicit hide still removes the complete consolidated view.
  await evaluate(`testState.widget_visibility['walkthrough-outline']=false;testRender(testState)`);
  assert.equal(await evaluate(`document.querySelectorAll('.walkthrough,[data-widget-id="current-step"]').length`),0);
  if(process.env.GAMEBUDDY_TEST_WALKTHROUGH_STATE){
    const replay=JSON.parse(readFileSync(process.env.GAMEBUDDY_TEST_WALKTHROUGH_STATE,'utf8'));
    replay.widget_visibility={'walkthrough-outline':true,'current-step':true,'guide-library':false,'location-map':false};
    await evaluate(`window.testState=${JSON.stringify(replay)};testRender(testState);window.walkthroughNode=document.querySelector('[data-widget-id="walkthrough-outline"]');walkthroughNode.style.cssText='left:20px;top:20px;width:400px;height:550px'`);
    await readable();
    assert.equal(await evaluate(`document.querySelector('.outline-next .instructions').textContent`),replay.navigation.guidance.now.instructions);
    assert.equal(await evaluate(`document.querySelectorAll('[data-widget-id="current-step"]').length`),0);
    await screenshot('walkthrough-saved-game',{x:20,y:20,width:400,height:550,scale:1});
    console.log('PASS saved walkthrough:',replay.guide.game,replay.guide.steps.length,'source steps');
  }
  await evaluate(`window.testState=${JSON.stringify(saved)};testRender(testState)`);
  console.log('PASS passive Qt walkthrough: one current action, automatic context sizing, chapter advance, long-instruction reading, stable live updates and no manual scrollbars');
}
