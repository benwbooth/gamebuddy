// Copy current reference data for load tests. Source database is read-only;
// settings, conversation history, and obsolete index revisions are excluded.
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {mkdirSync, existsSync, cpSync, writeFileSync} from 'node:fs';
import {resolve, join} from 'node:path';
const [sourcePath, destination] = process.argv.slice(2).map(p=>resolve(p));
assert(sourcePath && destination && sourcePath !== destination && !existsSync(destination), 'Use a new private fixture directory');
const source = new DatabaseSync(join(sourcePath,'gamebuddy.sqlite'),{readOnly:true});
const active = JSON.parse(source.prepare('SELECT body FROM documents WHERE key=?').get('active_guide').body);
const guide = JSON.parse(source.prepare('SELECT body FROM guides WHERE id=?').get(active).body);
mkdirSync(destination,{recursive:true});
const target = new DatabaseSync(join(destination,'gamebuddy.sqlite'));
target.exec('CREATE TABLE guides(id TEXT PRIMARY KEY,body TEXT NOT NULL); CREATE TABLE documents(key TEXT PRIMARY KEY,body TEXT NOT NULL)');
const putGuide = target.prepare('INSERT INTO guides VALUES(?,?)');
for(const row of source.prepare('SELECT id,body FROM guides').all())putGuide.run(row.id,row.body);
const put = target.prepare('INSERT INTO documents VALUES(?,?)');
const keys = ['active_guide','widget-layout',`index:${active}:${guide.revision}`,`progress:${active}:${guide.revision}`,`widgets:${active}:${guide.revision}`,
  ...['atlas','world','world-profile','world-status','world-map-status','world-observation','spatial-maps','map-track','widget-visibility','preparation'].map(k=>`${k}:${active}`)];
for(const key of keys){const row=source.prepare('SELECT body FROM documents WHERE key=?').get(key);if(row)put.run(key,row.body)}
const screen=source.prepare('SELECT body FROM documents WHERE key=?').get('live-screen');
if(screen){const capture=JSON.parse(screen.body);if(capture.jpeg)writeFileSync(join(destination,'saved-screen.jpg'),Buffer.from(capture.jpeg,'base64'))}
if(existsSync(join(sourcePath,'maps')))cpSync(join(sourcePath,'maps'),join(destination,'maps'),{recursive:true});
target.close();source.close();
console.log(JSON.stringify({destination,game:guide.game,steps:guide.steps.length,revision:guide.revision}));
