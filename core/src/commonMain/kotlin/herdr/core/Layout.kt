// New, renamed, moved and closed spaces and tabs, written to the desktop's
// `herdr-app` layout document the way the desktop writes it (herdr-app/src/
// window.rs `new_space`, `new_tab`, `space.rename`, `space.move`, `space.close`,
// `tab.rename`, `tab.close`; mirror.rs `Document`), so both
// screens show the same thing. Writes carry the version they were read at;
// another screen writing first means read again and redo the one step.
//
// The document stays JSON: fields the phone doesn't know pass through.

package herdr.core

import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.random.Random

private const val DOC = "herdr-app"
private const val TRIES = 5

// What the desktop says its terminals are (hc.rs `Core::run`).
private val ENV = buildJsonObject {
    put("TERM", "xterm-256color")
    put("COLORTERM", "truecolor")
    put("TERM_PROGRAM", "herdr")
}

/**
 * What runs in a new terminal: the machine's shell (neither), a core
 * template by name, or a program (an agent) that leaves a shell when it exits.
 */
data class Run(val template: String? = null, val argv: List<String>? = null, val task: String? = null)

/** Ids qualified like the model's: navigate with them. */
data class Created(val address: String, val space: String, val tab: String)

/** A row of the "new" menu: `tab` is the name the desktop gives the tab. */
sealed interface Choice {
    val key: String
    val name: String
    val tab: String
    val run: Run

    data class Shell(override val name: String = "Terminal") : Choice {
        override val key = "shell"
        override val tab = ""
        override val run = Run()
    }

    data class Template(
        override val name: String,
        val description: String,
        val error: String?,
    ) : Choice {
        override val key = "template:$name"
        override val tab = name
        override val run = Run(template = name)
    }

    data class Agent(
        override val key: String,
        override val name: String,
        val installed: Boolean,
        override val run: Run,
    ) : Choice {
        override val tab = name
    }
}

data class AgentProgram(val key: String, val name: String, val run: List<String>)

// programs.rs `AGENTS_FIRST`.
private val AGENTS_FIRST = listOf("claude", "codex", "pi", "opencode", "cursor", "copilot", "amp", "gemini", "grok")

/**
 * The agents the desktop's new menu offers (herdr-app defaults/programs.json
 * entries with `run`, in programs.rs `AGENTS_FIRST` order, then by key).
 */
val AGENTS: List<AgentProgram> = programTable.programs
    .filterValues { it.run.isNotEmpty() }
    .map { (key, e) -> AgentProgram(key, e.name ?: key, e.run) }
    .sortedWith(compareBy({ AGENTS_FIRST.indexOf(it.key).takeIf { i -> i >= 0 } ?: AGENTS_FIRST.size }, { it.key }))

/** The id inside its machine's document: `beta/sp1` → `sp1` (mirror.rs `local`). */
fun local(id: String, machine: String?): String =
    if (!machine.isNullOrEmpty() && id.startsWith("$machine/")) id.substring(machine.length + 1) else id

/** `sp`, `tb`, `p` + 32 hex digits, like model.rs `fresh_id`. */
fun freshId(prefix: String): String =
    prefix + (1..4).joinToString("") { Random.nextInt().toUInt().toString(16).padStart(8, '0') }

private fun need(client: Client, vararg methods: String) {
    val missing = methods.filter { !client.has(it) }
    if (missing.isNotEmpty()) error("this core can't ${missing.joinToString(", ")}")
}

private fun on(machine: String?): JsonElement = machine?.let(::JsonPrimitive) ?: JsonNull

private fun JsonObject.with(key: String, value: JsonElement) = JsonObject(this + (key to value))

private fun <T> List<T>.replaced(i: Int, x: T) = mapIndexed { j, y -> if (j == i) x else y }

internal class LayoutRead(val doc: JsonObject, val version: Long)

/** A machine's layout document (always with `spaces`) and the version it was read at. */
internal suspend fun readLayout(client: Client, machine: String?): LayoutRead {
    val r = client.call("doc.get", buildJsonObject {
        put("name", DOC)
        put("on", on(machine))
    }) as? JsonObject ?: JsonObject(emptyMap())
    val value = r["value"] as? JsonObject ?: JsonObject(emptyMap())
    val doc = if (value["spaces"] is JsonArray) value else value.with("spaces", JsonArray(emptyList()))
    return LayoutRead(doc, r.long("version") ?: 0)
}

