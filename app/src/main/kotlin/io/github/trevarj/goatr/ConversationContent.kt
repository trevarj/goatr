// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Activity
import android.util.TypedValue
import android.graphics.Typeface
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.text.Layout
import android.widget.TextView
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Text as UiText
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.TextButton
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.toArgb
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.font.FontFamily
import io.noties.markwon.AbstractMarkwonPlugin
import io.noties.markwon.Markwon
import io.noties.markwon.MarkwonConfiguration
import io.noties.markwon.core.MarkwonTheme
import io.noties.markwon.image.AsyncDrawableLoader
import org.commonmark.node.*
import java.net.URI
import kotlinx.coroutines.CancellationException

/** Native toolbar, accessibility and keyboard Copy all use the exact backing source. */
private class MarkdownTextView(context: Context):TextView(context) {
    var backing: String=""
    var renderer: Markwon?=null
    init {
        isSaveEnabled=false;isSaveFromParentEnabled=false;setTextIsSelectable(true);setPadding(0,0,0,0)
        customSelectionActionModeCallback=object: android.view.ActionMode.Callback {
            override fun onCreateActionMode(mode: android.view.ActionMode,menu: android.view.Menu):Boolean {
                menu.findItem(android.R.id.copy)?.title="Copy original backing text";return true
            }
            override fun onPrepareActionMode(mode: android.view.ActionMode,menu: android.view.Menu):Boolean {
                menu.findItem(android.R.id.copy)?.title="Copy original backing text";return false
            }
            override fun onActionItemClicked(mode: android.view.ActionMode,item: android.view.MenuItem):Boolean=false
            override fun onDestroyActionMode(mode: android.view.ActionMode) {}
        }
    }
    override fun onTextContextMenuItem(id: Int):Boolean {
        if(id!=android.R.id.copy) return super.onTextContextMenuItem(id)
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Goatr original backing text",backing))
        return true
    }
}

/** Browser navigation is deliberately narrower than native form URI validation. */
fun safeHttpUrl(raw: String): Uri? {
    if (!validUnicode(raw) || raw.any {it.isWhitespace() || Character.isISOControl(it)} || !validUri(raw)) return null
    val parsed = try {URI(raw)} catch (_: Exception) {return null}
    if ((!parsed.scheme.equals("https",true) && !parsed.scheme.equals("http",true)) || parsed.host.isNullOrEmpty() || parsed.rawUserInfo!=null || parsed.port !in -1..65535) return null
    return Uri.parse(parsed.toASCIIString())
}
fun openHttpUrl(context: Context,raw: String): Boolean {
    val uri = safeHttpUrl(raw) ?: return false
    return try {val intent=Intent(Intent.ACTION_VIEW,uri);if(context !is Activity) intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);context.startActivity(intent);true} catch (_: Exception) {false}
}

