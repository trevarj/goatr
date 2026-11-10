package herdr.core

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull
import kotlin.test.assertTrue

@OptIn(ExperimentalCoroutinesApi::class)
class LayoutTest {
    private val docs = compact.parseToJsonElement(fixture("docs.json").decodeToString()).jsonObject
    private fun stored(key: String) = docs[key]!!.jsonObject.let { Stored(it["value"]!!, it.long("version")!!) }
    private val localDoc = stored("local")
    private val beta = stored("beta")

    private val id = Regex("^(sp|tb|p)[0-9a-f]{32}$")
    private val expo = "sp28196228eedbe2dc329f454b7dce0ef7"

    private suspend fun TestScope.setup(
        templates: List<JsonObject> = emptyList(),
        installed: List<String> = emptyList(),
        live: List<JsonObject> = emptyList(),
    ): Pair<FakeHerdr, Client> {
        val herdr = FakeHerdr(mapOf("" to localDoc, "beta" to beta), templates, installed, live)
        val client = connect(FakeCore(this, herdr.handlers).duplex, dispatcher = StandardTestDispatcher(testScheduler))
        return herdr to client
    }

    private fun doc(h: FakeHerdr, machine: String = "") = h.docs[machine]!!.value.jsonObject
    private fun JsonObject.spaces() = objects("spaces")
    private fun JsonObject.tabs() = objects("tabs")
    private fun JsonObject.pane() = (this["root"]!!.jsonObject["children"] as JsonArray)[0].jsonObject
    private fun strs(vararg s: String) = JsonArray(s.map(::JsonPrimitive))

    private suspend inline fun <reified E : Throwable> rejects(pattern: String, block: () -> Unit) {
        val e = assertFailsWith<E> { block() }
        assertTrue(pattern in (e.message ?: ""), "${e.message} has $pattern")
    }

    @Test
    fun aNewTabALabeledKeptShellThenATabAfterTheShownOne() = runTest {
        val (herdr, client) = setup()
        val r = newTab(client, machine = null, space = expo, after = "tb7661ff13518383114ef8f0277e86ea04")
        assertEquals(
            buildJsonObject {
                put("argv", JsonArray(emptyList()))
                put("cwd", "/home/dev/Projects/herdr-app")
                put("env", buildJsonObject {
                    put("TERM", "xterm-256color")
                    put("COLORTERM", "truecolor")
                    put("TERM_PROGRAM", "herdr")
                })
                put("labels", buildJsonObject {
                    put("herdr.space", expo)
                    put("herdr.tab", r.tab)
                })
                put("on", JsonNull)
                put("keep", true)
            },
            herdr.runs[0],
        )
        assertEquals(expo, r.space)
        assertEquals("t100", r.address)
        assertTrue(id.matches(r.tab))
        val space = doc(herdr).spaces()[1]
        assertEquals(
            listOf(
                "tb7661ff13518383114ef8f0277e86ea04",
                r.tab,
                "tb78bf4af480b18fb5941930a2fc314dc7",
                "tbf2755b9c6b29e5eb60cfcb000286e6b1",
            ),
            space.tabs().map { it.string("id") },
        )
        val added = space.tabs()[1]
        val pane = added.pane().string("id")!!
        assertTrue(id.matches(pane))
        assertEquals(
            buildJsonObject {
                put("id", r.tab)
                put("name", "")
                put("root", buildJsonObject {
                    put("id", r.tab)
                    put("type", "row")
                    put("children", buildJsonArray {
                        add(buildJsonObject {
                            put("id", pane)
                            put("type", "block")
                            put("content", buildJsonObject { put("terminal", "t100") })
                        })
                    })
                })
            },
            added,
        )
        // The rest of the document is untouched; written at the version read.
        assertEquals(localDoc.value.jsonObject.spaces()[0], doc(herdr).spaces()[0])
        assertEquals(44L, herdr.sets[0].long("version"))
        assertEquals(45L, herdr.docs[""]?.version)
    }

