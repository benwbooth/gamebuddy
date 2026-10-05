// Native wheel scrolling keeps pixel deltas, horizontal axes and inertia.
// Chromium reports trackpad pinch as Ctrl+wheel; keep it inside the image
// instead of magnifying the entire overlay and invalidating its input regions.
document.addEventListener('wheel',event=>{if(event.ctrlKey)event.preventDefault()}, {passive:false});
function imageGestures(view,stage,{getZoom,setZoom,manual=()=>{}}){
  view.classList.add('image-viewport');view.tabIndex=0;
  view.setAttribute('aria-label','Image viewer. Scroll to pan; pinch or Control plus scroll to zoom.');
  view.title='Scroll to pan · Pinch or Ctrl+scroll to zoom · Drag to pan';
  const points=new Map();let previous=null;
  const center=()=>{const r=view.getBoundingClientRect();return {x:r.left+view.clientWidth/2,y:r.top+view.clientHeight/2}};
  function zoomTo(value,at=center()){
    const before=stage.getBoundingClientRect();if(!before.width||!before.height)return;
    const x=(at.x-before.left)/before.width,y=(at.y-before.top)/before.height;
    manual();setZoom(clamp(value,1,16));
    const after=stage.getBoundingClientRect();
    view.scrollLeft+=after.left+x*after.width-at.x;
    view.scrollTop+=after.top+y*after.height-at.y;
  }
  function fit(){manual();setZoom(1);view.scrollTo(0,0)}
  view.addEventListener('wheel',event=>{
    hideTip();
    if(event.ctrlKey){
      event.preventDefault();event.stopPropagation();
      const unit=event.deltaMode===1?16:event.deltaMode===2?view.clientHeight:1;
      zoomTo(getZoom()*Math.exp(-event.deltaY*unit*0.01),{x:event.clientX,y:event.clientY});
    }else if(event.deltaX||event.deltaY)manual();
  },{passive:false});
  const sample=()=>{
    const p=[...points.values()];if(!p.length)return null;
    return p.length===1?{...p[0],distance:0}:{x:(p[0].x+p[1].x)/2,y:(p[0].y+p[1].y)/2,distance:Math.hypot(p[0].x-p[1].x,p[0].y-p[1].y)};
  };
  view.addEventListener('pointerdown',event=>{
    if(event.button!==0||event.target.closest('button,a,input'))return;
    // Leave native scrollbar interaction to Chromium.
    const r=view.getBoundingClientRect();if(event.clientX>=r.left+view.clientWidth||event.clientY>=r.top+view.clientHeight)return;
    event.preventDefault();hideTip();view.focus({preventScroll:true});
    points.set(event.pointerId,{x:event.clientX,y:event.clientY});previous=sample();
    view.setPointerCapture(event.pointerId);view.classList.add('panning');
  });
  view.addEventListener('pointermove',event=>{
    if(!points.has(event.pointerId))return;
    points.set(event.pointerId,{x:event.clientX,y:event.clientY});const next=sample();
    manual();
    if(next.distance&&previous.distance)zoomTo(getZoom()*next.distance/previous.distance,previous);
    view.scrollLeft-=next.x-previous.x;view.scrollTop-=next.y-previous.y;previous=next;
  });
  const end=event=>{points.delete(event.pointerId);previous=sample();if(!points.size)view.classList.remove('panning')};
  for(const type of ['pointerup','pointercancel','lostpointercapture'])view.addEventListener(type,end);
  view.addEventListener('dragstart',event=>event.preventDefault());
  view.addEventListener('keydown',event=>{
    if(event.key==='+'||event.key==='='){event.preventDefault();zoomTo(getZoom()*1.5)}
    else if(event.key==='-'){event.preventDefault();zoomTo(getZoom()/1.5)}
    else if(event.key==='0'){event.preventDefault();fit()}
    else if(['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','PageUp','PageDown','Home','End'].includes(event.key))manual();
  });
  return {zoomTo,fit};
}
function pictureViewer(parent,img){
  const tools=text('div','','reader-actions'),view=text('div','','picture-viewport'),stage=text('div','','map-stage');
  let scale=1;stage.style.width='100%';stage.append(img);view.append(stage);parent.append(tools,view);
  const gestures=imageGestures(view,stage,{getZoom:()=>scale,setZoom:value=>{scale=value;stage.style.width=scale*100+'%'}});
  for(const [label,action] of [['Zoom out',()=>gestures.zoomTo(scale/1.5)],['Zoom in',()=>gestures.zoomTo(scale*1.5)],['Fit image',gestures.fit]]){
    const b=text('button',label,'action-button');b.type='button';b.addEventListener('click',action);tools.append(b);
  }
  return view;
}
