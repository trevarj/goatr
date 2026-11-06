// SPDX-License-Identifier: GPL-3.0-or-later
import {validateCatalog,compareRgba,classifyArtifact,normalizeReviews,createReviewStore,clampCrop,imageHeader} from './studio.js';
import {renderWireframe} from './wireframes.js';
function assert(condition,name){if(!condition)throw Error('Check failed: '+name);}
function rejects(fn,name,field){let message='';try{fn();}catch(error){message=error.message;}assert(Boolean(message)&&(!field||message.includes(field)),name+' (expected '+(field||'rejection')+', got '+message+')');}
export async function run(){
  const response=await fetch('/catalog.json',{cache:'no-store'});if(!response.ok)throw Error('checks catalog.json: HTTP '+response.status);const catalog=validateCatalog(await response.json()),results=[];
  const pass=(name,check)=>{check();results.push({name,status:'passed'});};
  pass('Invalid and duplicate catalog IDs are rejected',()=>{const invalid=structuredClone(catalog);invalid.screens[0].id='../unsafe';rejects(()=>validateCatalog(invalid),'invalid screen ID','screens[0].id');const duplicate=structuredClone(catalog);duplicate.features.push(structuredClone(duplicate.features[0]));rejects(()=>validateCatalog(duplicate),'duplicate feature ID','duplicate');});
  pass('Dangling feature and asset references are rejected',()=>{const feature=structuredClone(catalog);feature.screens[0].featureIds.push('not-a-feature');rejects(()=>validateCatalog(feature),'dangling feature','featureIds');const asset=structuredClone(catalog);asset.screens[0].variants[0].referenceAssetIds.push('not-an-asset');rejects(()=>validateCatalog(asset),'dangling asset','referenceAssetIds');});
  pass('Unsafe local asset paths are rejected',()=>{for(const path of ['../secret.png','/secret.png','https://example.org/x.png','reference/%2e%2e/x.png','reference\\x.png','reference/./x.png']){const c=structuredClone(catalog);c.assets[0].path=path;rejects(()=>validateCatalog(c),'unsafe path '+path,'.path');}});
  pass('Equal RGBA images produce zero differing pixels',()=>{const left=new ImageData(3,3);for(let i=3;i<left.data.length;i+=4)left.data[i]=255;const right=new ImageData(new Uint8ClampedArray(left.data),3,3);const result=compareRgba(left,right);assert(result.differentPixels===0&&result.diff instanceof ImageData,'equal images');});
  pass('One changed opaque pixel produces one difference',()=>{const left=new ImageData(3,3);for(let i=3;i<left.data.length;i+=4)left.data[i]=255;const right=new ImageData(new Uint8ClampedArray(left.data),3,3);right.data.set([255,255,255,255],16);const result=compareRgba(left,right);assert(result.differentPixels===1,'one changed pixel');assert(result.diff.data[16]===255&&result.diff.data[17]===0&&result.diff.data[18]===0,'visible red changed region');});
  pass('Unequal dimensions refuse comparison without resizing',()=>{rejects(()=>compareRgba(new ImageData(3,2),new ImageData(2,3)),'unequal dimensions','Different canvases — side-by-side only');});
  pass('Wireframe and fixture are not genuine runtime captures',()=>{assert(!classifyArtifact('wireframe').genuineRuntimeCapture,'wireframe evidence');assert(!classifyArtifact('fixture_capture').genuineRuntimeCapture,'fixture evidence');assert(classifyArtifact('emulator_capture').genuineRuntimeCapture&&classifyArtifact('physical_capture').genuineRuntimeCapture,'actual capture classes remain distinct');const c=structuredClone(catalog);c.assets[0].kind='wireframe';rejects(()=>validateCatalog(c),'wireframe asset masquerade','.kind');});
  pass('Screenshot-free variants retain explicit evidence limitations',()=>{const c=structuredClone(catalog),screen=c.screens.find(s=>s.variants.some(v=>!v.referenceAssetIds.length)),variant=screen.variants.find(v=>!v.referenceAssetIds.length);assert(Boolean(variant.evidenceNote.trim()),'actual variant evidence note');validateCatalog(c);variant.evidenceNote='';rejects(()=>validateCatalog(c),'missing evidence limitation','.evidenceNote');});
  pass('Changed design keeps notes but invalidates previous acceptance',()=>{const screen=catalog.screens[0],variant=screen.variants[0],saved={schemaVersion:1,referenceRevision:catalog.reference.revision,designRevision:catalog.designRevision,reviews:[{screenId:screen.id,variantId:variant.id,state:'accepted',note:'<img src=x onerror=alert(1)>'}]},changed=structuredClone(catalog);changed.designRevision++;const r=normalizeReviews(saved,changed)[0];assert(r.state==='unreviewed'&&r.stale&&r.previousState==='accepted'&&r.note===saved.reviews[0].note,'review needs renewal');const values=new Map([['goatr.studio.review.v1.'+catalog.reference.revision,JSON.stringify(saved)]]),storage={getItem:key=>values.get(key)||null,setItem:(key,value)=>values.set(key,value)};const store=createReviewStore(changed,storage);assert(store.get(screen.id,variant.id).stale,'visible store shares normalization');assert(store.export().reviews[0].state==='unreviewed','export cannot carry acceptance');});
  pass('Successive revisions and partial renewal retain every variant note',()=>{
    const values=new Map(),storage={getItem:key=>values.get(key)||null,setItem:(key,value)=>values.set(key,value)};
    const a=catalog.screens[0],b=catalog.screens[1],av=a.variants[0].id,bv=b.variants[0].id;
    const first=createReviewStore(catalog,storage);
    first.set(a.id,av,'accepted','A original');
    first.set(b.id,bv,'changes_requested','B must survive');
    const secondCatalog={...catalog,designRevision:catalog.designRevision+1},second=createReviewStore(secondCatalog,storage);
    second.set(a.id,av,'accepted','A renewed');
    const secondReload=createReviewStore(secondCatalog,storage);
    assert(secondReload.get(a.id,av).state==='accepted'&&!secondReload.get(a.id,av).stale,'renewed A stays current');
    assert(secondReload.get(b.id,bv).note==='B must survive'&&secondReload.get(b.id,bv).stale,'unrenewed B survives first reload');
    const thirdCatalog={...catalog,designRevision:catalog.designRevision+2},third=createReviewStore(thirdCatalog,storage);
    assert(third.get(a.id,av).stale&&third.get(a.id,av).previousDesignRevision===secondCatalog.designRevision,'renewed A becomes stale at its own revision');
    third.set(a.id,av,'changes_requested','A third revision');
    const thirdReload=createReviewStore(thirdCatalog,storage),retained=thirdReload.get(b.id,bv);
    assert(retained.note==='B must survive'&&retained.state==='unreviewed'&&retained.stale,'B survives successive partial renewal and reload');
    assert(retained.previousState==='changes_requested'&&retained.previousDesignRevision===catalog.designRevision,'B keeps its actual previous state and revision');
    const exported=thirdReload.export();
    assert(exported.designRevision===thirdCatalog.designRevision&&exported.reviews.find(r=>r.screenId===b.id&&r.variantId===bv)?.note===retained.note,'export includes unrenewed B');
    assert(exported.reviews.find(r=>r.screenId===b.id&&r.variantId===bv)?.state==='unreviewed','export cannot carry stale acceptance');
  });
  pass('Diff replacements pair with independent numbers and unified run order',()=>{
    const screen=catalog.screens.find(s=>s.id==='diff'),tablet=renderWireframe(screen,screen.variants.find(v=>v.viewport==='tablet'));
    const find=(svg,text)=>[...svg.querySelectorAll('text')].find(n=>n.textContent===text);
    const oldFunction=find(tablet,'- function delay(retry, random) {'),newFunction=find(tablet,'+ function backoff(retry, base) {');
    assert(oldFunction.getAttribute('y')===newFunction.getAttribute('y'),'paired replacement functions');
    assert(oldFunction.previousElementSibling.textContent==='7'&&newFunction.previousElementSibling.textContent==='5','independent old/new file numbers after deletion');
    for(const [text,word] of [[oldFunction,'delay'],[newFunction,'backoff']]){
      const cell=Number(text.getAttribute('textLength'))/text.textContent.length,x=Number(text.getAttribute('x'))+text.textContent.indexOf(word)*cell,y=Number(text.getAttribute('y'))-Number(text.getAttribute('font-size'))-2;
      assert([...tablet.querySelectorAll('rect')].some(r=>Math.abs(Number(r.getAttribute('x'))-x)<.01&&Number(r.getAttribute('y'))===y&&Math.abs(Number(r.getAttribute('width'))-word.length*cell)<.01),'word highlight follows '+word);
    }
    const phone=renderWireframe(screen,screen.variants.find(v=>v.viewport==='phone')),oldReturn=find(phone,'-   return delay * random();'),newStart=find(phone,'+ function backoff(retry, base) {');
    assert(Number(oldReturn.getAttribute('y'))<Number(newStart.getAttribute('y')),'unified removed run precedes added run');
  });
  pass('Unavailable storage uses memory with truthful persistence state',()=>{const unavailable={getItem(){throw Error('denied');},setItem(){throw Error('quota');}},store=createReviewStore(catalog,unavailable),screen=catalog.screens[0],variant=screen.variants[0];store.set(screen.id,variant.id,'changes_requested','literal <img src=x>');assert(!store.persisted&&store.get(screen.id,variant.id).note==='literal <img src=x>','memory fallback retains note');});
  pass('Numeric crops clamp to source bounds',()=>{const c=clampCrop({x:-7,y:999,width:999,height:-5},10,20);assert(c.x===0&&c.y===19&&c.width===10&&c.height===1,'crop clamping');});
  pass('Image headers reject invalid or decompression-oversize input',()=>{rejects(()=>imageHeader(new Uint8Array([1,2,3,4])),'invalid image');const bytes=new Uint8Array(24);bytes.set([137,80,78,71,13,10,26,10]);const view=new DataView(bytes.buffer);view.setUint32(16,4097);view.setUint32(20,1);rejects(()=>imageHeader(bytes),'oversize encoded dimensions','4096');});
  pass('Every authored variant renders with honest SVG evidence',()=>{for(const screen of catalog.screens)for(const variant of screen.variants){const svg=renderWireframe(screen,variant);assert(svg instanceof SVGSVGElement&&svg.dataset.artifactKind==='wireframe',screen.id+'/'+variant.id);assert(!svg.querySelector('script,foreignObject,image,a'),'no executable or external SVG content');if(variant.viewport==='book'){assert(svg.querySelector('[data-hinge="414,0,12,900"]'),'book hinge');const panes=[...svg.querySelectorAll('[data-safe-pane]')].map(n=>n.dataset.safePane);assert(panes.includes('0,0,414,900')&&panes.includes('426,0,414,900'),'independent safe panes');}}});
  return results;
}