    @Test
    fun withoutAfterATabGoesLastAnAgentStartsInAShellThatStays() = runTest {
        val (herdr, client) = setup()
        val r = newTab(client, null, expo, name = "Claude Code", run = Run(argv = listOf("claude"), task = " fix it "))
        assertEquals(
            strs("sh", "-c", "\"\$@\"; exec \"\${SHELL:-sh}\" -i", "sh", "claude", "fix it"),
            herdr.runs[0]["argv"],
        )
        val tabs = doc(herdr).spaces()[1].tabs()
        assertEquals(r.tab, tabs.last().string("id"))
        assertEquals("Claude Code", tabs.last().string("name"))

        newTab(client, null, expo, run = Run(template = "reviewer"))
        assertEquals(strs("reviewer"), herdr.runs[1]["argv"])
    }

    @Test
    fun aNewSpaceTheDesktopShapeLastWithItsFirstTabAndTerminal() = runTest {
        val (herdr, client) = setup()
        val r = newSpace(client, machine = null)
        assertEquals(JsonNull, herdr.runs[0]["cwd"], "the core's home")
        assertEquals(
            buildJsonObject {
                put("herdr.space", r.space)
                put("herdr.tab", r.tab)
            },
            herdr.runs[0]["labels"],
        )
        val spaces = doc(herdr).spaces()
        assertEquals(3, spaces.size)
        val added = spaces[2]
        val pane = added.tabs()[0].pane().string("id")!!
        assertTrue(id.matches(r.space))
        assertTrue(id.matches(pane))
        assertEquals(
            buildJsonObject {
                put("id", r.space)
                put("name", "")
                put("folder", "/home/fake")
                put("tabs", buildJsonArray {
                    add(buildJsonObject {
                        put("id", r.tab)
                        put("name", "")
                        put("root", buildJsonObject {
                            put("id", r.tab)
                            put("type", "row")
                            put("children", buildJsonArray {
                                add(buildJsonObject {
                                    put("id", pane)
                                    put("type", "block")
                                    put("content", buildJsonObject { put("terminal", r.address) })
                                })
                            })
                        })
                    })
                })
            },
            added,
        )
    }

    @Test
    fun anotherMachineOnLocalIdsInItsDocumentQualifiedIdsBack() = runTest {
        val (herdr, client) = setup()
        val space = "beta/spcae1915e68e0da788a1f79297633b3c1"
        val tab = newTab(client, "beta", space)
        assertEquals("beta", herdr.runs[0].string("on"))
        assertEquals(JsonNull, herdr.runs[0]["cwd"], "an empty folder: its home")
        assertEquals("beta/t100", tab.address)
        assertEquals(space, tab.space)
        assertTrue(Regex("^beta/tb[0-9a-f]{32}$").matches(tab.tab))
        val local = tab.tab.removePrefix("beta/")
        assertEquals(
            buildJsonObject {
                put("herdr.space", "spcae1915e68e0da788a1f79297633b3c1")
                put("herdr.tab", local)
            },
            herdr.runs[0]["labels"],
        )
        val added = doc(herdr, "beta").spaces()[0].tabs().last()
        assertEquals(local, added.string("id"))
        assertEquals("t100", (added.pane()["content"] as JsonObject).string("terminal"))
        assertEquals(localDoc.value, doc(herdr), "this core untouched")

        val sp = newSpace(client, "beta", name = " work ")
        assertTrue(sp.space.startsWith("beta/sp"))
        assertEquals("beta/t101", sp.address)
        val added2 = doc(herdr, "beta").spaces()[1]
        assertEquals("work", added2.string("name"))
        assertEquals("", added2.string("folder"), "its home, as the desktop leaves it")
        assertEquals("t101", (added2.tabs()[0].pane()["content"] as JsonObject).string("terminal"))
    }

