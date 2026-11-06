// SPDX-License-Identifier: GPL-3.0-or-later
import pixelmatch from './vendor/pixelmatch.js';
import {renderWireframe} from './wireframes.js';
const PIN = '17e890481ae5a80e142395ab17974cee8655c6a4';
const kinds = ['reference','fixture_capture','emulator_capture','physical_capture'];
const viewports = ['phone','tablet','folded','book'];
const themes = ['dark','light'];
const reviewStates = ['unreviewed','accepted','changes_requested'];
const badges = {reference:'Reference',wireframe:'Wireframe',fixture_capture:'Fixture capture',emulator_capture:'Emulator capture',physical_capture:'Physical capture'};
function fail(field,message){throw new Error(`${field}: ${message}`);}
function str(v,field){if(typeof v!=='string'||!v.trim())fail(field,'expected nonempty text');}
function one(v,values,field){if(!values.includes(v))fail(field,`expected ${values.join(' | ')}`);}
function array(v,field,nonempty=false){if(!Array.isArray(v)||nonempty&&!v.length)fail(field,nonempty?'expected nonempty array':'expected array');}
function ids(rows,field){array(rows,field,true);const result=new Set();rows.forEach((r,i)=>{if(!r||typeof r!=='object')fail(`${field}[${i}]`,'expected record');str(r.id,`${field}[${i}].id`);if(!/^[a-z0-9][a-z0-9-]*$/.test(r.id))fail(`${field}[${i}].id`,'expected lowercase identifier');if(result.has(r.id))fail(`${field}[${i}].id`,`duplicate ${r.id}`);result.add(r.id);});return result;}
function urls(rows,field){array(rows,field,true);rows.forEach((u,i)=>{str(u,`${field}[${i}]`);let url;try{url=new URL(u);}catch{fail(`${field}[${i}]`,'expected absolute HTTPS source URL');}if(url.protocol!=='https:'||url.username||url.password)fail(`${field}[${i}]`,'expected public HTTPS source URL');});}
function refs(list,known,field,nonempty=false){array(list,field,nonempty);const seen=new Set();list.forEach((id,i)=>{if(!known.has(id))fail(`${field}[${i}]`,`unknown reference ${id}`);if(seen.has(id))fail(`${field}[${i}]`,`duplicate reference ${id}`);seen.add(id);});}
export function validateCatalog(c){
  if(!c||typeof c!=='object')fail('catalog','expected object');
  if(c.schemaVersion!==1)fail('schemaVersion','expected 1');
  if(!Number.isSafeInteger(c.designRevision)||c.designRevision<1)fail('designRevision','expected positive integer');
  if(c.reference?.repository!=='https://github.com/ZingerLittleBee/Heeler')fail('reference.repository','expected frozen Heeler repository');
  if(c.reference?.revision!==PIN)fail('reference.revision','expected '+PIN);
  const sets={};for(const key of ['assets','screens','features','decisions','milestones','research'])sets[key]=ids(c[key],key);
  if(!sets.screens.has('console'))fail('screens','required fallback screen console is missing');
  c.decisions.forEach((r,i)=>{str(r.title,`decisions[${i}].title`);str(r.value,`decisions[${i}].value`);});
  c.assets.forEach((a,i)=>{const f=`assets[${i}](${a.id})`;one(a.kind,kinds,f+'.kind');str(a.path,f+'.path');if(!/^(?:[a-zA-Z0-9_-]+\/)*[a-zA-Z0-9_-]+\.(png|jpe?g|webp)$/.test(a.path))fail(f+'.path','unsafe asset path; expected local image without traversal, schemes or escapes');one(a.mime,['image/png','image/jpeg','image/webp'],f+'.mime');const extension=a.path.split('.').pop();if(({png:'image/png',jpg:'image/jpeg',jpeg:'image/jpeg',webp:'image/webp'})[extension]!==a.mime)fail(f+'.mime','extension does not match format');for(const k of ['width','height'])if(!Number.isSafeInteger(a[k])||a[k]<1||a[k]>4096)fail(f+'.'+k,'expected dimension 1–4096');if(a.width*a.height>16000000)fail(f+'.width','decoded pixel limit exceeded');if(!/^[a-f0-9]{64}$/.test(a.sha256))fail(f+'.sha256','expected SHA-256');if(a.kind==='reference'){urls([a.sourceUrl],f+'.sourceUrl');if(!a.sourceUrl.startsWith(`https://raw.githubusercontent.com/ZingerLittleBee/Heeler/${PIN}/docs/images/`))fail(f+'.sourceUrl','expected pinned upstream asset');}else if(a.sourceUrl)urls([a.sourceUrl],f+'.sourceUrl');one(a.viewport,[...viewports,'unknown'],f+'.viewport');one(a.theme,[...themes,'unknown'],f+'.theme');});
  c.screens.forEach((s,i)=>{const f=`screens[${i}](${s.id})`;str(s.title,f+'.title');str(s.group,f+'.group');urls(s.sourceUrls,f+'.sourceUrls');refs(s.featureIds,sets.features,f+'.featureIds',true);ids(s.variants,f+'.variants');const pairs=new Set();s.variants.forEach((v,j)=>{const p=`${f}.variants[${j}](${v.id})`;one(v.viewport,viewports,p+'.viewport');one(v.theme,themes,p+'.theme');const pair=v.viewport+'/'+v.theme;if(pairs.has(pair))fail(p+'.viewport','duplicate viewport/theme');pairs.add(pair);refs(v.referenceAssetIds,sets.assets,p+'.referenceAssetIds');str(v.evidenceNote,p+'.evidenceNote');array(v.annotations,p+'.annotations',true);if(v.annotations.length<3||v.annotations.length>6)fail(p+'.annotations','expected 3–6 concrete annotations');v.annotations.forEach((a,k)=>{if(!a||typeof a!=='object')fail(`${p}.annotations[${k}]`,'expected annotation record');str(a.text,`${p}.annotations[${k}].text`);one(a.basis,['screenshot','source','android_adaptation'],`${p}.annotations[${k}].basis`);});});if(!pairs.has('phone/dark'))fail(f+'.variants','dark phone variant required');});
  c.features.forEach((f,i)=>{const p=`features[${i}](${f.id})`;str(f.title,p+'.title');refs(f.screenIds,sets.screens,p+'.screenIds',true);array(f.backends,p+'.backends',true);f.backends.forEach((b,j)=>one(b,['tui','core','device'],`${p}.backends[${j}]`));one(f.disposition,['required','adapted','reference_unavailable','excluded'],p+'.disposition');str(f.behavior,p+'.behavior');for(const k of ['given','when','then'])str(f.acceptance?.[k],p+'.acceptance.'+k);urls(f.sourceUrls,p+'.sourceUrls');for(const sid of f.screenIds)if(!c.screens.find(s=>s.id===sid).featureIds.includes(f.id))fail(p+'.screenIds',`missing reciprocal screen feature reference ${sid}`);});
  c.screens.forEach((s,i)=>s.featureIds.forEach(fid=>{if(!c.features.find(f=>f.id===fid).screenIds.includes(s.id))fail(`screens[${i}].featureIds`,`missing reciprocal feature screen reference ${fid}`);}));
  c.milestones.forEach((m,i)=>{const p=`milestones[${i}]`;str(m.title,p+'.title');str(m.gate,p+'.gate');refs(m.featureIds,sets.features,p+'.featureIds',true);});
  c.research.forEach((r,i)=>{const p=`research[${i}]`;str(r.title,p+'.title');urls([r.sourceUrl],p+'.sourceUrl');str(r.finding,p+'.finding');str(r.qualification,p+'.qualification');});
  return c;
}
export function compareRgba(left,right){if(!(left instanceof ImageData)||!(right instanceof ImageData))throw Error('Comparison requires two ImageData inputs');if(left.width!==right.width||left.height!==right.height)throw Error('Different canvases — side-by-side only');const diff=new ImageData(left.width,left.height);const differentPixels=pixelmatch(left.data,right.data,diff.data,left.width,left.height,{threshold:0.1,includeAA:false});return {differentPixels,diff};}
export function classifyArtifact(kind){if(!(kind in badges))throw Error('Unknown artifact kind: '+kind);return {label:badges[kind],genuineRuntimeCapture:kind==='emulator_capture'||kind==='physical_capture'};}
export function normalizeReviews(saved,catalog){
  if(!saved)return [];
  if(saved.schemaVersion!==1||saved.referenceRevision!==catalog.reference.revision||!Number.isSafeInteger(saved.designRevision)||!Array.isArray(saved.reviews))throw Error('Stored review envelope is invalid');
  const seen=new Set();return saved.reviews.map((r,i)=>{const s=catalog.screens.find(s=>s.id===r.screenId);if(!s||!s.variants.some(v=>v.id===r.variantId))throw Error(`Stored reviews[${i}]: unknown screen/variant`);if(!reviewStates.includes(r.state)||typeof r.note!=='string')throw Error(`Stored reviews[${i}]: invalid state/note`);const revision=r.designRevision??saved.designRevision;if(!Number.isSafeInteger(revision)||revision<1)throw Error(`Stored reviews[${i}]: invalid design revision`);const key=r.screenId+'/'+r.variantId;if(seen.has(key))throw Error(`Stored reviews[${i}]: duplicate screen/variant`);seen.add(key);return {...r,state:revision===catalog.designRevision?r.state:'unreviewed',stale:revision!==catalog.designRevision,previousState:revision!==catalog.designRevision?r.state:null,previousDesignRevision:revision};});
}
export function createReviewStore(catalog,storage){
  const key='goatr.studio.review.v1.'+catalog.reference.revision;let persisted=true,records=[];
  try{const raw=storage.getItem(key),old=storage.getItem(key+'.previous');const older=normalizeReviews(old?JSON.parse(old):null,catalog),current=normalizeReviews(raw?JSON.parse(raw):null,catalog);const map=new Map(older.map(r=>[r.screenId+'/'+r.variantId,r]));current.forEach(r=>map.set(r.screenId+'/'+r.variantId,r));records=[...map.values()];}catch{persisted=false;}
  const envelope=(includeStale=true)=>({schemaVersion:1,referenceRevision:catalog.reference.revision,designRevision:catalog.designRevision,reviews:records.filter(r=>includeStale||!r.stale).map(({screenId,variantId,state,note})=>({screenId,variantId,state,note}))});
  // The archive keeps each unrenewed variant's original state and revision, not just the last envelope.
  const archive=()=>({...envelope(false),reviews:records.filter(r=>r.stale).map(r=>({screenId:r.screenId,variantId:r.variantId,state:r.previousState,note:r.note,designRevision:r.previousDesignRevision}))});
  return {get persisted(){return persisted;},get(screenId,variantId){return records.find(r=>r.screenId===screenId&&r.variantId===variantId)||{screenId,variantId,state:'unreviewed',note:'',stale:false};},set(screenId,variantId,state,note){if(!reviewStates.includes(state)||typeof note!=='string')throw Error('Invalid review state/note');if(!catalog.screens.some(s=>s.id===screenId&&s.variants.some(v=>v.id===variantId)))throw Error('Unknown review variant');records=records.filter(r=>r.screenId!==screenId||r.variantId!==variantId);records.push({screenId,variantId,state,note,stale:false});try{storage.setItem(key+'.previous',JSON.stringify(archive()));storage.setItem(key,JSON.stringify(envelope(false)));}catch{persisted=false;}},export(){return envelope();}};
}
export function clampCrop(crop,width,height){const integer=(v,fallback)=>Number.isFinite(Number(v))?Math.trunc(Number(v)):fallback;const x=Math.min(width-1,Math.max(0,integer(crop.x,0))),y=Math.min(height-1,Math.max(0,integer(crop.y,0)));return {x,y,width:Math.max(1,Math.min(width-x,integer(crop.width,width-x))),height:Math.max(1,Math.min(height-y,integer(crop.height,height-y)))};}
function cropped(image,crop){const {x,y,width,height}=clampCrop(crop,image.width,image.height);if(x===0&&y===0&&width===image.width&&height===image.height)return image;const result=new ImageData(width,height);for(let row=0;row<height;row++)result.data.set(image.data.subarray(((y+row)*image.width+x)*4,((y+row)*image.width+x+width)*4),row*width*4);return result;}
export function imageHeader(bytes){
  const v=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength),ascii=(a,n)=>String.fromCharCode(...bytes.subarray(a,a+n));let mime,width,height;
  if(bytes.length>=24&&bytes[0]===137&&ascii(1,3)==='PNG'&&bytes[4]===13&&bytes[5]===10&&bytes[6]===26&&bytes[7]===10){mime='image/png';width=v.getUint32(16);height=v.getUint32(20);}
  else if(bytes.length>=30&&ascii(0,4)==='RIFF'&&ascii(8,4)==='WEBP'){mime='image/webp';let offset=12;while(offset+8<=bytes.length){const chunk=ascii(offset,4),len=v.getUint32(offset+4,true),start=offset+8;if(start+len>bytes.length)throw Error('Invalid WebP chunk');if(chunk==='VP8X'&&len>=10){width=1+bytes[start+4]+(bytes[start+5]<<8)+(bytes[start+6]<<16);height=1+bytes[start+7]+(bytes[start+8]<<8)+(bytes[start+9]<<16);break;}if(chunk==='VP8 '&&len>=10){width=v.getUint16(start+6,true)&16383;height=v.getUint16(start+8,true)&16383;break;}if(chunk==='VP8L'&&len>=5){const bits=v.getUint32(start+1,true);width=1+(bits&16383);height=1+((bits>>>14)&16383);break;}offset=start+len+(len%2);}}
  else if(bytes.length>=4&&bytes[0]===255&&bytes[1]===216){mime='image/jpeg';let offset=2;while(offset+4<=bytes.length){if(bytes[offset]!==255)throw Error('Invalid JPEG marker');while(bytes[offset]===255)offset++;const marker=bytes[offset++];if(marker===217||marker===218)break;if(marker===1||marker>=208&&marker<=215)continue;if(offset+2>bytes.length)break;const len=v.getUint16(offset);if(len<2||offset+len>bytes.length)throw Error('Invalid JPEG segment');if([192,193,194,195,197,198,199,201,202,203,205,206,207].includes(marker)){if(len<7)throw Error('Invalid JPEG frame');height=v.getUint16(offset+3);width=v.getUint16(offset+5);break;}offset+=len;}}
  if(!mime||!width||!height)throw Error('Invalid image — PNG, JPEG or WebP required');if(width>4096||height>4096||width*height>16000000)throw Error('Image exceeds 4096 pixels per side or 16 million decoded pixels');return {mime,width,height};
}
async function decodeBlob(blob){const url=URL.createObjectURL(blob),img=new Image();try{img.src=url;await img.decode();const width=img.naturalWidth,height=img.naturalHeight;if(!width||!height||width>4096||height>4096||width*height>16000000)throw Error('Decoded image exceeds dimensions or pixel limit');const canvas=document.createElement('canvas');canvas.width=width;canvas.height=height;const context=canvas.getContext('2d',{willReadFrequently:true});context.drawImage(img,0,0);const rgba=context.getImageData(0,0,width,height);canvas.width=canvas.height=0;return rgba;}finally{img.removeAttribute('src');URL.revokeObjectURL(url);}}
export async function decodeLocalFile(file){if(file.size>16*1024*1024)throw Error('File exceeds 16 MiB');if(!['image/png','image/jpeg','image/webp'].includes(file.type))throw Error('PNG, JPEG or WebP required');const bytes=new Uint8Array(await file.arrayBuffer());const header=imageHeader(bytes);if(header.mime!==file.type)throw Error('Image content does not match its declared type');return decodeBlob(file);}
function safeSvg(svg){for(const n of [svg,...svg.querySelectorAll('*')]){if(['script','foreignObject','image','use','a','iframe','style'].includes(n.localName))throw Error('Unsafe SVG element '+n.localName);for(const a of n.attributes){if(/^on/i.test(a.name)||a.localName==='href'||/url\(/i.test(a.value)&&!/^url\(#[a-z0-9-]+\)$/i.test(a.value))throw Error('Unsafe SVG attribute '+a.name);}}return svg;}
function el(tag,text,cls){const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(cls)n.className=cls;return n;}
function sourceLinks(urls){const div=el('div',undefined,'source-links');urls.forEach((url,i)=>{const a=el('a',urls.length===1?'Pinned source':`Source ${i+1}`);a.href=url;a.target='_blank';a.rel='noopener noreferrer';a.title=url;div.append(a);});return div;}
function badge(kind){const n=el('span',classifyArtifact(kind).label,'badge');n.dataset.kind=kind;return n;}
function featureView(f){const details=el('details',undefined,'feature');details.append(el('summary',f.title),el('p',`${f.disposition} · ${f.backends.join(' / ')}`, 'muted'),el('p',f.behavior));const dl=el('dl');for(const k of ['given','when','then'])dl.append(el('dt',k),el('dd',f.acceptance[k]));details.append(dl,sourceLinks(f.sourceUrls));return details;}
function artifactCanvas(image,label){const c=el('canvas');c.width=image.width;c.height=image.height;c.setAttribute('role','img');c.setAttribute('aria-label',`${label} · ${image.width} by ${image.height} pixels`);c.textContent=label;c.getContext('2d').putImageData(image,0,0);return c;}
async function init(){
  const $=id=>document.getElementById(id);let catalog;
  try{const response=await fetch('catalog.json',{cache:'no-store'});if(!response.ok)throw Error('catalog.json: HTTP '+response.status);catalog=validateCatalog(await response.json());}catch(error){$('fatal').hidden=false;$('fatal').textContent='Catalog unavailable — '+error.message;return;}
  $('studio').hidden=false;let storage;try{storage=window.localStorage;}catch{storage={getItem(){throw Error('Unavailable');}};}const reviews=createReviewStore(catalog,storage);
  let selected,variant,sources=[null,null],crops=[null,null],selectionGeneration=0,pickerGeneration=[0,0];
  function notice(text){$('selection-message').textContent=text;$('selection-message').hidden=!text;}
  function showStorage(){$('storage-warning').hidden=reviews.persisted;}
  function showReview(){const r=reviews.get(selected.id,variant.id);$('review-state').value=r.state;$('review-note').value=r.note;$('review-status').textContent=r.stale?`Review needs renewal — previous ${r.previousState} at design revision ${r.previousDesignRevision}; notes retained. Effective state: unreviewed.`:`Design revision ${catalog.designRevision} · ${selected.id} / ${variant.id}`;showStorage();}
  function storeReview(){reviews.set(selected.id,variant.id,$('review-state').value,$('review-note').value);$('review-status').textContent=`Design revision ${catalog.designRevision} · ${selected.id} / ${variant.id}`;showStorage();}
  $('review-state').addEventListener('change',storeReview);$('review-note').addEventListener('input',storeReview);
  $('export-reviews').addEventListener('click',()=>{const url=URL.createObjectURL(new Blob([JSON.stringify(reviews.export(),null,2)+'\n'],{type:'application/json'}));const a=el('a');a.href=url;a.download='goatr-studio-reviews.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);});
  function list(){const query=$('search').value.trim().toLowerCase(),groups=new Map();let count=0;catalog.screens.forEach(s=>{const features=catalog.features.filter(f=>s.featureIds.includes(f.id));if(query&&![s.title,s.group,s.id,...features.map(f=>f.title+' '+f.behavior)].join(' ').toLowerCase().includes(query))return;if(!groups.has(s.group))groups.set(s.group,[]);groups.get(s.group).push(s);count++;});$('screen-list').replaceChildren();for(const [group,screens] of groups){const section=el('section');section.append(el('h3',group));screens.forEach(s=>{const b=el('button',s.title);b.type='button';b.dataset.screen=s.id;b.setAttribute('aria-current',String(s.id===selected?.id));b.addEventListener('click',()=>select(s.id,'phone','dark',true));section.append(b);});$('screen-list').append(section);}if(!count)$('screen-list').append(el('p','No matching screens'));$('catalog-count').textContent=`${count} / ${catalog.screens.length} scenes`;}
  $('search').addEventListener('input',list);
  function options(select,values,current){select.replaceChildren(...values.map(value=>{const option=el('option',value);option.value=value;option.selected=value===current;return option;}));}
  function showFeaturePanel(){const all=catalog.features.filter(f=>selected.featureIds.includes(f.id));$('feature-list').replaceChildren(...all.map(featureView));$('annotation-list').replaceChildren(...variant.annotations.map(a=>{const li=el('li',a.text);li.append(el('span',a.basis,'basis'));return li;}));$('provenance').replaceChildren(badge('wireframe'),el('p',`Android design · ${variant.viewport} / ${variant.theme}. Schematic geometry and fonts, not final typography or runtime proof.`),sourceLinks(selected.sourceUrls));for(const id of variant.referenceAssetIds){const a=catalog.assets.find(a=>a.id===id);$('provenance').append(assetView(a));}}
  function assetView(a){const div=el('div',undefined,'asset');div.append(badge(a.kind),el('p',a.id),el('p',`${a.mime} · ${a.width} × ${a.height} · ${a.viewport} / ${a.theme}`),el('code',a.sha256),sourceLinks([a.sourceUrl]));return div;}
  function cropForms(){const container=$('crops');container.replaceChildren();sources.forEach((s,i)=>{if(!s)return;const field=el('fieldset'),legend=el('legend',(i===0?'Left':'Right')+' crop · source '+s.image.width+' × '+s.image.height),form=el('form'),fields=el('div');const inputs={};for(const k of ['x','y','width','height']){const label=el('label',k),input=el('input');input.type='number';input.step='1';input.min=k==='x'||k==='y'?'0':'1';input.max=String(k==='x'||k==='width'?s.image.width:s.image.height);input.value=String(crops[i][k]);input.setAttribute('aria-label',(i===0?'Left':'Right')+' crop '+k);inputs[k]=input;label.append(input);fields.append(label);}const apply=el('button','Apply crop');apply.type='submit';const reset=el('button','Whole image');reset.type='button';reset.addEventListener('click',()=>{crops[i]={x:0,y:0,width:s.image.width,height:s.image.height};cropForms();compare();});form.append(fields,apply,reset);form.addEventListener('submit',event=>{event.preventDefault();crops[i]=clampCrop(Object.fromEntries(Object.entries(inputs).map(([k,n])=>[k,n.value])),s.image.width,s.image.height);cropForms();compare();});field.append(legend,form);container.append(field);});}
  function zoom(node,width){const value=$('zoom').value;if(value==='fit'){node.classList.add('fit');return;}const c=node.firstElementChild;if(c){c.style.width=width*Number(value)+'px';c.style.height='auto';}}
  function compare(){
    const workspace=$('comparison');
    workspace.querySelectorAll('canvas').forEach(canvas=>{canvas.width=canvas.height=0;});
    workspace.replaceChildren();
    const images=sources.map((s,i)=>s?cropped(s.image,crops[i]):null),equal=images.every(Boolean)&&images[0].width===images[1].width&&images[0].height===images[1].height;
    for(const option of $('mode').options)if(option.value!=='side')option.disabled=!equal;
    if(!equal)$('mode').value='side';const mode=$('mode').value;workspace.className='comparison '+mode;$('opacity-label').hidden=mode!=='overlay';
    let result=null;if(equal){result=compareRgba(images[0],images[1]);$('comparison-status').textContent=`${result.differentPixels.toLocaleString()} differing pixels · ${images[0].width} × ${images[0].height} each · diagnostic only.`;}else $('comparison-status').textContent=sources.every(Boolean)?'Different canvases — side-by-side only':'Reference unavailable — source-derived wireframe remains available';
    if(mode==='side'){
      sources.forEach((s,i)=>{
        const figure=el('figure'),caption=el('figcaption');
        caption.append(badge(s?.kind||(i===0?'reference':'wireframe')));
        if(s)caption.append(document.createTextNode(`${s.label} · intrinsic ${s.image.width} × ${s.image.height}; crop x=${crops[i].x}, y=${crops[i].y}, width=${crops[i].width}, height=${crops[i].height}`));
        else if(i===0){
          caption.append(document.createTextNode('Reference unavailable · '+(variant.referenceAssetIds.length?'image missing or undecodable':'no screenshot for this variant')));
          if(variant.referenceAssetIds.length)caption.append(sourceLinks(variant.referenceAssetIds.map(id=>catalog.assets.find(a=>a.id===id).sourceUrl)));
        }else caption.append(document.createTextNode('Wireframe loading or unavailable · not a reference screenshot'));
        figure.append(caption);
        if(s){
          const scroll=el('div',undefined,'canvas-scroll');
          scroll.append(artifactCanvas(images[i],classifyArtifact(s.kind).label+' · '+s.label));
          zoom(scroll,images[i].width);figure.append(scroll);
        }
        workspace.append(figure);
      });
    }else{
      const figure=el('figure'),caption=el('figcaption',sources.map((s,i)=>`${s.label}: crop x=${crops[i].x}, y=${crops[i].y}, width=${crops[i].width}, height=${crops[i].height}`).join(' / '));
      sources.forEach(s=>caption.prepend(badge(s.kind)));
      const scroll=el('div',undefined,'canvas-scroll');
      if(mode==='difference'){
        scroll.append(artifactCanvas(result.diff,'Diagnostic difference · red indicates changed pixels'));
        zoom(scroll,result.diff.width);
      }else{
        const layers=el('div',undefined,'layer-stage');
        layers.append(...images.map((image,i)=>artifactCanvas(image,sources[i].label)));
        layers.lastElementChild.style.opacity=Number($('opacity').value)/100;
        scroll.append(layers);
        if($('zoom').value!=='fit'){
          layers.style.width=images[0].width*Number($('zoom').value)+'px';layers.style.maxWidth='none';
        }
      }
      figure.append(caption,scroll);workspace.append(figure);
    }
  }
  function replaceSource(i,source){sources[i]=source;crops[i]=source?{x:0,y:0,width:source.image.width,height:source.image.height}:null;cropForms();compare();}
  async function referenceImage(id,generation){
    const ticket=pickerGeneration[0];replaceSource(0,null);if(!id)return;
    const a=catalog.assets.find(a=>a.id===id);
    try{
      const response=await fetch(a.path);if(!response.ok)throw Error('HTTP '+response.status);
      const bytes=new Uint8Array(await response.arrayBuffer()),header=imageHeader(bytes);
      if(header.mime!==a.mime||header.width!==a.width||header.height!==a.height)throw Error('Reference metadata mismatch');
      const sha=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),n=>n.toString(16).padStart(2,'0')).join('');
      if(sha!==a.sha256)throw Error('Reference SHA-256 mismatch');
      const image=await decodeBlob(new Blob([bytes],{type:a.mime}));
      if(generation!==selectionGeneration||ticket!==pickerGeneration[0])return;
      const mismatch=(a.viewport!==variant.viewport||a.theme!==variant.theme)?`RELATED ${a.viewport}/${a.theme} reference — selected ${variant.viewport}/${variant.theme}; ${variant.evidenceNote}`:'';
      replaceSource(0,{image,kind:a.kind,label:a.id+(mismatch?' · '+mismatch:'')});
    }catch(error){
      if(generation!==selectionGeneration||ticket!==pickerGeneration[0])return;
      replaceSource(0,null);$('comparison-status').textContent='Reference unavailable — '+a.id+': '+error.message;
    }
  }
  async function loadScene(){
    const generation=++selectionGeneration;
    pickerGeneration[0]++;pickerGeneration[1]++;
    const tickets=[...pickerGeneration];
    sources=[null,null];crops=[null,null];cropForms();compare();
    $('left-file').value=$('right-file').value='';$('file-error').hidden=true;
    const svg=safeSvg(renderWireframe(selected,variant));
    const blob=new Blob([new XMLSerializer().serializeToString(svg)],{type:'image/svg+xml'});
    try{
      const image=await decodeBlob(blob);
      if(generation!==selectionGeneration)return;
      if(tickets[1]===pickerGeneration[1])replaceSource(1,{image,kind:'wireframe',label:selected.title+' · '+variant.viewport+'/'+variant.theme});
    }catch(error){
      if(generation!==selectionGeneration)return;
      $('comparison-status').textContent='Wireframe unavailable — '+error.message;
    }
    if(generation===selectionGeneration&&tickets[0]===pickerGeneration[0])await referenceImage($('reference-choice').value,generation);
  }
  function select(screenId,viewport,theme,hash=false){selected=catalog.screens.find(s=>s.id===screenId);variant=selected?.variants.find(v=>v.viewport===viewport&&v.theme===theme);if(!variant){notice('Invalid or unavailable screen/form-factor/theme — restored console / phone / dark.');selected=catalog.screens.find(s=>s.id==='console');variant=selected.variants.find(v=>v.viewport==='phone'&&v.theme==='dark');hash=true;}
    $('scene-title').textContent=selected.title;$('scene-group').textContent=selected.group;$('variant-count').textContent=selected.variants.length+' variants';options($('viewport'),[...new Set(selected.variants.map(v=>v.viewport))],variant.viewport);options($('theme'),selected.variants.filter(v=>v.viewport===variant.viewport).map(v=>v.theme),variant.theme);$('evidence-note').textContent=variant.evidenceNote;const references=variant.referenceAssetIds.map(id=>catalog.assets.find(a=>a.id===id));$('reference-choice').replaceChildren(...references.map(a=>{const o=el('option',a.id+' · '+a.viewport+'/'+a.theme);o.value=a.id;return o;}));$('reference-choice-label').hidden=!references.length;list();showReview();showFeaturePanel();if(hash){const value=new URLSearchParams({screen:selected.id,viewport:variant.viewport,theme:variant.theme});history.replaceState(null,'','#'+value.toString());}void loadScene();}
  function hashSelection(){const params=new URLSearchParams(location.hash.slice(1));if(!location.hash||location.hash==='#contract'){select('console','phone','dark');return;}select(params.get('screen'),params.get('viewport'),params.get('theme'));}
  $('viewport').addEventListener('change',()=>{notice('');const v=selected.variants.find(v=>v.viewport===$('viewport').value&&v.theme===variant.theme)||selected.variants.find(v=>v.viewport===$('viewport').value);select(selected.id,v.viewport,v.theme,true);});$('theme').addEventListener('change',()=>{notice('');select(selected.id,variant.viewport,$('theme').value,true);});window.addEventListener('hashchange',hashSelection);
  $('reference-choice').addEventListener('change',()=>{pickerGeneration[0]++;void referenceImage($('reference-choice').value,selectionGeneration);});
  ['mode','zoom'].forEach(id=>$(id).addEventListener('change',compare));$('opacity').addEventListener('input',()=>{$('opacity-value').textContent=$('opacity').value+'%';const top=$('comparison').querySelector('.layer-stage>canvas:last-child');if(top)top.style.opacity=Number($('opacity').value)/100;});$('annotations-toggle').addEventListener('change',()=>{$('annotations').hidden=!$('annotations-toggle').checked;});
  ['left-file','right-file'].forEach((id,i)=>$(id).addEventListener('change',async()=>{const file=$(id).files[0];if(!file)return;const ticket=++pickerGeneration[i],generation=selectionGeneration;try{const image=await decodeLocalFile(file);if(ticket!==pickerGeneration[i]||generation!==selectionGeneration)return;replaceSource(i,{image,kind:'fixture_capture',label:file.name+' · local image, unverified origin'});$('file-error').hidden=true;}catch(error){if(ticket!==pickerGeneration[i]||generation!==selectionGeneration)return;$('file-error').textContent=error.message+' — previous comparison retained';$('file-error').hidden=false;}}));$('reset-images').addEventListener('click',()=>void loadScene());
  $('decisions').replaceChildren(...catalog.decisions.map(d=>{const a=el('article');a.append(el('h3',d.title),el('p',d.value));return a;}));$('research').replaceChildren(...catalog.research.map(r=>{const a=el('article');a.append(el('h3',r.title),el('p',r.finding),el('p',r.qualification,'notice'),sourceLinks([r.sourceUrl]));return a;}));$('milestones').replaceChildren(...catalog.milestones.map(m=>{const li=el('li');li.append(el('h3',m.title),el('p',m.gate),el('p',`${m.featureIds.length} catalog capabilities · required future checkpoint`,'muted'));return li;}));$('feature-count').textContent=catalog.features.length+' behavior-level requirements / adaptations / explicit exclusions; none has an implementation-pass field.';$('all-features').replaceChildren(...catalog.features.map(featureView));$('asset-inventory').replaceChildren(...catalog.assets.map(assetView));hashSelection();
}
if(typeof document!=='undefined'&&document.getElementById('studio'))void init().catch(error=>{const fatal=document.getElementById('fatal');fatal.hidden=false;fatal.textContent='Studio unavailable — '+error.message;});
