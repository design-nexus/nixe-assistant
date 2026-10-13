import { chromium } from 'playwright';
import { createInterface } from 'node:readline';
import { mkdir, chmod } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
export function webUrl(input) { const u = new URL(input); if (!['http:','https:'].includes(u.protocol) || u.username || u.password) throw Error('Only HTTP(S) URLs without credentials are supported'); return u.href; }
export function isChallenge(title,text) { return /captcha\b|verify (?:you are|you're) human|unusual traffic|robot check|just a moment|access denied/i.test(title) || /Unfortunately, bots use DuckDuckGo|Please complete the following challenge|Our systems have detected unusual traffic/i.test(text); }
export function sourceLinks(links,engine) {
 const seen=new Set();const results=[];
 for (const a of links) {let url;try {let u=new URL(a.href);if(u.hostname.endsWith('google.com') && u.pathname==='/url')u=new URL(u.searchParams.get('q')||u.searchParams.get('url'));if(u.hostname.endsWith('duckduckgo.com')&&u.searchParams.has('uddg'))u=new URL(u.searchParams.get('uddg'));url=webUrl(u.href);if(u.hostname.endsWith('google.com')||u.hostname.endsWith('duckduckgo.com'))continue;}catch{continue;}if(!a.title?.trim()||seen.has(url))continue;seen.add(url);results.push({title:a.title.trim().slice(0,300),url,snippet:(a.snippet||'').slice(0,600)});if(results.length===5)break;}
 return {engine,results};
}
async function navigate(p,url){const response=await p.goto(webUrl(url),{waitUntil:'domcontentloaded'});if(response&&!response.ok())throw Error(`HTTP ${response.status()} for ${url}`);}
const schema=(properties={},required=[])=>({type:'object',properties,required,additionalProperties:false});
const str={type:'string',minLength:1,maxLength:16384}; const small={type:'integer',minimum:0,maximum:200};
const definitions=[
 ['web.search','Background DuckDuckGo search for fresh facts. Returns up to five cited links.',schema({query:str},['query'])],
 ['web.read','Read a source URL in the background. Returns dated page text and URL.',schema({url:str},['url'])],
 ['browser.search','Show Google Search results in the visible assistant Chromium browser.',schema({query:str},['query'])],
 ['browser.navigate','Open a HTTP(S) URL in the visible browser.',schema({url:str},['url'])],
 ['browser.snapshot','Read the selected visible page. Optional filter finds relevant interactive elements.',schema({filter:str})],
 ['browser.tabs','List, select, create, or close a tab.',schema({index:small,action:{enum:['list','select','create','close']},url:str})],
 ['browser.remember_sources','Retain cited sources from the configured search provider without opening pages.',schema({sources:{type:'array',maxItems:5,items:schema({title:str,url:str},['title','url'])}},['sources'])],
 ['browser.open_source','Open an already cited source directly, without searching.',schema({index:small})],
 ['browser.click','Click a page element reference. Requires confirmation because it may submit or change data.',schema({ref:str},['ref'])],
 ['browser.fill','Fill a field, without pressing Enter. Requires confirmation because sites may submit while typing.',schema({ref:str,text:{type:'string',maxLength:16384}},['ref','text'])],
 ['browser.scroll','Scroll the visible page up or down.',schema({direction:{enum:['up','down']}},['direction'])],
 ['browser.takeover','Pause automation so the user can solve a challenge or log in.',schema()],
 ['browser.resume','Resume automation after user takeover.',schema()],
];
export class Browser {
 contexts=new Map(); pages=new Map();refs=new Map();revision=0;paused=false;lastSources=[];pendingSearch=null;cachedSearch=null;
 async page(kind) {
  if(!this.contexts.has(kind)) {
   const data=process.env.ASSISTANT_BROWSER_DATA || join(process.env.XDG_DATA_HOME||join(homedir(),'.local/share'),'nixe-assistant','browser-profiles');
   const dir=join(data,kind);await mkdir(dir,{recursive:true,mode:0o700});await chmod(dir,0o700);
   const options={headless:kind==='background'||process.env.ASSISTANT_BROWSER_HEADLESS==='1',acceptDownloads:false,viewport:{width:1200,height:850}};
   if(process.env.ASSISTANT_BROWSER_EXECUTABLE)options.executablePath=process.env.ASSISTANT_BROWSER_EXECUTABLE;
   const ctx=await chromium.launchPersistentContext(dir,options);ctx.setDefaultTimeout(10000);ctx.setDefaultNavigationTimeout(10000);ctx.on('close',()=>{this.contexts.delete(kind);this.pages.delete(kind);});this.contexts.set(kind,ctx);
  }
  let p=this.pages.get(kind);if(!p||p.isClosed()){const ctx=this.contexts.get(kind);p=ctx.pages()[0]||await ctx.newPage();this.pages.set(kind,p);}return p;
 }
 async content(p,refs=false,filter="") {
  const title=await p.title(); const text=(await p.locator('body').innerText({timeout:5000})).slice(0,12000);
  const url=p.url();const blocked=isChallenge(title,text);
  const date=await p.locator('meta[property="article:published_time"],meta[name="date"],time[datetime]').evaluateAll(els=>els.length ? els[0].getAttribute('content')||els[0].getAttribute('datetime') : null);
  const result={title,url,published_at:date,retrieved_at:new Date().toISOString(),text,blocked,needs_takeover:blocked};
  if(refs){this.refs.clear();this.revision++;result.elements=[];const elements=await p.locator('a[href],button,input,textarea,select,[role="button"]').elementHandles();const n=Math.min(elements.length,1000);
   for(let i=0;i<n;i++){const el=elements[i];if(!await el.isVisible().catch(()=>false))continue;const ref=`${this.revision}:${i}`;const info=await el.evaluate(el=>({tag:el.tagName.toLowerCase(),name:(el.getAttribute('aria-label')||el.innerText||el.getAttribute('placeholder')||el.getAttribute('name')||'').slice(0,200),href:el.href||null,type:el.getAttribute('type')}));if(filter&&!JSON.stringify(info).toLowerCase().includes(filter.toLowerCase()))continue;this.refs.set(ref,{el,page:p,url});result.elements.push({ref,...info});if(result.elements.length>=100)break;}
  }
  return result;
 }
 async search(query,kind) {
  if(kind==='background'&&this.cachedSearch?.query===query){const result=this.cachedSearch.result;this.cachedSearch=null;return result;}
  this.lastSources=[];
  const p=await this.page(kind);const google=kind==='visible';const engine=google?'google':'duckduckgo';
  await p.goto((google?'https://www.google.com/search?q=':'https://duckduckgo.com/?q=')+encodeURIComponent(query),{waitUntil:'domcontentloaded'});
  await p.locator(google?'a h3':'[data-testid="result-title-a"],.result__a').first().waitFor({timeout:4000}).catch(()=>{});
  const page=await this.content(p,google);if(page.blocked){if(!google)this.pendingSearch={query,url:p.url()};return {...page,engine,results:[],message:'Search requires user takeover. No factual answer was retrieved.'};}
  const links=await p.locator(google?'a:has(h3)':'[data-testid="result-title-a"],.result__a').evaluateAll(els=>els.map(el=>({href:el.href,title:el.innerText,snippet:(el.closest('article')||el.parentElement.parentElement).innerText})));
  const result={...sourceLinks(links,engine),retrieved_at:page.retrieved_at,search_url:p.url()};this.lastSources=result.results;
  if(!result.results.length){if(!google)this.pendingSearch={query,url:p.url()};return {...result,needs_takeover:true,message:'No usable search results. Open search for manual review.'};}
  return result;
 }
 async call(name,args) {
  if(this.paused&&!['browser.resume','browser.snapshot','browser.tabs','browser.takeover','browser.remember_sources'].includes(name))throw Error('Browser paused for user takeover; resume explicitly first');
  switch(name){
   case 'browser.remember_sources':{this.lastSources=args.sources.map(v=>({title:v.title.slice(0,300),url:webUrl(v.url)}));return {count:this.lastSources.length};}
   case 'web.search':return this.search(args.query,'background');
   case 'browser.search':return this.search(args.query,'visible');
   case 'web.read': {const p=await this.page('background');await navigate(p,args.url);return this.content(p);}
   case 'browser.navigate':{const p=await this.page('visible');await navigate(p,args.url);return this.content(p,true);}
   case 'browser.snapshot':return this.content(await this.page('visible'),true,args.filter||'');
   case 'browser.open_source':{const source=this.lastSources[args.index||0];if(!source)throw Error('No cited source at that index');const p=await this.page('visible');await navigate(p,source.url);return this.content(p,true);}
   case 'browser.tabs':{await this.page('visible');const ctx=this.contexts.get('visible');if(args.action==='create'){const p=await ctx.newPage();this.pages.set('visible',p);if(args.url)await navigate(p,args.url);}const pages=ctx.pages();if(args.action==='close'){if(!pages[args.index])throw Error('Unknown tab');await pages[args.index].close();this.refs.clear();return this.call('browser.tabs',{});}if(args.index!==undefined){if(!pages[args.index])throw Error('Unknown tab');this.pages.set('visible',pages[args.index]);await pages[args.index].bringToFront();this.refs.clear();}return {tabs:await Promise.all(pages.map(async(p,index)=>({index,title:await p.title(),url:p.url()})))};}
   case 'browser.click':case 'browser.fill':{const entry=this.refs.get(args.ref);if(!entry||entry.page.isClosed()||entry.url!==entry.page.url())throw Error('Stale reference; take a fresh snapshot');if(!await entry.el.evaluate(el=>el.isConnected).catch(()=>false))throw Error('Detached reference; take a fresh snapshot');if(name==='browser.click')await entry.el.click();else await entry.el.fill(args.text);this.refs.clear();return this.content(entry.page,true);}
   case 'browser.scroll':{const p=await this.page('visible');await p.mouse.wheel(0,args.direction==='up'?-650:650);return this.content(p,true);}
   case 'browser.takeover':{this.paused=true;const bg=this.pages.get('background');const p=await this.page('visible');if(this.pendingSearch&&bg&&!bg.isClosed())await this.contexts.get('visible').addCookies(await this.contexts.get('background').cookies());if(this.pendingSearch&&bg&&!bg.isClosed()&&bg.url().startsWith('http'))await p.goto(bg.url(),{waitUntil:'domcontentloaded'}).catch(()=>{});await p.bringToFront();return {paused:true,url:p.url(),message:'Complete the challenge/login in this window, then say resume browser.'};}
   case 'browser.resume':{const p=await this.page('visible');const page=await this.content(p,true);if(page.blocked)return {...page,resumed:false};this.paused=false;
    if(this.pendingSearch){const links=await p.locator('[data-testid="result-title-a"],.result__a').evaluateAll(els=>els.map(el=>({href:el.href,title:el.innerText,snippet:(el.closest('article')||el.parentElement.parentElement).innerText})));const result={...sourceLinks(links,'duckduckgo'),retrieved_at:page.retrieved_at,search_url:p.url()};if(result.results.length){this.lastSources=result.results;this.cachedSearch={query:this.pendingSearch.query,result};this.pendingSearch=null;}else return {...page,resumed:true,retry_search:true};if(this.contexts.has('background'))await this.contexts.get('background').addCookies(await this.contexts.get('visible').cookies());}
    return {...page,resumed:true,results:this.lastSources};}

   default:throw Error('Unknown browser tool');
  }
 }
 async close(){await Promise.allSettled([...this.contexts.values()].map(c=>c.close()));}
}
async function main(){
 const browser=new Browser(); const pending=new Map();let queue=Promise.resolve();
 const write=v=>process.stdout.write(JSON.stringify(v)+'\n');
 const result=(id,value)=>write({jsonrpc:'2.0',id,result:value});const error=(id,message,code=-32603)=>write({jsonrpc:'2.0',id,error:{code,message}});
 const lines=createInterface({input:process.stdin,crlfDelay:Infinity});
 lines.on('line',line=>{if(Buffer.byteLength(line)>1048576)return;let v;try{v=JSON.parse(line);}catch{return;}
  if(v.method==='notifications/cancelled'){const entry=pending.get(v.params?.requestId);if(entry){entry.cancelled=true;entry.page?.close().catch(()=>{});}return;}
  if(v.id===undefined)return;
  if(v.method==='initialize'){result(v.id,{protocolVersion:'2025-11-25',capabilities:{tools:{}},serverInfo:{name:'omarchy-browser',version:'0.1.0'}});return;}
  if(v.method==='ping'){result(v.id,{});return;}
  if(v.method==='tools/list'){result(v.id,{tools:definitions.map(([name,description,inputSchema])=>({name,description,inputSchema}))});return;}
  if(v.method!=='tools/call'){error(v.id,'Unknown method',-32601);return;}
  const entry={cancelled:false,page:null};pending.set(v.id,entry);
  queue=queue.then(async()=>{
   if(entry.cancelled){pending.delete(v.id);return;}
   const name=v.params?.name,args=v.params?.arguments||{};
   try {if(!definitions.some(d=>d[0]===name))throw Error('Unknown tool');const kind=name.startsWith('web.')?'background':'visible';
    if(!['browser.resume','browser.takeover','browser.remember_sources'].includes(name))entry.page=await browser.page(kind);
    if(entry.cancelled)return;const data=await browser.call(name,args);if(!entry.cancelled)result(v.id,{content:[{type:'text',text:JSON.stringify(data)}],structuredContent:data,isError:false});
   }catch(e){if(!entry.cancelled)result(v.id,{content:[{type:'text',text:String(e.message||e)}],isError:true});}finally{pending.delete(v.id);}
  }).catch(e=>console.error(e));
 });
 lines.on('close',async()=>{await browser.close();process.exit(0);});
 for(const sig of ['SIGINT','SIGTERM'])process.on(sig,async()=>{await browser.close();process.exit(0);});
}
if(import.meta.url===pathToFileURL(process.argv[1]||'').href)main().catch(e=>{console.error(e);process.exit(1);});
