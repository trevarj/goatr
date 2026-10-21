// SPDX-License-Identifier: GPL-3.0-or-later
// Loaded only through the stock per-process --extension. No provider instructions,
// tool invocation, file discovery, console logging or automatic message submission.
import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import * as fs from "node:fs";
import * as net from "node:net";
import * as crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const O_PATH = 0x200000; // Linux open(2), deliberately not an O_RDONLY probe.
const O_NOFOLLOW = 0x20000;
const O_DIRECTORY = 0x10000;
const MAX_DOCUMENT = 1048576; // Complete immutable snapshot; transfer uses bounded chunks.
const MAX_FRAME = 262144;
const PROC_SUPER_MAGIC = 0x9fa0;
type ObjectProof = {dev:bigint,ino:bigint,size:bigint,mtimeNs:bigint,ctimeNs:bigint,mode:bigint};
type Document = {ref:string,path:string,fd:number,proof:ObjectProof};
type Catalogue = {id:string,expires:number,commands:string,documents:Document[]};
type Owner = {version:number,cap:string,endpoint:string,moduleSha:string,packageRoot:string,files:{path:string,sha256:string,size:number}[]};
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
   if(o.version!==1||!/^[A-Za-z0-9_-]{43}$/.test(o.cap)||!o.endpoint.startsWith("/")||Buffer.byteLength(o.endpoint)>107||controls(o.endpoint)||!/^[a-f0-9]{64}$/.test(o.moduleSha)||!o.packageRoot.startsWith("/")||o.files.length>16)throw 0;
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
 let started=false;let dead=false;let socket:net.Socket|undefined;let context:ExtensionContext|undefined;let nativeId="";let catalogue:Catalogue|undefined;let busy=false;let requests=0;
 let frozen:{id:string,catalogue:Catalogue,document:Document,bytes:Buffer,sha256:string,expires:number}|undefined;
 function retireFrozen(){const old=frozen;frozen=undefined;old?.bytes.fill(0)}
 function retireCatalogue(){retireFrozen();const old=catalogue;catalogue=undefined;if(old)for(const document of old.documents)close(document.fd)}
 function stop(){dead=true;retireCatalogue();socket?.destroy()}
 function current(){return !dead&&context?.sessionManager.getSessionId()===nativeId}
 function commands(){
  if(!current())throw 0;
  const rows=pi.getCommands().filter(command=>command.source==="skill");
  if(rows.length>128)throw 0;
  const projected=rows.map(row=>{
   if(!row.path||!row.name||Buffer.byteLength(row.name)>256||Buffer.byteLength(row.description??"")>16384||controls(row.name)||controls(row.path)||!row.path.startsWith("/"))throw 0;
   return {name:row.name,description:row.description??"",path:row.path};
  });
  if(Buffer.byteLength(JSON.stringify(projected))>200000)throw 0;return projected;
 }
 function send(value:unknown){if(!current()||!socket||socket.destroyed)return;const bytes=Buffer.from(JSON.stringify(value)+"\n");if(bytes.length>MAX_FRAME){stop();return}socket.write(bytes)}
 async function request(value:unknown){
  const r=value as {id:string,method:string,catalogue?:string,documentRef?:string,readId?:string,offset?:number,limit?:number};
  if(!r||Object.keys(r).some(k=>!["id","method","catalogue","documentRef","readId","offset","limit"].includes(k))||typeof r.id!=="string"||!/^[a-f0-9-]{36}$/.test(r.id)||++requests>4096){stop();return}
  if(busy){send({id:r.id,error:"busy"});return}busy=true;
  try{
   if(!current())throw 0;
   const before=JSON.stringify(commands());
   if(r.method==="list"&&r.catalogue===undefined&&r.documentRef===undefined&&r.readId===undefined&&r.offset===undefined&&r.limit===undefined){
    retireCatalogue();const documents:Document[]=[];
    try{
     const entries=JSON.parse(before).map((row:{name:string,description:string,path:string})=>{
      let document:Document|undefined;try{document=pin(row.path);documents.push(document)}catch{}
      return {name:row.name,description:row.description,documentRef:document?.ref};
     });
     if(!current()||before!==JSON.stringify(commands()))throw 0;
     const created={id:crypto.randomUUID(),expires:performance.now()+300000,commands:before,documents};catalogue=created;
     context!.setTimeout(()=>{if(catalogue===created)retireCatalogue()},300000);
     send({id:r.id,nativeId,catalogue:created.id,entries});
    }catch{for(const document of documents)close(document.fd);throw 0}
   }else if(r.method==="open"&&typeof r.catalogue==="string"&&typeof r.documentRef==="string"&&typeof r.readId==="string"&&/^[a-f0-9-]{36}$/.test(r.readId)&&r.offset===undefined&&r.limit===undefined){
    if(frozen)throw 0;
    const original=catalogue;if(!original||original.id!==r.catalogue||performance.now()>=original.expires||original.commands!==before)throw 0;
    const document=original.documents.find(document=>document.ref===r.documentRef);if(!document)throw 0;
    const bytes=await fileBytes(document,MAX_DOCUMENT,()=>current()&&catalogue===original,performance.now()+5000);
    if(!current()||catalogue!==original||before!==JSON.stringify(commands()))throw 0;
    const text=new TextDecoder("utf-8",{fatal:true,ignoreBOM:true}).decode(bytes);
    for(const char of text)if(controls(char)&&char!=="\n"&&char!=="\r"&&char!=="\t")throw 0;
    const created={id:r.readId,catalogue:original,document,bytes,sha256:hash(bytes),expires:performance.now()+120000};frozen=created;
    context!.setTimeout(()=>{if(frozen===created)retireFrozen()},120000);
    send({id:r.id,nativeId,catalogue:original.id,documentRef:document.ref,readId:created.id,sha256:created.sha256,size:bytes.length});
   }else if(r.method==="read"&&typeof r.catalogue==="string"&&typeof r.readId==="string"&&Number.isSafeInteger(r.offset)&&Number.isSafeInteger(r.limit)&&r.offset!>=0&&r.limit!>=1&&r.limit!<=65536&&r.documentRef===undefined){
    const original=frozen;if(!original||original.id!==r.readId||original.catalogue!==catalogue||original.catalogue.id!==r.catalogue||performance.now()>=original.expires||original.catalogue.commands!==before||!same(original.document.proof,proof(original.document.fd))||r.offset!>original.bytes.length)throw 0;
    const end=Math.min(original.bytes.length,r.offset!+r.limit!);const data=original.bytes.subarray(r.offset!,end).toString("base64");
    if(!current()||before!==JSON.stringify(commands()))throw 0;
    send({id:r.id,nativeId,catalogue:r.catalogue,readId:r.readId,offset:r.offset,data,eof:end===original.bytes.length});
   }else if(r.method==="close"&&typeof r.readId==="string"&&r.offset===undefined&&r.limit===undefined&&r.documentRef===undefined){
    const found=frozen?.id===r.readId;if(found)retireFrozen();send({id:r.id,closed:found});
   }else throw 0;
  }catch{send({id:r.id,error:"unavailable"})}finally{busy=false}
 }
 pi.on("session_start",async(_event,ctx)=>{
  if(started)return;started=true;context=ctx;nativeId=ctx.sessionManager.getSessionId();
  if(!nativeId||nativeId.length>256||controls(nativeId)){stop();return}
  try{
   const owner=privateOwner();const module=pin(fileURLToPath(import.meta.url));try{if(hash(await fileBytes(module,65536,current,performance.now()+5000))!==owner.moduleSha)throw 0}finally{close(module.fd)}
   // Public Bun.main joins this actual entrypoint object, not an executable
   // label or an unrelated installed package with a matching version string.
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
   socket.once("connect",()=>{if(current())socket!.write(JSON.stringify({version:1,cap:owner.cap,nativeId,moduleSha:owner.moduleSha,packageVersion:"18.8.7"})+"\n")});
   let promoted=false;let buffered=Buffer.alloc(0);socket.on("data",chunk=>{
    if(buffered.length+chunk.length>MAX_FRAME){stop();return}buffered=Buffer.concat([buffered,chunk]);
    while(true){const end=buffered.indexOf(10);if(end<0)break;const line=buffered.subarray(0,end);buffered=buffered.subarray(end+1);try{
     const message=JSON.parse(line.toString("utf8"));
     if(!promoted){if(!current()||message.ready!==true||message.nativeId!==nativeId||Object.keys(message).some(key=>!["ready","nativeId"].includes(key)))throw 0;promoted=true;socket!.setTimeout(0);ctx.setTimeout(stop,3600000)}
     else void request(message);
    }catch{stop();return}}
   });socket.once("error",stop);socket.once("close",stop);
  }catch{stop()}
 });
 pi.on("session_switch",()=>{stop()});pi.on("session_shutdown",()=>{stop()});
}