/**
 * Writes `change(doc)` at the version it was read at. Another screen wrote
 * first (`conflict`): read again and redo it. The desktop instead takes the
 * other screen's document (mirror_sync.rs `round`); a phone edit is one step,
 * so redoing it on the newer document loses nothing. No doc: nothing to write.
 */
private suspend fun <T> update(
    client: Client,
    machine: String?,
    first: LayoutRead? = null,
    change: (JsonObject) -> Pair<JsonObject?, T>,
): T {
    var at = first ?: readLayout(client, machine)
    var i = 1
    while (true) {
        val (doc, result) = change(at.doc)
        if (doc == null) return result
        try {
            client.call("doc.set", buildJsonObject {
                put("name", DOC)
                put("on", on(machine))
                put("version", at.version)
                put("value", doc)
            })
            return result
        } catch (e: RpcError) {
            if (e.kind != "conflict" || i >= TRIES) throw e
        }
        i++
        at = readLayout(client, machine)
    }
}

private class Where(val space: String, val tab: String)

/**
 * Where a terminal already is: the desktop adopts a labeled terminal it
 * sees without a pane (window.rs `reconcile`/`adopt`), maybe before we write.
 */
private fun placed(doc: JsonObject, terminal: String): Where? {
    for (s in doc.objects("spaces")) {
        for (t in s.objects("tabs")) {
            if (terminal in layoutTerminals(t["root"])) return Where(s.string("id") ?: "", t.string("id") ?: "")
        }
    }
    return null
}

/** A tab with one terminal pane (model.rs `Tab::new`): its root row has the tab's id. */
private fun tabOf(id: String, name: String, pane: String, terminal: String) = buildJsonObject {
    put("id", id)
    put("name", name)
    put("root", buildJsonObject {
        put("id", id)
        put("type", "row")
        put("children", buildJsonArray {
            add(buildJsonObject {
                put("id", pane)
                put("type", "block")
                put("content", buildJsonObject { put("terminal", terminal) })
            })
        })
    })
}

/**
 * The argv the desktop starts (window.rs `agent_argv`): a template by name
 * with its task; a program in a shell that stays when it exits, in the
 * remote form (`sh`), since the phone never knows the machine's own shell.
 */
private fun argvOf(run: Run?): List<String> {
    val task = listOfNotNull(run?.task?.trim()?.takeIf { it.isNotEmpty() })
    val template = run?.template
    if (!template.isNullOrEmpty()) return listOf(template) + task
    val argv = run?.argv
    if (!argv.isNullOrEmpty()) {
        return listOf("sh", "-c", "\"\$@\"; exec \"\${SHELL:-sh}\" -i", "sh") + argv + task
    }
    return emptyList()
}

private fun strings(list: List<String>) = JsonArray(list.map(::JsonPrimitive))

/**
 * Starts a terminal labeled with where it goes (window.rs `spawn`): empty
 * cwd is the machine's home, `keep` makes a lost program come back as a shell.
 */
private suspend fun start(
    client: Client,
    machine: String?,
    run: Run?,
    folder: String,
    space: String,
    tab: String,
): TerminalRecord {
    val r = client.call("terminal.run", buildJsonObject {
        put("argv", strings(argvOf(run)))
        put("cwd", if (folder.isEmpty()) JsonNull else JsonPrimitive(folder))
        put("env", ENV)
        put("labels", buildJsonObject {
            put("herdr.space", space)
            put("herdr.tab", tab)
        })
        put("on", on(machine))
        put("keep", true)
    })
    return TerminalRecord(r as? JsonObject ?: JsonObject(emptyMap()))
}

/**
 * Writes a just-started terminal into the layout with `add`, unless the
 * desktop already adopted it. If the layout could not take it, ends the
 * terminal rather than leave one no screen shows: a running desktop would
 * adopt it into a tab, but the phone's step failed, so nothing should appear.
 */
private suspend fun settle(
    client: Client,
    machine: String?,
    first: LayoutRead,
    terminal: String,
    add: (JsonObject) -> Pair<JsonObject, Where>,
): Created {
    val address = qualify(terminal, machine)
    try {
        val where = update(client, machine, first) { doc -> placed(doc, terminal)?.let { null to it } ?: add(doc) }
        return Created(address, qualify(where.space, machine), qualify(where.tab, machine))
    } catch (e: Exception) {
        attempt { client.call("terminal.kill", buildJsonObject { put("terminal", address) }) }
        throw e
    }
}

