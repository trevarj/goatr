// SPDX-License-Identifier: GPL-3.0-or-later
//! Frozen Herdr endpoint generation 1. No native effects or sibling text escape this owner.
use super::*;
use unicode_width::UnicodeWidthStr;
use tokio::time::{Instant,timeout_at};

pub(super) const MAX_FRAME: usize = 2 * 1024 * 1024;
const MAX_NATIVE_CELLS: usize = 1_000_000;
type Result<T, E = ErrorCode> = std::result::Result<T, E>;
fn require(value: bool) -> Result<()> { if value { Ok(()) } else { Err(ErrorCode::NotReady) } }

/// Borrows the bounded wire buffer; discarded strings/graphics never become owned objects.
struct Decoder<'a> { bytes: &'a [u8], at: usize }
impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, at: 0 } }
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(count).ok_or(ErrorCode::ResourceLimit)?;
        let value = self.bytes.get(self.at..end).ok_or(ErrorCode::NotReady)?;
        self.at = end; Ok(value)
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn boolean(&mut self) -> Result<bool> { match self.byte()? { 0 => Ok(false), 1 => Ok(true), _ => Err(ErrorCode::NotReady) } }
    fn uint(&mut self, bits: u32) -> Result<u64> {
        let value = match self.byte()? {
            n @ 0..=250 => u64::from(n),
            251 if bits >= 16 => u64::from(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| ErrorCode::NotReady)?)),
            252 if bits >= 32 => u64::from(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ErrorCode::NotReady)?)),
            253 if bits >= 64 => u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| ErrorCode::NotReady)?),
            _ => return Err(ErrorCode::NotReady),
        };
        require(bits == 64 || value < (1u64 << bits))?; Ok(value)
    }
    fn u16(&mut self) -> Result<u16> { Ok(self.uint(16)? as u16) }
    fn u32(&mut self) -> Result<u32> { Ok(self.uint(32)? as u32) }
    fn u64(&mut self) -> Result<u64> { self.uint(64) }
    fn count(&mut self, minimum: usize) -> Result<usize> {
        let count = usize::try_from(self.u64()?).map_err(|_| ErrorCode::ResourceLimit)?;
        require(count <= (self.bytes.len() - self.at) / minimum.max(1))?; Ok(count)
    }
    fn blob(&mut self) -> Result<&'a [u8]> { let n = self.count(1)?; self.take(n) }
    fn string(&mut self) -> Result<&'a str> { std::str::from_utf8(self.blob()?).map_err(|_| ErrorCode::NotReady) }
    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        if self.boolean()? { read(self).map(Some) } else { Ok(None) }
    }
    fn variant(&mut self, variants: u32) -> Result<u32> { let n = self.u32()?; require(n < variants)?; Ok(n) }
    fn strings(&mut self) -> Result<()> { for _ in 0..self.count(1)? { self.string()?; } Ok(()) }
    fn pairs(&mut self) -> Result<()> { for _ in 0..self.count(2)? { self.string()?; self.string()?; } Ok(()) }
    fn done(&self) -> Result<()> { require(self.at == self.bytes.len()) }
}
fn put_uint(bytes: &mut Vec<u8>, value: u64) {
    if value <= 250 { bytes.push(value as u8); }
    else if value <= u64::from(u16::MAX) { bytes.push(251); bytes.extend_from_slice(&(value as u16).to_le_bytes()); }
    else if value <= u64::from(u32::MAX) { bytes.push(252); bytes.extend_from_slice(&(value as u32).to_le_bytes()); }
    else { bytes.push(253); bytes.extend_from_slice(&value.to_le_bytes()); }
}
fn put_string(bytes: &mut Vec<u8>, text: &str) { put_uint(bytes, text.len() as u64); bytes.extend_from_slice(text.as_bytes()); }
fn framed(mut bytes:Vec<u8>)->Result<Vec<u8>> {
    require(bytes.len()>4&&bytes.len()-4<=MAX_FRAME)?;
    let length=(bytes.len()-4) as u32;bytes[..4].copy_from_slice(&length.to_le_bytes());Ok(bytes)
}
pub(super) fn control(kind: &str, data: &Value) -> Result<Vec<u8>> {
    let data = serde_json::to_string(data).map_err(|_| ErrorCode::NotReady)?;
    let mut bytes = vec![0,0,0,0,20]; put_string(&mut bytes, kind); put_string(&mut bytes, &data); framed(bytes)
}
pub(super) fn command(boot: &str, id: &str, method: &str, params: Value) -> Result<Vec<u8>> {
    require(boot.len() <= 128 && id.len() <= 128)?;
    let request = serde_json::to_string(&json!({"id":id,"method":method,"params":params})).map_err(|_| ErrorCode::NotReady)?;
    require(request.len() <= 1024 * 1024)?;
    let mut bytes = vec![0,0,0,0,15]; put_string(&mut bytes, boot); put_string(&mut bytes, &request); framed(bytes)
}
pub(super) fn resize(columns: u16, rows: u16) -> Result<Vec<u8>> {
    require((1..=512).contains(&columns) && (1..=256).contains(&rows) && u32::from(columns) * u32::from(rows) <= 32768)?;
    let mut bytes = vec![0,0,0,0,12,0,0]; put_uint(&mut bytes, u64::from(columns)); put_uint(&mut bytes, u64::from(rows)); bytes.push(0); framed(bytes)
}
/// Frozen tag13 semantics. No VT encoding, generated text guesses or synthetic key releases.
pub(super) fn pane_input(pane:&Id,input:&ShellInput)->Result<Vec<u8>> {
    input.validate().map_err(|_|ErrorCode::InvalidRequest)?;
    pane.validate().map_err(|_|ErrorCode::InvalidRequest)?;
    let capacity=match input {ShellInput::Text{text}=>text.len()+32,ShellInput::Keys{events}=>events.len()*24+32};
    let mut bytes=Vec::with_capacity(4+pane.0.len()+capacity);
    bytes.extend_from_slice(&[0,0,0,0,13]);put_string(&mut bytes,&pane.0);
    match input {
        ShellInput::Text{text}=>{put_uint(&mut bytes,1);bytes.push(1);put_string(&mut bytes,text);}
        ShellInput::Keys{events}=>{
            put_uint(&mut bytes,events.len() as u64);
            for event in events {
                bytes.push(0);
                let code=match event.key {
                    ShellKey::Backspace=>0,ShellKey::Enter=>1,ShellKey::Left=>2,ShellKey::Right=>3,
                    ShellKey::Up=>4,ShellKey::Down=>5,ShellKey::Home=>6,ShellKey::End=>7,
                    ShellKey::PageUp=>8,ShellKey::PageDown=>9,ShellKey::Tab if event.modifiers.shift=>11,
                    ShellKey::Tab=>10,ShellKey::Delete=>12,
                    ShellKey::Insert=>13,ShellKey::Escape=>14,ShellKey::Character(_)=>15,
                    ShellKey::F1|ShellKey::F2|ShellKey::F3|ShellKey::F4|ShellKey::F5|ShellKey::F6|
                    ShellKey::F7|ShellKey::F8|ShellKey::F9|ShellKey::F10|ShellKey::F11|ShellKey::F12=>16,
                };
                bytes.push(code);
                match event.key {
                    ShellKey::Character(value)=>{let mut utf8=[0;4];bytes.extend_from_slice(value.encode_utf8(&mut utf8).as_bytes());}
                    ShellKey::F1=>bytes.push(1),ShellKey::F2=>bytes.push(2),ShellKey::F3=>bytes.push(3),
                    ShellKey::F4=>bytes.push(4),ShellKey::F5=>bytes.push(5),ShellKey::F6=>bytes.push(6),
                    ShellKey::F7=>bytes.push(7),ShellKey::F8=>bytes.push(8),ShellKey::F9=>bytes.push(9),
                    ShellKey::F10=>bytes.push(10),ShellKey::F11=>bytes.push(11),ShellKey::F12=>bytes.push(12),_=>{},
                }
                let modifiers=&event.modifiers;
                bytes.push(u8::from(modifiers.shift)|u8::from(modifiers.ctrl)<<1|u8::from(modifiers.alt)<<2|u8::from(modifiers.super_key)<<3);
                bytes.push(match event.phase {ShellKeyPhase::Press=>0,ShellKeyPhase::Repeat=>1,ShellKeyPhase::Release=>2});
                // One real Android event is one native event, never cumulative repeatCount.
                bytes.extend_from_slice(&[1,0,0,u8::from(event.physical_key_id.is_some()),u8::from(event.physical_key_id.is_some())]);
                if let Some(id)=event.physical_key_id {put_uint(&mut bytes,id.0);}
                bytes.push(0); // No Windows keyboard record.
            }
        }
    }
    framed(bytes)
}
pub(super) fn hello(columns: u16, rows: u16) -> Result<Vec<u8>> {
    native_geometry(columns, rows)?;
    control("endpoint.hello.v1", &json!({"generation":1,"cell_width_px":0,"cell_height_px":0,
        "surface_size":{"cols":columns,"rows":rows},"pixel_mouse":false,"direct_graphics":false,
        "endpoint_keybindings":false,"mouse_capture":false,"surface_active":false,"surface_reuse":false,"surface_delta":false,
        "snapshot_codecs":["shell.snapshot.v1"],"surface_codecs":["shell.surface.v1"],
        "input_codecs":["shell.input.semantic.v1"],"blob_codecs":["shell.blob.v1"]}))
}
fn native_geometry(width: u16, height: u16) -> Result<()> {
    require(width > 0 && height > 0 && width <= 4096 && height <= 4096 && usize::from(width) * usize::from(height) <= MAX_NATIVE_CELLS)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct Rect { pub x: u16, pub y: u16, pub width: u16, pub height: u16 }
impl Rect {
    fn read(d: &mut Decoder<'_>) -> Result<Self> { Ok(Self { x:d.u16()?, y:d.u16()?, width:d.u16()?, height:d.u16()? }) }
    fn right(self) -> u32 { u32::from(self.x) + u32::from(self.width) }
    fn bottom(self) -> u32 { u32::from(self.y) + u32::from(self.height) }
    pub(super) fn contains(self, other: Self) -> bool { other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom() }
    fn point(self, x: u16, y: u16) -> bool { x >= self.x && y >= self.y && u32::from(x) < self.right() && u32::from(y) < self.bottom() }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct PaneGeometry { id: String, content: u64, rect: Rect, inner: Rect, scrollbar: Option<Rect>, focused: bool, alternate: bool, pixel_width: u32, pixel_height: u32 }
impl PaneGeometry {
    fn read(d: &mut Decoder<'_>) -> Result<Self> {
        let id = d.string()?; require(!id.is_empty() && id.len() <= 4096)?;
        let content = d.u64()?; let rect = Rect::read(d)?; let inner = Rect::read(d)?; let scrollbar = d.option(Rect::read)?;
        d.option(|d| { let offset = d.u64()?; let maximum = d.u64()?; d.u64()?; require(offset <= maximum) })?;
        let focused = d.boolean()?; d.boolean()?; d.boolean()?; let alternate = d.boolean()?; let pixel_width = d.u32()?; let pixel_height = d.u32()?;
        require(rect.contains(inner) && scrollbar.is_none_or(|value| rect.contains(value)))?;
        Ok(Self { id:id.to_owned(), content, rect, inner, scrollbar, focused, alternate, pixel_width, pixel_height })
    }
    fn same_geometry(&self, other: &Self) -> bool {
        self.id == other.id && self.rect == other.rect && self.inner == other.inner && self.focused == other.focused && self.pixel_width == other.pixel_width && self.pixel_height == other.pixel_height
    }
}
fn panes(d: &mut Decoder<'_>) -> Result<Vec<PaneGeometry>> {
    let n = d.count(1)?; require(n <= 512)?; let mut panes = Vec::with_capacity(n);
    for _ in 0..n { let pane = PaneGeometry::read(d)?; require(!panes.iter().any(|p: &PaneGeometry| p.id == pane.id))?; panes.push(pane); } Ok(panes)
}
#[derive(Clone, Copy)]
struct Cursor { x: u16, y: u16, visible: bool, shape: u8 }
fn cursor(d: &mut Decoder<'_>) -> Result<Cursor> {
    let value = Cursor { x:d.u16()?, y:d.u16()?, visible:d.boolean()?, shape:d.byte()? }; require(value.shape <= 6)?; Ok(value)
}
#[derive(Clone)]
struct RawCell { text: String, foreground: u32, background: u32, modifier: u16, skip: bool }
#[derive(Clone, Copy)]
struct Cell<'a> { text: &'a str, foreground: u32, background: u32, modifier: u16, skip: bool }
impl Cell<'_> {
    fn owned(self) -> RawCell { RawCell { text:self.text.to_owned(), foreground:self.foreground, background:self.background, modifier:self.modifier, skip:self.skip } }
}
fn cell<'a>(d: &mut Decoder<'a>) -> Result<Cell<'a>> {
    let text = d.string()?; let foreground = d.u32()?; let background = d.u32()?; let modifier = d.u16()?; let skip = d.boolean()?; d.option(Decoder::u32)?;
    color(foreground)?; color(background)?;
    Ok(Cell { text, foreground, background, modifier, skip })
}
struct Frame<'a> { cells: &'a [u8], count: usize, width: u16, height: u16, cursor: Option<Cursor>, graphics: bool }
fn frame<'a>(d: &mut Decoder<'a>) -> Result<Frame<'a>> {
    let count = d.count(6)?; require(count <= MAX_NATIVE_CELLS)?; let start = d.at;
    for _ in 0..count { cell(d)?; } let cells = &d.bytes[start..d.at];
    let width=d.u16()?;let height=d.u16()?;require(width<=4096&&height<=4096&&count==usize::from(width)*usize::from(height))?;
    let cursor = d.option(cursor)?; require(cursor.is_none_or(|c| c.x < width && c.y < height))?;
    d.strings()?; let graphics = !d.blob()?.is_empty(); Ok(Frame { cells, count, width, height, cursor, graphics })
}
fn popup(d: &mut Decoder<'_>) -> Result<()> {
    d.string()?; d.string()?;
    for _ in 0..2 { d.option(|d| match d.variant(2)? { 0 => { d.u16()?; Ok(()) }, _ => { require(d.byte()? <= 100) } })?; }
    frame(d)?; d.boolean()?; d.boolean()?; d.u32()?; d.u32()?; Ok(())
}
fn asset(d: &mut Decoder<'_>, selected: &str) -> Result<(bool,u64)> {
    let selected = match d.variant(2)? {
        0 => { let selected = match d.variant(2)? { 0 => d.string()? == selected, _ => { d.string()?; false } }; d.u32()?; selected },
        _ => { let same = d.string()? == selected; d.string()?; same },
    };
    d.u32()?; d.u32()?; d.variant(3)?; let size = d.u64()?; d.u64()?; Ok((selected,size))
}
fn graphics(d:&mut Decoder<'_>,selected:&str,crop:Option<Rect>,width:u16,height:u16)->Result<(bool,Vec<bool>)> {
    let mut present=false;
    let stride=crop.map_or(0,|crop|usize::from(crop.width)+1);
    let mut differences=vec![0i32;crop.map_or(0,|crop|stride*(usize::from(crop.height)+1))];
    for _ in 0..d.count(1)? {let (_,size)=asset(d,selected)?;require(d.blob()?.len() as u64==size)?;present=true;}
    for _ in 0..d.count(1)? {
        let (same,_)=asset(d,selected)?;d.u32()?;let x=d.u16()?;let y=d.u16()?;let cols=d.u32()?;let rows=d.u32()?;
        let right=u32::from(x).checked_add(cols).ok_or(ErrorCode::NotReady)?;
        let bottom=u32::from(y).checked_add(rows).ok_or(ErrorCode::NotReady)?;
        require(right<=u32::from(width)&&bottom<=u32::from(height))?;
        for _ in 0..8{d.u32()?;}
        present=true;
        if same&&let Some(crop)=crop {
            let left=u32::from(x).max(u32::from(crop.x));let top=u32::from(y).max(u32::from(crop.y));
            let right=right.min(crop.right());let bottom=bottom.min(crop.bottom());
            if left<right&&top<bottom {
                let left=(left-u32::from(crop.x)) as usize;let right=(right-u32::from(crop.x)) as usize;
                let top=(top-u32::from(crop.y)) as usize;let bottom=(bottom-u32::from(crop.y)) as usize;
                differences[top*stride+left]+=1;differences[top*stride+right]-=1;
                differences[bottom*stride+left]-=1;differences[bottom*stride+right]+=1;
            }
        }
    }
    for _ in 0..d.count(1)?{asset(d,selected)?;present=true;}
    let mut coverage=vec![false;crop.map_or(0,|crop|usize::from(crop.width)*usize::from(crop.height))];
    if let Some(crop)=crop {for y in 0..usize::from(crop.height){for x in 0..usize::from(crop.width){
        let i=y*stride+x;
        let sum=differences[i]+if x>0{differences[i-1]}else{0}+if y>0{differences[i-stride]}else{0}-if x>0&&y>0{differences[i-stride-1]}else{0};
        differences[i]=sum;coverage[y*usize::from(crop.width)+x]=sum>0;
    }}}
    Ok((present,coverage))
}
fn color(value: u32) -> Result<GridColor> {
    match value { 0 => Ok(GridColor::Default), 1..=16 => Ok(GridColor::Indexed { index:Uint(u64::from(value-1)) }),
        0x01000000..=0x010000ff => Ok(GridColor::Indexed { index:Uint(u64::from(value&255)) }),
        0x02000000..=0x02ffffff => Ok(GridColor::Rgb { value:Uint(u64::from(value&0xffffff)) }), _ => Err(ErrorCode::NotReady) }
}
fn attributes(value: u16) -> GridAttributes {
    GridAttributes { bold:value&1!=0, faint:value&2!=0, italic:value&4!=0, blink:value&0x30!=0, strike:value&0x100!=0,
        inverse:value&0x40!=0, concealed:value&0x80!=0,
        underline:if value&8==0 { GridUnderline::None } else { match (value>>12)&15 { 2=>GridUnderline::Double,3=>GridUnderline::Curly,4=>GridUnderline::Dotted,5=>GridUnderline::Dashed,_=>GridUnderline::Single } }, underline_color:None }
}
fn width(text: &str) -> usize {
    let mut chars = text.chars();
    if let (Some(first),Some(second),None) = (chars.next(),chars.next(),chars.next())
        && ('\u{ff66}'..='\u{ff9d}').contains(&first) && matches!(second,'\u{ff9e}'|'\u{ff9f}') { return 2; }
    UnicodeWidthStr::width(text)
}
fn prefix_bit(prefix: &[u8], index: usize) -> bool { prefix[index/8] & (1 << (index%8)) != 0 }
fn set_prefix(prefix: &mut [u8], index: usize, value: bool) { let mask=1 << (index%8); if value { prefix[index/8]|=mask; } else { prefix[index/8]&=!mask; } }

