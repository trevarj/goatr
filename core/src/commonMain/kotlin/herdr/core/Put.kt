// Files for a terminal's program, as herdr-app pastes an image into a
// terminal on another machine (`terminal.paste_image`): `terminal.put`
// copies it beside the terminal, on its machine, and its path is pasted.

package herdr.core

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64

/** The most one `terminal.put` takes (herdr-core `put::MAX_BYTES`). */
const val PUT_LIMIT = 8 shl 20

/**
 * Writes `bytes` beside `terminal` (its core's address), on its machine,
 * under `name` (kept, so `.png` stays `.png`): where it landed, as that
 * machine sees it. Types nothing: paste [pasted] yourself.
 */
suspend fun put(client: Client, terminal: String, name: String, bytes: ByteArray): String {
    if (!client.has("terminal.put")) error("this core can't take files: update herdr-core")
    if (bytes.size > PUT_LIMIT) error("$name is over ${PUT_LIMIT shr 20} MB")
    val r = client.call("terminal.put", buildJsonObject {
        put("terminal", terminal)
        put("name", name)
        put("data", Base64.encode(bytes))
    }, timeoutMs = 120_000) as? JsonObject ?: error("terminal.put: no reply")
    return r.string("path") ?: error("terminal.put: no path")
}

/**
 * Paths as a terminal pastes dropped files: each escaped for a shell,
 * space-separated, a space after the last so typing goes on after them.
 * A Windows path (`C:\…`) is quoted instead, when it needs it.
 */
fun pasted(paths: List<String>): String = paths.joinToString(" ", postfix = " ") { p ->
    if (Regex("^[A-Za-z]:[\\\\/]").containsMatchIn(p)) {
        if (p.any { it == ' ' || it in "&()^;," }) "\"$p\"" else p
    } else {
        buildString { for (c in p) { if (c in SHELL_SPECIAL) append('\\'); append(c) } }
    }
}

private const val SHELL_SPECIAL = " \\\t\"'`\$&;|*?!#~<>()[]{}"