/**
 * The "new" menu (new-menu/main.js): the shell first, then the core's
 * templates (this core's only: `templates.list` takes no `on`), then the
 * agents, installed ones first and two that aren't (shown disabled).
 */
suspend fun templates(client: Client, machine: String? = null): List<Choice> {
    val own = if (machine == null && client.has("templates.list")) {
        attempt { (client.call("templates.list") as? JsonObject)?.objects("templates") }.orEmpty()
    } else emptyList()
    val have = installed(client, machine, AGENTS.map { it.run[0] })
    val agents = AGENTS.map { Choice.Agent(it.key, it.name, it.run[0] in have, Run(argv = it.run)) }
    return listOf(Choice.Shell()) +
        own.map { Choice.Template(it.string("name") ?: "", it.string("description") ?: "", it.string("error")) } +
        agents.filter { it.installed } +
        agents.filter { !it.installed }.take(2)
}

/**
 * Which commands the machine's `sh` finds (programs.rs `installed`, the
 * remote form: the one agents start with). Unknown: none.
 */
private suspend fun installed(client: Client, machine: String?, commands: List<String>): List<String> {
    if (!client.has("machine.exec")) return emptyList()
    val check = "for c; do command -v \"\$c\" >/dev/null 2>&1 && echo \"herdr-has:\$c\"; done"
    val r = attempt {
        client.call("machine.exec", buildJsonObject {
            put("argv", strings(listOf("sh", "-c", check, "sh") + commands))
            put("on", on(machine))
            put("timeout", 20)
        })
    } as? JsonObject ?: return emptyList()
    val out = attempt { Base64.decode(r.string("stdout") ?: "").decodeToString() } ?: return emptyList()
    val found = out.split("herdr-has:").drop(1).map { it.trim().split(Regex("\\s+"))[0] }.toSet()
    return commands.filter { it in found }
}

/**
 * Where a new tab in `space` starts (window.rs `folder_for`): the shown
 * terminal's folder (Herdr's "follow") when it's in that space, else null:
 * the space's own.
 */
fun tabFolder(shown: Terminal?, space: String): String? =
    shown?.takeIf { it.space == space }?.cwd?.ifEmpty { null }

/**
 * Where a new space on `machine` starts (window.rs `new_space`): the shown
 * terminal's folder, which only means something on its own machine; else
 * null, its home.
 */
fun spaceFolder(shown: Terminal?, machine: String?): String? =
    shown?.takeIf { it.machine == machine }?.cwd?.ifEmpty { null }

/**
 * A tab with a new terminal in a space (window.rs `new_tab`), after `after`
 * (the tab the phone shows; the desktop puts it after its selected one),
 * else last. `folder`: default the space's; the desktop uses the shown
 * terminal's ([tabFolder]). `name`: "" shows its position.
 */
suspend fun newTab(
    client: Client,
    machine: String?,
    space: String,
    name: String? = null,
    folder: String? = null,
    after: String? = null,
    run: Run? = null,
): Created {
    need(client, "terminal.run", "doc.set")
    val sid = local(space, machine)
    val afterId = after?.takeIf { it.isNotEmpty() }?.let { local(it, machine) }
    val first = readLayout(client, machine)
    val at = first.doc.objects("spaces").find { it.string("id") == sid } ?: error("no space $space")
    val tab = freshId("tb")
    val pane = freshId("p")
    val record = start(client, machine, run, folder ?: at.string("folder") ?: "", sid, tab)
    val terminal = local(record.address, machine)
    return settle(client, machine, first, terminal) { doc ->
        val spaces = doc.objects("spaces")
        val i = spaces.indexOfFirst { it.string("id") == sid }
        if (i < 0) error("space $space was closed")
        val tabs = spaces[i].objects("tabs")
        val k = if (afterId != null) tabs.indexOfFirst { it.string("id") == afterId } else -1
        val pos = if (k >= 0) k + 1 else tabs.size
        val added = tabOf(tab, name?.trim() ?: "", pane, terminal)
        val s = spaces[i].with("tabs", JsonArray(tabs.toMutableList().apply { add(pos, added) }))
        doc.with("spaces", JsonArray(spaces.replaced(i, s))) to Where(sid, tab)
    }
}