/// Only selected native cells survive a message. Prefix bits retain traversal, never sibling content.
pub(super) struct Surface {
    boot: String, projection: u64, revision: u64, width: u16, height: u16,
    panes: Vec<PaneGeometry>, selected: usize, cells: Vec<RawCell>, prefix: Vec<u8>,
    owners:Vec<u16>,
    cursor: Option<Cursor>, popup: bool, graphics: bool, coverage: Vec<bool>,
}
pub(super) struct Projection { pub begin: GridBegin, pub cells: Vec<GridCell> }
impl Surface {
    fn full(d: &mut Decoder<'_>, selected: &str) -> Result<Option<Self>> {
        let boot=d.string()?.to_owned(); require(!boot.is_empty() && boot.len() <=128)?;
        let projection=d.u64()?; let revision=d.u64()?; require(projection>0 && revision>0 && revision<u64::MAX)?;
        let frame=frame(d)?;native_geometry(frame.width,frame.height)?;let panes=panes(d)?;
        let bounds=Rect{x:0,y:0,width:frame.width,height:frame.height};
        for pane in &panes { require(bounds.contains(pane.rect))?; }
        let mut owners=vec![0u16;frame.count];
        for (index,pane) in panes.iter().enumerate(){for y in pane.rect.y..pane.rect.bottom() as u16 {for x in pane.rect.x..pane.rect.right() as u16 {
            let owner=&mut owners[usize::from(y)*usize::from(frame.width)+usize::from(x)];
            require(*owner==0)?;*owner=index as u16+1;
        }}}
        for _ in 0..d.count(1)? { d.variant(2)?; d.u16()?; let area=Rect::read(d)?; let hit=Rect::read(d)?; require(bounds.contains(area)&&bounds.contains(hit))?; for _ in 0..d.count(1)? {d.boolean()?;} }
        let popup=d.option(popup)?.is_some(); let selected_index=panes.iter().position(|p|p.id==selected);
        let crop=selected_index.map(|index|panes[index].inner);
        if let Some(crop)=crop{require(crop.width>0&&crop.width<=512&&crop.height>0&&crop.height<=256&&usize::from(crop.width)*usize::from(crop.height)<=32768)?;}
        let (graphics,coverage)=graphics(d,selected,crop,frame.width,frame.height)?;
        let Some(index)=selected_index else { return Ok(None) };
        let crop=panes[index].inner;require(panes[index].content%2==0)?;
        let mut cells=Vec::with_capacity(usize::from(crop.width)*usize::from(crop.height));
        let mut prefix=vec![0; (usize::from(crop.x)*usize::from(crop.height)).div_ceil(8)];
        let mut data=Decoder::new(frame.cells);
        for n in 0..frame.count {
            let cell=cell(&mut data)?; let x=(n%usize::from(frame.width)) as u16; let y=(n/usize::from(frame.width)) as u16;
            if y>=crop.y && u32::from(y)<crop.bottom() {
                if x<crop.x { let w=width(cell.text); require(cell.skip || w<=2)?; set_prefix(&mut prefix,usize::from(y-crop.y)*usize::from(crop.x)+usize::from(x),!cell.skip&&w==2); }
                else if crop.point(x,y) { cells.push(cell.owned()); }
            }
        }
        Ok(Some(Self {boot,projection,revision,width:frame.width,height:frame.height,panes,selected:index,cells,prefix,owners,cursor:frame.cursor,popup,graphics:graphics||frame.graphics,coverage}))
    }
    fn patch(&mut self,d:&mut Decoder<'_>)->Result<()> {
        require(!self.popup && !self.graphics && d.string()?==self.boot && d.u64()?==self.projection && d.u64()?==self.revision)?;
        let revision=d.u64()?; require(self.revision.checked_add(1)==Some(revision) && revision<u64::MAX)?;
        let rows=d.count(1)?; let start=d.at;
        for _ in 0..rows {d.u16()?;d.u16()?;for _ in 0..d.count(6)? {cell(d)?;}}
        let row_bytes=&d.bytes[start..d.at]; let updated=panes(d)?; let cursor=d.option(cursor)?;
        require(cursor.is_none_or(|c|c.x<self.width&&c.y<self.height))?;
        d.done()?;
        let mut changed=vec![None;self.panes.len()];
        for (update,pane) in updated.iter().enumerate(){let index=self.panes.iter().position(|old|old.id==pane.id).ok_or(ErrorCode::NotReady)?;let old=&self.panes[index];require(old.same_geometry(pane)&&old.alternate==pane.alternate&&(index!=self.selected||pane.content%2==0))?;changed[index]=Some(update);}
        // Validate all spans before mutating the retained baseline, including sibling-only updates.
        let mut rows_decoder=Decoder::new(row_bytes);
        for _ in 0..rows {
            let x=rows_decoder.u16()?;let y=rows_decoder.u16()?;let count=rows_decoder.count(6)?;
            require(count>0 && usize::from(x).checked_add(count).is_some_and(|right|right<=usize::from(self.width)) && y<self.height)?;
            let span=Rect{x,y,width:u16::try_from(count).map_err(|_|ErrorCode::NotReady)?,height:1};
            let owner=usize::from(self.owners[usize::from(y)*usize::from(self.width)+usize::from(x)].checked_sub(1).ok_or(ErrorCode::NotReady)?);
            let pane=&updated[changed[owner].ok_or(ErrorCode::NotReady)?];let old=&self.panes[owner];
            require(pane.inner.contains(span)||pane.scrollbar.or(old.scrollbar).is_some_and(|bar|bar.x==x&&usize::from(bar.width)==count&&bar.point(x,y)))?;
            for _ in 0..count {cell(&mut rows_decoder)?;}
        }
        let crop=self.panes[self.selected].inner; let mut rows_decoder=Decoder::new(row_bytes);
        for _ in 0..rows {
            let x=rows_decoder.u16()?;let y=rows_decoder.u16()?;let count=rows_decoder.count(6)?;
            for offset in 0..count {
                let cell=cell(&mut rows_decoder)?;let x=x+offset as u16;
                if y>=crop.y&&u32::from(y)<crop.bottom() {
                    if x<crop.x {let w=width(cell.text);require(cell.skip||w<=2)?;set_prefix(&mut self.prefix,usize::from(y-crop.y)*usize::from(crop.x)+usize::from(x),!cell.skip&&w==2);}
                    else if crop.point(x,y) { self.cells[usize::from(y-crop.y)*usize::from(crop.width)+usize::from(x-crop.x)]=cell.owned(); }
                }
            }
        }
        for pane in updated {let index=self.panes.iter().position(|old|old.id==pane.id).ok_or(ErrorCode::NotReady)?;self.panes[index]=pane;}
        self.cursor=cursor;self.revision=revision;Ok(())
    }
    pub(super) fn projection(&self,revision:u64)->Result<Projection> {
        let selected=&self.panes[self.selected];let crop=selected.inner;let mut cells:Vec<GridCell>=Vec::with_capacity(self.cells.len());
        for y in 0..crop.height {
            let mut consumed=false;
            for x in 0..crop.x { if consumed {consumed=false;} else {consumed=prefix_bit(&self.prefix,usize::from(y)*usize::from(crop.x)+usize::from(x));} }
            require(!consumed)?;
            for x in 0..crop.width {
                let index=usize::from(y)*usize::from(crop.width)+usize::from(x);let raw=&self.cells[index];
                if consumed {let lead=cells.last().ok_or(ErrorCode::NotReady)?;let tail=GridCell{text:String::new(),width:Uint(0),foreground:lead.foreground.clone(),background:lead.background.clone(),attributes:lead.attributes.clone()};cells.push(tail);consumed=false;continue;}
                let w=width(&raw.text); let omitted=(raw.skip||w==0)&&self.coverage.get(index).copied().unwrap_or(false);
                require(omitted || !raw.skip && (1..=2).contains(&w))?;
                require(omitted || raw.text.len()<=128)?;
                let cell=GridCell{text:if omitted {" ".into()} else {raw.text.clone()},width:Uint(if omitted {1}else{w as u64}),foreground:color(raw.foreground)?,background:color(raw.background)?,attributes:attributes(raw.modifier)};
                cell.validate().map_err(|_|ErrorCode::NotReady)?;
                consumed=!omitted&&w==2;require(!consumed||x+1<crop.width)?;cells.push(cell);
            }
        }
        require(self.popup||self.cursor.is_none_or(|cursor|!cursor.visible||crop.point(cursor.x,cursor.y)))?;
        let cursor=self.cursor.filter(|c|crop.point(c.x,c.y)).map(|c|GridCursor{column:Uint(u64::from(c.x-crop.x)),row:Uint(u64::from(c.y-crop.y)),visible:c.visible&&!self.popup,shape:Uint(u64::from(c.shape))});
        let begin=GridBegin{mode:GridMode::Replace,revision:Uint(revision),columns:Uint(u64::from(crop.width)),rows:Uint(u64::from(crop.height)),cursor,alternate_screen:selected.alternate,graphics_omitted:self.graphics,popup_blocked:self.popup,part_count:Uint(cells.len().div_ceil(512) as u64)};
        begin.validate().map_err(|_|ErrorCode::NotReady)?;Ok(Projection{begin,cells})
    }
    pub(super) fn dimensions(&self)->(u16,u16){(self.width,self.height)}
    pub(super) fn popup(&self)->bool{self.popup}
    pub(super) fn revisions(&self)->(&str,u64,u64){(&self.boot,self.projection,self.revision)}
    pub(super) fn focused(&self)->bool{self.panes[self.selected].focused}
}

