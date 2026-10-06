// The overlay is a passive view of the route. The guide library owns browsing.
function walkthroughContext(content, sourceSteps=content.guide.steps) {
  const steps=content.guide.steps||[],byId=new Map(steps.map(s=>[s.id,s])),source=new Map(sourceSteps.map(s=>[s.id,s]));
  const current=content.progress?.current_step,now=content.guidance?.now?.id===current?content.guidance.now:byId.get(current);
  const completed=new Set([...(content.progress?.completed||[]),...(content.progress?.reported_completed||[])]);
  const skipped=new Map((content.planning?.not_needed||[]).map(s=>[s.step_id,s.reason]));
  function inherited(id,values){const seen=new Set();while(id&&!seen.has(id)){if(values.has(id))return true;seen.add(id);id=(source.get(id)||byId.get(id))?.parent_id}return false}
  function path(step){const result=[],seen=new Set();let id=step?.parent_id;while(id&&!seen.has(id)){seen.add(id);const parent=byId.get(id);if(!parent)break;result.unshift(parent.title);id=parent.parent_id}return result}
  const children=new Map();for(const step of steps){const parent=byId.has(step.parent_id)?step.parent_id:null;if(!children.has(parent))children.set(parent,[]);children.get(parent).push(step)}
  const ordered=[],seen=new Set();function visit(parent){for(const step of children.get(parent)||[]){if(seen.has(step.id))continue;seen.add(step.id);if(!children.has(step.id)||step.id===current)ordered.push(step);visit(step.id)}}visit(null);
  const position=ordered.findIndex(s=>s.id===current),describe=step=>({id:step.id,title:step.title,instructions:step.instructions,path:path(step).join(' › '),done:inherited(step.id,completed),skipped:inherited(step.id,skipped)});
  const earlier=position>=0?ordered.slice(Math.max(0,position-3),position).map(describe):[];
  const preview=content.guidance?.next||[],future=preview.length?preview:position>=0?ordered.slice(position+1):[];
  const used=new Set([current]);
  const later=now?future.filter(s=>{if(used.has(s.id)||inherited(s.id,completed)||inherited(s.id,skipped))return false;used.add(s.id);return true}).slice(0,4).map(describe):[];
  return {guide:content.guide.id,now:now?{id:now.id,title:now.title,instructions:now.instructions}:null,path:path(byId.get(current)||now).join(' › '),earlier,later,
    recoveryStatus:content.recovery?.status,laterLabel:preview.length?'Coming up':'Later in walkthrough',status:content.guidance?.status||'ready',reason:['pending','repairing','needs_information'].includes(content.recovery?.status)?(content.recovery.message||'Updating the walkthrough…'):(content.guidance?.reason||''),requirements:(content.guidance?.requirements||[]).map(s=>s.title),preparation:content.preparation?.message||''};
}