    private fun shown(address: String, cwd: String, space: String?) = Terminal(
        address = address, machine = machineOf(address), name = null, agent = null, program = "fish",
        shown = programOf(null, listOf("fish")), title = "", detail = null, cwd = cwd, status = null,
        attention = null, statusMs = null, space = space, tab = null, where = null, parent = null, pinned = false,
        record = TerminalRecord(JsonObject(emptyMap())),
    )

    @Test
    fun newTerminalsStartWhereTheShownOneIsOnItsOwnSpaceOrMachine() = runTest {
        val beta = "beta/spcae1915e68e0da788a1f79297633b3c1"
        val here = shown("t1", "/src/app", expo)
        assertEquals("/src/app", tabFolder(here, expo))
        assertNull(tabFolder(here, beta), "another space: its own folder")
        assertNull(tabFolder(shown("t1", "", expo), expo), "not read yet: the space's")
        assertNull(tabFolder(null, expo))
        assertEquals("/src/app", spaceFolder(here, null))
        assertNull(spaceFolder(here, "beta"), "a folder here means nothing there")
        assertEquals("/srv", spaceFolder(shown("beta/t2", "/srv", beta), "beta"))

        val (herdr, client) = setup()
        newTab(client, null, expo, folder = tabFolder(here, expo))
        assertEquals(JsonPrimitive("/src/app"), herdr.runs[0]["cwd"])
        newSpace(client, null, folder = spaceFolder(here, null))
        assertEquals(JsonPrimitive("/src/app"), herdr.runs[1]["cwd"])
        assertEquals("/src/app", doc(herdr).spaces().last().string("folder"), "made in it")
    }

    @Test
    fun anotherScreenWritesFirstReadAgainAndRedoTheStepOnItsDocument() = runTest {
        val (herdr, client) = setup()
        var other = 0
        herdr.beforeSet = { m ->
            // The desktop renames a space between our read and our write, twice.
            if (other++ < 2) {
                val spaces = doc(herdr, m).spaces()
                val renamed = JsonObject(spaces[0] + ("name" to JsonPrimitive("renamed $other")))
                herdr.write(m, JsonObject(mapOf("spaces" to JsonArray(listOf(renamed) + spaces.drop(1)))))
            }
        }
        val r = newTab(client, null, expo)
        assertEquals(1, herdr.sets.size)
        assertEquals(46L, herdr.sets[0].long("version"))
        assertEquals("renamed 2", doc(herdr).spaces()[0].string("name"), "their edit kept")
        assertEquals(r.tab, doc(herdr).spaces()[1].tabs().last().string("id"), "ours too")
        assertEquals(emptyList(), herdr.kills)
    }

    @Test
    fun theDesktopAdoptedTheTerminalFirstNoSecondTab() = runTest {
        val (herdr, client) = setup()
        herdr.beforeSet = { m ->
            val d = doc(herdr, m)
            if ("t100" !in d.toString()) {
                // window.rs `adopt`: a tab with the labeled id, a pane of its own.
                val labels = herdr.terminals.last()["labels"]!!.jsonObject
                val tabId = labels.string("herdr.tab")!!
                val tab: JsonElement = buildJsonObject {
                    put("id", tabId)
                    put("name", "")
                    put("root", buildJsonObject {
                        put("id", tabId)
                        put("type", "row")
                        put("children", buildJsonArray {
                            add(buildJsonObject {
                                put("id", "pdesk")
                                put("type", "block")
                                put("content", buildJsonObject { put("terminal", "t100") })
                            })
                        })
                    })
                }
                val spaces = d.spaces().map {
                    if (it.string("id") == labels.string("herdr.space")) {
                        JsonObject(it + ("tabs" to JsonArray(it.tabs() + tab)))
                    } else it
                }
                herdr.write(m, JsonObject(mapOf("spaces" to JsonArray(spaces))))
            }
        }
        val r = newTab(client, null, expo)
        assertEquals(0, herdr.sets.size)
        val tabs = doc(herdr).spaces()[1].tabs()
        assertEquals(4, tabs.size)
        assertEquals(r.tab, tabs.last().string("id"))
        assertEquals(emptyList(), herdr.kills)
    }