pub(super) enum Message<'a> { Control(&'a str,&'a str), Chunk{boot:&'a str,id:&'a str,complete:bool,bytes:&'a [u8]}, Surface, Auxiliary }
fn legacy_snapshot(d:&mut Decoder<'_>)->Result<()> {
    d.string()?;d.u64()?;d.option(Decoder::string)?;
    d.option(|d|{for _ in 0..4{d.string()?;}d.boolean()?;Ok(())})?;
    d.option(Decoder::string)?;d.string()?;d.option(Decoder::string)?;d.boolean()?;d.boolean()?;d.string()?;
    d.option(|d|{d.string()?;d.string()?;d.boolean()?;Ok(())})?;
    for _ in 0..3{d.option(Decoder::string)?;}
    for _ in 0..d.count(1)?{d.string()?;d.boolean()?;}d.string()?;d.option(Decoder::string)?;d.strings()?;
    for _ in 0..d.count(1)? {d.string()?;d.string()?;d.string()?;d.u64()?;d.string()?;d.boolean()?;d.option(Decoder::string)?;d.option(|d|{d.u64()?;d.u64()?;Ok(())})?;d.pairs()?;d.option(|d|{d.string()?;d.string()?;d.boolean()?;Ok(())})?;d.boolean()?;d.variant(5)?;}
    for _ in 0..d.count(1)?{d.string()?;d.string()?;d.u64()?;d.string()?;for _ in 0..3{d.boolean()?;}d.variant(5)?;}
    for _ in 0..d.count(1)?{for _ in 0..3{d.string()?;}for _ in 0..3{d.option(Decoder::string)?;}d.boolean()?;d.boolean()?;}
    for _ in 0..d.count(1)?{for _ in 0..3{d.string()?;}for _ in 0..6{d.option(Decoder::string)?;}d.variant(5)?;d.u64()?;d.pairs()?;d.pairs()?;d.boolean()?;}
    for _ in 0..d.count(1)?{d.string()?;d.string()?;d.strings()?;d.variant(5)?;d.option(Decoder::string)?;}Ok(())
}
pub(super) fn decode<'a>(bytes:&'a [u8],selected:&str,baseline:&mut Option<Surface>)->Result<Message<'a>> {
    decode_bounded(bytes,selected,baseline,MAX_FRAME)
}
fn decode_bounded<'a>(bytes:&'a [u8],selected:&str,baseline:&mut Option<Surface>,maximum:usize)->Result<Message<'a>> {
    require(!bytes.is_empty()&&bytes.len()<=maximum)?;let mut d=Decoder::new(bytes);
    let message=match d.variant(21)? {
        0=>{d.u32()?;d.variant(2)?;d.option(Decoder::string)?;Message::Auxiliary},
        1=>{d.u64()?;d.u16()?;d.u16()?;d.boolean()?;d.blob()?;Message::Auxiliary},
        2=>{d.blob()?;Message::Auxiliary},
        3=>{d.option(Decoder::string)?;return Err(ErrorCode::StaleSession)},
        4=>{d.variant(3)?;d.string()?;d.option(Decoder::string)?;Message::Auxiliary},
        5=>{d.string()?;Message::Auxiliary},6=>{d.option(Decoder::string)?;Message::Auxiliary},7=>Message::Auxiliary,
        8=>{d.boolean()?;d.boolean()?;Message::Auxiliary},9=>{d.u16()?;Message::Auxiliary},
        10=>{d.string()?;d.u64()?;d.u32()?;d.u64()?;d.blob()?;d.string()?;d.option(|d|asset(d,selected))?;Message::Auxiliary},
        11=>{d.u64()?;d.u32()?;Message::Auxiliary},12=>{legacy_snapshot(&mut d)?;Message::Auxiliary},
        13=>{let next=Surface::full(&mut d,selected)?;d.done()?;*baseline=next;Message::Surface},
        14=>{d.variant(4)?;d.string()?;d.option(Decoder::string)?;d.option(|d|d.variant(2))?;for _ in 0..4{d.option(Decoder::string)?;}d.option(|d|d.variant(4))?;Message::Auxiliary},
        15=>{d.string()?;return Err(ErrorCode::NotReady)},16=>{d.u16()?;d.byte()?;Message::Auxiliary},17=>{d.boolean()?;Message::Auxiliary},
        18=>Message::Chunk{boot:d.string()?,id:d.string()?,complete:d.boolean()?,bytes:d.blob()?},
        19=>{baseline.as_mut().ok_or(ErrorCode::NotReady)?.patch(&mut d)?;Message::Surface},
        20=>Message::Control(d.string()?,d.string()?),_=>return Err(ErrorCode::NotReady),
    };d.done()?;Ok(message)
}