function mountWalkthrough(body,initial) {
  body.classList.add('walkthrough');
  const path=text('div','','walkthrough-path'),before=text('section','','walkthrough-context earlier'),after=text('section','','walkthrough-context later');
  const focus=text('section','','outline-next'),label=text('div','','eyebrow'),title=text('strong','','walkthrough-title');
  const viewport=text('div','','walkthrough-copy'),copy=text('div','','instructions'),notice=text('div','','walkthrough-note');
  const progress=text('div','','walkthrough-reading'),bar=text('span','');progress.append(bar);progress.setAttribute('aria-hidden','true');
  viewport.append(copy,notice);focus.append(label,title,viewport,progress);body.append(path,before,focus,after);
  let contextKey='',readingKey='',neighborsKey='',model=null,priorRows=[],nextRows=[],layoutFrame=0,animationFrame=0,dwell=0,disposed=false;
  let lastTime=0,position=0,maximum=0;
  function stopScroll(){cancelAnimationFrame(animationFrame);clearTimeout(dwell);animationFrame=0;dwell=0}
  function animate(at){
    animationFrame=0;
    if(disposed||!body.isConnected)return;
    // A content update can briefly shrink the reading area before its queued
    // layout pass. Keep the animation alive across that frame, without another
    // initial reading delay. A collapsed widget can wait for its resize event.
    if(!viewport.clientHeight){if(body.clientHeight)animationFrame=requestAnimationFrame(animate);return}
    position=Math.min(maximum,position+Math.min(100,at-lastTime)*.012);lastTime=at;
    viewport.scrollTop=position;bar.style.width=(maximum?position/maximum*100:100)+'%';
    if(position<maximum)animationFrame=requestAnimationFrame(animate);
    else dwell=setTimeout(()=>{dwell=0;position=0;viewport.scrollTop=0;bar.style.width='0%';resumeScroll()},6000);
  }
  function resumeScroll(){stopScroll();if(maximum>1&&viewport.clientHeight)dwell=setTimeout(()=>{dwell=0;lastTime=performance.now();animationFrame=requestAnimationFrame(animate)},5500)}
  function fit(){
    layoutFrame=0;if(disposed||!body.isConnected||!body.clientHeight){stopScroll();return}
    body.classList.toggle('compact',body.clientHeight<350);
    // Start with the nearest neighbor on each side; spend spare height on
    // additional context only after leaving readable space for the instruction.
    priorRows.forEach((row,i)=>row.hidden=i!==priorRows.length-1);nextRows.forEach((row,i)=>row.hidden=i!==0);
    for(const row of [...priorRows,...nextRows])row.querySelector('.walkthrough-preview').hidden=true;
    const contentHeight=copy.offsetHeight+notice.offsetHeight+(notice.hidden?0:10);
    const readable=Math.min(contentHeight,Math.max(88,body.clientHeight*.48));
    const previous=[...priorRows].reverse(),candidates=[[nextRows,1],[previous,1],[nextRows,2],[previous,2],[nextRows,3]];
    for(const [rows,index] of candidates){const row=rows[index];if(!row||rows[index-1].hidden)continue;row.hidden=false;if(viewport.clientHeight<readable)row.hidden=true}
    for(const row of [nextRows[0],priorRows.at(-1),...nextRows.slice(1),...priorRows.slice(0,-1).reverse()].filter(Boolean)){
      const preview=row.querySelector('.walkthrough-preview');if(row.hidden||!preview.textContent)continue;
      preview.hidden=false;if(viewport.clientHeight<readable)preview.hidden=true;
    }
    maximum=Math.max(0,viewport.scrollHeight-viewport.clientHeight);
    position=Math.min(position,maximum);viewport.scrollTop=position;
    progress.hidden=maximum<2;bar.style.width=(maximum?position/maximum*100:100)+'%';
    if(maximum<2)stopScroll();else if(!animationFrame&&!dwell)resumeScroll();
    console.info('GAMEBUDDY_OUTLINE '+JSON.stringify({current:model.now?.id||null,rows:[...body.querySelectorAll('[data-step-id]')].filter(n=>n.getBoundingClientRect().height>0).map(n=>({id:n.dataset.stepId,rect:bounds(n)}))}));
  }
  function scheduleFit(){if(!disposed&&!layoutFrame)layoutFrame=requestAnimationFrame(fit)}
  function neighbors(section,heading,steps){
    section.replaceChildren();section.hidden=!steps.length;if(!steps.length)return [];
    section.append(text('div',heading,'walkthrough-context-label'));const list=text('ol','');section.append(list);
    return steps.map(step=>{
      const row=text('li','','walkthrough-neighbor'+(step.done?' done':'')),detail=text('div','','walkthrough-neighbor-detail'),name=text('div',step.title,'walkthrough-neighbor-title');
      name.dataset.stepId=step.id;detail.append(name);row.append(text('span',step.done?'✓':step.skipped?'↷':'·','walkthrough-mark'),detail);
      if(step.path&&step.path!==model.path){const section=step.path.split(' › '),current=model.path.split(' › ');while(section.length&&section[0]===current[0]){section.shift();current.shift()}detail.append(text('small',section.join(' › '),'walkthrough-section'))}
      if(step.skipped)detail.append(text('small','Not needed','walkthrough-section'));
      detail.append(text('div',step.instructions||'','walkthrough-preview'));
      evidence(name,'Step sources',()=>stepEvidence([step.id]));list.append(row);return row;
    });
  }
  function update(content){
    const next=walkthroughContext(content,latestState?.guide?.steps||content.guide.steps),key=JSON.stringify(next);
    if(key===contextKey)return;contextKey=key;model=next;
    path.textContent=model.path;path.title=model.path;path.hidden=!model.path;
    const nextReading=JSON.stringify([model.guide,model.now,model.status,model.requirements,model.reason]);
    if(nextReading!==readingKey){readingKey=nextReading;stopScroll();position=0;viewport.scrollTop=0;maximum=0}
    const status=['pending','repairing'].includes(model.recoveryStatus)?'Updating walkthrough':model.requirements.length?'Check prerequisite':({ready:'Now',plan:'Suggested next step',unlocated:'Location unconfirmed',blocked:'Plan needs attention',goals_complete:'Selected goal complete'})[model.status]||'Current step';
    label.textContent=status;title.textContent=model.now?.title||(!model.reason?'Preparing player actions…':'');title.hidden=!title.textContent;
    if(model.now){title.dataset.stepId=model.now.id;title.setAttribute('aria-current','step');evidence(title,'Step sources',()=>stepEvidence([model.now.id]));evidence(copy,'Step sources',()=>stepEvidence([model.now.id]))}
    else {delete title.dataset.stepId;title.removeAttribute('aria-current');delete title.dataset.evidence;delete copy.dataset.evidence}
    if(copy.textContent!==(model.now?.instructions||''))copy.textContent=model.now?.instructions||'';copy.hidden=!copy.textContent;
    notice.textContent=[model.requirements.length?'Unconfirmed: '+model.requirements.join(' · '):'',model.reason,model.preparation].filter(Boolean).join('\n\n');notice.hidden=!notice.textContent;
    const nextNeighbors=JSON.stringify([model.earlier,model.later,model.laterLabel]);
    if(nextNeighbors!==neighborsKey){neighborsKey=nextNeighbors;priorRows=neighbors(before,'Earlier in walkthrough',model.earlier);nextRows=neighbors(after,model.laterLabel,model.later)}
    scheduleFit();
  }
  const observer=new ResizeObserver(scheduleFit);observer.observe(body);observer.observe(copy);observer.observe(notice);
  update(initial);
  return {update,destroy(){disposed=true;observer.disconnect();cancelAnimationFrame(layoutFrame);stopScroll()}};
}
