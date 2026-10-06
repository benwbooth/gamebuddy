import assert from 'node:assert/strict';
import {existsSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';

export async function seenScreenInputs({evaluate,until,screenshot,root}) {
  const saved=await evaluate('structuredClone(testState)');
  // The reader fixture already has local images; an optional recorded game
  // frame also verifies the real JPEG/crop dimensions in the native renderer.
  const path=process.env.GAMEBUDDY_TEST_SEEN_FRAME||root+'/picture-0.jpg';
  assert(existsSync(path),'Recorded game frame does not exist');
  const state=JSON.parse(execFileSync(fileURLToPath(new URL('../target/debug/examples/seen_screen_probe',import.meta.url)),[root,path],{encoding:'utf8'}));
  const seen=state.seen_screen,id=seen.assistant.image_id;
  await evaluate(`testState.seen_screen=${JSON.stringify(seen)};testState.widget_visibility['last-seen-screen']=true;testRender(testState);window.seenWidget=document.querySelector('[data-widget-id="last-seen-screen"]');window.seenImage=seenWidget.querySelector('img')`);
  await until(()=>evaluate(`seenImage.complete&&seenImage.naturalWidth>0&&!seenImage.hidden`),'Exact shared screenshot did not load through Qt');
  assert.equal(await evaluate('seenImage.dataset.imageId'),id);
  const pixels=await evaluate('seenImage.src');
  assert(pixels.startsWith("data:image/jpeg;base64,"));
  assert(await evaluate(`seenWidget.querySelector('small').textContent.includes('Sent for answer')`));
  for(let i=0;i<12;i++)await evaluate(`testState.screen={captured_at_ms:Date.now()+${i},status:'live'};testState.frame_url='file:///newer-capture-must-not-appear.jpg';testRender(testState)`);
  assert(await evaluate(`seenWidget===document.querySelector('[data-widget-id="last-seen-screen"]')&&seenImage===seenWidget.querySelector('img')`),'Live refresh replaced the preview');
  assert.equal(await evaluate('seenImage.getAttribute("src")'),pixels,'New raw capture replaced the last shared frame');
  await evaluate(`testState.seen_screen.assistant.captured_at_ms=Date.now()-32000;testRender(testState)`);
  assert(await evaluate(`seenWidget.querySelector('small').textContent.includes('older frame')`));
  await screenshot('last-seen-game-screen');
  await evaluate(`testState.seen_screen={status:'paused'};testRender(testState)`);
  assert(await evaluate(`seenImage.hidden&&!seenImage.hasAttribute('src')&&seenWidget.innerText.includes('paused')`),'Paused preview kept revoked pixels');
  await evaluate(`testState.seen_screen={status:'waiting'};testRender(testState)`);
  assert(await evaluate(`seenImage.hidden&&seenWidget.innerText.includes('Waiting')`),'New session retained the old screenshot');
  await evaluate(`window.testState=${JSON.stringify(saved)};testRender(testState)`);
  console.log('PASS Qt exact last-seen screenshot, stable image, age, pause and session reset');
}
