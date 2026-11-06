// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.*
import androidx.compose.ui.semantics.*

/** Only a complete replacement is drawable. Cells are moved from staging, never copied per part. */
class TerminalGrid(val begin:GridBegin,val cells:List<GridCell>,val hasBlinkingCells:Boolean) {
    override fun toString()="TerminalGrid(redacted)"
}
internal class TerminalGridReducer(private val ansiObserver:Boolean=false,private val history:Boolean=false) {
    var committed:TerminalGrid?=null
        private set
    private var stage:ArrayList<GridCell>?=null
    private var begin:GridBegin?=null
    private var sequence=0L
    private var revision=0L
    private var index=0L
    private var wireBytes=0
    private var wide=false
    private var blinking=false
    private var deadline=0L
    val transactionDeadline:Long get()=deadline
    fun clear(){committed=null;stage=null;begin=null;wireBytes=0;wide=false;blinking=false;deadline=0}
    fun expired(now:Long=System.nanoTime())=stage!=null && now>=deadline
    fun open(frame:GridBegin,bytes:Int){requireWire(frame.revision.value==0L);sequence=0;revision=0;start(frame,bytes)}
    private fun start(frame:GridBegin,bytes:Int){
        frame.validate();requireWire(stage==null && frame.revision.value==revision)
        requireWire(frame.scroll==null || history&&!ansiObserver&&!frame.alternateScreen&&!frame.popupBlocked)
        if(ansiObserver)requireWire(frame.columns.value*frame.rows.value<=8192 && frame.graphicsOmitted && !frame.alternateScreen && !frame.popupBlocked)
        blinking=false
        // ponytail: negotiated Agent has <=8192 cells/260 bytes; Shell retains <=32768/128.
        begin=frame;stage=ArrayList((frame.columns.value*frame.rows.value).toInt());index=0;wireBytes=bytes;wide=false
        deadline=System.nanoTime()+15_000_000_000L;requireWire(wireBytes<=8*1024*1024)
    }
    /** Returns true only for a new complete commit; duplicates never refresh a deadline. */
    fun event(event:Event,bytes:Int):Boolean {
        if(event.seq.value<=sequence)return false
        requireWire(sequence<Protocol.UINT_MAX && event.seq.value==sequence+1 && !expired())
        when(val body=event.body){
            is EventBody.GridBegin->{requireWire(stage==null && revision<Protocol.UINT_MAX);revision++;start(body.data.frame,bytes)}
            is EventBody.GridPart->{
                val frame=begin ?: invalidWire();val cells=stage ?: invalidWire();val part=body.data;part.validate()
                requireWire(part.revision.value==revision && part.index.value==index && index<frame.partCount.value)
                wireBytes+=bytes;requireWire(wireBytes<=8*1024*1024)
                val columns=frame.columns.value.toInt();val total=columns*frame.rows.value.toInt()
                for(span in part.spans){
                    requireWire(span.row.value==cells.size.toLong()/columns && span.column.value==cells.size.toLong()%columns && span.column.value+span.cells.size<=columns)
                    for(cell in span.cells){
                        requireWire(utf8Size(cell.text)<=if(ansiObserver)260 else 128)
                        requireWire(cells.size<total && (cell.width.value==0L)==wide)
                        cells.add(cell);wide=cell.width.value==2L
                        blinking=blinking || cell.width.value!=0L && cell.attributes.blink && !cell.attributes.concealed
                        requireWire(!wide || cells.size%columns!=0)
                    }
                }
                index++
            }
            is EventBody.GridCommit->{
                val frame=begin ?: invalidWire();val cells=stage ?: invalidWire();wireBytes+=bytes
                requireWire(body.data.revision.value==revision && index==frame.partCount.value && cells.size.toLong()==frame.columns.value*frame.rows.value && !wide && wireBytes<=8*1024*1024)
                committed=TerminalGrid(frame,cells,blinking);stage=null;begin=null;deadline=0;sequence=event.seq.value;return true
            }
            else->invalidWire()
        }
        sequence=event.seq.value;return false
    }
}