/// The direct observer borrows its ANSI blob; all other stock messages are validated
/// and discarded without materializing a second screen or invoking endpoint control.
pub(super) enum ObserverMessage<'a> {
    Welcome {version:u32,encoding:u32,error:bool},
    Frame {seq:u64,width:u16,height:u16,full:bool,bytes:&'a [u8]},
    Auxiliary,
    Shutdown,
}
pub(super) fn observer_decode(bytes:&[u8])->Result<ObserverMessage<'_>> {
    require(!bytes.is_empty()&&bytes.len()<=32*1024*1024)?;
    let mut d=Decoder::new(bytes);
    let message=match d.variant(21)? {
        0=>ObserverMessage::Welcome{version:d.u32()?,encoding:d.variant(2)?,error:d.option(Decoder::string)?.is_some()},
        1=>ObserverMessage::Frame{seq:d.u64()?,width:d.u16()?,height:d.u16()?,full:d.boolean()?,bytes:d.blob()?},
        3=>{d.option(Decoder::string)?;ObserverMessage::Shutdown},
        13=>{
            d.string()?;d.u64()?;d.u64()?;let f=frame(&mut d)?;panes(&mut d)?;
            for _ in 0..d.count(1)? {d.variant(2)?;d.u16()?;Rect::read(&mut d)?;Rect::read(&mut d)?;for _ in 0..d.count(1)?{d.boolean()?;}}
            d.option(popup)?;graphics(&mut d,"",None,f.width,f.height)?;
            ObserverMessage::Auxiliary
        },
        19=>{
            d.string()?;d.u64()?;d.u64()?;d.u64()?;
            for _ in 0..d.count(1)?{d.u16()?;d.u16()?;for _ in 0..d.count(6)?{cell(&mut d)?;}}
            panes(&mut d)?;d.option(cursor)?;ObserverMessage::Auxiliary
        },
        15=>{d.string()?;ObserverMessage::Auxiliary},
        _=>{
            // The ordinary decoder already freezes every auxiliary field position.
            // Its size check is separate from this direct client's 32MiB framing cap.
            let mut baseline=None;
            decode_auxiliary(bytes,&mut baseline)?;
            return Ok(ObserverMessage::Auxiliary);
        },
    };
    d.done()?;Ok(message)
}
fn decode_auxiliary(bytes:&[u8],baseline:&mut Option<Surface>)->Result<()> {
    decode_bounded(bytes,"",baseline,32*1024*1024).map(|_|())
}