/**
 * A space with one tab and its terminal, last among the machine's spaces
 * (window.rs `new_space`, model.rs `add_space`). `name`: "" is named after
 * its folder by every screen. `folder`: the desktop's is the shown terminal's
 * ([spaceFolder]); default this core's home (where its terminal starts) or,
 * on another machine, "" (its home, as the desktop).
 */
suspend fun newSpace(
    client: Client,
    machine: String?,
    name: String? = null,
    folder: String? = null,
    tab: String? = null,
    run: Run? = null,
): Created {
    need(client, "terminal.run", "doc.set")
    // The layout must be readable first (window.rs `require_document`).
    val first = readLayout(client, machine)
    val space = freshId("sp")
    val tabId = freshId("tb")
    val pane = freshId("p")
    val record = start(client, machine, run, folder ?: "", space, tabId)
    val dir = folder ?: if (machine != null) "" else record.cwd
    val terminal = local(record.address, machine)
    return settle(client, machine, first, terminal) { doc ->
        val added = buildJsonObject {
            put("id", space)
            put("name", name?.trim() ?: "")
            put("folder", dir)
            put("tabs", JsonArray(listOf(tabOf(tabId, tab?.trim() ?: "", pane, terminal))))
        }
        doc.with("spaces", JsonArray(doc.objects("spaces") + added)) to Where(space, tabId)
    }
}

/**
 * Renames a space (window.rs `space.rename`): "" names it after its folder
 * again, on every screen.
 */
suspend fun renameSpace(client: Client, machine: String?, space: String, name: String) {
    need(client, "doc.set")
    val sid = local(space, machine)
    update(client, machine) { doc ->
        val spaces = doc.objects("spaces")
        val i = spaces.indexOfFirst { it.string("id") == sid }
        if (i < 0) return@update null to Unit
        doc.with("spaces", JsonArray(spaces.replaced(i, spaces[i].with("name", JsonPrimitive(name.trim()))))) to Unit
    }
}

/** Renames a tab (window.rs `tab.rename`): "" shows its position again. */
suspend fun renameTab(client: Client, machine: String?, tab: String, name: String) {
    need(client, "doc.set")
    val tid = local(tab, machine)
    update(client, machine) { doc ->
        val spaces = doc.objects("spaces")
        val i = spaces.indexOfFirst { s -> s.objects("tabs").any { it.string("id") == tid } }
        if (i < 0) return@update null to Unit
        val tabs = spaces[i].objects("tabs").map { if (it.string("id") == tid) it.with("name", JsonPrimitive(name.trim())) else it }
        doc.with("spaces", JsonArray(spaces.replaced(i, spaces[i].with("tabs", JsonArray(tabs))))) to Unit
    }
}

/**
 * Moves a space to `index` among its machine's (model.rs `move_space`, which
 * keeps it among them: each machine's spaces are its own document's).
 */
suspend fun moveSpace(client: Client, machine: String?, space: String, index: Int) {
    need(client, "doc.set")
    val sid = local(space, machine)
    update(client, machine) { doc ->
        val spaces = doc.objects("spaces").toMutableList()
        val from = spaces.indexOfFirst { it.string("id") == sid }
        val to = index.coerceIn(0, (spaces.size - 1).coerceAtLeast(0))
        if (from < 0 || from == to) return@update null to Unit
        spaces.add(to, spaces.removeAt(from))
        doc.with("spaces", JsonArray(spaces)) to Unit
    }
}

/**
 * Pins an agent to the top of the agent lists, or unpins it (window.rs
 * `terminal.pin`): kept on the terminal as its `herdr.pinned` label, so every
 * screen shows it, and gone when the terminal ends.
 */
suspend fun pin(client: Client, terminal: String, pinned: Boolean) {
    need(client, "terminal.label")
    client.call("terminal.label", buildJsonObject {
        put("terminal", terminal)
        put("labels", buildJsonObject { put("herdr.pinned", if (pinned) "1" else "") })
    })
}

data class Confirm(val title: String, val detail: String)

/**
 * What the desktop asks before a close (window.rs `space.close`, shown by
 * mods/confirm): closing a space, or the last tab of one (which closes it).
 * Another tab closes without asking: null. Its wording, plural and all.
 */