    @Test
    fun aWriteThatNeverLandsEndsTheTerminalItStarted() = runTest {
        val (herdr, client) = setup()
        herdr.beforeSet = { m -> herdr.write(m, doc(herdr, m)) }
        rejects<RpcError>("conflict") { newTab(client, null, expo) }
        assertEquals(listOf("t100"), herdr.kills)
        assertEquals(49L, herdr.docs[""]?.version, "five tries, each beaten")

        herdr.beforeSet = null
        herdr.failing["doc.set"] = "unavailable"
        rejects<RpcError>("unavailable") { newSpace(client, "beta") }
        assertEquals(listOf("t100", "beta/t101"), herdr.kills)

        // The space went meanwhile: nothing to put the tab in.
        herdr.failing.clear()
        herdr.beforeSet = { m ->
            val spaces = doc(herdr, m).spaces()
            if (spaces.size > 1) herdr.write(m, JsonObject(mapOf("spaces" to JsonArray(listOf(spaces[0])))))
        }
        rejects<IllegalStateException>("was closed") { newTab(client, null, expo) }
        assertEquals(listOf("t100", "beta/t101", "t102"), herdr.kills)

        // Unknown space or an unreadable layout: nothing is started.
        herdr.beforeSet = null
        rejects<IllegalStateException>("no space") { newTab(client, null, "spnope") }
        herdr.failing["doc.get"] = "unavailable"
        rejects<RpcError>("unavailable") { newSpace(client, null) }
        assertEquals(3, herdr.runs.size)
    }

    @Test
    fun theNewMenuTheShellThisCoresTemplatesInstalledAgentsThenTwoMore() = runTest {
        val (_, client) = setup(
            templates = listOf(buildJsonObject {
                put("name", "reviewer")
                put("description", "Reviews")
                put("command", "claude")
            }),
            installed = listOf("opencode", "claude"),
        )
        val here = templates(client)
        assertEquals(
            listOf(
                listOf("Shell", "Terminal", "", Run()),
                listOf("Template", "reviewer", "reviewer", Run(template = "reviewer")),
                listOf("Agent", "Claude Code", "Claude Code", Run(argv = listOf("claude"))),
                listOf("Agent", "OpenCode", "OpenCode", Run(argv = listOf("opencode"))),
                listOf("Agent", "Codex", "Codex", Run(argv = listOf("codex"))),
                listOf("Agent", "Pi", "Pi", Run(argv = listOf("pi"))),
            ),
            here.map { listOf(it::class.simpleName, it.name, it.tab, it.run) },
        )
        val there = templates(client, "beta")
        assertEquals(listOf("Shell", "Agent", "Agent", "Agent", "Agent"), there.map { it::class.simpleName })
    }

    private fun live(address: String, parent: String?, labels: Map<String, String> = emptyMap()) = buildJsonObject {
        put("address", address)
        put("parent", parent?.let(::JsonPrimitive) ?: JsonNull)
        put("labels", JsonObject(labels.mapValues { JsonPrimitive(it.value) }))
        put("state", "live")
    }

    @Test
    fun closingATabEndsItsTerminalsAndTheirSubAgentsNotKeptOnes() = runTest {
        val (herdr, client) = setup(
            live = listOf(
                live("t13", null),
                live("t40", "t13"),
                live("t41", "t40"),
                live("t42", "t13", mapOf("herdr.space" to expo, "herdr.tab" to "tb1")),
            ),
        )
        val r = closeTab(client, null, "tb78bf4af480b18fb5941930a2fc314dc7")
        assertEquals(Closed(expo, false, listOf("t13", "t40", "t41")), r)
        assertEquals(listOf("t13", "t40", "t41"), herdr.kills)
        assertEquals(
            listOf("tb7661ff13518383114ef8f0277e86ea04", "tbf2755b9c6b29e5eb60cfcb000286e6b1"),
            doc(herdr).spaces()[1].tabs().map { it.string("id") },
        )
    }

