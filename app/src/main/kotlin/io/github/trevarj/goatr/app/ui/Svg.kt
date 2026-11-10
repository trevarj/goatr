// Every icon is an SVG in assets: the desktop's program icons in icons/ and
// Lucide's glyphs (ISC) in lucide/. Each is parsed once, in the background at
// start, and drawn straight onto the canvas: no bitmaps, sharp at any size.

package io.github.trevarj.goatr.app.ui

import android.content.Context
import android.content.res.AssetManager
import android.util.Log
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.painter.ColorPainter
import androidx.compose.ui.graphics.painter.Painter
import com.caverock.androidsvg.RenderOptions
import com.caverock.androidsvg.SVG
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap

object Svgs {
    private val parsed = ConcurrentHashMap<String, SVG>()
    private lateinit var assets: AssetManager

    /** Parses every icon off the main thread; one drawn before its turn is parsed then. */
    fun preload(context: Context, scope: CoroutineScope) {
        assets = context.assets
        scope.launch(Dispatchers.Default) {
            for (dir in listOf("lucide", "icons")) {
                for (file in assets.list(dir).orEmpty()) if (file.endsWith(".svg")) get("$dir/$file")
            }
        }
    }

    fun get(path: String): SVG? = parsed[path] ?: try {
        assets.open(path).use { SVG.getFromInputStream(it) }.also {
            // Fill whatever it's drawn in: the files' own sizes (24px, 1em) would win otherwise.
            it.setDocumentWidth("100%")
            it.setDocumentHeight("100%")
            parsed[path] = it
        }
    } catch (e: Exception) {
        Log.w("Herdr", "can't read $path", e)
        null
    }
}

// Scaled to whatever size it's drawn at. A color filter (an Icon's tint)
// recolors the whole drawing, as the desktop tints its one-color glyphs.
private class SvgPainter(private val svg: SVG) : Painter() {
    private var filter: ColorFilter? = null

    override val intrinsicSize = Size.Unspecified

    override fun applyColorFilter(colorFilter: ColorFilter?): Boolean {
        filter = colorFilter
        return true
    }

    override fun DrawScope.onDraw() = drawIntoCanvas { canvas ->
        val tint = filter?.let { Paint().apply { colorFilter = it } }
        if (tint != null) canvas.saveLayer(Rect(0f, 0f, size.width, size.height), tint)
        svg.renderToCanvas(canvas.nativeCanvas, RenderOptions().viewPort(0f, 0f, size.width, size.height))
        if (tint != null) canvas.restore()
    }
}

private val none = ColorPainter(Color.Transparent)

/** The SVG at `path` in assets (`lucide/x.svg`), nothing when it can't be read. */
@Composable
fun svgPainter(path: String): Painter = remember(path) { Svgs.get(path)?.let(::SvgPainter) ?: none }

/** The Lucide glyphs the app uses, drawn in the content color by `Icon`. */
object Lucide {
    val Bot @Composable get() = svgPainter("lucide/bot.svg")
    val Layers @Composable get() = svgPainter("lucide/layers.svg")
    val SquareTerminal @Composable get() = svgPainter("lucide/square-terminal.svg")
    val Settings @Composable get() = svgPainter("lucide/settings.svg")
    val X @Composable get() = svgPainter("lucide/x.svg")
    val SquarePen @Composable get() = svgPainter("lucide/square-pen.svg")
    val Pin @Composable get() = svgPainter("lucide/pin.svg")
    val PinOff @Composable get() = svgPainter("lucide/pin-off.svg")
    val GitBranch @Composable get() = svgPainter("lucide/git-branch.svg")
    val Plus @Composable get() = svgPainter("lucide/plus.svg")
    val Minus @Composable get() = svgPainter("lucide/minus.svg")
    val Copy @Composable get() = svgPainter("lucide/copy.svg")
    val Server @Composable get() = svgPainter("lucide/server.svg")
    val Trash2 @Composable get() = svgPainter("lucide/trash.svg")
    val Monitor @Composable get() = svgPainter("lucide/monitor.svg")
    val ChevronRight @Composable get() = svgPainter("lucide/chevron-right.svg")
    val ChevronDown @Composable get() = svgPainter("lucide/chevron-down.svg")
    val FileCode @Composable get() = svgPainter("lucide/file-code.svg")
    val KeyRound @Composable get() = svgPainter("lucide/key-round.svg")
    val Sliders @Composable get() = svgPainter("lucide/sliders-horizontal.svg")
    val Palette @Composable get() = svgPainter("lucide/palette.svg")
    val PanelLeft @Composable get() = svgPainter("lucide/panel-left.svg")
    val Check @Composable get() = svgPainter("lucide/check.svg")
    val Plug @Composable get() = svgPainter("lucide/plug.svg")
    val Eye @Composable get() = svgPainter("lucide/eye.svg")
    val RefreshCw @Composable get() = svgPainter("lucide/refresh-cw.svg")
    val ImagePlus @Composable get() = svgPainter("lucide/image-plus.svg")
    val Paperclip @Composable get() = svgPainter("lucide/paperclip.svg")
    val File @Composable get() = svgPainter("lucide/file.svg")
}
