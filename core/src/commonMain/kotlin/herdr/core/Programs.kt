// What runs in a terminal, as people see it: a name, an icon and a tile, from
// the desktop's own table (ProgramTable.kt) and by its rules (herdr-app
// src/programs.rs), so the phone shows what the desktop shows.

package herdr.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
internal data class ProgramEntry(
    val name: String? = null,
    /** `icons/<file>.svg`, as the desktop ships it. */
    val icon: String? = null,
    /** A one-color glyph, drawn in `color` (or the text color); else a picture. */
    val tint: Boolean? = null,
    val tile: String? = null,
    val color: String? = null,
    @SerialName("match") val also: List<String> = emptyList(),
    val run: List<String> = emptyList(),
)

@Serializable
internal data class ProgramTable(
    val shells: List<String> = emptyList(),
    val interpreters: List<String> = emptyList(),
    val programs: Map<String, ProgramEntry> = emptyMap(),
)

/** How a terminal shows: the table's key, name, icon (`icons/x.svg`) and tile. */
data class Program(
    val key: String,
    val name: String,
    val icon: String?,
    val tint: Boolean,
    val tile: String?,
    val color: String?,
    val shell: Boolean,
    val agent: Boolean,
)

internal val programTable: ProgramTable by lazy { lenient.decodeFromString(ProgramTable.serializer(), PROGRAMS_JSON) }

/** The table's entry for an agent key or name (`claude`, `claude-code`), for menus. */
fun programNamed(name: String): Program? = find(name)?.let { (k, e) -> shown(k, e, agent = true) }

private fun base(s: String): String {
    val last = s.substringAfterLast('/').substringAfterLast('\\').trimStart('-')
    return if (last.endsWith(".exe", ignoreCase = true)) last.dropLast(4) else last
}

private val PS_QUOTES = setOf('\'', '\u2018', '\u2019', '\u201A', '\u201B')

/** The words back from the desktop's `& 'prog' 'arg'` PowerShell line. */
private fun powershellWords(line: String): List<String>? {
    if (!line.startsWith("& ")) return null
    val chars = line.substring(2)
    val words = mutableListOf<String>()
    var i = 0
    while (i < chars.length) {
        val c = chars[i++]
        if (c == ' ') continue
        if (c !in PS_QUOTES) return null
        words += buildString {
            while (i < chars.length) {
                val ch = chars[i++]
                if (ch in PS_QUOTES) {
                    // A doubled quote is a quote.
                    if (i < chars.length && chars[i] in PS_QUOTES) {
                        append(chars[i++])
                        continue
                    }
                    break
                }
                append(ch)
            }
        }
    }
    return words
}

private fun find(name: String): Pair<String, ProgramEntry>? {
    val lower = name.lowercase()
    val programs = programTable.programs
    return programs.entries.firstOrNull { (k, _) -> k.lowercase() == lower }?.toPair()
        ?: programs.entries.firstOrNull { (_, e) -> e.also.any { it.lowercase() == lower } }?.toPair()
}

private fun entry(key: String) = programTable.programs[key] ?: ProgramEntry()

private val SCRIPT_EXTENSIONS = listOf(".js", ".mjs", ".cjs", ".ts", ".py", ".rb")

/** An agent's own report (`app`) wins; else the program it runs (`argv`). */
fun programOf(app: String?, argv: List<String>): Program {
    val t = programTable
    if (!app.isNullOrEmpty()) {
        return find(app)?.let { (k, e) -> shown(k, e, agent = true) } ?: shown("agent", entry("agent"), app, agent = true)
    }
    // The desktop starts commands in the person's shell: `zsh -ic '"$@"; exec "$0" -i' zsh <cmd>...`.
    if (argv.size > 4 && base(argv[0]) in t.shells && argv[1].startsWith('-') && argv[1].endsWith('c') &&
        ("\"\$@\"" in argv[2] || argv[2] == "command \$argv[2..]; exec \$argv[1] -i")
    ) {
        return programOf(null, argv.drop(4))
    }
    // On Windows: `pwsh -NoExit -Command "& 'codex' ..."`.
    if (argv.size == 4 && base(argv[0]) in t.shells && argv[1] == "-NoExit" && argv[2] == "-Command") {
        powershellWords(argv[3])?.let { return programOf(null, it) }
    }
    val first = argv.firstOrNull()?.let(::base) ?: return shown("shell", entry("shell"), shell = true)
    // `node .../vite.js dev`, `npx vite`, `uv run pytest`: the program they run, when it's known.
    if (first in t.interpreters) {
        for (arg in argv.drop(1).filter { !it.startsWith('-') }) {
            // A script's name without its extension (`vite.js`), never a module path (`http.server`).
            val name = base(arg)
            val stem = SCRIPT_EXTENSIONS.firstOrNull { name.endsWith(it) }?.let { name.removeSuffix(it) } ?: name
            find(stem)?.takeIf { (k, _) -> k !in t.interpreters }?.let { (k, e) -> return shown(k, e) }
        }
    }
    find(first)?.let { (k, e) -> return shown(k, e) }
    if (first in t.shells) return shown("shell", entry("shell"), first, shell = true)
    return shown("program", entry("program").copy(tile = hashColor(first), color = "#ffffff"), first)
}

private fun shown(key: String, e: ProgramEntry, name: String? = null, shell: Boolean = false, agent: Boolean = false) =
    Program(
        key = key,
        name = name ?: e.name ?: key,
        icon = e.icon,
        tint = e.tint ?: true,
        tile = e.tile,
        color = e.color,
        shell = shell,
        agent = agent,
    )

private val HASH_PALETTE = listOf("#6d5bd0", "#2f80ed", "#0f9d8a", "#d9822b", "#c2417d", "#3a7d44", "#8a5a44", "#4e5d94")

/** A color for a program nobody listed, the same every time (FNV-1a, as the desktop). */
private fun hashColor(s: String): String {
    var h = 2166136261u
    for (b in s.encodeToByteArray()) h = (h xor b.toUByte().toUInt()) * 16777619u
    return HASH_PALETTE[(h % HASH_PALETTE.size.toUInt()).toInt()]
}