    @Test
    fun aSpacesLastTabTakesItWithItClosingASpaceEndsAllOfIt() = runTest {
        val (herdr, client) = setup()
        val one = listOf(Tab("a", "1", listOf("t1"), null))
        assertEquals(Confirm("Close space?", "x — 1 tabs, 1 terminals"), confirmClose("x", one, "a"))
        assertNull(confirmClose("x", one + one, "a"))

        val space = "beta/spcae1915e68e0da788a1f79297633b3c1"
        val r = closeSpace(
            client,
            "beta",
            space,
            live = listOf(live("beta/t1", null), live("beta/t9", "t1"), live("t9", null)).map(::TerminalRecord),
        )
        assertEquals(Closed(space, true, listOf("beta/t1", "beta/t4", "beta/t9")), r)
        assertEquals(emptyList(), doc(herdr, "beta").spaces())

        val before = doc(herdr).spaces().size
        val sp = newSpace(client, null)
        val last = closeTab(client, null, sp.tab)
        assertEquals(Closed(sp.space, true, listOf(sp.address)), last)
        assertEquals(before, doc(herdr).spaces().size)

        // Already gone: nothing written, nothing ended.
        val sets = herdr.sets.size
        assertEquals(Closed(null, false, emptyList()), closeSpace(client, "beta", space))
        assertEquals(sets, herdr.sets.size)
    }

    @Test
    fun renamingASpaceOrATabWritesTheNameTrimmedEmptyIsAutomaticAgain() = runTest {
        val (herdr, client) = setup()
        renameSpace(client, null, expo, "  api  ")
        assertEquals("api", doc(herdr).spaces().first { it.string("id") == expo }.string("name"))
        renameTab(client, null, "tb7661ff13518383114ef8f0277e86ea04", "build")
        assertEquals(
            "build",
            doc(herdr).spaces().flatMap { it.tabs() }.first { it.string("id") == "tb7661ff13518383114ef8f0277e86ea04" }.string("name"),
        )
        renameSpace(client, null, expo, "")
        assertEquals("", doc(herdr).spaces().first { it.string("id") == expo }.string("name"), "named after its folder again")
        val before = herdr.docs["beta"]
        renameSpace(client, null, "sp-gone", "x")
        assertEquals(before, herdr.docs["beta"], "a space that's gone: nothing written")
    }

    @Test
    fun aSpaceMovesAmongItsMachinesSpacesClampedToTheEnds() = runTest {
        val (herdr, client) = setup()
        val ids = doc(herdr).spaces().map { it.string("id") }
        moveSpace(client, null, expo, 0)
        assertEquals(listOf(expo) + (ids - expo), doc(herdr).spaces().map { it.string("id") })
        moveSpace(client, null, expo, 99)
        assertEquals((ids - expo) + expo, doc(herdr).spaces().map { it.string("id") })
        assertEquals(beta.value, herdr.docs["beta"]!!.value, "another machine's document is its own")
    }

    @Test
    fun pinningLabelsTheTerminalUnpinningClearsTheLabel() = runTest {
        val (herdr, client) = setup()
        val labeled = mutableListOf<JsonObject>()
        herdr.handlers["terminal.label"] = { p -> labeled += p; JsonObject(emptyMap()) }
        pin(client, "beta/t7", pinned = true)
        pin(client, "beta/t7", pinned = false)
        assertEquals(
            listOf("1", ""),
            labeled.map { it["labels"]!!.jsonObject.string("herdr.pinned") },
        )
        assertEquals(listOf("beta/t7", "beta/t7"), labeled.map { it.string("terminal") })
    }
}