/// Partial reads live on the actual owner; selecting a resize/release cannot lose a frame suffix.
pub(super) struct Reader { header:[u8;4], header_read:usize, payload:Vec<u8>, payload_read:usize, pub(super) deadline:Option<Instant>, maximum:usize }
impl Reader {
    pub(super) fn new()->Self{Self::bounded(MAX_FRAME)}
    pub(super) fn observer()->Self{Self::bounded(32*1024*1024)}
    fn bounded(maximum:usize)->Self{Self{header:[0;4],header_read:0,payload:Vec::new(),payload_read:0,deadline:None,maximum}}
    pub(super) async fn read(&mut self,reader:&mut BufReader<UnixStream>)->Result<Vec<u8>> {
        if self.header_read==0 {
            let n=reader.read(&mut self.header).await.map_err(policy)?;require(n>0)?;
            self.header_read=n;self.deadline=Some(Instant::now()+Duration::from_secs(15));
        }
        let deadline=self.deadline.ok_or(ErrorCode::NotReady)?;
        while self.header_read<4 {let n=timeout_at(deadline,reader.read(&mut self.header[self.header_read..])).await.map_err(|_|ErrorCode::Timeout)?.map_err(policy)?;require(n>0)?;self.header_read+=n;}
        if self.payload.is_empty(){let n=u32::from_le_bytes(self.header) as usize;require(n>0&&n<=self.maximum)?;self.payload.resize(n,0);}
        while self.payload_read<self.payload.len(){let n=timeout_at(deadline,reader.read(&mut self.payload[self.payload_read..])).await.map_err(|_|ErrorCode::Timeout)?.map_err(policy)?;require(n>0)?;self.payload_read+=n;}
        self.header_read=0;self.payload_read=0;Ok(std::mem::take(&mut self.payload))
    }
}