@Composable
fun MarkdownText(backing: String,onLink: (String)->Unit,modifier: Modifier=Modifier) {
    val context=LocalContext.current
    val link by rememberUpdatedState(onLink)
    val density=LocalDensity.current
    val foreground=MaterialTheme.colorScheme.onSurface.toArgb()
    val linkColor=MaterialTheme.colorScheme.primary.toArgb()
    val codeBackground=MaterialTheme.colorScheme.surfaceVariant.toArgb()
    val codeMargin=(8f*density.density).toInt()
    val codeSize=(13f*density.density*density.fontScale).toInt()
    val markwon=remember(context,foreground,codeBackground,codeMargin,codeSize) {
        Markwon.builder(context).usePlugin(object: AbstractMarkwonPlugin() {
            override fun configureTheme(builder: MarkwonTheme.Builder) {
                builder.codeTextColor(foreground).codeBlockTextColor(foreground)
                    .codeBackgroundColor(codeBackground).codeBlockBackgroundColor(codeBackground)
                    .codeTypeface(Typeface.MONOSPACE).codeBlockTypeface(Typeface.MONOSPACE)
                    .codeTextSize(codeSize).codeBlockTextSize(codeSize).codeBlockMargin(codeMargin)
            }
            override fun configureConfiguration(builder: MarkwonConfiguration.Builder) {
                builder.asyncDrawableLoader(AsyncDrawableLoader.noOp()).linkResolver {_,url->if(safeHttpUrl(url)!=null) link(url)}
            }
            override fun beforeRender(node: Node) {
                // Core only: preserve image alt text, but never render HTML or fetch images.
                fun remove(parent: Node) {
                    var child=parent.firstChild
                    while(child!=null) {
                        val next=child.next
                        when(child) {
                            is HtmlBlock -> {child.insertBefore(org.commonmark.node.Text("[HTML omitted; exact backing is available]"));child.unlink()}
                            is HtmlInline -> child.unlink()
                            is Image -> {
                                remove(child)
                                while(child.firstChild!=null) {val alt=child.firstChild;alt.unlink();child.insertBefore(alt)}
                                child.insertBefore(org.commonmark.node.Text(" (image not loaded)"));child.unlink()
                            }
                            else -> remove(child)
                        }
                        child=next
                    }
                }
                remove(node)
            }
        }).build()
    }
    AndroidView(factory={MarkdownTextView(it)},modifier=modifier.fillMaxWidth().testTag("markdown_native"),update={view->
        view.setTextSize(TypedValue.COMPLEX_UNIT_PX,13f*density.density*density.fontScale)
        // Wrap long paths and code in the message width instead of widening the transcript.
        view.setHorizontallyScrolling(false)
        view.breakStrategy=Layout.BREAK_STRATEGY_HIGH_QUALITY
        view.hyphenationFrequency=Layout.HYPHENATION_FREQUENCY_NONE
        view.setLineSpacing(2f*density.density*density.fontScale,1f)
        view.setTextColor(foreground);view.setLinkTextColor(linkColor)
        if(view.backing!=backing || view.renderer!==markwon) {view.backing=backing;view.renderer=markwon;markwon.setMarkdown(view,backing)}
    },onRelease={it.text="";it.backing="";it.renderer=null})
}

