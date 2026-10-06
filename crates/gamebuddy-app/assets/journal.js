// A durable playthrough timeline. Live navigation updates preserve drafts,
// search focus, expanded screenshots and the reader's scroll position.
const journal = (() => {
  let state, mounted, sequence=1000000, requests=new Map(), run=null, query='', pinned=false, generation=0, rows=[], next=null, stamp='', form, list, status, runs, search, pinFilter;
  const button=(label,action)=>{const b=text('button',label);b.type='button';b.addEventListener('click',action);return b};
  function emit(operation,extra={}) {
    if(!state?.guide)return;
    const id=++sequence, request={request_id:id,guide_id:state.guide.id,revision:state.guide.revision,action:'journal',request:{operation,...extra}};
    requests.set(id,{...request,generation});console.info('GAMEBUDDY_READER '+JSON.stringify(request));return id;
  }
  function load(older=false) {
    if(!mounted)return;
    if(!older)generation++;
    status.textContent='Loading journal…';
    emit('list',{query,run_id:run,before_id:older?next:null,pinned_only:pinned});
  }
  const labels={observed_completion:'Observed milestone',player_report:'Player reported',reader_checkbox:'Player checked',world_report:'Player reported',player_location:'Player reported',plan_recovery:'Walkthrough repair',player_note:'Your note',agent_note:'Companion note'};
  function draw() {
    if(!mounted)return;
    const top=list.scrollTop, pictures=new Map([...list.querySelectorAll('.journal-entry')].map(card=>[Number(card.dataset.journalEntry),[...card.querySelectorAll('.journal-images img')]]));list.replaceChildren();
    if(!rows.length)list.append(text('p',query||pinned?'No matching entries.':'Your run starts here. Milestones will appear as you play. Save clues, choices and anything you want to remember.','journal-empty'));
    for(const entry of rows) {
      const card=text('article','','journal-entry');card.dataset.journalEntry=entry.id;
      const header=text('div','','journal-entry-head'),heading=text('strong',entry.title),pin=button(entry.pinned?'★ Pinned':'☆ Pin',()=>emit('pin',{entry_id:entry.id,pinned:!entry.pinned}));pin.setAttribute('aria-label',(entry.pinned?'Unpin ':'Pin ')+entry.title);header.append(heading,pin);
      const date=new Date(entry.at.includes('T')?entry.at:entry.at.replace(' ','T')+'Z');
      card.append(header,text('div',`${Number.isNaN(date.getTime())?entry.at:date.toLocaleString()} · ${labels[entry.evidence_kind]||entry.evidence_kind} · ${entry.category}`,'journal-meta'),text('p',entry.body,'journal-copy'));
      if(entry.image_count) {
        const show=button(`${entry.image_count} screenshot${entry.image_count===1?'':'s'}`,()=>{show.disabled=true;emit('images',{entries:Array.from({length:Math.min(8,entry.image_count)},(_,i)=>[entry.id,i])})});const gallery=text('div','','journal-images');for(const image of pictures.get(entry.id)||[])gallery.append(image);show.disabled=gallery.childElementCount>0;card.append(show,gallery);
      }
      list.append(card);
    }
    if(next&&rows.length>=30)list.append(button('Older entries',()=>load(true)));
    list.scrollTop=top;
  }
  function response(reply) {
    const request=requests.get(reply.request_id);if(!request)return;requests.delete(reply.request_id);
    if(!mounted||reply.guide_id!==state.guide.id||(['list','images'].includes(request.request.operation)&&request.generation!==generation))return;
    if(reply.error){status.textContent=reply.error;form.querySelector('button[type=submit]').disabled=false;return;}
    const data=reply.data,operation=request.request.operation;
    if(operation==='list') {
      run=data.run_id;const menu=picker('Journal run',data.runs.map(item=>({id:item.id,label:item.title})),id=>{run=id;load()});menu.set(run);runs.replaceWith(menu.node);runs=menu.node;
      rows=request.request.before_id?[...rows,...data.entries]:data.entries;next=data.next_before_id;status.textContent='Saved across launches · Historical record';draw();
    } else if(operation==='images') {
      for(const picture of data.images){const card=list.querySelector(`[data-journal-entry="${picture.id}"]`),gallery=card?.querySelector('.journal-images');if(!gallery)continue;
        if(picture.error){gallery.append(text('p',picture.error));continue;}if(!picture.data?.startsWith('data:image/jpeg;base64,'))continue;
        const img=document.createElement('img');img.src=picture.data;img.alt='Game screenshot saved with this entry';img.tabIndex=0;
        const open=()=>{mounted.querySelector('.reader-lightbox')?.remove();const box=text('div','','reader-lightbox'),large=document.createElement('img');box.setAttribute('role','dialog');box.setAttribute('aria-label','Journal screenshot');const close=button('Close picture',()=>box.remove());box.append(close);large.src=img.src;large.alt=img.alt;pictureViewer(box,large);box.addEventListener('keydown',e=>{if(e.key==='Escape'){e.stopPropagation();box.remove()}});mounted.append(box);close.focus({preventScroll:true});regions()};
        img.addEventListener('click',open);img.addEventListener('keydown',e=>{if(e.key==='Enter')open()});gallery.append(img);
      }
    } else {
      if(operation==='note'){form.reset();form.querySelector('button[type=submit]').disabled=false;}
      if(operation==='start_run'){run=data.run_id;query='';search.value='';pinned=false;pinFilter.checked=false;}
      load();
    }
  }
  function mount(body,initial) {
    mounted=body;state=initial;body.classList.add('journal');
    const toolbar=text('div','','journal-toolbar');runs=text('div','','picker');
    search=document.createElement('input');search.type='search';search.placeholder='Search this run';search.setAttribute('aria-label','Search journal');search.value=query;let timer;
    search.addEventListener('input',()=>{clearTimeout(timer);query=search.value;timer=setTimeout(()=>load(),180)});
    const pinLabel=text('label','Pinned');pinFilter=document.createElement('input');pinFilter.type='checkbox';pinFilter.checked=pinned;pinFilter.addEventListener('change',()=>{pinned=pinFilter.checked;load()});pinLabel.prepend(pinFilter);toolbar.append(runs,search,pinLabel);
    const newRun=text('details','','journal-new-run'),newTitle=document.createElement('input');newRun.append(text('summary','Start another run'));newTitle.placeholder='Name this run';newTitle.maxLength=160;newRun.append(newTitle,button('Create run',()=>{if(newTitle.value.trim()){emit('start_run',{title:newTitle.value.trim()});newRun.open=false;newTitle.value=''}}),text('small','Keeps older journal entries. Game progress is unchanged.'));
    const compose=text('details','','journal-compose');compose.append(text('summary','Add a note'));form=document.createElement('form');
    const title=document.createElement('input');title.name='title';title.placeholder='Title';title.required=true;title.maxLength=160;title.setAttribute('aria-label','Note title');
    const copy=document.createElement('textarea');copy.name='body';copy.placeholder='Clue, code, choice, discovery…';copy.required=true;copy.maxLength=6000;copy.setAttribute('aria-label','Note text');
    const captureLabel=text('label','Include current screenshot'),capture=document.createElement('input');capture.type='checkbox';captureLabel.prepend(capture);
    const save=button('Save note',()=>{});save.type='submit';form.append(title,copy,captureLabel,save);form.addEventListener('submit',e=>{e.preventDefault();save.disabled=true;emit('note',{note:{title:title.value,body:copy.value,category:'note',capture_screenshot:capture.checked}})});compose.append(form);
    status=text('div','','journal-meta');list=text('div','','journal-timeline');body.append(toolbar,newRun,compose,status,list);
    const guideId=initial.guide.id;let first=true;
    const refresh=nextState=>{state=nextState;if(state.guide.id!==guideId)return;
      const key=JSON.stringify([state.guide.revision,state.journal?.run_id,state.journal?.latest_id,state.journal?.count]);
      if(first||key!==stamp){first=false;stamp=key;if(!run)run=state.journal?.run_id||null;load()}
    };refresh(initial);return refresh;
  }
  return {mount,response,unmount(){mounted=null;generation++;requests.clear();stamp='';run=null;rows=[]}};
})();