fn json(bytes:&[u8])->Result<Value> {
    crate::native_json::parse(bytes,crate::native_json::Limits{bytes:MAX_FRAME,depth:64,members:131072}).map_err(|_|ErrorCode::NotReady)
}
fn text<'a>(value:&'a Value,key:&str)->Result<&'a str> {
    let value=value.get(key).and_then(Value::as_str).ok_or(ErrorCode::NotReady)?;
    require(!value.is_empty()&&value.len()<=4096)?;Ok(value)
}
#[derive(Clone)]
struct View {
    boot:String,revision:u64,focused_workspace:Option<String>,focused_tab:Option<String>,focused_pane:Option<String>,
}
impl View {
    fn read(value:&Value,route:&shells::ShellIdentity)->Result<Self> {
        let boot=text(value,"boot_id")?;require(boot.len()<=128)?;
        let revision=value.get("revision").and_then(Value::as_u64).ok_or(ErrorCode::NotReady)?;
        require(revision>0&&revision<u64::MAX)?;
        let panes=value.get("panes").and_then(Value::as_array).ok_or(ErrorCode::NotReady)?;
        require(panes.len()<=512)?;
        let mut found=false;let mut ids=HashSet::new();
        for pane in panes {
            let id=text(pane,"pane_id")?;require(ids.insert(id))?;
            if id==route.pane_id.0 {
                require(text(pane,"workspace_id")?==route.workspace_id.0&&text(pane,"tab_id")?==route.tab_id.0)?;
                found=true;
            }
        }
        require(found)?;
        let agents=value.get("agents").and_then(Value::as_array).ok_or(ErrorCode::NotReady)?;
        require(agents.len()<=512)?;
        for agent in agents {
            require(text(agent,"pane_id")?!=route.pane_id.0)?;
        }
        let optional=|key|->Result<Option<String>>{match value.get(key){None|Some(Value::Null)=>Ok(None),Some(Value::String(value))if!value.is_empty()&&value.len()<=4096=>Ok(Some(value.clone())),_=>Err(ErrorCode::NotReady)}};
        Ok(Self{boot:boot.to_owned(),revision,focused_workspace:optional("focused_workspace_id")?,focused_tab:optional("focused_tab_id")?,focused_pane:optional("focused_pane_id")?})
    }
    fn selected(&self,route:&shells::ShellIdentity)->bool {
        self.focused_workspace.as_deref()==Some(route.workspace_id.0.as_str())
            &&self.focused_tab.as_deref()==Some(route.tab_id.0.as_str())
            &&self.focused_pane.as_deref()==Some(route.pane_id.0.as_str())
    }
}
pub(super) struct Session {
    pub client:Client,
    reader:Reader,
    original:Source,
    route:shells::ShellIdentity,
    lease:std::sync::Weak<grid::GridLease>,
    auth:Arc<Authorized>,
    pub surface:Option<Surface>,
    view:Option<View>,
    welcome:bool,
    floor:u64,
    active:bool,
    frame_incomplete:bool,
}
pub(super) enum Incoming {
    Surface,
    Metadata,
    Auxiliary,
    Chunk{boot:String,id:String,complete:bool,frame:Vec<u8>,range:std::ops::Range<usize>},
}
impl Session {
    pub(super) async fn connect(context:&shells::LogicalShellContext,lease:std::sync::Weak<grid::GridLease>,auth:Arc<Authorized>)->Result<Self> {
        context.source.recheck().map_err(policy)?;
        let parent=context.source.socket_path.parent().ok_or(ErrorCode::NotReady)?;
        let stem=context.source.socket_path.file_stem().and_then(|value|value.to_str()).unwrap_or("herdr");
        let client=Client::connect(&parent.join(format!("{stem}-client.sock"))).await.map_err(policy)?;
        // Each socket retains its OWN dev/inode. Only actual server process identities join.
        require(client.evidence.linux_boot_id==context.source.evidence.linux_boot_id
            &&client.evidence.pid==context.source.evidence.pid
            &&client.evidence.pid_start_ticks==context.source.evidence.pid_start_ticks)?;
        context.source.recheck().map_err(policy)?;
        Ok(Self{client,reader:Reader::new(),original:context.source.clone(),route:context.proof.identity.clone(),lease,auth,surface:None,view:None,welcome:false,floor:0,active:false,frame_incomplete:false})
    }
    pub(super) fn recheck(&self)->Result<()> {
        self.original.recheck_identity().map_err(policy)?;
        self.client.recheck().map_err(policy)
    }
    pub(super) fn transaction_deadline(&self)->Instant {
        self.reader.deadline.expect("only a complete native message has a transaction")
    }
    pub(super) fn read_in_progress(&self)->bool {self.reader.header_read!=0||!self.reader.payload.is_empty()}
    pub(super) async fn send_input(&mut self,bytes:&[u8],owner:&pairing::ShellInputProgressOwner,reply:&tokio::sync::oneshot::Sender<grid::InputCompletion>)->Result<()> {
        require(!self.frame_incomplete)?;
        let lease=self.lease.upgrade().ok_or(ErrorCode::StaleSession)?;
        let mut progress=owner.progress()?;let mut written=0usize;
        while written<bytes.len() {
            std::future::poll_fn(|cx|{
                use std::task::Poll;
                if reply.is_closed(){return Poll::Ready(Err(ErrorCode::StaleSession));}
                if let Err(code)=self.auth.live_connection().and_then(|()|lease.input_ready()).and_then(|()|self.recheck()){
                    return Poll::Ready(Err(code));
                }
                match tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(self.client.reader.get_mut()),cx,&bytes[written..]) {
                    Poll::Pending=>Poll::Pending,
                    Poll::Ready(Ok(0))=>Poll::Ready(Err(ErrorCode::NotReady)),
                    Poll::Ready(Err(error))=>Poll::Ready(Err(policy(error))),
                    Poll::Ready(Ok(count))=>{
                        written+=count;progress.socket_bytes_written=Some(written as u32);
                        progress.frame_complete=Some(written==bytes.len());
                        self.frame_incomplete=written!=bytes.len();
                        Poll::Ready(owner.retain(&progress))
                    }
                }
            }).await?;
        }
        Ok(())
    }
    pub(super) async fn send(&mut self,bytes:&[u8])->Result<()> {
        require(!self.frame_incomplete)?;
        self.recheck()?;
        let lease=self.lease.upgrade().ok_or(ErrorCode::StaleSession)?;
        let active=self.active;
        // Cancellation of write_all cannot prove a boundary; close rather than append
        // a deactivation frame to a potentially incomplete native message.
        self.frame_incomplete=true;
        {
            let mut write=std::pin::pin!(self.client.reader.get_mut().write_all(bytes));
            std::future::poll_fn(|cx|{
                if let Err(code)=self.auth.live_connection().and_then(|()|if active{lease.screen()}else{lease.check_current()}) {
                    return std::task::Poll::Ready(Err(code));
                }
                write.as_mut().poll(cx).map(|result|result.map_err(policy))
            }).await?;
        }
        self.frame_incomplete=false;
        self.recheck()
    }
    pub(super) async fn next(&mut self)->Result<Incoming> {
        let bytes=self.reader.read(&mut self.client.reader).await?;
        self.recheck()?;
        let old=self.surface.as_ref().map(|surface|surface.revision);
        match decode(&bytes,&self.route.pane_id.0,&mut self.surface)? {
            Message::Control(kind,data)=>{
                if kind=="endpoint.welcome.v1" {
                    require(!self.welcome)?;
                    let value=json(data.as_bytes())?;
                    require(value.get("generation").and_then(Value::as_u64)==Some(1)
                        &&value.get("error").is_none_or(Value::is_null)
                        &&value.get("snapshot_codec").and_then(Value::as_str)==Some("shell.snapshot.v1")
                        &&value.get("surface_codec").and_then(Value::as_str)==Some("shell.surface.v1")
                        &&value.get("input_codec").and_then(Value::as_str)==Some("shell.input.semantic.v1")
                        &&value.get("blob_codec").and_then(Value::as_str)==Some("shell.blob.v1"))?;
                    text(&value,"server_version")?;
                    let methods=value.get("methods").and_then(Value::as_array).ok_or(ErrorCode::NotReady)?;
                    require(methods.len()<=512&&["client_shell.surface.set","workspace.focus","tab.focus","pane.focus"].iter().all(|method|methods.iter().any(|value|value.as_str()==Some(method))))?;
                    self.welcome=true;
                } else if kind=="shell.snapshot.v1" {
                    require(self.welcome)?;
                    let view=View::read(&json(data.as_bytes())?,&self.route)?;
                    if let Some(old)=&self.view {require(view.boot==old.boot&&view.revision>=old.revision)?;}
                    self.view=Some(view);
                }
                Ok(Incoming::Metadata)
            }
            Message::Surface=>{
                require(self.active)?;
                if let Some(surface)=&self.surface {
                    let view=self.view.as_ref().ok_or(ErrorCode::NotReady)?;
                    require(surface.boot==view.boot)?;
                    if let Some(revision)=old {require(surface.revision>revision)?;}
                }
                Ok(Incoming::Surface)
            }
            Message::Chunk{boot,id,complete,bytes:data}=>{
                require(boot.len()<=128&&id.len()<=128&&data.len()<=512*1024)?;
                let start=(data.as_ptr() as usize).checked_sub(bytes.as_ptr() as usize).ok_or(ErrorCode::NotReady)?;
                let end=start.checked_add(data.len()).ok_or(ErrorCode::ResourceLimit)?;
                require(end<=bytes.len())?;let boot=boot.to_owned();let id=id.to_owned();
                Ok(Incoming::Chunk{boot,id,complete,frame:bytes,range:start..end})
            }
            Message::Auxiliary=>Ok(Incoming::Auxiliary),
        }
    }
    pub(super) async fn negotiate(&mut self,columns:u16,rows:u16)->Result<()> {
        self.send(&hello(columns,rows)?).await?;
        while !self.welcome||self.view.is_none() {
            require(!matches!(self.next().await?,Incoming::Chunk{..}|Incoming::Surface))?;
        }
        Ok(())
    }
    pub(super) async fn rpc(&mut self,method:&str,params:Value,expected:&str)->Result<Value> {
        let boot=self.view.as_ref().ok_or(ErrorCode::NotReady)?.boot.clone();
        let id=Uuid::random().0;
        self.send(&command(&boot,&id,method,params)?).await?;
        let mut response=Vec::new();
        loop {
            if let Incoming::Chunk{boot:actual,id:actual_id,complete,frame,range}=self.next().await? {
                require(actual==boot&&actual_id==id)?;
                let size=response.len().checked_add(range.len()).ok_or(ErrorCode::ResourceLimit)?;
                require(size<=1024*1024)?;response.extend_from_slice(&frame[range]);
                if !complete {continue;}
                let mut value=json(&response)?;
                require(value.get("id").and_then(Value::as_str)==Some(id.as_str())
                    &&value.get("result").is_some()!=value.get("error").is_some())?;
                if value.get("error").is_some(){return Err(ErrorCode::NotReady);}
                let result=value.get_mut("result").ok_or(ErrorCode::NotReady)?.take();
                require(result.get("type").and_then(Value::as_str)==Some(expected))?;
                return Ok(result);
            }
        }
    }
    pub(super) async fn activate(&mut self)->Result<()> {
        let prior=self.view.as_ref().ok_or(ErrorCode::NotReady)?.revision;
        // Surface traffic may precede its command response; only post-floor coherent views publish.
        self.active=true;
        let result=self.rpc("client_shell.surface.set",json!({"active":true}),"client_shell_surface_set").await?;
        require(result.get("active").and_then(Value::as_bool)==Some(true))?;
        self.floor=result.get("projection_revision").and_then(Value::as_u64).ok_or(ErrorCode::NotReady)?;
        require(self.floor>prior&&self.floor<u64::MAX)
    }
    pub(super) async fn focus_workspace(&mut self)->Result<()> {
        let result=self.rpc("workspace.focus",json!({"workspace_id":self.route.workspace_id}),"workspace_info").await?;
        require(result.get("workspace").and_then(|value|value.get("workspace_id")).and_then(Value::as_str)==Some(self.route.workspace_id.0.as_str()))
    }
    pub(super) async fn focus_tab(&mut self)->Result<()> {
        let result=self.rpc("tab.focus",json!({"tab_id":self.route.tab_id}),"tab_info").await?;
        let tab=result.get("tab").ok_or(ErrorCode::NotReady)?;
        require(tab.get("tab_id").and_then(Value::as_str)==Some(self.route.tab_id.0.as_str())&&tab.get("workspace_id").and_then(Value::as_str)==Some(self.route.workspace_id.0.as_str()))
    }
    pub(super) async fn focus_pane(&mut self)->Result<()> {
        let result=self.rpc("pane.focus",json!({"pane_id":self.route.pane_id}),"pane_info").await?;
        let pane=result.get("pane").ok_or(ErrorCode::NotReady)?;
        require(pane.get("pane_id").and_then(Value::as_str)==Some(self.route.pane_id.0.as_str())
            &&pane.get("terminal_id").and_then(Value::as_str)==Some(self.route.terminal_id.0.as_str())
            &&pane.get("workspace_id").and_then(Value::as_str)==Some(self.route.workspace_id.0.as_str())
            &&pane.get("tab_id").and_then(Value::as_str)==Some(self.route.tab_id.0.as_str()))
    }
    pub(super) fn coherent(&self)->bool {
        match (&self.view,&self.surface) {
            (Some(view),Some(surface))=>self.active&&view.selected(&self.route)&&surface.focused()
                &&surface.boot==view.boot&&surface.projection==view.revision&&surface.projection>=self.floor,
            _=>false,
        }
    }
    pub(super) async fn deactivate(&mut self) {
        self.surface=None;
        if !self.active||self.frame_incomplete{return;}
        self.active=false;
        if let Some(view)=&self.view
            &&let Ok(bytes)=command(&view.boot,&Uuid::random().0,"client_shell.surface.set",json!({"active":false}))
        {
            // Exact owned-connection cleanup only. Never select/focus or recover a replacement.
            if let Some(lease)=self.lease.upgrade()
                &&self.recheck().is_ok()&&self.auth.live_connection().is_ok()&&lease.screen().is_ok()
            {
                let mut write=std::pin::pin!(self.client.reader.get_mut().write_all(&bytes));
                let auth=&self.auth;
                let cleanup=std::future::poll_fn(|cx|{
                    if let Err(code)=auth.live_connection().and_then(|()|lease.screen()){return std::task::Poll::Ready(Err(code));}
                    write.as_mut().poll(cx).map(|result|result.map_err(policy))
                });
                let _=tokio::time::timeout(Duration::from_secs(5),cleanup).await;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    pub(in crate::herdr) async fn established(context:&shells::LogicalShellContext,lease:std::sync::Weak<grid::GridLease>,auth:Arc<Authorized>)->(Session,UnixStream) {
        use std::os::unix::fs::PermissionsExt;
        let parent=context.source.socket_path.parent().unwrap();
        let stem=context.source.socket_path.file_stem().unwrap().to_str().unwrap();
        let path=parent.join(format!("{stem}-client.sock"));
        let listener=tokio::net::UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path,fs::Permissions::from_mode(0o600)).unwrap();
        let (session,peer)=tokio::join!(Session::connect(context,lease,auth),listener.accept());
        let mut session=session.unwrap();let (peer,_)=peer.unwrap();
        session.view=Some(View::read(&view(1),&context.proof.identity).unwrap());
        decode(&surface(1,1,1,&[("x",0,0,0,false)],Rect{x:0,y:0,width:1,height:1},false),"p",&mut session.surface).unwrap();
        session.welcome=true;session.active=true;session.floor=1;
        assert!(session.coherent());(session,peer)
    }
    pub fn native_control(kind:&str,value:Value)->Vec<u8>{control(kind,&value).unwrap()}
    pub fn response(id:&str,result:Value)->Vec<u8>{
        let value=serde_json::to_vec(&json!({"id":id,"result":result})).unwrap();let mut bytes=vec![0,0,0,0,18];
        put_string(&mut bytes,"b");put_string(&mut bytes,id);bytes.push(1);put_uint(&mut bytes,value.len() as u64);bytes.extend(value);framed(bytes).unwrap()
    }
    pub fn read_client(bytes:&[u8])->(u32,Value) {
        let mut d=Decoder::new(bytes);let tag=d.u32().unwrap();
        let value=match tag {
            20=>{assert_eq!(d.string().unwrap(),"endpoint.hello.v1");json(d.string().unwrap().as_bytes()).unwrap()},
            15=>{assert_eq!(d.string().unwrap(),"b");json(d.string().unwrap().as_bytes()).unwrap()},
            12=>{assert_eq!(d.u32().unwrap(),0);assert_eq!(d.u32().unwrap(),0);let cols=d.u16().unwrap();let rows=d.u16().unwrap();assert!(!d.boolean().unwrap());json!({"columns":cols,"rows":rows})},
            _=>panic!("unexpected fixture client tag"),
        };d.done().unwrap();(tag,value)
    }
    pub fn view(revision:u64)->Value {json!({"boot_id":"b","revision":revision,"focused_workspace_id":"w","focused_tab_id":"t","focused_pane_id":"p","panes":[{"pane_id":"p","workspace_id":"w","tab_id":"t"}],"agents":[]})}
    pub fn cell_bytes(out:&mut Vec<u8>,text:&str,fg:u32,bg:u32,modifier:u16,skip:bool) {
        put_string(out,text);put_uint(out,u64::from(fg));put_uint(out,u64::from(bg));put_uint(out,u64::from(modifier));out.push(u8::from(skip));out.push(0);
    }
    fn rect(out:&mut Vec<u8>,value:Rect){for n in [value.x,value.y,value.width,value.height]{put_uint(out,u64::from(n));}}
    pub fn pane(out:&mut Vec<u8>,id:&str,area:Rect,inner:Rect,focused:bool) {
        put_string(out,id);out.push(2);rect(out,area);rect(out,inner);
        out.extend_from_slice(&[0,0,u8::from(focused),0,0,0,0,0]);
    }
    pub fn surface(columns:u16,revision:u64,projection:u64,cells:&[(&str,u32,u32,u16,bool)],crop:Rect,popup_present:bool)->Vec<u8> {
        let mut out=vec![13];put_string(&mut out,"b");put_uint(&mut out,projection);put_uint(&mut out,revision);
        put_uint(&mut out,cells.len() as u64);for &(text,fg,bg,modifier,skip) in cells {cell_bytes(&mut out,text,fg,bg,modifier,skip);}
        put_uint(&mut out,u64::from(columns));out.extend_from_slice(&[1,0,0,0]);
        out.push(if crop.x>0{2}else{1});
        if crop.x>0{let left=Rect{x:0,y:0,width:crop.x,height:1};pane(&mut out,"sibling",left,left,false);}
        pane(&mut out,"p",crop,crop,true);out.push(0);out.push(u8::from(popup_present));
        if popup_present {
            put_string(&mut out,"popup");put_string(&mut out,"private");out.extend_from_slice(&[0,0,1]);
            cell_bytes(&mut out,"x",0,0,0,false);out.extend_from_slice(&[1,1,0,0,0,0,0,0,0]);
        }
        out.extend_from_slice(&[0,0,0]);out
    }
    pub fn patch(base:u64,rows:&[(u16,Vec<(&str,u32,u32,u16,bool)>)],crop:Rect)->Vec<u8> {
        let mut out=vec![19];put_string(&mut out,"b");out.push(1);put_uint(&mut out,base);put_uint(&mut out,base+1);put_uint(&mut out,rows.len() as u64);
        for (x,cells) in rows {put_uint(&mut out,u64::from(*x));out.push(0);put_uint(&mut out,cells.len() as u64);for &(text,fg,bg,modifier,skip) in cells{cell_bytes(&mut out,text,fg,bg,modifier,skip);}}
        out.push(if crop.x>0{2}else{1});if crop.x>0{let left=Rect{x:0,y:0,width:crop.x,height:1};pane(&mut out,"sibling",left,left,false);}pane(&mut out,"p",crop,crop,true);out.push(0);out
    }
    pub fn packet(payload:&[u8])->Vec<u8>{let mut bytes=(payload.len() as u32).to_le_bytes().to_vec();bytes.extend_from_slice(payload);bytes}
}
#[cfg(test)]
mod tests {
    use super::*;
    use fixture::*;
    #[test]
    fn selected_agent_rows_veto_shell_control_even_without_display_fields() {
        let route=shells::ShellIdentity{pane_id:Id::new("p").unwrap(),terminal_id:Id::new("terminal").unwrap(),workspace_id:Id::new("w").unwrap(),tab_id:Id::new("t").unwrap(),read_identity:None};
        for agent in [json!({"pane_id":"p"}),json!({"pane_id":"p","agent":null,"display_agent":null,"name":null})] {
            for pane in ["p","sibling"] {
                let mut value=view(1);let mut agent=agent.clone();agent["pane_id"]=json!(pane);
                value["panes"].as_array_mut().unwrap().push(json!({"pane_id":"sibling","workspace_id":"w","tab_id":"t"}));
                value["agents"]=json!([agent]);
                let bytes=native_control("shell.snapshot.v1",value);
                let Message::Control(kind,data)=decode(&bytes[4..],"p",&mut None).unwrap() else {panic!("expected snapshot control")};
                assert_eq!(kind,"shell.snapshot.v1");
                let result=View::read(&json(data.as_bytes()).unwrap(),&route);
                if pane=="p" {assert!(matches!(result,Err(ErrorCode::NotReady)));}
                else {assert!(result.unwrap().selected(&route));}
            }
        }
    }
    #[test]
    fn committed_text_and_real_key_phases_match_frozen_native_tag13() {
        use crate::protocol::{ShellKeyEvent,ShellKeyModifiers,Uint};
        let pane=Id::new("p").unwrap();
        assert_eq!(pane_input(&pane,&ShellInput::Text{text:"界".into()}).unwrap(),
            [9,0,0,0,13,1,b'p',1,1,3,0xe7,0x95,0x8c]);
        let event=ShellKeyEvent{key:ShellKey::Character('𝛌'),modifiers:ShellKeyModifiers{shift:true,ctrl:true,alt:true,super_key:true},phase:ShellKeyPhase::Repeat,physical_key_id:Some(Uint(u32::MAX.into()))};
        assert_eq!(pane_input(&pane,&ShellInput::Keys{events:vec![event]}).unwrap(),
            [23,0,0,0,13,1,b'p',1,0,15,240,157,155,140,15,1,1,0,0,1,1,252,255,255,255,255,0]);
        for (phase,tag) in [(ShellKeyPhase::Press,0),(ShellKeyPhase::Repeat,1),(ShellKeyPhase::Release,2)] {
            let event=ShellKeyEvent{key:ShellKey::Tab,modifiers:ShellKeyModifiers{shift:true,ctrl:false,alt:false,super_key:false},phase,physical_key_id:Some(Uint(42))};
            assert_eq!(pane_input(&pane,&ShellInput::Keys{events:vec![event]}).unwrap(),
                [15,0,0,0,13,1,b'p',1,0,11,1,tag,1,0,0,1,1,42,0]);
        }
        let event=ShellKeyEvent{key:ShellKey::F12,modifiers:ShellKeyModifiers{shift:false,ctrl:false,alt:false,super_key:false},phase:ShellKeyPhase::Press,physical_key_id:None};
        assert_eq!(pane_input(&pane,&ShellInput::Keys{events:vec![event]}).unwrap(),
            [15,0,0,0,13,1,b'p',1,0,16,12,0,0,1,0,0,0,0,0]);
    }
    #[test]
    fn frozen_native_tags_and_inactive_geometry_are_not_optional() {
        let mut auxiliaries=vec![vec![0,0,0,0],vec![1,0,0,0,0,0],vec![2,0],vec![4,0,0,0],vec![5,0],vec![6,0],vec![7],vec![8,0,0],vec![8,1,1],vec![9,251,251,0],vec![10,0,0,0,0,0,0,0],vec![11,0,0],vec![14,1,0,0,0,0,0,0,0,0],vec![16,0,0],vec![17,0]];
        let mut legacy=vec![12];legacy.extend([0;23]);auxiliaries.push(legacy);
        for bytes in auxiliaries {
            assert!(matches!(decode(&bytes,"p",&mut None),Ok(Message::Auxiliary)));
        }
        for bytes in [vec![21],vec![8,2,0],vec![17,0,0],vec![5,1,255],vec![5,253,255,255,255,255,255,255,255,255]]{assert!(decode(&bytes,"p",&mut None).is_err());}
        assert!(matches!(decode(&[3,0],"p",&mut None),Err(ErrorCode::StaleSession)));
        assert!(matches!(decode(&[15,0],"p",&mut None),Err(ErrorCode::NotReady)));
        assert!(matches!(decode(&[18,1,b'b',1,b'x',1,0],"p",&mut None),Ok(Message::Chunk{complete:true,..})));
        assert!(matches!(decode(&[20,0,0],"p",&mut None),Ok(Message::Control(_,_))));
        assert_eq!(resize(80,24).unwrap(),vec![6,0,0,0,12,0,0,80,24,0]);
        let bytes=hello(80,24).unwrap();let (tag,value)=read_client(&bytes[4..]);assert_eq!(tag,20);
        assert_eq!(value["surface_size"],json!({"cols":80,"rows":24}));
        for flag in ["surface_active","surface_reuse","surface_delta","pixel_mouse","direct_graphics","endpoint_keybindings","mouse_capture"]{assert_eq!(value[flag],false);}
        assert!(hello(0,24).is_err());
    }
    #[test]
    fn native_width_colors_and_raw_modifiers_preserve_complete_rows() {
        for (text,expected) in [("ｶﾞ",2),("界",2),("👩‍💻",2),("❤️",2),("🇺🇸",2),("  ",2),("x",1)] {assert_eq!(width(text),expected);}
        let crop=Rect{x:0,y:0,width:2,height:1};
        let bytes=surface(2,1,1,&[("界",0x02010203,16,0x40|0x80|8|0x2000,false),("retained raw tail",0,0,0,true)],crop,false);
        let mut baseline=None;decode(&bytes,"p",&mut baseline).unwrap();let projected=baseline.as_ref().unwrap().projection(0).unwrap();
        assert_eq!(projected.cells[0].width,Uint(2));assert_eq!(projected.cells[1].text,"");assert_eq!(projected.cells[1].width,Uint(0));
        assert_eq!(projected.cells[0].foreground,GridColor::Rgb{value:Uint(0x010203)});assert_eq!(projected.cells[0].background,GridColor::Indexed{index:Uint(15)});
        assert!(projected.cells[0].attributes.inverse&&projected.cells[0].attributes.concealed);
        assert_eq!(projected.cells[0].attributes.underline,GridUnderline::Double);assert!(projected.cells[0].attributes.underline_color.is_none());
        for malformed in [17,0x01000100,0x03000001]{assert!(color(malformed).is_err());}
        assert_eq!(color(0x010000ff).unwrap(),GridColor::Indexed{index:Uint(255)});
    }
    #[test]
    fn arbitrary_overlapping_patches_keep_raw_cells_and_exact_baseline_revisions() {
        let crop=Rect{x:0,y:0,width:2,height:1};let mut baseline=None;
        decode(&surface(2,1,1,&[("界",0,0,0,false),("hidden column",0,0,0,true)],crop,false),"p",&mut baseline).unwrap();
        let patch=patch(1,&[(0,vec![("=",0,0,0,false)]),(1,vec![("Z",0,0,0,false)]),(0,vec![("m",0,0,0,false)])],crop);
        decode(&patch,"p",&mut baseline).unwrap();
        let projected=baseline.as_ref().unwrap().projection(1).unwrap();assert_eq!(projected.cells.iter().map(|cell|cell.text.as_str()).collect::<Vec<_>>(),vec!["m","Z"]);
        assert!(decode(&patch,"p",&mut baseline).is_err());
        decode(&[19,1,b'b',1,2,3,0,0,0],"p",&mut baseline).unwrap();assert_eq!(baseline.as_ref().unwrap().revision,3);
        let mut bad=fixture::patch(3,&[(0,vec![("q",0,0,0,false)])],crop);bad.push(0);assert!(decode(&bad,"p",&mut baseline).is_err());assert_eq!(baseline.as_ref().unwrap().revision,3);
    }
    #[test]
    fn sibling_prefix_changes_or_orphaned_skip_never_shift_the_crop() {
        let crop=Rect{x:1,y:0,width:2,height:1};let mut baseline=None;
        decode(&surface(3,1,1,&[("s",0,0,0,false),("a",0,0,0,false),("b",0,0,0,false)],crop,false),"p",&mut baseline).unwrap();
        assert_eq!(baseline.as_ref().unwrap().cells.len(),2);
        decode(&patch(1,&[(0,vec![("界",0,0,0,false)])],crop),"p",&mut baseline).unwrap();
        assert!(baseline.as_ref().unwrap().projection(1).is_err());
        let crop=Rect{x:0,y:0,width:1,height:1};
        for cell in [("",0,0,0,false),("x",0,0,0,true),("界",0,0,0,false)] {
            decode(&surface(1,1,1,&[cell],crop,false),"p",&mut baseline).unwrap();assert!(baseline.as_ref().unwrap().projection(0).is_err());
        }
    }
    #[test]
    fn popup_full_transitions_and_truncated_messages_cannot_publish_partial_state() {
        let crop=Rect{x:0,y:0,width:1,height:1};let bytes=surface(1,1,1,&[("x",0,0,0,false)],crop,true);
        for n in 0..bytes.len(){let mut baseline=None;assert!(decode(&bytes[..n],"p",&mut baseline).is_err());assert!(baseline.is_none());}
        let mut baseline=None;decode(&bytes,"p",&mut baseline).unwrap();assert!(baseline.as_ref().unwrap().projection(0).unwrap().begin.popup_blocked);
        assert!(decode(&patch(1,&[],crop),"p",&mut baseline).is_err());
        decode(&surface(1,2,1,&[("x",0,0,0,false)],crop,false),"p",&mut baseline).unwrap();assert!(!baseline.as_ref().unwrap().projection(1).unwrap().begin.popup_blocked);
    }
    #[tokio::test]
    async fn partial_native_frames_keep_owned_read_position_and_refuse_length_before_allocation() {
        let (read,mut write)=UnixStream::pair().unwrap();let mut read=BufReader::new(read);let mut decoder=Reader::new();
        write.write_all(&[3,0]).await.unwrap();
        {let pending=decoder.read(&mut read);tokio::pin!(pending);assert!(futures_util::poll!(&mut pending).is_pending());}
        write.write_all(&[0,0,8,0,0]).await.unwrap();assert_eq!(decoder.read(&mut read).await.unwrap(),vec![8,0,0]);
        write.write_all(&((MAX_FRAME as u32)+1).to_le_bytes()).await.unwrap();assert!(decoder.read(&mut read).await.is_err());assert!(decoder.payload.is_empty());
    }
}

#[cfg(test)]
mod graphics_tests {
    use super::*;
    #[test]
    fn only_actual_selected_graphics_coverage_can_replace_an_unrepresentable_cell() {
        let crop=Rect{x:0,y:0,width:1,height:1};let mut baseline=None;
        let mut bytes=fixture::surface(1,1,1,&[("",0,0,0,true)],crop,false);
        decode(&bytes,"p",&mut baseline).unwrap();assert!(baseline.as_ref().unwrap().projection(0).is_err());
        bytes.truncate(bytes.len()-3);
        bytes.extend_from_slice(&[0,1,0,0,1,b'p',0,1,1,1,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0,0]);
        decode(&bytes,"p",&mut baseline).unwrap();let projection=baseline.as_ref().unwrap().projection(0).unwrap();
        assert!(projection.begin.graphics_omitted);assert_eq!(projection.cells[0].text," ");assert_eq!(projection.cells[0].width,Uint(1));
        assert!(decode(&fixture::patch(1,&[],crop),"p",&mut baseline).is_err());
    }
}
