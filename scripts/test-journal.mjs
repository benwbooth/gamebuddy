import assert from 'node:assert/strict';

// Called only by the private reader integration fixture, through the real Qt
// request bridge and persisted SQLite records rather than mocked tool replies.
export async function journalInputs({call,evaluate,until,screenshot,guide,db}) {
  const original=await evaluate('structuredClone(testState)');
  await evaluate(`testState.guide=${JSON.stringify(guide())};testState.widgets=[];testState.widget_visibility={'run-journal':true,'guide-library':false,'walkthrough-outline':false,'location-map':false};testState.layouts['run-journal']={x:20,y:20,width:600,height:650};testRender(testState)`);
  await until(()=>evaluate(`document.querySelector('.journal-timeline')?.textContent.includes('0417')`),'Journal did not load via Qt');
  await evaluate(`window.journalNode=document.querySelector('.journal');document.querySelector('.journal-compose').open=true;document.querySelector('[aria-label="Note title"]').value='Eastern passage';document.querySelector('[aria-label="Note text"]').value='Return here after finding the key.';document.querySelector('[aria-label="Note text"]').focus()`);
  await evaluate(`for(let i=0;i<20;i++){testState.navigation.location.detail='Update '+i;testRender(testState)}`);
  assert(await evaluate(`journalNode===document.querySelector('.journal') && document.activeElement.getAttribute('aria-label')==='Note text' && document.activeElement.value==='Return here after finding the key.'`),'Live updates lost the journal draft or focus');
  await evaluate(`document.querySelector('.journal form').requestSubmit()`);
  await until(()=>evaluate(`document.querySelector('.journal-timeline').textContent.includes('Return here after finding the key.')`),'Note save did not return');
  assert.equal(db.prepare("SELECT count(*) AS n FROM journal_entries WHERE title='Eastern passage'").get().n,1);
  await evaluate(`document.querySelector('[aria-label="Pin Eastern passage"]').click()`);
  await until(()=>evaluate(`!!document.querySelector('[aria-label="Unpin Eastern passage"]')`),'Pin did not persist');
  await evaluate(`document.querySelector('.journal-compose').open=false;document.querySelector('[aria-label="Search journal"]').value='0417';document.querySelector('[aria-label="Search journal"]').dispatchEvent(new Event('input'))`);
  await until(()=>evaluate(`document.querySelectorAll('.journal-entry').length===1&&document.querySelector('.journal-entry').textContent.includes('0417')`),'Journal search failed');
  await evaluate(`document.querySelector('.journal-entry > button').click()`);
  await until(()=>evaluate(`document.querySelector('.journal-images img')?.naturalWidth>0`),'Retained screenshot did not load');
  await screenshot('journal-clue-screenshot',{x:20,y:20,width:600,height:650,scale:1});
  const bounds=await evaluate(`JSON.stringify(document.querySelector('[data-widget-id="run-journal"]').getBoundingClientRect().toJSON())`);
  await evaluate(`document.querySelector('.journal-images img').click()`);assert(await evaluate(`document.querySelector('.reader-lightbox img')?.src.startsWith('data:image/jpeg')`));
  assert.equal(await evaluate(`JSON.stringify(document.querySelector('[data-widget-id="run-journal"]').getBoundingClientRect().toJSON())`),bounds,'Opening a picture scrolled the overlay');
  await evaluate(`document.querySelector('.reader-lightbox button').click();document.querySelector('[aria-label="Search journal"]').value='';document.querySelector('[aria-label="Search journal"]').dispatchEvent(new Event('input'))`);
  await until(()=>evaluate(`document.querySelector('.journal-timeline').textContent.includes('Eastern passage')`),'Search reset failed');
  // Recreate the widget, proving its contents come back from storage.
  await evaluate(`testState.widget_visibility['run-journal']=false;testRender(testState);testState.widget_visibility['run-journal']=true;testRender(testState)`);
  await until(()=>evaluate(`!!document.querySelector('[aria-label="Unpin Eastern passage"]')`),'Persisted entry/pin disappeared after remount');
  await screenshot('journal-timeline',{x:20,y:20,width:600,height:650,scale:1});
  await evaluate(`document.querySelector('.journal-new-run').open=true;document.querySelector('.journal-new-run input').value='Replay';document.querySelector('.journal-new-run button').click()`);
  await until(()=>evaluate(`document.querySelector('.journal-empty')?.textContent.includes('Your run starts here')`),'New run did not isolate the timeline');
  await evaluate(`document.querySelector('[aria-label="Journal run"]').click();[...document.querySelectorAll('[aria-label="Journal run choices"] button')].find(n=>n.textContent==='Current run').click()`);
  await until(()=>evaluate(`document.querySelector('.journal-timeline').textContent.includes('Eastern passage')`),'Previous run was lost');
  await evaluate(`window.testState=${JSON.stringify(original)};testRender(testState)`);
  console.log('PASS native journal: notes, search, pins, retained screenshots, live draft/focus, remount persistence and isolated runs');
}
