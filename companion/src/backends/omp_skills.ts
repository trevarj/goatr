// SPDX-License-Identifier: GPL-3.0-or-later
// Loaded only through the stock per-process --extension. No provider instructions,
// tool invocation, file discovery, console logging or automatic message submission.
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import type { Model, ImageContent, TextContent } from "@oh-my-pi/pi-ai";
import * as fs from "node:fs";
import * as net from "node:net";
import * as crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const O_PATH = 0x200000; // Linux open(2), deliberately not an O_RDONLY probe.
const O_NOFOLLOW = 0x20000;
const O_DIRECTORY = 0x10000;
const MAX_FRAME = 262144;
const PROC_SUPER_MAGIC = 0x9fa0;
type ObjectProof = {dev:bigint,ino:bigint,size:bigint,mtimeNs:bigint,ctimeNs:bigint,mode:bigint};
type Document = {ref:string,path:string,fd:number,proof:ObjectProof};
type Catalogue = {id:string,expires:number,commands:string,entries:{reference:string,name:string,description:string,path:string}[]};
type ModelCatalogue = {id:string,expires:number,models:Map<string,Model>};
type Owner = {version:number,cap:string,endpoint:string,moduleSha:string,packageRoot:string,files:{path:string,sha256:string,size:number}[],allowControls:boolean,allowImages:boolean};
const hash = (value:Uint8Array|string) => crypto.createHash("sha256").update(value).digest("hex");
const proof = (fd:number):ObjectProof => {
 const s=fs.fstatSync(fd,{bigint:true});return {dev:s.dev,ino:s.ino,size:s.size,mtimeNs:s.mtimeNs,ctimeNs:s.ctimeNs,mode:s.mode};
};
const same = (a:ObjectProof,b:ObjectProof) => a.dev===b.dev&&a.ino===b.ino&&a.size===b.size&&a.mtimeNs===b.mtimeNs&&a.ctimeNs===b.ctimeNs&&a.mode===b.mode;
const regular = (p:ObjectProof) => (p.mode&0o170000n)===0o100000n;
const controls = (s:string) => /\p{Cc}/u.test(s);
const close = (fd:number) => {try{fs.closeSync(fd)}catch{}};
function pin(path:string):Document {
 if(!path.startsWith("/")||Buffer.byteLength(path)>4096||controls(path))throw 0;
 const fd=fs.openSync(path,O_PATH|O_NOFOLLOW);try{const p=proof(fd);if(!regular(p))throw 0;return {ref:crypto.randomUUID(),path,fd,proof:p}}catch{close(fd);throw 0}
}
// Reopen a proven original object, never the mutable catalogue pathname. Linux
// procfs self/fd is checked before this numeric-FD-only route is used.
function readable(document:Document):number {
 if(!same(document.proof,proof(document.fd))||!regular(document.proof))throw 0;
 const proc=fs.openSync("/proc/self/fd",O_PATH|O_DIRECTORY);
 try{
  if(Number(fs.statfsSync(`/proc/self/fd/${proc}`).type)!==PROC_SUPER_MAGIC)throw 0;
  const fd=fs.openSync(`/proc/self/fd/${proc}/${document.fd}`,fs.constants.O_RDONLY|fs.constants.O_NONBLOCK);
  try{if(!same(document.proof,proof(fd))||!regular(proof(fd)))throw 0;return fd}catch{close(fd);throw 0}
 }finally{close(proc)}
}
function privateOwner():Owner {
 const path=fileURLToPath(new URL("./owner.json",import.meta.url));const parent=fileURLToPath(new URL(".",import.meta.url));
 const dir=fs.lstatSync(parent);if(!dir.isDirectory()||dir.isSymbolicLink()||dir.uid!==process.getuid?.()||(dir.mode&0o777)!==0o700)throw 0;
 const object=pin(path);try{
  const st=fs.fstatSync(object.fd);if(st.uid!==process.getuid?.()||(st.mode&0o777)!==0o600||st.nlink!==1||object.proof.size>8192n)throw 0;
  const fd=readable(object);try{
   const bytes=Buffer.alloc(Number(object.proof.size));let at=0;while(at<bytes.length){const n=fs.readSync(fd,bytes,at,bytes.length-at,at);if(n===0)throw 0;at+=n}if(!same(object.proof,proof(fd)))throw 0;
   const o=JSON.parse(bytes.toString("utf8")) as Owner;
   if(o.version!==2||typeof o.allowControls!=="boolean"||typeof o.allowImages!=="boolean"||!/^[A-Za-z0-9_-]{43}$/.test(o.cap)||!o.endpoint.startsWith("/")||Buffer.byteLength(o.endpoint)>107||controls(o.endpoint)||!/^[a-f0-9]{64}$/.test(o.moduleSha)||!o.packageRoot.startsWith("/")||o.files.length>16)throw 0;
   return o;
  }finally{close(fd)}
 }finally{close(object.fd)}
}
async function fileBytes(object:Document,limit:number,current:()=>boolean,deadline:number):Promise<Buffer> {
 if(object.proof.size<0n||object.proof.size>BigInt(limit))throw 0;const fd=readable(object);
 try{
  const bytes=Buffer.alloc(Number(object.proof.size));let offset=0;
  while(offset<bytes.length){
   if(!current()||performance.now()>=deadline)throw 0;
   const length=await new Promise<number>((resolve,reject)=>fs.read(fd,bytes,offset,Math.min(65536,bytes.length-offset),offset,(error,n)=>error?reject(0):resolve(n)));
   // An expired/cancelled in-flight read retains this FD until the callback has
   // actually returned. Nothing closes/reuses it behind a still-running read.
   if(!current()||performance.now()>=deadline||length===0)throw 0;offset+=length;
  }
  if(!same(object.proof,proof(fd))||!same(object.proof,proof(object.fd))||!current()||performance.now()>=deadline)throw 0;
  return bytes;
 }finally{close(fd)}
}

