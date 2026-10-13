import test from 'node:test';import assert from 'node:assert/strict';import {webUrl,isChallenge,sourceLinks} from './worker.mjs';
test('URL boundaries, challenge detection, and cited result extraction',()=>{assert.throws(()=>webUrl('file:///etc/passwd'));assert.throws(()=>webUrl('https://user:pass@example.com'));assert.equal(webUrl('https://example.com'),'https://example.com/');assert.ok(isChallenge('Google','Our systems have detected unusual traffic'));assert.ok(!isChallenge('News about CAPTCHAs','A research article'));assert.deepEqual(sourceLinks([{title:'Source',href:'https://www.google.com/url?q=https%3A%2F%2Fexample.com',snippet:'Summary'},{title:'Duplicate',href:'https://example.com'},{title:'Unsafe',href:'javascript:alert(1)'}],'google').results,[{title:'Source',url:'https://example.com/',snippet:'Summary'}]);});

import {Browser} from './worker.mjs';
import {createServer} from 'node:http';import {mkdtemp,rm} from 'node:fs/promises';import {tmpdir} from 'node:os';import {join} from 'node:path';
test('real browser references, HTTP failures, tabs, and takeover boundaries',async()=>{
 const dir=await mkdtemp(join(tmpdir(),'assistant-browser-'));process.env.ASSISTANT_BROWSER_DATA=dir;process.env.ASSISTANT_BROWSER_HEADLESS='1';
 const server=createServer((req,res)=>{if(req.url==='/missing'){res.writeHead(404);res.end('Missing');return;}res.setHeader('content-type','text/html');res.end('<title>Fixture</title><body><input aria-label="Message"><button id="original" onclick="this.textContent=\'Clicked original\'">Original</button><button id="other">Other</button></body>');});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const url=`http://127.0.0.1:${server.address().port}`;const b=new Browser();
 try {const snapshot=await b.call('browser.navigate',{url});const ref=snapshot.elements.find(e=>e.name==='Original').ref;const page=b.pages.get('visible');
  await page.evaluate(()=>document.body.prepend(document.getElementById('other')));const clicked=await b.call('browser.click',{ref});assert.ok(clicked.text.includes('Clicked original'));
  const input=clicked.elements.find(e=>e.name==='Message').ref;await b.call('browser.fill',{ref:input,text:'Literal $() `quotes`'});assert.equal(await page.locator('input').inputValue(),'Literal $() `quotes`');
  const stale=(await b.call('browser.snapshot',{})).elements.find(e=>e.name==='Other').ref;await page.locator('#other').evaluate(el=>el.remove());await assert.rejects(b.call('browser.click',{ref:stale}),/Detached reference/);
  await assert.rejects(b.call('web.read',{url:url+'/missing'}),/HTTP 404/);
  await b.call('browser.remember_sources',{sources:[{title:'External citation',url}]});assert.equal(b.lastSources[0].url,url+'/');assert.equal((await b.call('browser.open_source',{})).url,url+'/');
  await assert.rejects(b.call('browser.remember_sources',{sources:[{title:'Unsafe',url:'file:///etc/passwd'}]}),/HTTP/);
  await b.call('browser.tabs',{action:'create',url});assert.equal((await b.call('browser.tabs',{})).tabs.length,2);await b.call('browser.tabs',{action:'close',index:1});
  await b.call('browser.takeover',{});await assert.rejects(b.call('browser.navigate',{url}),/paused/);assert.equal((await b.call('browser.resume',{})).resumed,true);
 }finally{await b.close();await new Promise(r=>server.close(r));await rm(dir,{recursive:true,force:true});}
});