/** The original viewer job invalidates a queued main delivery immediately on dismissal/background. */
internal class TerminalGridLease(private val original:Job) {
    private val open=java.util.concurrent.atomic.AtomicBoolean(true)
    private var image:TerminalGrid?=null
    val stopped=CompletableDeferred<Unit>()
    fun active()=open.get() && original.isActive
    fun close():Boolean {
        val changed=synchronized(this){
            if(!open.compareAndSet(true,false))false else {image=null;true}
        }
        if(changed)stopped.complete(Unit)
        return changed
    }
    fun stage(frame:TerminalGrid){synchronized(this){if(active())image=frame;Unit}}
    fun pause(){synchronized(this){image=null}}
    suspend fun clearPaused(display:(TerminalGrid?,TerminalGridLease)->Unit)=withContext(Dispatchers.Main.immediate){
        synchronized(this@TerminalGridLease){if(active()&&image==null)display(null,this@TerminalGridLease);Unit}
    }
    suspend fun deliver(display:(TerminalGrid?,TerminalGridLease)->Unit):Unit=withContext(Dispatchers.Main.immediate) {
        // The queued continuation captures only the lease. Close clears its pending handoff reference.
        synchronized(this@TerminalGridLease){if(active())image?.let {display(it,this@TerminalGridLease)};Unit}
    }
    suspend fun clear(display:(TerminalGrid?,TerminalGridLease)->Unit)=withContext(Dispatchers.Main.immediate) {
        display(null,this@TerminalGridLease)
    }
}
internal fun publishTerminalGrid(state:MutableState<TerminalSnapshot?>,key:ConversationKey,source:Uuid,ticket:Long,
    foreground:Boolean,selection:ConversationKey?,screen:Screen,sourceCurrent:Boolean,lease:TerminalGridLease,frame:TerminalGrid?) {
    val shown=state.value ?: return
    if(shown.ticket!=ticket || shown.key!=key || shown.source!=source)return
    if(frame!=null && (!lease.active() || !foreground || selection!=key || screen!=Screen.Conversation || !sourceCurrent))return
    state.value=shown.copy(grid=frame,loading=false,error=frame==null)
}
internal fun publishShellGrid(state:MutableState<ShellSnapshot?>,selection:ShellSelection,source:Uuid,ticket:Long,
    admitted:Boolean,lease:TerminalGridLease,frame:TerminalGrid?) {
    val shown=state.value ?: return
    if(shown.ticket!=ticket || shown.selection!=selection || shown.source!=source)return
    if(frame!=null && (!lease.active() || !admitted || !shown.controlBegun || shown.error))return
    val failed=shown.error || frame==null && shown.controlBegun && !lease.active()
    state.value=shown.copy(grid=frame,loading=frame==null && shown.controlBegun && lease.active() && !failed,error=failed)
    shown.history.committed(frame,lease)
}
enum class ShellHistoryMode {Live,Browsing,Pending,Uncertain}
/** Current committed display plus one explicit absolute intent, scoped by its viewer owner. */
class ShellHistoryState {
    var mode by mutableStateOf(ShellHistoryMode.Live);private set
    var metrics by mutableStateOf<GridScroll?>(null);private set
    var action by mutableStateOf<Uuid?>(null);private set
    private var requested=0L
    private var prior=ShellHistoryMode.Live
    private var bottomAck=false
    private var settled=false
    private var owner:TerminalGridLease?=null
    internal fun active()=owner?.active()==true
    internal fun committed(frame:TerminalGrid?,lease:TerminalGridLease) {
        if(owner==null)owner=lease
        if(owner!==lease)return
        if(!lease.active()){metrics=null;return}
        metrics=frame?.begin?.takeUnless {it.alternateScreen||it.popupBlocked}?.scroll
        if(metrics?.offsetFromBottom?.value?.let {it>0}==true&&mode==ShellHistoryMode.Live){mode=ShellHistoryMode.Browsing;bottomAck=false}
        resume()
    }
    fun begin(id:Uuid,offset:Long) {
        requireWire(active()&&mode!=ShellHistoryMode.Pending&&metrics!=null)
        prior=if(mode==ShellHistoryMode.Live&&metrics?.offsetFromBottom?.value==0L)ShellHistoryMode.Live else if(mode==ShellHistoryMode.Uncertain)ShellHistoryMode.Uncertain else ShellHistoryMode.Browsing
        action=id;requested=offset;mode=ShellHistoryMode.Pending;bottomAck=false;settled=false
    }
    fun receipt(receipt:Receipt) {
        if(receipt.actionId!=action||receipt.method!=ActionMethod.ShellScroll||settled)return
        when(receipt.state) {
            ReceiptState.Succeeded->{val actual=receipt.result as? ActionResultShellScroll ?: return;settled=true;mode=ShellHistoryMode.Browsing;bottomAck=requested==0L&&actual.offsetFromBottom.value==0L;resume()}
            ReceiptState.Failed->{settled=true;refused()}
            ReceiptState.Uncertain->{mode=ShellHistoryMode.Uncertain;bottomAck=false}
            ReceiptState.Reserved,ReceiptState.Dispatching,ReceiptState.Dispatched->Unit
        }
    }
    fun uncertain(){mode=ShellHistoryMode.Uncertain;bottomAck=false}
    fun refused(){mode=if(prior==ShellHistoryMode.Live&&(!active()||metrics?.offsetFromBottom?.value!=0L))ShellHistoryMode.Browsing else prior;bottomAck=false}
    private fun resume(){if(active()&&bottomAck&&metrics?.offsetFromBottom?.value==0L){mode=ShellHistoryMode.Live;bottomAck=false}}
}
/** Shared by the real renderer and its geometry regression; no per-draw range/list allocation. */
internal class TerminalGridViewport(val columns:Int,val rows:Int,val panX:Float,val panY:Float,
    val cellWidth:Float,val cellHeight:Float,val width:Float,val height:Float) {
    val firstColumn=if(cellWidth>0)kotlin.math.floor((-panX/cellWidth).toDouble()).toInt().coerceIn(0,columns) else 0
    val endColumn=if(cellWidth>0 && width>0)kotlin.math.ceil(((width-panX)/cellWidth).toDouble()).toInt().coerceIn(0,columns) else 0
    val firstRow=if(cellHeight>0)kotlin.math.floor((-panY/cellHeight).toDouble()).toInt().coerceIn(0,rows) else 0
    val endRow=if(cellHeight>0 && height>0)kotlin.math.ceil(((height-panY)/cellHeight).toDouble()).toInt().coerceIn(0,rows) else 0
    inline fun backgrounds(grid:TerminalGrid,draw:(GridCell,Float,Float)->Unit) {
        var row=firstRow
        while(row<endRow){var col=firstColumn;while(col<endColumn){
            draw(grid.cells[row*columns+col],panX+col*cellWidth,panY+row*cellHeight);col++
        };row++}
    }
    inline fun glyphs(grid:TerminalGrid,draw:(GridCell,Float,Float,Float)->Unit) {
        var row=firstRow
        while(row<endRow){var col=(firstColumn-1).coerceAtLeast(0);while(col<endColumn){
            val cell=grid.cells[row*columns+col];val x=panX+col*cellWidth
            val right=x+cellWidth*cell.width.value
            if(cell.width.value!=0L && !cell.attributes.concealed && right>0 && x<width)draw(cell,x,panY+row*cellHeight,right)
            col++
        };row++}
    }
    fun cursorVisible(cursor:GridCursor):Boolean {
        val x=panX+cursor.column.value*cellWidth;val y=panY+cursor.row.value*cellHeight
        return width>0 && height>0 && cursor.visible && x+cellWidth>0 && x<width && y+cellHeight>0 && y<height
    }
    fun needsBlink(grid:TerminalGrid):Boolean {
        grid.begin.cursor?.let {if(cursorVisible(it) && it.shape.value!=2L && it.shape.value!=4L && it.shape.value!=6L)return true}
        if(grid.hasBlinkingCells)glyphs(grid){cell,_,_,_->if(cell.attributes.blink)return true}
        return false
    }
}
private class TerminalGridMetrics(val width:Float,val height:Float,val baseline:Float,val textSize:Float)
@Composable
private fun rememberTerminalMotion():Boolean {
    val resolver=androidx.compose.ui.platform.LocalContext.current.contentResolver
    fun enabled():Boolean=try {android.provider.Settings.Global.getFloat(resolver,android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,1f)>0f}catch(_:SecurityException){false}
    var motion by remember(resolver){mutableStateOf(enabled())}
    DisposableEffect(resolver) {
        val observer=object:android.database.ContentObserver(android.os.Handler(android.os.Looper.getMainLooper())) {
            override fun onChange(selfChange:Boolean){motion=enabled()}
        }
        val registered=try {resolver.registerContentObserver(android.provider.Settings.Global.getUriFor(android.provider.Settings.Global.ANIMATOR_DURATION_SCALE),false,observer);true}catch(_:SecurityException){motion=false;false}
        onDispose {if(registered)resolver.unregisterContentObserver(observer)}
    }
    return motion
}