@Composable
fun ContentView(
    content: Content,key: ConversationKey?,source: Uuid?,model: GoatrViewModel?,
    completed: Boolean=false,markdown: Boolean=false,onLoaded: ((String)->Unit)?=null,
) {
    var expanded by remember(content,key,source) {mutableStateOf(false)}
    var loaded by remember(content,key,source) {mutableStateOf<String?>(null)}
    var error by remember(content,key,source) {mutableStateOf<String?>(null)}
    val context=LocalContext.current
    val callback by rememberUpdatedState(onLoaded)
    val text=(content as? ContentText)?.text?.value ?: loaded
    LaunchedEffect(content,key,source,expanded) {
        if(content is ContentRef && expanded && model!=null && key!=null) {
            try {loaded=model.loadContent(key,source,content,completed)}
            catch(cancel: CancellationException) {throw cancel}
            catch(_: Exception) {error="Content expired or unavailable. Refresh the current item; never reuse an old source handle."}
        }
    }
    LaunchedEffect(text) {text?.let {callback?.invoke(it)}}
    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        when {
            text!=null -> {
                if(markdown) MarkdownText(text,{url->model?.forms?.values?.forEach(FormMemory::clearSecrets);openHttpUrl(context,url)})
                else SelectionContainer {UiText(text,style=MaterialTheme.typography.bodyLarge,modifier=Modifier.fillMaxWidth())}
                if(completed) TextButton(onClick={context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Goatr backing text",text))},modifier=Modifier.heightIn(min=48.dp).testTag("content_copy")) {IconLabel(MaterialGlyph.Copy,"Copy original")}
            }
            content is ContentUnavailable -> UiText("Content unavailable: ${content.reason}${content.byteLength?.let {" (${it.value} bytes)"}.orEmpty()}",style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
            content is ContentRef -> {
                UiText("${content.byteLength.value} bytes · ${content.mime}${if(source==null) " · stale cache only" else ""}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                Action(if(expanded) "Loading content" else "Expand content","content_expand",enabled=!expanded && model!=null) { expanded=true }
            }
        }
        error?.let {message->UiText(message);if(source!=null && model!=null) Action("Refresh authoritative item","content_refresh",enabled=!model.busy) { model.refresh() }}
    }
    DisposableEffect(content,key,source) {onDispose {loaded=null}}
}

@Composable
fun HistoryCard(record: HistoryRecord,key: ConversationKey,source: Uuid?,model: GoatrViewModel) {
    var toolExpanded by remember(record.id,key,source) {mutableStateOf(false)}
    var originalExpanded by remember(record.id,key,source) {mutableStateOf(false)}
    var details by remember(record.id,key,source) {mutableStateOf(false)}
    val user=record.role==Role.User
    Box(Modifier.fillMaxWidth(),contentAlignment=if(user) Alignment.CenterEnd else Alignment.CenterStart) {
    Surface(color=if(user) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,contentColor=if(user) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,shape=MaterialTheme.shapes.large,border=if(user) null else BorderStroke(1.dp,MaterialTheme.colorScheme.outlineVariant),modifier=Modifier.widthIn(max=if(user) 560.dp else 720.dp).fillMaxWidth(if(user) .88f else 1f).testTag(if(user) "history_user_message" else "history_native_message")) {
    Column(Modifier.padding(if(user) 12.dp else 10.dp),verticalArrangement=Arrangement.spacedBy(6.dp)) {
        Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=Alignment.CenterVertically) {
            UiText(if(user) "You" else record.role.name,style=MaterialTheme.typography.labelLarge,color=if(user) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,modifier=Modifier.weight(1f))
            DetailDisclosure(details,{
                details=!details
                if(!details) originalExpanded=false
            },"history_details_${record.id.value}")
        }
        if(!record.complete) UiText("In progress",style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.tertiary)
        if(record.sourceTruncated) UiText("Native source truncated; this is not full content.",style=MaterialTheme.typography.bodySmall)
        if(record.kind==RecordKind.Unsupported) UiText("Unsupported native history entry; safe original display is available when supplied.",style=MaterialTheme.typography.bodySmall)
        record.text?.let {ContentView(it,key,source,model,record.complete,record.role==Role.Assistant)}
        record.tool?.let {tool->
            GroupedSurface {
                // Native tool names can be paths; give name and status independent wrapping lines.
                UiText("Tool · ${tool.name}",style=MaterialTheme.typography.titleSmall,modifier=Modifier.fillMaxWidth())
                UiText(tool.status.toString(),style=MaterialTheme.typography.labelMedium,color=if(tool.status==ToolStatus.Failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant)
                TextButton(onClick={toolExpanded=!toolExpanded},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag("tool_expand")) {
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=Alignment.CenterVertically) {
                        GoatrIcon(MaterialGlyph.ExpandMore,null)
                        UiText(if(toolExpanded) "Collapse input and output" else "View input and output",modifier=Modifier.weight(1f))
                    }
                }
                if(toolExpanded) {GoatrSection("Input");tool.input?.let {ContentView(it,key,source,model,record.complete)} ?: UiText("Input unavailable");GoatrSection("Output");tool.output?.let {ContentView(it,key,source,model,record.complete)} ?: UiText("Output unavailable")}
            }
        }
        if(details) {
            SelectionContainer {UiText("Record ${record.id.value}${record.parentId?.let {"\nParent ${it.value}"}.orEmpty()}${record.turnId?.let {"\nTurn ${it.value}"}.orEmpty()}",style=MaterialTheme.typography.bodySmall,fontFamily=FontFamily.Monospace,color=MaterialTheme.colorScheme.onSurfaceVariant,modifier=Modifier.fillMaxWidth())}
            record.original?.let {original->
                TextButton(onClick={originalExpanded=!originalExpanded},modifier=Modifier.fillMaxWidth().heightIn(min=48.dp).testTag("original_expand")) {
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp),verticalAlignment=Alignment.CenterVertically) {
                        GoatrIcon(MaterialGlyph.ExpandMore,null)
                        UiText(if(originalExpanded) "Hide original backing" else "Show original backing",modifier=Modifier.weight(1f))
                    }
                }
                if(originalExpanded) ContentView(original,key,source,model,record.complete)
            }
        }
    }
    }
    }
}
