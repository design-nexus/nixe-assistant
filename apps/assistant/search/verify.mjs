// SPDX-License-Identifier: GPL-3.0-only
// Focused integration failures, using fixtures rather than live search queries.
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
import {join} from 'node:path';
const load=name=>import(pathToFileURL(join(process.argv[2],'dist/tools',name+'.js')));
const {boundedText}=await load('shared');
const {searchDuckDuckGo}=await load('searchDuckDuckGo');
const {extractWebContent}=await load('extractWebContent');
await assert.rejects(boundedText(new Response('x'.repeat(1048577))),/exceeds/);
const original=globalThis.fetch;
try {
 globalThis.fetch=async()=>new Response('<form id="challenge-form">Verify</form>');
 await assert.rejects(searchDuckDuckGo('fixture challenge',1),/manual verification/);
 const controller=new AbortController();controller.abort();
 globalThis.fetch=async(_url,{signal})=>{signal.throwIfAborted();return new Response('unexpected');};
 await assert.rejects(searchDuckDuckGo('fixture cancellation',1,controller.signal),/abort/i);
 globalThis.fetch=async()=>new Response('<title>Fixture</title><meta property="article:published_time" content="2026-10-07"><body><nav>Menu</nav><main><h1>Story</h1>Verified content.</main></body>');
 const page=await extractWebContent('https://example.com/fixture');
 assert.equal(page.publishedAt,'2026-10-07');assert.ok(page.markdown.includes('Verified content'));assert.ok(!page.markdown.includes('Menu'));
}finally{globalThis.fetch=original;}
console.log('Search size, challenge, cancellation, and extraction fixtures passed');