@Composable
internal fun TerminalGridCanvas(grid:TerminalGrid,appearance:TerminalAppearance,modifier:Modifier=Modifier,displayScale:TerminalDisplayScaleState?=null,motionEnabled:Boolean=rememberTerminalMotion()){
    val dark=when(appearance){TerminalAppearance.Light->false;TerminalAppearance.Dark->true;TerminalAppearance.FollowApp->MaterialTheme.colorScheme.surface.toArgb().let {android.graphics.Color.red(it)+android.graphics.Color.green(it)+android.graphics.Color.blue(it)<384}}
    val defaultBackground=if(dark)Color(0xff111111)else Color(0xfff7f7f2)
    val defaultForeground=if(dark)Color(0xffe3e5de)else Color(0xff1b1c19)
    val paint=remember {Paint(Paint.ANTI_ALIAS_FLAG)}
    val underlinePath=remember {android.graphics.Path()}
    val faces=remember {arrayOf(Typeface.create(Typeface.MONOSPACE,Typeface.NORMAL),Typeface.create(Typeface.MONOSPACE,Typeface.BOLD),Typeface.create(Typeface.MONOSPACE,Typeface.ITALIC),Typeface.create(Typeface.MONOSPACE,Typeface.BOLD_ITALIC))}
    var zoom by remember(displayScale) {mutableFloatStateOf(displayScale?.value ?: TERMINAL_DISPLAY_SCALE_DEFAULT)}
    var pan by remember {mutableStateOf(Offset.Zero)}
    var canvasSize by remember {mutableStateOf(IntSize.Zero)}
    val density=LocalDensity.current.density
    val baseTextPixels=with(LocalDensity.current){14.sp.toPx()}
    LaunchedEffect(displayScale?.value) {
        displayScale?.value?.let {selected->if(zoom!=selected)zoom=selected}
    }
    LaunchedEffect(zoom,displayScale) {
        if(displayScale!=null&&zoom==displayScale.value) {
            delay(300)
            displayScale.persist()
        }
    }
    DisposableEffect(displayScale) {
        onDispose {displayScale?.persist()}
    }
    LaunchedEffect(displayScale?.resetEpoch) {
        if(displayScale!=null) {
            zoom=displayScale.value
            pan=Offset.Zero
        }
    }
    fun updateZoom(next:Float) {
        zoom=next.coerceIn(TERMINAL_DISPLAY_SCALE_MIN,TERMINAL_DISPLAY_SCALE_MAX)
        displayScale?.set(zoom)
    }
    val localActions=listOf(
        CustomAccessibilityAction("Zoom in locally"){updateZoom(zoom*1.25f);true},
        CustomAccessibilityAction("Zoom out locally"){updateZoom(zoom/1.25f);true},
        CustomAccessibilityAction("Move view left"){pan+=Offset((canvasSize.width*.75f).coerceAtLeast(24*density),0f);true},
        CustomAccessibilityAction("Move view right"){pan-=Offset((canvasSize.width*.75f).coerceAtLeast(24*density),0f);true},
        CustomAccessibilityAction("Move view up"){pan+=Offset(0f,(canvasSize.height*.75f).coerceAtLeast(24*density));true},
        CustomAccessibilityAction("Move view down"){pan-=Offset(0f,(canvasSize.height*.75f).coerceAtLeast(24*density));true},
        CustomAccessibilityAction("Reset local view"){if(displayScale==null)zoom=TERMINAL_DISPLAY_SCALE_DEFAULT else displayScale.reset();pan=Offset.Zero;true}
    )

    val metrics=remember(zoom,baseTextPixels){
        paint.textSize=baseTextPixels*zoom;paint.typeface=faces[0]
        val font=paint.fontMetrics
        TerminalGridMetrics(paint.measureText("M"),font.descent-font.ascent,-font.ascent,paint.textSize)
    }
    val viewport=remember(grid,pan,metrics,canvasSize){TerminalGridViewport(grid.begin.columns.value.toInt(),grid.begin.rows.value.toInt(),
        pan.x,pan.y,metrics.width,metrics.height,canvasSize.width.toFloat(),canvasSize.height.toFloat())}
    val needsBlink=remember(grid,viewport){viewport.needsBlink(grid)}
    var blink by remember {mutableStateOf(true)}
    LaunchedEffect(needsBlink,motionEnabled){blink=true;if(needsBlink&&motionEnabled)while(true){delay(500);blink=!blink}}
    val visibleBlink=!motionEnabled||blink
    Column(modifier){
        if(grid.begin.alternateScreen)UiText("Alternate screen")
        if(grid.begin.graphicsOmitted)UiText("Terminal graphics omitted")
        if(grid.begin.popupBlocked)UiText("Desktop attention required · a native popup pauses terminal input")
        Canvas(Modifier.fillMaxSize().testTag("terminal_grid").semantics {contentDescription="Local terminal view. Pan and zoom do not change native geometry.";stateDescription="${(zoom*100).toInt()} percent zoom, ${if(pan==Offset.Zero)"view origin" else "local pan active"}";customActions=localActions}.onSizeChanged {canvasSize=it}.pointerInput(Unit){detectTransformGestures {_,delta,scale,_->updateZoom(zoom*scale);pan+=delta}}){
            drawRect(defaultBackground)
            paint.textSize=metrics.textSize
            val width=metrics.width;val height=metrics.height
            drawIntoCanvas {canvas->
                val native=canvas.nativeCanvas
                native.save();native.clipRect(0f,0f,size.width,size.height)
                viewport.backgrounds(grid){cell,x,y->
                    paint.color=resolvedGridBackground(cell,defaultForeground,defaultBackground).toArgb();paint.alpha=255
                    native.drawRect(x,y,x+width,y+height,paint)
                }
                viewport.glyphs(grid){cell,x,y,right->
                    val attrs=cell.attributes
                    if(!attrs.concealed && (!attrs.blink || visibleBlink)){
                        paint.color=resolvedGridForeground(cell,defaultForeground,defaultBackground).toArgb();paint.alpha=if(attrs.faint)128 else 255
                        paint.typeface=faces[(if(attrs.bold)1 else 0)+(if(attrs.italic)2 else 0)]
                        native.save();native.clipRect(x,y,right,y+height)
                        native.drawText(cell.text,x,y+metrics.baseline,paint)
                        paint.strokeWidth=(zoom*1.dp.toPx()).coerceAtLeast(1f)
                        if(attrs.strike)native.drawLine(x,y+height*.5f,right,y+height*.5f,paint)
                        paint.color=(attrs.underlineColor?.let {gridColor(it,defaultForeground)} ?: resolvedGridForeground(cell,defaultForeground,defaultBackground)).toArgb()
                        val underline=y+height-2*zoom
                        when(attrs.underline){
                            GridUnderline.None->Unit
                            GridUnderline.Single->native.drawLine(x,underline,right,underline,paint)
                            GridUnderline.Double->{native.drawLine(x,underline,right,underline,paint);native.drawLine(x,underline-3*zoom,right,underline-3*zoom,paint)}
                            GridUnderline.Curly->{
                                underlinePath.reset();underlinePath.moveTo(x,underline)
                                var at=x;var up=true
                                while(at<right){val end=(at+4*zoom).coerceAtMost(right);underlinePath.quadTo((at+end)/2,underline+(if(up)-2 else 2)*zoom,end,underline);at=end;up=!up}
                                paint.style=Paint.Style.STROKE;native.drawPath(underlinePath,paint);paint.style=Paint.Style.FILL
                            }
                            GridUnderline.Dotted,GridUnderline.Dashed->{
                                val step=if(attrs.underline==GridUnderline.Dotted)2*zoom else 4*zoom
                                var at=x
                                while(at<right){val end=(at+step*.6f).coerceAtMost(right);native.drawLine(at,underline,end,underline,paint);at+=step}
                            }
                        }
                        native.restore()
                    }
                }
                grid.begin.cursor?.takeIf {viewport.cursorVisible(it) && (it.shape.value==2L || it.shape.value==4L || it.shape.value==6L || visibleBlink)}?.let {cursor->
                    val x=pan.x+cursor.column.value*width;val y=pan.y+cursor.row.value*height
                    paint.color=defaultForeground.toArgb();paint.alpha=128
                    when(cursor.shape.value){3L,4L->native.drawRect(x,y+height-2*zoom,x+width,y+height,paint);5L,6L->native.drawRect(x,y,x+2*zoom,y+height,paint);else->native.drawRect(x,y,x+width,y+height,paint)}
                }
                native.restore()
            }
        }
    }
}
/** Resolve each color's original role before applying raw Ratatui inverse exactly once. */
internal fun resolvedGridForeground(cell:GridCell,foreground:Color,background:Color):Color =
    if(cell.attributes.inverse)gridColor(cell.background,background)else gridColor(cell.foreground,foreground)
internal fun resolvedGridBackground(cell:GridCell,foreground:Color,background:Color):Color =
    if(cell.attributes.inverse)gridColor(cell.foreground,foreground)else gridColor(cell.background,background)
private fun gridColor(color:GridColor,default:Color):Color=when(color){
    GridColor.Default->default
    is GridColor.Rgb->Color(0xff000000L or color.value.value)
    is GridColor.Indexed->{
        val n=color.index.value.toInt()
        val rgb=when {
            n<16->gridPalette[n]
            n<232->{
                val value=n-16
                fun level(v:Int):Int {
                    return if(v==0)0 else 55+40*v
                }
                (level(value/36)shl 16)or(level(value/6%6)shl 8)or level(value%6)
            }
            else->{val v=8+10*(n-232);(v shl 16)or(v shl 8)or v}
        };Color(0xff000000L or rgb.toLong())
    }
}
private val gridPalette=intArrayOf(0x000000,0x800000,0x008000,0x808000,0x000080,0x800080,0x008080,0xc0c0c0,0x808080,0xff0000,0x00ff00,0xffff00,0x0000ff,0xff00ff,0x00ffff,0xffffff)