export default function(pi:ExtensionAPI){
 const uuid=/^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;
 const limit=Symbol("limit");
 let started=false;let dead=false;let socket:net.Socket|undefined;let context:ExtensionContext|undefined;let nativeId="";let owner:Owner|undefined;let busy=false;let requests=0;
 const catalogues=new Map<string,Catalogue>();const modelCatalogues=new Map<string,ModelCatalogue>();
 function stop(){dead=true;catalogues.clear();modelCatalogues.clear();socket?.destroy()}
 function current(){return !dead&&context?.sessionManager.getSessionId()===nativeId}
 function keys(value:Record<string,unknown>,allowed:string[]){return Object.keys(value).every(key=>allowed.includes(key))}
 function commands(){
  if(!current())throw 0;
  const rows=pi.getCommands().filter(command=>command.source==="skill");
  if(rows.length>128)throw limit;
  const projected=rows.map(row=>{
   if(!row.path||!row.name||Buffer.byteLength(row.name)>256||Buffer.byteLength(row.description??"")>16384||controls(row.name)||controls(row.path)||!row.path.startsWith("/"))throw 0;
   return {name:row.name,description:row.description??"",path:row.path};
  });
  if(Buffer.byteLength(JSON.stringify(projected))>200000)throw limit;return projected;
 }
 function send(id:string,body:Record<string,unknown>,effect:"none"|"applied"|"dispatched"|"uncertain"="none"){
  if(!current()||!socket||socket.destroyed)return;
  try{
   let bytes=Buffer.from(JSON.stringify({id,nativeId,effect,...body})+"\n");
   if(bytes.length>MAX_FRAME)bytes=Buffer.from(JSON.stringify({id,nativeId,effect:effect==="none"?"none":"uncertain",error:"limit"})+"\n");
   socket.write(bytes);
  }catch{stop()}
 }
 function actual(){
  if(!current())throw 0;
  const model=context!.models.current();const effort=pi.getThinkingLevel();
  return {provider:model?.provider,model:model?.id,effort};
 }
 function identity(model:Model|undefined){return model?`${model.provider}\0${model.id}`:undefined}
 const pngMarker=Buffer.from([137,80,78,71,13,10,26,10]);
 function mime(bytes:Buffer):string {
  if(bytes.length>=8&&bytes.subarray(0,8).equals(pngMarker))return "image/png";
  if(bytes.length>=3&&bytes[0]===255&&bytes[1]===216&&bytes[2]===255)return "image/jpeg";
  if(bytes.length>=6&&["GIF87a","GIF89a"].includes(bytes.subarray(0,6).toString("ascii")))return "image/gif";
  if(bytes.length>=12&&bytes.subarray(0,4).toString("ascii")==="RIFF"&&bytes.subarray(8,12).toString("ascii")==="WEBP")return "image/webp";
  throw 0;
 }
 function imageModel(model:Model|undefined,count:number,types:string[]){
  if(!model||!model.input.includes("image"))throw 0;
  if(model.imageCountBudget!==undefined&&(!Number.isSafeInteger(model.imageCountBudget)||model.imageCountBudget<0||count>model.imageCountBudget))throw 0;
  if(model.imageInputDecoder==="stb"&&types.includes("image/webp"))throw 0;
 }
 async function request(value:unknown){
  if(!value||typeof value!=="object"||Array.isArray(value)){stop();return}
  const r=value as Record<string,unknown>;
  if(typeof r.id!=="string"||!uuid.test(r.id)||typeof r.method!=="string"||++requests>4096){stop();return}
  const id=r.id;
  if(busy){send(id,{error:"busy"});return}busy=true;
  let entered=false;
  try{
   if(!current())throw 0;
   if(r.method==="list"&&keys(r,["id","method"])){
    const now=performance.now();for(const [id,c]of catalogues)if(now>=c.expires)catalogues.delete(id);
    if(catalogues.size>=32)throw limit;
    const before=JSON.stringify(commands());
    const entries=(JSON.parse(before) as {name:string,description:string,path:string}[]).map(row=>({reference:crypto.randomUUID(),...row}));
    if(!current()||before!==JSON.stringify(commands()))throw 0;
    const created={id:crypto.randomUUID(),expires:performance.now()+300000,commands:before,entries};catalogues.set(created.id,created);
    context!.setTimeout(()=>{if(catalogues.get(created.id)===created)catalogues.delete(created.id)},300000);
    send(id,{catalogue:created.id,entries});
   }else if(r.method==="document"&&keys(r,["id","method","catalogue","reference"])&&typeof r.catalogue==="string"&&typeof r.reference==="string"){
    const original=catalogues.get(r.catalogue);
    if(!original||performance.now()>=original.expires||original.commands!==JSON.stringify(commands()))throw 0;
    const row=original.entries.find(row=>row.reference===r.reference);if(!row)throw 0;
    send(id,{catalogue:original.id,reference:row.reference,path:row.path});
   }else if(r.method==="controls.read"&&keys(r,["id","method"])){
    if(!owner?.allowControls)throw 0;
    const now=performance.now();for(const [id,c]of modelCatalogues)if(now>=c.expires)modelCatalogues.delete(id);
    if(modelCatalogues.size>=32)throw limit;
    const available=context!.models.list();if(available.length>256)throw limit;
    const models=new Map<string,Model>();
    const rows=available.map(model=>{
     if(!model.provider||!model.id||!model.name||controls(model.provider)||controls(model.id)||controls(model.name))throw 0;
     const efforts=model.thinking?.efforts??[];if(efforts.length>64)throw limit;
     const reference=crypto.randomUUID();models.set(reference,model);
     return {reference,provider:model.provider,model:model.id,label:model.name,efforts:[...efforts],images:model.input.includes("image")};
    });
    // Keep the authenticated parser's existing aggregate structural budget.
    if(9+rows.reduce((members,row)=>members+7+row.efforts.length,0)>4096)throw limit;
    if(!current())throw 0;
    const created={id:crypto.randomUUID(),expires:performance.now()+300000,models};modelCatalogues.set(created.id,created);
    context!.setTimeout(()=>{if(modelCatalogues.get(created.id)===created)modelCatalogues.delete(created.id)},300000);
    send(id,{catalogue:created.id,models:rows,current:actual()});
   }else if(r.method==="controls.apply"&&keys(r,["id","method","catalogue","modelRef","effort","expectedModel"])&&typeof r.catalogue==="string"){
    if(!owner?.allowControls||r.modelRef===undefined&&r.effort===undefined)throw 0;
    const catalogue=modelCatalogues.get(r.catalogue);if(!catalogue||performance.now()>=catalogue.expires)throw 0;
    if(r.modelRef!==undefined&&(typeof r.modelRef!=="string"||!catalogue.models.has(r.modelRef)))throw 0;
    if(r.effort!==undefined&&typeof r.effort!=="string")throw 0;
    const retained=r.modelRef===undefined?context!.models.current():catalogue.models.get(r.modelRef as string);
    const selected=retained&&context!.models.list().find(model=>identity(model)===identity(retained));
    if(!selected)throw 0;
    if(r.modelRef===undefined){
     const expected=r.expectedModel as Record<string,unknown>|undefined;
     if(!expected||typeof expected!=="object"||Array.isArray(expected)||!keys(expected,["provider","model"])
       ||expected.provider!==selected.provider||expected.model!==selected.id)throw 0;
    }else if(r.expectedModel!==undefined)throw 0;
    if(r.effort!==undefined&&!selected.thinking?.efforts.some(effort=>effort===r.effort))throw 0;
    if(r.modelRef!==undefined&&identity(context!.models.current())!==identity(selected)){
     entered=true;
     if(!await pi.setModel(selected)){entered=false;send(id,{error:"refused"});return}
    }
    if(!current())throw 0;
    if(r.effort!==undefined&&pi.getThinkingLevel()!==r.effort){
     const effective=context!.models.current();
     const effort=effective?.thinking?.efforts.find(value=>value===r.effort);
     if(identity(effective)!==identity(selected)||effort===undefined)throw 0;
     entered=true;pi.setThinkingLevel(effort);
    }
    send(id,{current:actual()},entered?"applied":"none");
   }else if(r.method==="prompt"&&keys(r,["id","method","text","delivery","images"])&&typeof r.text==="string"){
    if(Buffer.byteLength(r.text)>262144)throw limit;
    if(r.delivery!==undefined&&!["native","steer","follow_up","aside"].includes(r.delivery as string))throw 0;
    if(r.delivery!==undefined&&r.delivery!=="native"&&!owner?.allowControls)throw 0;
    if(r.images!==undefined&&(!Array.isArray(r.images)||r.images.length===0||r.images.length>4||!owner?.allowImages))throw 0;
    const images=(r.images??[]) as unknown[];
    if(!r.text&&images.length===0)throw 0;
    const content:(TextContent|ImageContent)[]=[];if(r.text)content.push({type:"text",text:r.text});
    const allocated:Buffer[]=[];const types:string[]=[];let total=0;
    const original=context!.models.current();if(images.length)imageModel(original,images.length,types);
    try{
     for(const raw of images){
      if(!raw||typeof raw!=="object"||Array.isArray(raw))throw 0;const image=raw as Record<string,unknown>;
      if(!keys(image,["path","size","sha256","device","inode"])||typeof image.path!=="string"||typeof image.size!=="number"
          ||!Number.isSafeInteger(image.size)||image.size<=0||image.size>8388608||typeof image.sha256!=="string"||!/^[a-f0-9]{64}$/.test(image.sha256)
          ||typeof image.device!=="string"||typeof image.inode!=="string"||image.device.length>20||image.inode.length>20
          ||!/^(0|[1-9][0-9]*)$/.test(image.device)||!/^(0|[1-9][0-9]*)$/.test(image.inode))throw 0;
      total+=image.size;if(total>8388608)throw limit;
      const object=pin(image.path);
      try{
       const metadata=fs.fstatSync(object.fd,{bigint:true});
       if(object.proof.dev!==BigInt(image.device)||object.proof.ino!==BigInt(image.inode)||object.proof.size!==BigInt(image.size)
           ||metadata.uid!==BigInt(process.getuid!())||(metadata.mode&0o7777n)!==0o600n||metadata.nlink!==1n)throw 0;
       const bytes=await fileBytes(object,8388608,current,performance.now()+5000);allocated.push(bytes);
       if(hash(bytes)!==image.sha256)throw 0;
       const mimeType=mime(bytes);types.push(mimeType);content.push({type:"image",data:bytes.toString("base64"),mimeType});
      }finally{close(object.fd)}
     }
     if(!current())throw 0;
     if(images.length){const effective=context!.models.current();if(identity(effective)!==identity(original))throw 0;imageModel(effective,images.length,types)}
     const delivery=r.delivery==="follow_up"?"followUp":r.delivery==="native"?undefined:r.delivery as "steer"|"aside"|undefined;
     entered=true;pi.sendUserMessage(images.length?content:r.text,delivery?{deliverAs:delivery}:undefined);
     send(id,{},"dispatched");
    }finally{for(const bytes of allocated)bytes.fill(0)}
   }else throw 0;
  }catch(error){send(id,{error:error===limit?"limit":"unavailable"},entered?"uncertain":"none")}finally{busy=false}
 }
 pi.on("session_start",async(_event,ctx)=>{
  if(started)return;started=true;context=ctx;nativeId=ctx.sessionManager.getSessionId();
  if(!nativeId||nativeId.length>256||controls(nativeId)){stop();return}
  try{
   owner=privateOwner();const module=pin(fileURLToPath(import.meta.url));try{if(hash(await fileBytes(module,65536,current,performance.now()+5000))!==owner.moduleSha)throw 0}finally{close(module.fd)}
   // Public Bun.main joins the actual entrypoint object to the exact qualified package.
   const mainPath=(globalThis as typeof globalThis & {Bun?:{main:string}}).Bun?.main;if(typeof mainPath!=="string")throw 0;
   const main=pin(mainPath);try{
    for(const file of owner.files){
     if(!file.path||file.path.startsWith("/")||file.path.split("/").some(component=>component===".."||!component)||!/^[a-f0-9]{64}$/.test(file.sha256)||file.size>33554432)throw 0;
     const object=pin(`${owner.packageRoot}/${file.path}`);try{
      if(file.path==="dist/cli.js"&&!same(main.proof,object.proof))throw 0;
      if(object.proof.size!==BigInt(file.size)||hash(await fileBytes(object,33554432,current,performance.now()+10000))!==file.sha256||!same(main.proof,proof(main.fd)))throw 0;
     }finally{close(object.fd)}
    }
   }finally{close(main.fd)}
   if(!current())throw 0;
   socket=net.createConnection({path:owner.endpoint});socket.setTimeout(30000,stop);
   socket.once("connect",()=>{try{if(current()&&!socket!.destroyed)socket!.write(JSON.stringify({version:2,cap:owner!.cap,nativeId,moduleSha:owner!.moduleSha,packageVersion:"18.12.5"})+"\n")}catch{stop()}});
   let promoted=false;let buffered=Buffer.alloc(0);socket.on("data",chunk=>{
    if(buffered.length+chunk.length>MAX_FRAME){stop();return}buffered=Buffer.concat([buffered,chunk]);
    while(true){const end=buffered.indexOf(10);if(end<0)break;const line=buffered.subarray(0,end);buffered=buffered.subarray(end+1);try{
     const message=JSON.parse(new TextDecoder("utf-8",{fatal:true,ignoreBOM:true}).decode(line));
     if(!promoted){if(!current()||message.ready!==true||message.nativeId!==nativeId||Object.keys(message).some(key=>!["ready","nativeId"].includes(key)))throw 0;promoted=true;socket!.setTimeout(0);ctx.setTimeout(stop,3600000)}
     else void request(message).catch(stop);
    }catch{stop();return}}
   });socket.once("error",stop);socket.once("close",stop);
  }catch{stop()}
 });
 pi.on("session_switch",()=>{stop()});pi.on("session_shutdown",()=>{stop()});
}
