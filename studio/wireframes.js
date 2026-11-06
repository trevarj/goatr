// SPDX-License-Identifier: GPL-3.0-or-later
// Static illustrations only: depicted app controls never dispatch actions.
const NS = 'http://www.w3.org/2000/svg';
const sizes = {phone:[412,915],tablet:[1280,800],folded:[360,800],book:[840,900]};
const palette = theme => theme === 'light'
  ? {bg:'#f3f3f8',panel:'#ffffff',raised:'#e4e5ec',ink:'#141519',muted:'#70727c',line:'#cdd0d8',blue:'#087aca',green:'#26794d',pink:'#b4496c',amber:'#966b14',red:'#b33737',terminal:'#f8f8f4'}
  : {bg:'#050506',panel:'#202123',raised:'#303235',ink:'#f2f2f3',muted:'#96979e',line:'#3b3d41',blue:'#50adf5',green:'#9ad09b',pink:'#e48aab',amber:'#dfca85',red:'#ec8e8e',terminal:'#111213'};
function node(tag, attrs={}, text){const n=document.createElementNS(NS,tag);for(const [k,v] of Object.entries(attrs))n.setAttribute(k,String(v));if(text!==undefined)n.textContent=text;return n;}
// Google Material Icons (Apache-2.0); pinned sources and modifications: vendor/material-icons-NOTICE.txt.
const materialPaths = {
  add:'M19,13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z',
  arrow_back:'M20,11H7.83l5.59,-5.59L12,4l-8,8 8,8 1.41,-1.41L7.83,13H20v-2z',
  filter_list:'M10 18h4v-2h-4v2zM3 6v2h18V6H3zm3 7h12v-2H6v2z',
  menu:'M3,18h18v-2H3v2zm0,-5h18v-2H3v2zm0,-7v2h18V6H3z',
  expand_more:'M16.59,8.59L12,13.17 7.41,8.59 6,10l6,6 6,-6z',
  terminal:'M20,4H4C2.89,4,2,4.9,2,6v12c0,1.1,0.89,2,2,2h16c1.1,0,2-0.9,2-2V6C22,4.9,21.11,4,20,4z M20,18H4V8h16V18z M18,17h-6v-2 h6V17z M7.5,17l-1.41-1.41L8.67,13l-2.59-2.59L7.5,9l4,4L7.5,17z',
  settings:'M19.14,12.94c0.04,-0.3 0.06,-0.61 0.06,-0.94c0,-0.32 -0.02,-0.64 -0.07,-0.94l2.03,-1.58c0.18,-0.14 0.23,-0.41 0.12,-0.61l-1.92,-3.32c-0.12,-0.22 -0.37,-0.29 -0.59,-0.22l-2.39,0.96c-0.5,-0.38 -1.03,-0.7 -1.62,-0.94L14.4,2.81c-0.04,-0.24 -0.24,-0.41 -0.48,-0.41h-3.84c-0.24,0 -0.43,0.17 -0.47,0.41L9.25,5.35c-0.59,0.24 -1.13,0.57 -1.62,0.94L5.24,5.33c-0.22,-0.08 -0.47,0 -0.59,0.22L2.74,8.87c-0.12,0.21 -0.08,0.47 0.12,0.61l2.03,1.58c-0.05,0.3 -0.09,0.63 -0.09,0.94s0.02,0.64 0.07,0.94l-2.03,1.58c-0.18,0.14 -0.23,0.41 -0.12,0.61l1.92,3.32c0.12,0.22 0.37,0.29 0.59,0.22l2.39,-0.96c0.5,0.38 1.03,0.7 1.62,0.94l0.36,2.54c0.05,0.24 0.24,0.41 0.48,0.41h3.84c0.24,0 0.44,-0.17 0.47,-0.41l0.36,-2.54c0.59,-0.24 1.13,-0.56 1.62,-0.94l2.39,0.96c0.22,0.08 0.47,0 0.59,-0.22l1.92,-3.32c0.12,-0.22 0.07,-0.47 -0.12,-0.61zM12,15.6c-1.98,0 -3.6,-1.62 -3.6,-3.6s1.62,-3.6 3.6,-3.6s3.6,1.62 3.6,3.6S13.98,15.6 12,15.6z',
  auto_awesome:'M19 9l1.25-2.75L23 5l-2.75-1.25L19 1l-1.25 2.75L15 5l2.75 1.25L19 9zm-7.5.5L9 4 6.5 9.5 1 12l5.5 2.5L9 20l2.5-5.5L17 12l-5.5-2.5zM19 15l-1.25 2.75L15 19l2.75 1.25L19 23l1.25-2.75L23 19l-2.75-1.25L19 15z',
  bar_chart:'M4 9h4v11H4zM16 13h4v7h-4zM10 4h4v16h-4z',
  dns:'M20 13H4c-.55 0-1 .45-1 1v6c0 .55.45 1 1 1h16c.55 0 1-.45 1-1v-6c0-.55-.45-1-1-1zM7 19c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zM20 3H4c-.55 0-1 .45-1 1v6c0 .55.45 1 1 1h16c.55 0 1-.45 1-1V4c0-.55-.45-1-1-1zM7 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2z',
  keyboard:'M20 5H4c-1.1 0-1.99.9-1.99 2L2 17c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm-9 3h2v2h-2V8zm0 3h2v2h-2v-2zM8 8h2v2H8V8zm0 3h2v2H8v-2zm-1 2H5v-2h2v2zm0-3H5V8h2v2zm9 7H8v-2h8v2zm0-4h-2v-2h2v2zm0-3h-2V8h2v2zm3 3h-2v-2h2v2zm0-3h-2V8h2v2z',
  arrow_upward:'M4 12l1.41 1.41L11 7.83V20h2V7.83l5.58 5.59L20 12l-8-8-8 8z',
  push_pin:'M16,9V4l1,0c0.55,0,1-0.45,1-1v0c0-0.55-0.45-1-1-1H7C6.45,2,6,2.45,6,3v0 c0,0.55,0.45,1,1,1l1,0v5c0,1.66-1.34,3-3,3h0v2h5.97v7l1,1l1-1v-7H19v-2h0C17.34,12,16,10.66,16,9z',
  pan_tool:'M23,5.5V20c0,2.2-1.8,4-4,4h-7.3c-1.08,0-2.1-0.43-2.85-1.19L1,14.83c0,0,1.26-1.23,1.3-1.25 c0.22-0.19,0.49-0.29,0.79-0.29c0.22,0,0.42,0.06,0.6,0.16C3.73,13.46,8,15.91,8,15.91V4c0-0.83,0.67-1.5,1.5-1.5S11,3.17,11,4v7 h1V1.5C12,0.67,12.67,0,13.5,0S15,0.67,15,1.5V11h1V2.5C16,1.67,16.67,1,17.5,1S19,1.67,19,2.5V11h1V5.5C20,4.67,20.67,4,21.5,4 S23,4.67,23,5.5z',
  warning:'M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z',
  call_split:'M14 4l2.29 2.29-2.88 2.88 1.42 1.42 2.88-2.88L20 10V4zm-4 0H4v6l2.29-2.29 4.71 4.7V20h2v-8.41l-5.29-5.3z',
};
function pen(group,w,p){
  const add=(tag,a,t)=>{const n=node(tag,a,t);group.append(n);return n;};
  const icon=(name,x,y,size=18,color=p.ink,rotation=0,label='')=>add('path',{d:materialPaths[name],transform:`translate(${x} ${y}) scale(${size/24}) rotate(${rotation} 12 12)`,fill:color,'fill-rule':name==='push_pin'?'evenodd':'nonzero',...(label?{role:'img','aria-label':label}:{'aria-hidden':'true'})});
  const rect=(x,y,width,height,fill=p.panel,r=12,extra={})=>add('rect',{x,y,width,height,rx:r,fill,...extra});
  const text=(s,x,y,size=14,fill=p.ink,extra={})=>add('text',{x,y,'font-size':size,fill,'font-family':'system-ui, sans-serif',...extra},s);
  const line=(x,y,x2,y2,color=p.line)=>add('line',{x1:x,y1:y,x2,y2,stroke:color});
  const wrap=(s,x,y,width=w-x-18,size=14,color=p.ink,max=6)=>{let rows=[],row='';const count=Math.max(8,Math.floor(width/(size*.54)));for(const word of s.split(/\s+/)){if((row+' '+word).length>count&&row){rows.push(row);row='';}row+=(row?' ':'')+word;}if(row)rows.push(row);rows.slice(0,max).forEach((r,i)=>text(i===max-1&&rows.length>max?r.slice(0,-1)+'…':r,x,y+i*(size+7),size,color));return Math.min(rows.length,max)*(size+7);};
  const pill=(s,x,y,width,fill=p.raised,color=p.ink,glyph='')=>{rect(x,y,width,36,fill,18);if(glyph)icon(glyph,x+10,y+9,18,color);text(s,x+(glyph?34:12),y+23,12,color);};
  const section=(s,y)=>text(s.toUpperCase(),18,y,11,p.muted,{'letter-spacing':1});
  const row=(title,detail,y,trailing='',color=p.ink)=>{text(title,26,y,15,color);if(detail)text(detail,26,y+22,11,p.muted);if(trailing==='expand_more')icon('expand_more',w-44,y-14,18,p.muted);else if(trailing)text(trailing,w-26,y,12,p.muted,{'text-anchor':'end'});line(26,y+38,w-24,y+38);};
  return {add,rect,text,line,wrap,pill,section,row,icon,w,p};
}
function header(d,title,detail='Studio Linux',large=false){const {w,p,rect,text}=d;if(large){text(title,18,110,32,p.ink,{'font-weight':700});rect(w-132,39,114,36,p.raised,18);['filter_list','menu','add'].forEach((name,i)=>d.icon(name,w-122+i*34,47,20,p.ink,0,['Filter','Menu','Add'][i]));return 145;}rect(12,38,w-24,58,p.panel,29);d.icon('arrow_back',23,55,24,p.ink,0,'Back');text(title,60,61,14,p.ink,{'font-weight':650});text(detail,60,81,10,p.muted);return 113;}
function nav(d,h,selected='Agents'){const labels=['Agents','Terminals','Usage','Hosts','Settings'];d.rect(12,h-88,d.w-24,58,d.p.panel,29);const slot=(d.w-24)/5;labels.forEach((s,i)=>{if(s===selected)d.rect(17+i*slot,h-83,slot-10,48,d.p.raised,24);d.icon(['auto_awesome','terminal','bar_chart','dns','settings'][i],12+slot*(i+.5)-11,h-77,22,s===selected?d.p.blue:d.p.ink);d.text(s,12+slot*(i+.5),h-41,9,s===selected?d.p.blue:d.p.ink,{'text-anchor':'middle'});});}
function dock(d,h){d.rect(8,h-93,d.w-16,55,d.p.panel,22);['Payments','Storefront','Docs'].forEach((s,i)=>{const x=18+i*(d.w-52)/3,color=i===0?d.p.green:d.p.muted;d.add('circle',{cx:x+3,cy:h-64,r:3,fill:color});d.text(s,x+11,h-60,10,color);});d.icon('keyboard',d.w-32,h-76,18,d.p.blue,0,'Keyboard');}
function composer(d,y,label='Message Agent'){d.add('circle',{cx:22,cy:y-16,r:3,fill:d.p.green});d.text('Done  |',30,y-12,11,d.p.green);d.icon('call_split',82,y-24,14,d.p.green);d.text('main',101,y-12,11,d.p.green);d.icon('arrow_upward',133,y-24,14,d.p.green);d.text('2',148,y-12,11,d.p.green);d.rect(10,y,d.w-20,65,d.p.panel,25);d.icon('add',23,y+22,24,d.p.muted,0,'Add attachment or tool');d.text(label,62,y+38,14,d.p.muted);d.rect(d.w-52,y+13,35,35,d.p.raised,18);d.icon('arrow_upward',d.w-47,y+18,24,d.p.ink,0,'Send');}
const agents=[['C','Payments API','Review the retry policy','Blocked','Build Linux'],['C','Storefront','Keep cart after a decline','Blocked','Studio Linux'],['>','Docs','Refresh setup guide','Done','Studio Linux'],['P','Search','Tune result ranking','Working','Build Linux'],['O','Payments API','Add retry tests','Working','Build Linux'],['C','Storefront','Agent list row fields','Idle','Studio Linux'],['P','Docs','Preview attachments','Idle','Studio Linux']];
function agentRows(d,y,count=7,compact=false){
  const {w,p}=d;
  agents.slice(0,count).forEach(([mark,title,detail,status,host],i)=>{
    const yy=y+i*(compact?77:82),color={Blocked:p.pink,Done:p.green,Working:p.amber,Idle:p.line}[status];
    d.add('circle',{cx:34,cy:yy+10,r:17,fill:p.panel,stroke:color,'stroke-width':2});
    if(mark==='>')d.icon('terminal',25,yy+1,18,color,0,'Terminal');else d.text(mark,28,yy+15,13,color);d.text(title,63,yy+4,15,p.ink,{'font-weight':600});
    if(i===0){d.icon('push_pin',w-50,yy-7,12,p.muted);d.text('Pin',w-18,yy+4,10,p.muted,{'text-anchor':'end'});}
    const limit=Math.floor((w-86)/6);
    d.text(detail.length>limit?detail.slice(0,limit-1)+'…':detail,63,yy+25,11,p.muted);
    d.text(host,63,yy+43,10,p.muted);d.text(status,w-18,yy+43,10,color,{'text-anchor':'end'});
  });
}
function directory(d,h){header(d,'Agents','Hosts · directories');d.pill('Search Agents',16,112,d.w-32);d.text('STUDIO LINUX · /home/demo/work',18,180,10,d.p.muted);agentRows(d,212,5,true);d.section('WORKSPACES',638);d.row('Payments API','/home/demo/work/project',674);d.row('Storefront','checkout-retry · linked worktree',743);if(h>800)d.text('Directory / agent navigation',18,h-54,11,d.p.muted);}
function consoleScene(d,h,id){header(d,'Agents','',true);if(id==='console-empty'){d.text('No Agents yet',d.w/2,340,24,d.p.ink,{'text-anchor':'middle'});d.wrap('Connected to Studio Linux. No Agents in this session. New Agent starts only after an explicit launch.',35,380,d.w-70,14,d.p.muted);d.pill('New Agent',d.w/2-77,490,154,d.p.raised,d.p.ink,'add');}
else if(id==='console-disconnected'){d.rect(16,151,d.w-32,100,d.p.panel);d.text('Reconnecting · Studio Linux',29,181,16,d.p.amber);d.wrap('Agent inventory is unknown. Saved chats remain readable; stale rows are not live targets.',29,207,d.w-60,12,d.p.muted);agentRows(d,290,4);d.text('Last known · not current inventory',24,h-131,11,d.p.amber);}
else if(id==='console-row-layout'){d.section('AGENT LIST FIELDS · STUDIO LINUX',160);[['Row 1','Title','Host name'],['Row 2','Working directory','Changes totals'],['Row 3','Branch','Status badge']].forEach((r,i)=>{d.rect(16,181+i*106,d.w-32,94);d.text(r[0],29,204+i*106,12,d.p.muted);d.text(r[1],29,233+i*106,15);d.text(r[2],d.w-29,253+i*106,11,d.p.blue,{'text-anchor':'end'});});d.pill('Sync from plugin · TUI',18,528,d.w-36);d.pill('Apply to all Hosts',18,578,d.w-36);d.wrap('Three fixed slots. Core offers terminal labels, not plugin-only fields.',24,655,d.w-48,13,d.p.muted);}
else if(id==='console-search'){d.pill('Search: retry',16,147,d.w-32);d.pill('Host: Build Linux',16,194,190);agentRows(d,267,1);d.text('1 result · existing priority order',24,377,12,d.p.muted);}
else if(id==='console-grouped'){d.pill('Flat   By Host   [By Workspace]',16,145,d.w-32);[['Payments API · main','Build Linux · 2 Agents'],['retry-policy','Linked worktree · 1 Blocked'],['Storefront · checkout-retry','Studio Linux · 1 Blocked']].forEach(([a,b],i)=>{const y=221+i*167;d.rect(14,y-27,d.w-28,145);d.icon('expand_more',26,y-18,20);if(i===1)d.icon('call_split',49,y-16,16,d.p.muted);d.text(a,i===1?70:52,y,17);d.text(b,29,y+25,11,d.p.muted);d.add('circle',{cx:34,cy:y+60,r:4,fill:d.p.pink});d.text(i===2?'Keep cart after a decline':'Review the retry policy',46,y+65,13,d.p.pink);d.text('Collapsed cards retain attention counts',29,y+91,10,d.p.muted);});}
else{d.add('circle',{cx:22,cy:153,r:3,fill:d.p.muted});d.text('2 Hosts connected · Blocked first',30,157,11,d.p.muted);agentRows(d,196,Math.min(7,Math.floor((h-280)/82)));}nav(d,h);}
function terminal(d,h,id,short=false){const {w,p}=d;header(d,'Payments API','Claude Code · /home/demo/work/project');const keyboard=id==='terminal-keyboard';const end=short?Math.min(390,h-260):keyboard?h-471:h-242;d.rect(5,108,w-10,end-108,p.terminal,0);const lines=[['❯ Review the retry policy',p.ink],['',p.ink],['● Read src/retry.ts',p.blue],['  MAX_ATTEMPTS = 5',p.amber],['  const delay = base * 2 ** retry;',p.ink],['',p.ink],['  24 - return response.status;',p.red],['  24 + return reply.status;',p.green],['',p.ink],['✓ Unicode: café  界  λ →',p.green],['  bold · italic · underline',p.muted],['',p.ink],['● Working (16s · Esc to interrupt)',p.amber],['  https://example.org/retry',p.blue],['',p.ink],['❯ _',p.ink]];lines.slice(0,Math.floor((end-126)/23)).forEach(([s,c],i)=>d.text(s,14,135+i*23,12,c,{'font-family':'monospace'}));if(id==='terminal-selection-links'){d.rect(10,191,w-20,25,p.raised,2);d.text('const delay = base * 2 ** retry;',17,209,12,p.ink,{'font-family':'monospace'});d.rect(8,Math.min(370,h-460),w-16,241,p.panel,22);d.text('Select terminal text',25,Math.min(400,h-430),17);d.pill('Copy',25,Math.min(421,h-409),88);d.pill('Select all',125,Math.min(421,h-409),112);d.text('Collected web links',25,Math.min(492,h-338),14);d.text('https://example.org/retry',25,Math.min(525,h-305),12,p.blue);d.text('Only HTTP(S); never executable schemes',25,Math.min(559,h-271),10,p.muted);}
if(short)return;
if(keyboard){d.add('circle',{cx:21,cy:end+19,r:3,fill:p.amber});d.text('Working · Direct Input',29,end+23,11,p.amber);d.pill('Esc    Tab    ⇧Tab    ↑  ↓  ←  →',10,end+35,w-20);dock(d,end+163);fullKeyboard(d,h-345,270);d.text('Controls    Skills    Snippets    Appearance',20,h-51,10,p.blue);}else{composer(d,h-198);dock(d,h);}d.rect(w-22,Math.min(287,end-40),22,65,p.panel,11);d.icon('expand_more',w-20,Math.min(309,end-17),18,p.muted,90,'Open agent drawer');}
function fullKeyboard(d,y,height){const rows=[['Esc','Tab','Home','End','PgUp','PgDn','Del'],['1','2','3','4','5','6','7','8','9','0'],['Q','W','E','R','T','Y','U','I','O','P'],['A','S','D','F','G','H','J','K','L'],['⇧','Z','X','C','V','B','N','M','⌫'],['Ctrl','Alt','Fn','#+=','Space','←','↓','↑','↵']];const rh=height/6;rows.forEach((r,i)=>{const kw=(d.w-16)/r.length;r.forEach((s,j)=>{d.rect(8+j*kw,y+i*rh,kw-4,rh-5,d.p.raised,6);d.text(s,8+(j+.5)*kw-2,y+i*rh+rh*.6,s.length>3?9:12,d.p.ink,{'text-anchor':'middle'});});});}
function table(d,x,y,width){const cw=width/2;d.rect(x,y,width,143,d.p.panel,8);for(let i=0;i<4;i++)d.line(x,y+35*i,x+width,y+35*i);d.line(x+cw,y,x+cw,y+143);[['Status','Checks'],['Proposed','Retry on 429'],['Unchanged','Cart contents'],['Pending','Run host tests']].forEach((r,i)=>r.forEach((s,j)=>d.text(s,x+10+j*cw,y+23+35*i,11,i===0?d.p.ink:d.p.muted)));}
function blocked(d,y){d.rect(10,y,d.w-20,273,d.p.panel,20);d.icon('pan_tool',25,y+12,20,d.p.ink);d.text('Bash command',52,y+30,17);d.text('Run the test suite and show the diff',25,y+62,12,d.p.muted);d.rect(23,y+78,d.w-46,42,d.p.raised,10);d.text('npm test && git diff --stat',31,y+104,11,d.p.ink,{'font-family':'monospace'});d.pill('Yes · this command',23,y+134,d.w-46,d.p.blue,d.p.bg);d.pill('Yes, allow for this session',23,y+178,d.w-46);d.icon('warning',29,y+216,15,d.p.amber);d.text('Changes permission scope',50,y+229,11,d.p.amber);d.text('No · do not run',31,y+255,12,d.p.red);}
function chat(d,h,id){
  header(d,'Keep cart after a decline','Claude Code · Storefront');
  if(id==='chat-history-offline'){
    d.pill('Load earlier messages',d.w/2-105,113,210);
    d.rect(15,162,d.w-30,64,d.p.panel);
    d.text('Offline · saved conversation',27,188,15,d.p.amber);
    d.text('Not a current Host snapshot',27,211,11,d.p.muted);
    const top=242,bottom=h-253,clipId=`offline-transcript-${d.w}-${h}`;
    const clip=d.add('clipPath',{id:clipId});
    clip.append(node('rect',{x:10,y:top,width:d.w-20,height:bottom-top}));
    const transcript=pen(d.add('g',{'clip-path':`url(#${clipId})`,'data-region':'offline-transcript'}),d.w,d.p);
    transcript.text('Earlier context · compacted',20,262,12,d.p.muted);
    transcript.rect(d.w*.2,279,d.w*.75,50,d.p.panel,20);
    transcript.wrap('Keep the cart when a payment is declined.',d.w*.2+13,301,d.w*.75-26,12,d.p.ink,2);
    transcript.text('Worked for 16s',18,350,11,d.p.muted);transcript.icon('expand_more',105,337,16,d.p.muted,-90,'Expand turn');
    transcript.wrap('The checkout keeps its cart and promo code. The retry policy is now explicit:',18,379,d.w-36,13,d.p.ink,2);
    table(transcript,18,414,d.w-36);
    transcript.rect(15,569,d.w-30,41,d.p.panel,10);
    transcript.icon('expand_more',23,581,18);transcript.text('2 tools · Read source · Edit file',47,595,11);
    transcript.wrap('Review this proposed behavior before running the command.',18,637,d.w-36,12,d.p.ink,2);
    d.rect(d.w-8,top+12,3,bottom-top-24,d.p.raised,2);
    d.rect(d.w-8,bottom-100,3,85,d.p.muted,2);
    d.text('Send unavailable while disconnected',18,h-230,11,d.p.amber);
    composer(d,h-198,'Draft kept locally');dock(d,h);return;
  }
  let y=128;
  d.rect(d.w*.2,y,d.w*.75,60,d.p.panel,20);d.wrap('Keep the cart when a payment is declined.',d.w*.2+13,y+24,d.w*.75-26,12,d.p.ink,2);y+=91;d.text('Worked for 16s',18,y,11,d.p.muted);d.icon('expand_more',105,y-13,16,d.p.muted,-90,'Expand turn');y+=34;d.wrap('The checkout keeps its cart and promo code. The retry policy is now explicit:',18,y,d.w-36,14,d.p.ink,3);y+=70;
  if(id==='chat-blocked'){d.rect(15,y,d.w-30,44,d.p.panel,10);d.icon('expand_more',23,y+12,18);d.text('2 tools · Read checkout · Edit retry',47,y+27,12);const by=Math.max(y+64,h-392);blocked(d,by);dock(d,h);}else{table(d,18,y,d.w-36);y+=168;d.rect(15,y,d.w-30,41,d.p.panel,10);d.icon('expand_more',23,y+12,18);d.text('2 tools · Read source · Edit file',47,y+26,11);d.wrap('Review this proposed behavior before running the command.',18,y+78,d.w-36,13,d.p.ink,2);composer(d,h-198);dock(d,h);}
}
function tools(d,h,id){terminal(d,h,id,true);const y=Math.min(420,h-420);d.rect(5,y,d.w-10,h-y-30,d.p.panel,18);d.text({ 'composer-tools':'Composer tools',skills:'Skills',snippets:'Snippets',attachments:'Add image or file',voice:'Voice Input'}[id],22,y+31,20);if(id==='skills'){d.pill('Search Skills',20,y+49,d.w-40);d.row('/review','Project · review changes before merging',y+120);d.row('/test','Global · focused test plan',y+188);d.row('/compact','Agent command · preserve key context',y+256);d.text('Refresh · Insert into draft, never Send',24,y+319,11,d.p.blue);}else if(id==='snippets'){d.row('Review carefully','Inspect edge cases; do not change files yet.',y+88);d.row('Explain the diff','Summarize behavior and affected callsites.',y+166);d.pill('New Snippet',20,y+217,d.w-40,d.p.raised,d.p.ink,'add');d.text('Global device set · title + reusable text',24,y+291,11,d.p.muted);}else if(id==='attachments'){d.pill('Photo   File   Paste Image',20,y+54,d.w-40);d.row('retry-notes.txt','Staging to private Host temporary storage',y+128,'42%');d.rect(25,y+169,d.w-50,5,d.p.raised,2);d.rect(25,y+169,(d.w-50)*.42,5,d.p.blue,2);d.row('diagram.png','Upload interrupted · Retry or Remove',y+219,'Error',d.p.red);d.wrap('Add inserts the Host path into your draft. It does not send or prove image acceptance. Linux limit: 64 MiB.',24,y+283,d.w-48,12,d.p.muted);}else if(id==='voice'){d.add('circle',{cx:29,cy:y+73,r:5,fill:d.p.red});d.text('Recording 00:12',43,y+80,19,d.p.red);for(let i=0;i<28;i++){let a=8+(i*17%55);d.rect(24+i*(d.w-48)/28,y+155-a,4,a*2,d.p.blue,2);}d.pill('Cancel recording',20,y+231,d.w-40);d.wrap('Only after consent: send audio to your chosen speech service. Text returns to this draft; Send stays hidden.',24,y+296,d.w-48,12,d.p.muted);d.text('If service fails: Retry / Settings / Dismiss',24,y+367,10,d.p.amber);}else{d.pill('Controls   Skills   Snippets',20,y+54,d.w-40);d.pill('Esc · interrupt',20,y+110,d.w-40);d.pill('Enter · explicit key',20,y+158,d.w-40);d.pill('Ctrl+C · terminal interrupt',20,y+206,d.w-40);d.wrap('Keys reach the selected Agent. Skills and Snippets only insert. Direct Input preserves the local draft.',24,y+282,d.w-48,13,d.p.muted);}}
function shells(d,h){header(d,'Terminals','',true);d.icon('expand_more',18,147,20,d.p.green);d.text('Studio Linux · Connected',44,165,15,d.p.green);[['Payments API','2 shells · /home/demo/work/project'],['Storefront','1 shell · checkout-retry'],['Docs','No shell yet · explicit New Terminal']].forEach(([a,b],i)=>{const y=211+i*148;d.rect(15,y-20,d.w-30,130);d.text(a,29,y+7,17);d.text(b,29,y+33,11,d.p.muted);d.icon(i===2?'add':'terminal',27,y+60,20,d.p.blue);d.text(i===2?'New Terminal':'Shell',54,y+77,14,d.p.blue);if(i!==2)d.icon('expand_more',91,y+62,18,d.p.blue,-90);});d.pill('Text   |   Keys',20,h-159,d.w-40);nav(d,h,'Terminals');}
function shellLive(d,h){
  header(d,'Shell · Payments API','Studio Linux · /home/demo/work/project');
  const rows=['❯ git status --short',' M src/retry.ts','?? retry-notes.txt','','❯ pwd','/home/demo/work/project','','❯ _'];
  rows.forEach((s,i)=>d.text(s,18,144+i*25,14,i===0||i===4?d.p.green:d.p.ink,{'font-family':'monospace'}));
  d.rect(8,h-366,d.w-16,48,d.p.panel,12);
  d.text('Paste     [Text]   Keys',24,h-336,13,d.p.muted);d.text('More',d.w-93,h-336,13,d.p.muted);d.icon('keyboard',d.w-48,h-352,20,d.p.muted,0,'Keyboard');
  fullKeyboard(d,h-305,256);
}
function creation(d,h,id){header(d,id==='launch-agent'?'New Agent':'New Terminal');d.section('HOST / WORKSPACE',140);d.row('Studio Linux','Herdr TUI · default session',177,'expand_more');d.row('Payments API','/home/demo/work/project',255,'Browse');if(id==='launch-agent'){d.row('Program','Claude Code · available on Host PATH',333,'expand_more');d.row('Launch Preset','Review · Default Account',411,'expand_more');d.row('Arguments','--model default',489);d.pill('Existing   New Workspace   Worktree',18,550,d.w-36);d.row('Worktree branch','review/retry · explicit new checkout',634);}else{d.section('EXISTING SHELL',334);d.rect(16,355,d.w-32,100);d.text('Shell · Payments API',28,385,17);d.text('Reuse keeps the current shell and directory',28,416,11,d.p.muted);d.pill('Reuse this shell',18,486,d.w-36);d.pill('Explicitly create a new shell',18,540,d.w-36,d.p.raised,d.p.ink,'add');d.wrap('Creation uses the selected directory, never another focused pane. Text and Keys are shell input modes.',24,631,d.w-48,13,d.p.muted);}d.pill(id==='launch-agent'?'Start Agent · explicit action':'Create Terminal · explicit action',18,h-105,d.w-36,d.p.blue,d.p.bg);}
function worktrees(d,h){header(d,'Worktree Details','Payments API · review/retry');d.row('Linked worktree','/home/demo/work/project',151);d.row('Branch retained after removal','review/retry',231);d.section('DESTRUCTIVE PREVIEW',313);d.rect(14,337,d.w-28,238);d.wrap('Remove this checkout and close its Workspace?',28,368,d.w-56,18,d.p.ink,3);d.wrap('2 Terminals end. Uncommitted changes and submodules may be lost. Other Workspaces on this checkout are named before confirmation.',28,441,d.w-56,13,d.p.amber,5);d.pill('Remove Worktree · confirm',20,597,d.w-40,d.p.red,d.p.bg);d.rect(14,661,d.w-28,110);d.text('Partial outcome example',28,692,16,d.p.amber);d.wrap('Terminal end was not confirmed. Checkout remains. Branch remains. Do not claim removal succeeded.',28,722,d.w-56,12,d.p.muted);}
function changes(d,h){header(d,'Changes','Storefront');d.rect(15,118,d.w-30,108);d.text('storefront',29,146,20);d.icon('call_split',27,158,18,d.p.muted);d.text('checkout-retry',51,174,14,d.p.muted);d.icon('arrow_upward',165,160,16,d.p.muted,180);d.text('1',182,174,14,d.p.muted);d.icon('arrow_upward',197,160,16,d.p.muted);d.text('2',214,174,14,d.p.muted);d.text('Wire retry policy · 2 days ago',29,202,12,d.p.muted);d.section('STAGED · 2',264);d.row('RetryPolicy.kt','src/checkout',302,'+7  A',d.p.green);d.row('ShippingRates.kt','src/checkout',374,'+1 −1  R');d.section('CHANGES · 4',455);d.row('feature-flags.json','config',496,'+1 −1  M');d.row('catalog.csv','data',568,'+30 −30  M');d.row('logo.png','resources · Binary',640,'M');d.section('UNTRACKED',720);d.row('retry-notes.txt','No staging / commit controls',756,'?');}
function diff(d,h){
  header(d,'webhooks.js','Modified · Unstaged  +27 −23');
  const split=d.w>650;
  d.pill(split?'[Side by Side]   Unified':'[Unified]   Side by Side',15,111,Math.min(d.w-30,280));
  // Authored old/new rows pair replacement runs; absent sides do not consume a file line.
  const rows=[
    [1,1,'import { setTimeout } from timers;','import { setTimeout } from timers;'],
    [2,2,'',''],
    [3,null,'const MAX_ATTEMPTS = 5;',null],
    [4,null,'const BASE_DELAY_MS = 1000;',null],
    [5,3,'',''],
    [6,4,'// Retry with jitter','// Retry with jitter'],
    [7,5,'function delay(retry, random) {','function backoff(retry, base) {',['delay','random'],['backoff','base']],
    [8,6,'  return delay * random();','  return base * 2 ** retry;',['delay','random()'],['base','2 ** retry']],
    [9,7,'}','}'],
    [10,8,'',''],
    [11,9,'export async function deliver() {','export async function deliver() {'],
    [null,10,null,'  let lastError;'],
    [12,11,'  // preserve failed response','  // preserve failed response'],
    [13,12,'}','}']
  ];
  const font=split?12:11,charWidth=font*.6;
  const drawLine=(x,y,width,oldNumber,newNumber,text,kind,words=[])=>{
    if(kind)d.rect(x,y-18,width-3,26,kind===1?'#123820':'#401d22',0);
    const contentX=x+(split?32:54),prefix=kind<0?'- ':kind>0?'+ ':'  ',shown=prefix+text;
    for(const word of words){
      const start=text.indexOf(word);
      if(start>=0)d.rect(contentX+(prefix.length+start)*charWidth,y-font-2,word.length*charWidth,font+6,kind===1?'#306541':'#803a40',2);
    }
    if(split)d.text(String(oldNumber??newNumber),x+4,y,10,d.p.muted,{'font-family':'monospace'});
    else{
      if(oldNumber!==null)d.text(String(oldNumber),x+4,y,10,d.p.muted,{'font-family':'monospace'});
      if(newNumber!==null)d.text(String(newNumber),x+27,y,10,d.p.muted,{'font-family':'monospace'});
    }
    d.text(shown,contentX,y,font,kind?kind===1?'#b3e7b8':'#f2b4b7':d.p.ink,{'font-family':'monospace',textLength:shown.length*charWidth,lengthAdjust:'spacingAndGlyphs'});
  };
  let unifiedRow=0;
  for(let i=0;i<rows.length;i++){
    const [oldNumber,newNumber,oldText,newText,oldWords,newWords]=rows[i],changed=oldText!==newText;
    if(split){
      const width=(d.w-20)/2,y=179+i*27;
      if(oldText!==null)drawLine(10,y,width,oldNumber,null,oldText,changed?-1:0,oldWords);
      if(newText!==null)drawLine(10+width,y,width,null,newNumber,newText,changed?1:0,newWords);
    }else if(!changed)drawLine(10,179+unifiedRow++*27,d.w-20,oldNumber,newNumber,oldText,0);
    else{
      let end=i+1;
      while(end<rows.length&&rows[end][2]!==rows[end][3])end++;
      for(let j=i;j<end;j++){
        const [oldNumber,,oldText,,oldWords]=rows[j];
        if(oldText!==null)drawLine(10,179+unifiedRow++*27,d.w-20,oldNumber,null,oldText,-1,oldWords);
      }
      for(let j=i;j<end;j++){
        const [,newNumber,,newText,,newWords]=rows[j];
        if(newText!==null)drawLine(10,179+unifiedRow++*27,d.w-20,null,newNumber,newText,1,newWords);
      }
      i=end-1;
    }
  }
  if(split)d.line(d.w/2,157,d.w/2,h-70);
  d.text('Read-only diff · word changes highlighted',18,h-49,11,d.p.muted);
}
function hosts(d,h){header(d,'Hosts','',true);d.section('TRYING · 1',174);d.rect(16,192,d.w-32,93);d.add('circle',{cx:38,cy:218,r:6,fill:'none',stroke:d.p.amber,'stroke-width':2});d.text('Build Linux',54,225,18,d.p.amber);d.text('Reconnecting · relay unavailable',31,254,12,d.p.muted);d.section('CONNECTED · 1',333);d.rect(16,355,d.w-32,112);d.add('circle',{cx:38,cy:383,r:6,fill:d.p.green});d.text('Studio Linux',54,390,19,d.p.green);d.text('Companion · iroh direct · DEMO',31,420,12,d.p.muted);d.text('Herdr TUI · default session',31,444,11,d.p.muted);d.wrap('Source SSH route controls are replaced by identity-bound iroh connectivity. No SSH/VPN enrollment here.',24,534,d.w-48,14,d.p.muted);nav(d,h,'Hosts');}
function hostDetail(d,h){header(d,'Studio Linux','Saved identity · demonstration only');d.row('Backend','Herdr TUI · confirmed selection',153,'expand_more');d.row('Session','default · discovery before selection',233);d.row('Connection','Direct preferred · encrypted relay fallback',313);d.row('Identity','DEMO identity preview, not a usable key',393);d.pill('Reconnect',20,460,d.w-40);d.row('Host Helper / plugin','Health and exact installed files',550,'Details');d.row('Provider config edits','Preview · explicit confirmation · Undo',630);d.pill('Remove Host & forget trust',20,h-113,d.w-40,d.p.panel,d.p.red);}
function pairing(d,h,id){header(d,id==='pair-entry'?'Add a Host':id==='pair-preview'?'Confirm Host identity':'Pairing refused','Companion / iroh adaptation');if(id==='pair-entry'){d.rect(35,158,d.w-70,213,d.p.panel,24);d.text('Scan invitation',d.w/2,225,23,d.p.ink,{'text-anchor':'middle'});d.text('CAMERA AREA · NO QR PAYLOAD',d.w/2,274,11,d.p.muted,{'text-anchor':'middle'});d.pill('Paste invitation',25,416,d.w-50);d.wrap('DEMO — NOT A PAIRING CODE',26,496,d.w-52,19,d.p.amber,2);d.wrap('A genuine companion invitation leads to identity and trust preview before confirmation.',26,583,d.w-52,14,d.p.muted);}else if(id==='pair-preview'){d.text('Studio Linux',25,165,29);d.row('Identity to trust','DEMO — NOT A PAIRING CODE',228);d.row('Backend selection','Herdr TUI or herdr-core · discovered',316);d.row('Remote reachability','Direct / n0 public relay fallback',404);d.wrap('Verify this identity on the intended Linux host. Nothing is saved until you explicitly confirm.',26,508,d.w-52,16,d.p.amber);d.pill('Confirm identity · illustration',25,651,d.w-50,d.p.blue,d.p.bg);d.pill('Cancel',25,705,d.w-50);}else{d.text('Identity mismatch',25,186,27,d.p.red);d.wrap('The invitation expired or the Host presented a different identity. Do not connect or silently replace trust.',26,254,d.w-52,17,d.p.ink);d.rect(22,389,d.w-44,136);d.wrap('Nothing paired. No queued input. Request a fresh invitation from the intended companion.',36,426,d.w-72,14,d.p.muted);d.pill('Back to Scan / Paste',25,590,d.w-50);}}
function accounts(d,h,id){header(d,{accounts:'Agents & Accounts','launch-presets':'Launch Presets','core-templates':'Core Templates'}[id]);if(id==='accounts'){d.section('STUDIO LINUX · HOST FILES',147);d.row('Claude Code · Default','CLAUDE_CONFIG_DIR → ~/.claude',185);d.row('Claude Code · Work','/home/demo/.config/claude-work',271,'Rename');d.row('Codex · Default','CODEX_HOME → ~/.codex',357);d.pill('Add folder · Discover / Browse',18,424,d.w-36);d.pill('Ignore suggestion / Remove Account',18,480,d.w-36);d.wrap('Accounts are host-side JSON shared across devices. An unreadable file blocks launch; a refresh error retains the last list.',24,574,d.w-48,14,d.p.muted);}else if(id==='launch-presets'){d.row('Name','Review',155);d.row('Program','Claude Code',235);d.row('Account folder','~/.claude · expanded on chosen Host',315);d.row('Arguments','--model default · before launch form args',395);d.row('Environment','REVIEW_MODE=careful',475);d.wrap('Device-local preset, not a core API. Resolve current Account variables before launching. No credentials in this demo.',24,585,d.w-48,15,d.p.muted);d.pill('Save Preset · illustration',18,724,d.w-36);}else{d.section('NATIVE HERDR-CORE ROLES',148);d.row('Review role','Host-saved command, not device preset',190);d.row('Command','claude --model default',272);d.row('First message','Inspect the retry policy',354);d.row('Submit / Stop keys','Native template-owned key sequences',436);d.rect(18,502,d.w-36,141);d.wrap('Starting can take up to 35 seconds. Missing template is refused. An unconfirmed start is not replayed.',32,538,d.w-64,14,d.p.amber);d.pill('Choose in New Agent',18,696,d.w-36);}}
function usage(d,h){
  header(d,'Usage','',true);
  const wide=d.w>650,chartX=wide?Math.round(d.w*.52):16,chartY=wide?217:470,chartWidth=wide?d.w-chartX-18:d.w-32;
  d.pill('Est. Cost   Tokens',17,146,d.w*.47);d.pill('Today   7D   30D',d.w*.53,146,d.w*.43);
  d.text('Estimated API cost · last 30 days',22,220,13,d.p.muted);d.text('$25.24',22,268,43,d.p.ink,{'font-weight':650});
  d.text('83.8M tokens · demo figures, not billing',22,295,12,d.p.muted);d.text('Usage Limits',22,342,18);
  const half=((wide?chartX:d.w)-48)/2;
  [['Claude Code',27],['Codex',7]].forEach(([s,n],i)=>{
    const x=18+i*(half+12);d.rect(x,360,half,89);
    d.add('circle',{cx:x+30,cy:402,r:19,fill:'none',stroke:d.p.line,'stroke-width':5});
    d.text(n+'%',x+30,407,12,d.p.ink,{'text-anchor':'middle'});d.text(s,x+58,391,11);
    d.text('Weekly',x+58,413,12);d.text('resets in 4d',x+58,432,10,d.p.muted);
  });
  d.rect(chartX,chartY,chartWidth,206);
  for(let i=0;i<3;i++){
    const y=chartY+32+i*66;d.line(chartX+14,y,chartX+chartWidth-14,y);
    d.text(['$30','$15','$0'][i],chartX+chartWidth-39,y-5,10,d.p.muted);
  }
  const points=Array.from({length:22},(_,i)=>`${chartX+16+i*(chartWidth-40)/21},${chartY+172-((i*29)%121)}`).join(' ');
  d.add('polyline',{points,fill:'none',stroke:d.p.pink,'stroke-width':2});
  d.text('Claude Code',chartX+11,chartY+188,11,d.p.muted);d.add('circle',{cx:chartX+90,cy:chartY+184,r:3,fill:d.p.muted});d.text('Codex · hold a day',chartX+99,chartY+188,11,d.p.muted);
  d.pill('Host / Account / Model / Directory',wide?chartX:18,wide?chartY+229:699,wide?chartWidth:d.w-36);
  d.text('Claude usage query OFF by default',23,wide?512:765,12,d.p.amber);
  nav(d,h,'Usage');
}
function notifications(d,h,id){header(d,id==='notification-settings'?'Notifications':id==='ongoing-notification'?'Android notification shade':'Home-screen widgets','Android system adaptation');if(id==='notification-settings'){d.row('Permission','Android notification permission required',150,'Allowed');d.row('UnifiedPush distributor','ntfy app · health & registration',236,'Check');d.row('Delivery encryption','End-to-end payload encryption required',322);d.row('Per Host','Blocked / Done · core adds Error',408,'Enabled');d.row('Foreground banners','Suppress duplicate current-target alerts',494);d.row('Privacy / seen state','Identity-bound target · device-local Done',580);d.wrap('Managed ntfy is delivery, not a Goatr FCM sender. HMAC-only messages do not meet the encryption requirement.',24,687,d.w-48,14,d.p.amber);}else if(id==='ongoing-notification'){d.text('09:41     Wi-Fi   Battery',24,143,15,d.p.muted);d.rect(13,189,d.w-26,178,d.p.panel,22);d.text('GOATR · ONGOING',28,220,11,d.p.muted);d.text('2 Agents working',28,252,21);d.text('Studio Linux · Payments API',28,284,13);d.text('Connection status · last updated',28,310,12,d.p.muted);d.text('Open console',28,342,13,d.p.blue);d.rect(13,402,d.w-26,157,d.p.panel,22);d.text('GOATR · NEEDS ATTENTION',28,432,11,d.p.pink);d.text('Storefront is Blocked',28,466,21);d.text('Review the command on the intended Host',28,497,12,d.p.muted);d.text('Open identity-checked target',28,531,13,d.p.blue);d.wrap('No Dynamic Island claim. Android owns shade, permission, channel and background scheduling behavior.',25,640,d.w-50,15,d.p.muted);}else{d.rect(14,140,d.w-28,185,d.p.panel,22);d.text('Goatr · Studio Linux',30,175,17);d.text('Claude Code                 Codex',30,212,14);d.text('Weekly 27%                  Weekly 7%',30,244,15);d.rect(30,264,d.w-60,5,d.p.raised,2);d.rect(30,264,(d.w-60)*.27,5,d.p.pink,2);d.text('Updated 09:30 · open Usage',30,301,11,d.p.muted);d.rect(14,351,d.w-28,169,d.p.panel,22);d.text('Today · All Hosts',30,384,16);d.text('$25.24',30,432,35);d.text('Estimated API cost · 83.8M tokens',30,462,12,d.p.muted);d.text('Off switch clears figures, not stale success',30,496,11,d.p.amber);d.wrap('Android home-screen equivalents. Portable lock-screen widgets and Apple accessory placements are not promised.',25,587,d.w-50,16,d.p.amber);}}
function settings(d,h,id){header(d,id==='appearance'?'Appearance':'Settings','Goatr · native Android Views');if(id==='appearance'){d.section('APP APPEARANCE',143);d.pill('System   Light   Dark',18,164,d.w-36);d.row('Terminal · Dark slot','30 source themes · selected dark palette',261,'expand_more');d.row('Terminal · Light slot','Independent light palette',347,'expand_more');d.row('Font','JetBrains Mono / IBM Plex Mono',433,'expand_more');d.row('Text scale','Pinch / font size · enlarged text reflows',519);d.rect(20,586,d.w-40,117,d.p.terminal,8);d.text('❯ git status',33,619,15,d.p.blue,{'font-family':'monospace'});d.text('On branch main',33,647,14,d.p.green,{'font-family':'monospace'});d.text('Unicode: λ café 界',33,678,14,d.p.ink,{'font-family':'monospace'});}else{[['Agents & Accounts','Host folders · Launch Presets'],['Appearance','Theme · terminal · text scale'],['Voice Input','Destination consent · model · key'],['Notifications','Permission · distributor · privacy'],['Usage','Global off switch · limits opt-in'],['Chat Cache','Offline storage · clear data'],['About & acknowledgments','Identity · docs · licenses']].forEach(([a,b],i)=>d.row(a,b,151+i*86));}nav(d,h,'Settings');}
function drawScene(d,h,id){if(id.startsWith('console'))consoleScene(d,h,id);else if(id.startsWith('pair-'))pairing(d,h,id);else if(['agent-terminal','terminal-keyboard','terminal-selection-links'].includes(id))terminal(d,h,id);else if(['agent-chat','chat-blocked','chat-history-offline'].includes(id))chat(d,h,id);else if(['composer-tools','skills','snippets','attachments','voice'].includes(id))tools(d,h,id);else if(id==='shells')shells(d,h);else if(id==='launch-agent'||id==='launch-terminal')creation(d,h,id);else if(id==='worktrees')worktrees(d,h);else if(id==='changes')changes(d,h);else if(id==='diff')diff(d,h);else if(id==='hosts')hosts(d,h);else if(id==='host-detail')hostDetail(d,h);else if(['accounts','launch-presets','core-templates'].includes(id))accounts(d,h,id);else if(id==='usage')usage(d,h);else if(['notification-settings','ongoing-notification','widgets'].includes(id))notifications(d,h,id);else if(['appearance','settings'].includes(id))settings(d,h,id);else throw Error('No authored wireframe scene: '+id);}
export function renderWireframe(screen,variant){
  const size=sizes[variant.viewport];if(!size)throw Error('Unknown wireframe viewport');const [width,height]=size,p=palette(variant.theme);
  const svg=node('svg',{xmlns:NS,viewBox:`0 0 ${width} ${height}`,width,height,role:'img','aria-label':`${screen.title} · ${variant.viewport} ${variant.theme} wireframe, schematic design only`,'data-artifact-kind':'wireframe','data-screen':screen.id,'data-variant':variant.id});svg.append(node('title',{},`${screen.title} — wireframe, not a runtime capture`));svg.append(node('rect',{width,height,fill:p.bg}));
  const defs=node('defs');svg.append(defs);
  const pane=(x,w,id)=>{const clipId=`safe-${screen.id}-${variant.id}-${x}`;const clip=node('clipPath',{id:clipId});clip.append(node('rect',{x:0,y:0,width:w,height:height-28}));defs.append(clip);const g=node('g',{transform:`translate(${x} 0)`,'clip-path':`url(#${clipId})`,'data-safe-pane':`${x},0,${w},${height}`});svg.append(g);const d=pen(g,w,p);d.text('9:41',21,25,11,p.muted);d.text('Android system inset',w-21,25,9,p.muted,{'text-anchor':'end'});if(id==='directory')directory(d,height);else if(id==='shell-live')shellLive(d,height);else drawScene(d,height,id);return d;};
  if(variant.viewport==='book'){pane(0,414,'directory');pane(426,414,screen.id==='console'?'chat-blocked':screen.id);svg.append(node('rect',{x:414,y:0,width:12,height,fill:'#5b5367','data-hinge':'414,0,12,900'}));}
  else if(variant.viewport==='tablet'){pane(0,310,screen.id==='shells'?'shells':'directory');const d=pane(310,970,screen.id==='console'?'chat-blocked':screen.id==='shells'?'shell-live':screen.id);d.line(0,31,0,height-28,p.line);}
  else pane(0,width,screen.id);
  const bottom=pen(svg,width,p);
  if(variant.viewport==='book'){
    for(const x of [0,426]){
      bottom.rect(x,height-28,414,28,p.bg,0);
      bottom.text('ANDROID NAVIGATION INSET · WIREFRAME',x+207,height-10,9,p.muted,{'text-anchor':'middle'});
    }
  }else{
    bottom.rect(0,height-28,width,28,p.bg,0);
    bottom.text('ANDROID NAVIGATION INSET · WIREFRAME',width/2,height-10,9,p.muted,{'text-anchor':'middle'});
  }
  return svg;
}