fun confirmClose(name: String, tabs: List<Tab>, tab: String? = null): Confirm? {
    if (tab != null && tabs.size > 1) return null
    val terminals = tabs.sumOf { it.terminals.size }
    return Confirm("Close space?", "$name — ${tabs.size} tabs, $terminals terminals")
}

fun confirmClose(space: Space, tab: String? = null) = confirmClose(space.name, space.tabs, tab)

data class Closed(
    /** Qualified; null if it was already gone. */
    val space: String?,
    /** The whole space went (its last tab, or [closeSpace]). */
    val spaceClosed: Boolean,
    /** Every terminal ended, sub-agents included. */
    val terminals: List<String>,
)

/**
 * Closes a tab and ends its terminals (window.rs `tab.close`). Its space's
 * last tab takes the space with it: confirm first ([confirmClose]).
 * `live`: the terminals the phone knows (the session's records), to find
 * sub-agents; else `terminal.list`.
 */
suspend fun closeTab(
    client: Client,
    machine: String?,
    tab: String,
    live: List<TerminalRecord>? = null,
): Closed {
    need(client, "doc.set")
    val tid = local(tab, machine)
    class Gone(val space: String?, val closed: Boolean, val terminals: List<String>)
    val gone = update(client, machine) { doc ->
        val spaces = doc.objects("spaces")
        val i = spaces.indexOfFirst { s -> s.objects("tabs").any { it.string("id") == tid } }
        if (i < 0) return@update null to Gone(null, false, emptyList())
        val s = spaces[i]
        val tabs = s.objects("tabs")
        val terminals = layoutTerminals(tabs.find { it.string("id") == tid }?.get("root"))
        val closed = tabs.size == 1
        val next = if (closed) spaces.filterIndexed { j, _ -> j != i }
        else spaces.replaced(i, s.with("tabs", JsonArray(tabs.filter { it.string("id") != tid })))
        doc.with("spaces", JsonArray(next)) to Gone(s.string("id"), closed, terminals)
    }
    return Closed(
        space = gone.space?.let { qualify(it, machine) },
        spaceClosed = gone.closed,
        terminals = kill(client, gone.terminals.map { qualify(it, machine) }, live),
    )
}

/** Closes a space and ends every terminal in it (window.rs `space.close`). */
suspend fun closeSpace(
    client: Client,
    machine: String?,
    space: String,
    live: List<TerminalRecord>? = null,
): Closed {
    need(client, "doc.set")
    val sid = local(space, machine)
    val terminals = update(client, machine) { doc ->
        val spaces = doc.objects("spaces")
        val s = spaces.find { it.string("id") == sid } ?: return@update null to null
        doc.with("spaces", JsonArray(spaces.filter { it !== s })) to
            s.objects("tabs").flatMap { layoutTerminals(it["root"]) }
    } ?: return Closed(null, false, emptyList())
    return Closed(space, true, kill(client, terminals.map { qualify(it, machine) }, live))
}

/**
 * Ends terminals and their sub-agents all the way down (window.rs `kill`):
 * the core keeps a sub-agent running when its parent dies. A sub-agent the
 * person kept (it has a `herdr.space`, i.e. its own tab) is spared.
 */
private suspend fun kill(client: Client, addresses: List<String>, live: List<TerminalRecord>?): List<String> {
    if (addresses.isEmpty()) return emptyList()
    val records = live ?: attempt {
        compact.decodeFromJsonElement(Snapshot.serializer(), client.call("terminal.list")).terminals
    }.orEmpty()
    val doomed = addresses.toMutableList()
    var i = 0
    while (i < doomed.size) {
        for (r in records) {
            val m = machineOf(r.address)
            val parent = r.parent?.takeIf { it.isNotEmpty() }?.let { if ('/' in it) it else qualify(it, m) }
            val kept = !r.labels["herdr.space"].isNullOrEmpty()
            if (r.state == "live" && parent == doomed[i] && !kept && r.address !in doomed) doomed += r.address
        }
        i++
    }
    // The layout already changed: a kill that fails is a terminal that ended.
    coroutineScope {
        doomed.map { a ->
            async { attempt { client.call("terminal.kill", buildJsonObject { put("terminal", a) }, 30_000) } }
        }.awaitAll()
    }
    return doomed
}
