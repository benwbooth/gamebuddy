// Included inside widgets.html. Keep the reader alive across live OSD refreshes:
// browsing and typing must not be displaced by automatic location updates.
const library = (() => {
  let mounted=null, state=null, manifest=null, scope='', sequence=0, requests=new Map();
  let tab='outline', selected=null, query='', onlyBookmarks=false, page=0, mapId=null, zoom=1;
  let opened=new Set(), images=new Map(), error='', editing=false, snapshotKey='', renderBody=null, resetDetails=false;
  const button=(label,action)=>{const b=text('button',label);b.type='button';b.addEventListener('click',action);return b};
  const emit=(action,extra={})=>{
    if(!state?.guide)return;
    const id=++sequence, request={request_id:id,guide_id:state.guide.id,revision:state.guide.revision,action,...extra};
    requests.set(id,request);console.info('GAMEBUDDY_READER '+JSON.stringify(request));return id;
  };
  const current=()=>state?.navigation?.cursor?.id || state?.progress?.current_step;
  const bookmarks=()=>new Set(manifest?.reader?.bookmarks||[]);
  const steps=()=>state?.guide?.steps||[];
  const stepById=id=>steps().find(s=>s.id===id);
  function complete(id){
    const done=new Set([...(state?.progress?.completed||[]),...(state?.progress?.reported_completed||[])]);
    let step=stepById(id);while(step){if(done.has(step.id))return true;step=stepById(step.parent_id)}
    const children=steps().filter(s=>s.parent_id===id);return children.length>0&&children.every(s=>complete(s.id));
  }
  function mutate(action,extra){if(editing)return;editing=true;error='';emit(action,extra);renderBody?.()}
  function checkbox(step){
    const input=document.createElement('input');input.type='checkbox';input.checked=complete(step.id);input.disabled=editing;
    input.setAttribute('aria-label','Completed: '+step.title);
    input.title='Save your completion report. Unchecking a chapter clears completion inside it.';
    input.addEventListener('change',()=>mutate('complete',{step_id:step.id,enabled:input.checked,epoch:state.progress?.epoch||0}));return input;
  }
  function link(parent,label,url){
    const safe=urlFor(url);if(!safe)return;
    const b=button(label,()=>console.info('GAMEBUDDY_SOURCE_OPEN '+JSON.stringify(safe)));b.className='reader-link';b.title=safe;parent.append(b);
  }
  function sources(parent,entries){
    for(const entry of entries){const card=text('div','','reader-card');card.append(text('strong',entry.title||'Reference'));if(entry.detail)card.append(text('div',entry.detail,'reader-status'));link(card,'Open source',entry.url);parent.append(card)}
  }
  function choose(id){selected=id;tab='outline';page=0;resetDetails=true;let step=stepById(id);while(step?.parent_id){opened.add(step.parent_id);step=stepById(step.parent_id)}renderBody?.()}
  function stepLink(parent,id){const step=stepById(id);if(step)parent.append(button(step.title,()=>choose(id)))}
  function pager(parent,total){
    page=Math.min(page,Math.max(0,Math.ceil(total/8)-1));
    const row=text('div','','reader-actions'),prev=button('Previous pictures',()=>{page--;renderBody()}),next=button('Next pictures',()=>{page++;renderBody()});
    prev.disabled=page===0;next.disabled=(page+1)*8>=total;
    row.append(prev,text('span',total?`${page*8+1}–${Math.min(total,(page+1)*8)} of ${total}`:'No pictures indexed yet','reader-status'),next);parent.append(row);
  }
  function lightbox(frame){
    const cached=images.get(frame.id);if(!cached?.data||!mounted)return;
    mounted.querySelector('.reader-lightbox')?.remove();
    const box=text('div','','reader-lightbox'),img=document.createElement('img');box.setAttribute('role','dialog');box.setAttribute('aria-label','Walkthrough picture');
    const close=button('Close picture',()=>{box.remove();regions()});box.append(close);img.src=cached.data;img.alt='Walkthrough at '+time(frame.at_ms);box.append(img);
    sources(box,[{title:state.guide.sources.find(s=>s.id===frame.source_id)?.title,detail:'Source screenshot at '+time(frame.at_ms),url:urlFor(state.guide.sources.find(s=>s.id===frame.source_id)?.url,frame.at_ms)}]);
    box.addEventListener('keydown',event=>{if(event.key==='Escape'){event.stopPropagation();box.remove()}});mounted.append(box);close.focus();regions();
  }
  function gallery(parent,frames){
    pager(parent,frames.length);const grid=text('div','','reader-gallery');parent.append(grid);
    const missing=[];
    for(const frame of frames.slice(page*8,page*8+8)){
      const figure=document.createElement('figure'),img=document.createElement('img'),caption=text('figcaption','');
      img.dataset.readerImage=frame.id;img.alt='Walkthrough screenshot at '+time(frame.at_ms);img.tabIndex=0;
      const source=state.guide.sources.find(s=>s.id===frame.source_id);
      caption.append(text('div',(source?.title||frame.source_id)+' · '+time(frame.at_ms)));
      link(caption,'Watch this moment',urlFor(source?.url,frame.at_ms));
      for(const id of frame.steps||[])stepLink(caption,id);
      img.addEventListener('click',()=>lightbox(frame));img.addEventListener('keydown',e=>{if(e.key==='Enter')lightbox(frame)});
      const cached=images.get(frame.id);if(cached?.data)img.src=cached.data;else if(cached?.error)caption.append(text('div',cached.error,'reader-error'));else missing.push(frame.id);
      evidence(figure,'Screenshot source',[{title:source?.title,detail:time(frame.at_ms),url:urlFor(source?.url,frame.at_ms)}]);figure.append(img,caption);grid.append(figure);
    }
    const pending=new Set([...requests.values()].filter(r=>r.action==='images'&&r.guide_id===scope&&r.revision===state.guide.revision).flatMap(r=>r.ids));
    const needed=missing.filter(id=>!pending.has(id));if(needed.length)emit('images',{ids:needed});
  }
  function mapCard(parent,map){
    const card=text('div','','reader-card');card.append(text('h3',map.title));sources(card,mapEvidence(map));
    const controls=text('div','','reader-actions');controls.append(button('−',()=>{zoom=Math.max(1,zoom/1.5);renderBody()}),button('+',()=>{zoom=Math.min(8,zoom*1.5);renderBody()}),button('Fit map',()=>{zoom=1;renderBody()}));card.append(controls);
    const view=text('div','','reader-map-view'),img=document.createElement('img');img.alt=map.title;img.dataset.readerMap=map.id;img.style.width=(zoom*100)+'%';
    const safe=urlFor(map.url);if(safe)img.src=safe;
    img.addEventListener('error',()=>{if(img.src.startsWith('data:'))return;img.hidden=true;view.append(text('div','Map preview unavailable. Open the original map image above.','reader-error'))});view.append(img);card.append(view);parent.append(card);
    // Cached original pixels stay local; this callback also handles offline maps.
    queueMicrotask(()=>console.info('GAMEBUDDY_MAP_REQUEST '+map.id));
    const atlas=manifest?.atlas||state.navigation?.map||{};
    for(const place of (atlas.places||[]).filter(p=>p.map_id===map.id)){
      const box=text('div','','reader-card');box.append(text('strong',place.label),text('div',place.landmarks||'','reader-status'));
      const association=(atlas.map_links||[]).find(l=>l.place_id===place.id);if(association)box.append(text('div',association.evidence,'reader-status'));
      for(const binding of (atlas.bindings||[]).filter(b=>b.place_id===place.id)){stepLink(box,binding.step_id);box.append(text('div',binding.evidence,'reader-status'))}card.append(box);
    }
  }
  function detail(parent,step){
    parent.append(text('h2',step.title));
    const actions=text('div','','reader-actions'),label=text('label','Completed ');label.prepend(checkbox(step));actions.append(label);
    const saved=bookmarks().has(step.id),goal=state.guide.goals?.some(g=>g.id==='reader-goal:'+step.id);
    const star=button(saved?'★ Bookmarked':'☆ Bookmark',()=>mutate('bookmark',{step_id:step.id,enabled:!saved}));star.disabled=editing;actions.append(star);
    const target=button(goal?'Remove my goal':'Make this a goal',()=>mutate('goal',{step_id:step.id,enabled:!goal}));target.disabled=editing;actions.append(target);parent.append(actions);
    if(step.id===current())parent.append(text('p','Current walkthrough position','location'));
    parent.append(text('p',step.instructions,'reader-copy'));
    if(step.optional)parent.append(text('p','Optional task','reader-status'));
    for(const signal of step.completion_signals||[])parent.append(text('div','Completion check: '+signal,'reader-status'));
    const route=text('div','','reader-card');route.append(text('h3','Order & dependencies'));
    route.append(text('div','Required before this task (model-inferred):','reader-status'));for(const id of step.prerequisites||[])stepLink(route,id);
    if(!step.prerequisites?.length)route.append(text('span','No explicit prerequisites recorded.','reader-status'));
    route.append(text('div','Next in the recorded route:','reader-status'));for(const id of step.next||[])stepLink(route,id);
    const children=steps().filter(s=>s.parent_id===step.id);if(children.length){route.append(text('div','Substeps:','reader-status'));for(const child of children)stepLink(route,child.id)}parent.append(route);
    parent.append(text('h3','Source evidence'));sources(parent,stepEvidence([step.id]));
    const atlas=manifest?.atlas||{};let bound=step,binding;
    while(bound&&!binding){binding=atlas.bindings?.find(b=>b.step_id===bound.id);bound=stepById(bound.parent_id)}
    const place=atlas.places?.find(p=>p.id===binding?.place_id),map=atlas.maps?.find(m=>m.id===place?.map_id);
    if(place){parent.append(text('h3','Place & map'),text('div',place.label+' · '+(binding?.evidence||''),'reader-status'));if(map)parent.append(button('Inspect map: '+map.title,()=>{mapId=map.id;tab='maps';zoom=1;renderBody()}))}
    parent.append(text('h3','Walkthrough pictures'));
    const frames=(manifest?.frames||[]).filter(f=>(step.anchors||[]).some(a=>a.source_id===f.source_id && a.start_ms!=null && f.at_ms>=a.start_ms && f.at_ms<(a.end_ms??Infinity)) || f.steps?.includes(step.id));
    gallery(parent,frames);
  }
  function outline(parent){
    const split=text('div','','reader-split'),tree=text('div','','reader-tree'),panel=text('article','','reader-detail');parent.append(split);split.append(tree,panel);
    const all=steps(),children=new Map(),matches=new Set(),term=query.toLowerCase(),marks=bookmarks();
    const expand=text('div','','reader-actions');expand.append(button('Expand all',()=>{opened=new Set(all.map(s=>s.id));renderBody()}),button('Collapse all',()=>{opened.clear();renderBody()}));tree.append(expand);
    for(const step of all){const key=step.parent_id||'';if(!children.has(key))children.set(key,[]);children.get(key).push(step);
      if((!term||(step.title+' '+step.instructions).toLowerCase().includes(term))&&(!onlyBookmarks||marks.has(step.id))){let s=step;while(s){matches.add(s.id);s=stepById(s.parent_id)}}}
    function branch(id){const ul=document.createElement('ul');for(const step of children.get(id)||[]){if(!matches.has(step.id))continue;
      const li=document.createElement('li'),row=text('div','','reader-row'+(step.id===selected?' selected':'')+(step.id===current()?' current':''));row.dataset.readerStep=step.id;
      const title=button((marks.has(step.id)?'★ ':'')+step.title,()=>choose(step.id));if(step.id===current())title.setAttribute('aria-current','step');evidence(title,'Step sources',()=>stepEvidence([step.id]));row.append(checkbox(step),title);
      if(children.has(step.id)){const d=document.createElement('details'),summary=text('summary','');d.open=!!query||onlyBookmarks||opened.has(step.id);summary.append(row);d.append(summary,branch(step.id));d.addEventListener('toggle',()=>{if(!d.isConnected)return;if(d.open)opened.add(step.id);else opened.delete(step.id)});li.append(d)}else li.append(row);ul.append(li)}return ul}
    tree.append(branch(''));if(!matches.size)tree.append(text('p','No matching steps.'));
    const step=stepById(selected);if(step)detail(panel,step);else panel.append(text('p','Choose any step to inspect its instructions, pictures, maps and sources. Browsing does not move your live walkthrough position.'));
  }
  function renderSources(parent){
    parent.append(text('h2','Reference material'));
    for(const source of state.guide.sources){
      const card=text('div','','reader-card');card.append(text('h3',source.title),text('div',source.kind+(source.duration_ms?' · '+time(source.duration_ms):''),'reader-status'));
      const metadata=manifest?.source_metadata?.[source.id];if(metadata?.channel)card.append(text('p','Channel: '+metadata.channel,'reader-status'));
      const pref=state.guide.reference_preferences?.find(p=>p.source_id===source.id);
      card.append(text('p',pref?.enabled===false?'Disabled reference':'Enabled reference','reader-status'));if(pref?.preference)card.append(text('p',pref.preference));
      link(card,'Open original source',source.url);if(!source.url)card.append(text('p','Local/imported material; no original URL was recorded.','reader-status'));
      if(source.kind!=='text')card.append(text('p',source.captions_path?'Timed captions were imported.':'No imported caption file. See coverage notes for transcription or missing audio.','reader-status'));
      const matches=steps().filter(s=>s.anchors?.some(a=>a.source_id===source.id));
      card.append(text('p',matches.length+' outline nodes cite this source.','reader-status'),button('Browse cited steps',()=>{query='';onlyBookmarks=false;choose(matches[0]?.id)}));parent.append(card);
    }
  }
  function mount(body,initial){
    body.classList.add('library');mounted=body;
    const tools=text('div','','reader-tools'),search=document.createElement('input');search.type='search';search.placeholder='Search the complete outline';search.setAttribute('aria-label','Search guide');search.value=query;
    search.addEventListener('input',()=>{query=search.value;tab='outline';page=0;renderBody()});tools.append(search,button('Current step',()=>{query='';search.value='';onlyBookmarks=false;choose(current());body.querySelector('.reader-row.current')?.scrollIntoView({block:'nearest'})}));
    const star=button('Bookmarks',()=>{onlyBookmarks=!onlyBookmarks;tab='outline';renderBody()});tools.append(star);
    const tabs=text('nav','','reader-tabs'),status=text('div','','reader-status'),notice=text('div','','reader-error'),content=text('div','','reader-content');notice.setAttribute('role','status');status.setAttribute('aria-live','polite');
    const tabButtons=[];for(const [id,label] of [['outline','Outline'],['pictures','All pictures'],['maps','All maps'],['goals','My goals'],['sources','Sources'],['history','How it was built']]){const b=button(label,()=>{tab=id;page=0;renderBody()});tabButtons.push([id,b]);tabs.append(b)}
    body.append(tools,tabs,status,notice,content);
    renderBody=()=>{
      if(!mounted?.isConnected && mounted!==body)return;
      if(body.querySelector('.picker.open')){body.dataset.readerDeferred='true';return}delete body.dataset.readerDeferred;
      const treeScroll=content.querySelector('.reader-tree')?.scrollTop||0,detailScroll=resetDetails?0:content.querySelector('.reader-detail')?.scrollTop||0,scroll=content.scrollTop;resetDetails=false;
      content.replaceChildren();notice.textContent=error;if(error)notice.append(button('Reload library',()=>{error='';emit('snapshot')}));star.setAttribute('aria-pressed',String(onlyBookmarks));for(const [id,b] of tabButtons)b.setAttribute('aria-pressed',String(tab===id));
      status.textContent=(state.preparation?.message||'')+' · '+steps().length+' nodes · '+(manifest?.frames?.length||0)+' pictures · '+(manifest?.atlas?.maps?.length||0)+' maps'+(editing?' · Saving…':'')+(manifest?'':' · Loading library…');
      if(tab==='outline')outline(content);
      else if(tab==='pictures')gallery(content,manifest?.frames||[]);
      else if(tab==='maps'){
        const maps=manifest?.atlas?.maps||[];if(!maps.length)content.append(text('p','Maps are still being prepared, or none have been attached. You can ask GameBuddy to add a reference map.'));
        else {if(!maps.some(m=>m.id===mapId))mapId=maps[0].id;const select=picker('Library map',maps.map(m=>({id:m.id,label:m.title})),id=>{mapId=id;zoom=1;renderBody()});select.set(mapId);const bar=text('div','','map-toolbar');bar.append(select.node);content.append(bar);mapCard(content,maps.find(m=>m.id===mapId))}
      } else if(tab==='sources')renderSources(content);
      else if(tab==='goals'){
        content.append(text('p','Add a step with “Make this a goal”, or ask GameBuddy to customize your goals. These are the same goals used by your guide.'));
        for(const goal of state.guide.goals||[]){const card=text('div','','reader-card');card.append(text('h3',goal.description));
          for(const id of goal.step_ids||[]){const s=stepById(id);if(s){const row=text('div','','reader-row');row.append(checkbox(s));stepLink(row,id);card.append(row)}}
          if(!goal.step_ids?.length)card.append(text('p','No steps linked yet. Ask GameBuddy to break this goal into tasks.','reader-status'));content.append(card)}
        if(!state.guide.goals?.length)content.append(text('p','No personal goals added yet.'));
      } else {
        content.append(text('h2','How this guide was assembled'),text('p',manifest?.method||'Loading recorded history…'));
        const coverage=manifest?.coverage;if(coverage){
          content.append(text('p',`${coverage.chapter_count} chapters · ${coverage.leaf_count} individual actions · ${coverage.reviewed_section_count} reviewed sections · ${coverage.pending_step_ids.length} pending sections`));
          for(const source of coverage.sources||[]){const title=state.guide.sources.find(s=>s.id===source.source_id)?.title||source.source_id;const card=text('div','','reader-card');card.append(text('strong',title),text('p',time(source.reviewed_ms)+' reviewed out of '+time(source.duration_ms)));
            for(const gap of source.unreviewed_intervals||[])card.append(text('div','Unreviewed: '+time(gap.start_ms)+'–'+time(gap.end_ms),'reader-status'));content.append(card)}
          content.append(text('p','Independent gameplay verification: '+(coverage.independent_playthrough_verified?'recorded':'not recorded'),'reader-status'));
        }
        content.append(text('h3','Coverage & evidence gaps'));for(const note of state.guide.coverage_notes||[])content.append(text('p',note,'reader-card'));
        content.append(text('h3','Revision history'));for(const revision of manifest?.history||[])content.append(text('div','Revision '+revision.revision+' · '+revision.summary,'reader-card'));
      }
      const tree=content.querySelector('.reader-tree'),detail=content.querySelector('.reader-detail');if(tree)tree.scrollTop=treeScroll;if(detail)detail.scrollTop=detailScroll;content.scrollTop=scroll;
      if(tipOwner&&!tipOwner.isConnected)hideTip();
    };
    let signature='';
    const refresh=next=>{
      state=next;const key=state.guide.id+':'+state.guide.revision;
      if(scope!==state.guide.id){body.querySelector('.reader-lightbox')?.remove();scope=state.guide.id;manifest=null;selected=null;opened=new Set();images.clear();requests.clear();snapshotKey='';editing=false;error='';query='';search.value='';onlyBookmarks=false;page=0;mapId=null}
      if(selected&&!stepById(selected))selected=null;
      if(!selected){selected=current()||steps()[0]?.id;let s=stepById(selected);while(s?.parent_id){opened.add(s.parent_id);s=stepById(s.parent_id)}}
      const wanted=key+':'+(state.reference_count||0)+':'+(state.navigation?.map?.reviewed_steps||0)+':'+(state.navigation?.map?.status||'');
      if(wanted!==snapshotKey){snapshotKey=wanted;emit('snapshot')}
      const nextSignature=JSON.stringify([key,state.progress,current(),state.preparation,state.navigation?.map?.status]);
      if(signature!==nextSignature||body.dataset.readerDeferred){signature=nextSignature;renderBody()}
    };
    refresh(initial);renderBody();return refresh;
  }
  function response(reply){
    const request=requests.get(reply.request_id);if(!request)return;requests.delete(reply.request_id);
    if(reply.guide_id!==scope||request.guide_id!==scope)return;
    const mutation=['bookmark','goal','complete'].includes(request.action);if(mutation)editing=false;
    if(reply.error){error=reply.error;if(request.action==='images')for(const id of request.ids)images.set(id,{error:reply.error});renderBody?.();return}
    const data=reply.data;
    if(request.action==='images'){
      if(request.revision!==state.guide.revision){renderBody?.();return}
      for(const item of data.images||[]){images.set(item.id,item);for(const img of mounted?.querySelectorAll('[data-reader-image]')||[]){if(img.dataset.readerImage===item.id){if(item.data)img.src=item.data;else img.after(text('span',item.error,'reader-error'))}}}
      // Bound image memory independently of the length of the source video.
      while(images.size>48)images.delete(images.keys().next().value);
      return;
    }
    if(request.action==='snapshot'){
      if(data.revision!==state.guide.revision)return;
      manifest=data;
    } else {
      if(data.reader&&manifest)manifest.reader=data.reader;
      if(data.progress)state={...state,progress:data.progress};
    }
    error='';renderBody?.();
  }
  function mapPreview(id,data){if(!data.startsWith('data:image/jpeg;base64,'))return;for(const img of mounted?.querySelectorAll('[data-reader-map]')||[])if(img.dataset.readerMap===id){img.hidden=false;img.src=data}}
  return {mount,response,mapPreview,unmount(){mounted=null;renderBody=null}};
})();
