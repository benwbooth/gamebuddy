// Measure actual recorded output over the request-to-answer interval.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
const [directory] = process.argv.slice(2);
assert(directory.startsWith('/tmp/gamebuddy-realtime-test-'));
const log = readFileSync(directory+'/runtime.log','utf8');
const window = log.match(/GAMEBUDDY_WORK_PROGRESS_PASS start_ms=(\d+) end_ms=(\d+)/);
assert(window, 'Missing native work-progress result');
const recording = Number(readFileSync(directory+'/recording-start-ms','utf8'));
const start = (Number(window[1])-recording)/1000, duration = (Number(window[2])-Number(window[1]))/1000;
assert(duration >= 35, 'Test did not span a long request');
const output = spawnSync('ffmpeg',['-nostdin','-hide_banner','-ss',String(start),'-t',String(duration),'-i',directory+'/response.wav','-af','silencedetect=noise=-45dB:d=0.15','-f','null','-'],{encoding:'utf8'});
assert.equal(output.status,0,output.stderr);
const gaps=[...output.stderr.matchAll(/silence_duration: ([\d.]+)/g)].map(m=>Number(m[1]));
assert(gaps.length,'No silence intervals found in the recording');
const longest=Math.max(...gaps);
assert(longest<12,`Recorded dead air lasted ${longest.toFixed(2)} seconds`);
console.log(`Recorded work progress: ${duration.toFixed(1)} seconds pending, longest silent gap ${longest.toFixed(2)} seconds.`);
