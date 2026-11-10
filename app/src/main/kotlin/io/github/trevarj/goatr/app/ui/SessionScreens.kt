// What needs you (Agents) and where things are (Spaces): the desktop's
// sidebar as two phone lists, plus the "new" menu both of them and the
// terminal's tabs open.

package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.app.AgentsViewModel
import io.github.trevarj.goatr.app.LayoutViewModel
import io.github.trevarj.goatr.app.SpacesViewModel
import io.github.trevarj.goatr.app.session.Connection
import io.github.trevarj.goatr.app.session.Machine
import io.github.trevarj.goatr.app.session.MachineState
import io.github.trevarj.goatr.app.session.Session
import io.github.trevarj.goatr.app.session.Space
import io.github.trevarj.goatr.app.session.Terminal
import io.github.trevarj.goatr.app.session.agents
import io.github.trevarj.goatr.app.session.duration
import io.github.trevarj.goatr.app.session.hostOf
import io.github.trevarj.goatr.app.session.localOf
import io.github.trevarj.goatr.app.session.ref
import io.github.trevarj.goatr.app.session.spaceAttention
import herdr.core.Attention
import herdr.core.Choice
import herdr.core.Nested
import herdr.core.PlaceGit
import herdr.core.gitTail
import herdr.core.nest
import herdr.core.Repo
import herdr.core.programNamed
import kotlin.math.roundToInt

/** A tab's screen: its top bar (the tab bar's icon and title, and where the phone is connected) over its content. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TabScreen(title: String, icon: Painter, session: Session, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(title, fontWeight = FontWeight.SemiBold) },
            navigationIcon = { Icon(icon, null, Modifier.padding(start = 16.dp, end = 4.dp).size(22.dp)) },
            actions = { ConnectionBadge(session.host, session.connection) },
        )
        content()
    }
}

/** Figures that line up: each digit as wide as the others. */
internal val tabular = TextStyle(fontFeatureSettings = "tnum")

@Composable
private fun selected(on: Boolean) =
    if (on) ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.secondaryContainer) else ListItemDefaults.colors()

@Composable
fun AgentsScreen(vm: AgentsViewModel, onOpen: (String) -> Unit, onAddHost: () -> Unit) {
    val session by vm.session.collectAsStateWithLifecycle()
    TabScreen("Agents", Lucide.Bot, session) { AgentsList(vm, onOpen, onAddHost) }
}

/**
 * The agents by what needs you most: the Agents tab, or the sidebar's lower
 * half, `dense` (one line a row, as the desktop's sidebar), or its `rail`
 * (folded: each agent's tool and status, nothing else).
 */
@Composable
fun AgentsList(vm: AgentsViewModel, onOpen: (String) -> Unit, onAddHost: () -> Unit, dense: Boolean = false, rail: Boolean = false) {
    val session by vm.session.collectAsStateWithLifecycle()
    val current by vm.current.collectAsStateWithLifecycle()
    val now = rememberNow()
    Column {
        if (session.connection == Connection.Idle) {
            if (rail) {
                IconButton(onAddHost, Modifier.fillMaxWidth()) { Icon(Lucide.Plus, "Add a host", Modifier.size(18.dp)) }
                return
            }
            Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Connect to a host", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Add a machine that runs herdr-core.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Button(onAddHost) { Text("Add a host") }
            }
            return
        }
        val agents = session.agents()
        if (agents.isEmpty()) {
            if (rail) return
            Text(
                when (session.connection) {
                    Connection.Connected -> "No agents yet."
                    Connection.Lost -> "Not connected."
                    else -> "Connecting…"
                },
                Modifier.padding(horizontal = 20.dp),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            return
        }
        LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
            // Pinned ones first, as the desktop's agents list (agents/main.js), then by what they need.
            val pinned = agents.filter { it.pinned }
            if (pinned.isNotEmpty()) {
                if (!rail) item(key = "pinned") { GroupTitle("Pinned", pinned.size, MaterialTheme.colorScheme.onSurfaceVariant, dense) }
                items(pinned, key = { it.address }) { t -> AgentRow(t, t.address == current, now, onOpen, vm, dense, rail) }
            }
            for (a in Attention.entries) {
                val members = agents.filter { it.attention == a && !it.pinned }
                if (members.isEmpty()) continue
                if (!rail) item(key = a.name) { GroupTitle(SECTION.getValue(a), members.size, tone(a), dense) }
                items(members, key = { it.address }) { t -> AgentRow(t, t.address == current, now, onOpen, vm, dense, rail) }
            }
        }
    }
}

@Composable
private fun GroupTitle(title: String, count: Int, color: Color, dense: Boolean = false) {
    val style = if (dense) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge
    Row(
        Modifier.padding(start = 16.dp, top = if (dense) 8.dp else 12.dp, bottom = if (dense) 2.dp else 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(title, style = style, color = color)
        Text("$count", style = style, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** A sidebar row, dense or on the rail: one line, rounded, the selected one filled. */
@Composable
private fun SidebarRow(
    on: Boolean,
    click: Modifier,
    height: Dp,
    arrangement: Arrangement.Horizontal,
    inset: Dp,
    content: @Composable RowScope.() -> Unit,
) {
    Row(
        Modifier.padding(horizontal = 8.dp).fillMaxWidth().height(height).clip(RoundedCornerShape(8.dp))
            .background(if (on) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
            .then(click).padding(horizontal = inset),
        horizontalArrangement = arrangement,
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

/** A dense row: as the desktop's 30px rows, sized for a finger. */
@Composable
private fun SlimRow(on: Boolean, click: Modifier, height: Dp = 36.dp, content: @Composable RowScope.() -> Unit) =
    SidebarRow(on, click, height, Arrangement.spacedBy(9.dp), inset = 8.dp, content)

/**
 * A rail row: the icon and its status mark, each in a slot of its own, so the
 * icons line up in one column whether a row has a mark or not.
 */
@Composable
private fun RailRow(on: Boolean, click: Modifier, height: Dp = 36.dp, mark: @Composable () -> Unit = {}, icon: @Composable () -> Unit) =
    SidebarRow(on, click, height, Arrangement.spacedBy(4.dp, Alignment.CenterHorizontally), inset = 0.dp) {
        Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) { icon() }
        Box(Modifier.size(16.dp), contentAlignment = Alignment.Center) { mark() }
    }

@Composable
private fun AgentRow(
    t: Terminal,
    on: Boolean,
    now: Long,
    onOpen: (String) -> Unit,
    vm: AgentsViewModel,
    dense: Boolean = false,
    rail: Boolean = false,
) {
    var menu by remember { mutableStateOf(false) }
    val pin = { vm.pin(t.address, !t.pinned) }
    // A mouse over the row shows the desktop's pin button (agents/main.js).
    val hover = remember { MutableInteractionSource() }
    val hovered by hover.collectIsHoveredAsState()
    val click = Modifier.hoverable(hover)
        .combinedClickable(onClickLabel = "Open terminal") { onOpen(t.address) }
        .rowMenu("Agent menu", onMenu = { menu = true })
    val agentMenu = @Composable {
        RowMenu(
            menu,
            { menu = false },
            listOf(MenuItem(if (t.pinned) "Unpin" else "Pin to the top", { if (t.pinned) Lucide.PinOff else Lucide.Pin }, onClick = pin)),
        )
    }
    if (rail) {
        Box {
            RailRow(on, click, mark = { StatusMark(t.attention, compact = true) }) { ProgramIcon(t.program, 16.dp) }
            agentMenu()
        }
        return
    }
    if (dense) {
        // One line: the tool's icon, the session's title, where and how long.
        val dim = MaterialTheme.colorScheme.onSurfaceVariant
        Box {
            SlimRow(on, click) {
                ProgramIcon(t.program, 16.dp)
                Text(t.title ?: t.name, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (hovered) HeadAction(if (t.pinned) Lucide.PinOff else Lucide.Pin, if (t.pinned) "Unpin" else "Pin to the top", pin)
                t.machine?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = dim, maxLines = 1) }
                t.since?.let { Text(duration(now - it), style = MaterialTheme.typography.labelSmall.merge(tabular), color = dim) }
                StatusMark(t.attention, compact = true)
            }
            agentMenu()
        }
        return
    }
    // The session's title on top, the tool (claude-code, opencode, …) under it.
    Box {
        ListItem(
            headlineContent = { Text(t.title ?: t.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            supportingContent = t.title?.let { { Text(t.name, maxLines = 1, overflow = TextOverflow.Ellipsis) } },
            leadingContent = { ProgramIcon(t.program, 20.dp) },
            trailingContent = {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(horizontalAlignment = Alignment.End) {
                        t.since?.let { Text(duration(now - it), style = tabular) }
                        t.machine?.let { Text(it, style = MaterialTheme.typography.labelSmall) }
                    }
                    StatusMark(t.attention, compact = true)
                }
            },
            colors = selected(on),
            modifier = click,
        )
        agentMenu()
    }
}

@Composable
fun SpacesScreen(vm: SpacesViewModel, onOpen: (String) -> Unit) {
    val session by vm.session.collectAsStateWithLifecycle()
    TabScreen("Spaces", Lucide.Layers, session) { SpacesList(vm, onOpen) }
}

/** Each machine's spaces: the Spaces tab, or the sidebar's upper half, `dense`, or its `rail`. */
@Composable
fun SpacesList(vm: SpacesViewModel, onOpen: (String) -> Unit, dense: Boolean = false, rail: Boolean = false) {
    val session by vm.session.collectAsStateWithLifecycle()
    val current by vm.current.collectAsStateWithLifecycle()
    val dialog by vm.dialog.collectAsStateWithLifecycle()
    val git by vm.git.collectAsStateWithLifecycle()
    LaunchedEffect(vm) { vm.opened.collect(onOpen) }
    AppDialog(dialog, vm::dismiss)
    var folded by rememberSaveable { mutableStateOf(listOf<String>()) }
    val now = rememberNow()
    // The machine a new space is being made on (the menu is open), if any:
    // its host and its name there (see machineKey).
    var making by rememberSaveable { mutableStateOf<String?>(null) }
    making?.let { m ->
        val host = hostOf(m)
        val machine = localOf(m).ifEmpty { null }
        NewMenu("New space", host, machine, vm, onClose = { making = null }) { vm.newSpace(host, machine, it) }
    }
    val grouped = session.machines.size > 1
    val here = session.terminals.find { it.address == current }?.space
    val reorder = remember { Reorder() }
    LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
        // The desktop rail's +: a new space on the host's own machine.
        val home = session.machines.firstOrNull { it.name == null && it.state == MachineState.Connected }
        if (rail && home != null) item(key = "new") {
            RailRow(false, Modifier.combinedClickable(onClickLabel = "New space") { making = ref(home.host, "") }, height = 30.dp) {
                Icon(Lucide.Plus, "New space", Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        for (m in session.machines) {
            val id = ref(m.host, m.name ?: "")
            val shut = id in folded
            val fold = { folded = if (shut) folded - id else folded + id }
            item(key = "machine:$id") {
                if (dense || rail) {
                    MachineHead(m, grouped, shut, rail, now, onNew = { making = id }, onReconnect = { vm.reconnect(m) }, onFold = fold)
                } else if (grouped) {
                    ListItem(
                        headlineContent = { Text(m.label, style = MaterialTheme.typography.labelLarge) },
                        leadingContent = { Icon(machineIcon(m), null, Modifier.size(18.dp)) },
                        supportingContent = if (m.state != MachineState.Connected) {
                            {
                                Column {
                                    Text(m.stateLabel(now))
                                    m.problem?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                                }
                            }
                        } else null,
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                when (m.state) {
                                    MachineState.Connected -> IconButton({ making = id }) { Icon(Lucide.Plus, "New space") }
                                    MachineState.Lost -> IconButton({ vm.reconnect(m) }) { Icon(Lucide.RefreshCw, "Reconnect ${m.label}") }
                                    else -> {}
                                }
                                Icon(foldIcon(shut), foldLabel(shut), Modifier.size(18.dp))
                            }
                        },
                        modifier = Modifier.combinedClickable(onClick = fold),
                    )
                } else {
                    ListItem(
                        headlineContent = { Text("New space") },
                        leadingContent = { Icon(Lucide.Plus, null) },
                        modifier = Modifier.combinedClickable { making = id },
                    )
                }
            }
            if (!shut) spaceRows(session.spaces.filter { it.host == m.host && it.machine == m.name }, git, session.terminals, here, vm, onOpen, reorder, dense, rail)
        }
    }
}

@Composable
private fun machineIcon(m: Machine) = if (m.name != null) Lucide.Server else Lucide.Monitor

@Composable
private fun foldIcon(shut: Boolean) = if (shut) Lucide.ChevronRight else Lucide.ChevronDown

private fun foldLabel(shut: Boolean) = if (shut) "Expand" else "Collapse"

/** The sidebar's machine header: icon, name (and state when not connected), +, fold. On the rail, the icon alone. */
@Composable
private fun MachineHead(
    m: Machine,
    grouped: Boolean,
    shut: Boolean,
    rail: Boolean,
    now: Long,
    onNew: () -> Unit,
    onReconnect: () -> Unit,
    onFold: () -> Unit,
) {
    val dim = MaterialTheme.colorScheme.onSurfaceVariant
    val fold = Modifier.combinedClickable(onClickLabel = foldLabel(shut), onClick = onFold)
    if (rail) {
        // The rail's + (above the list) makes spaces; a machine's head only folds it.
        if (!grouped) return
        RailRow(false, fold, height = 30.dp) { Icon(machineIcon(m), m.label, Modifier.size(14.dp), tint = dim) }
        return
    }
    if (!grouped) {
        SlimRow(false, Modifier.combinedClickable(onClick = onNew), height = 30.dp) {
            Icon(Lucide.Plus, null, Modifier.size(14.dp), tint = dim)
            Text("New space", style = MaterialTheme.typography.labelMedium, color = dim)
        }
        return
    }
    SlimRow(false, fold, height = 30.dp) {
        Icon(machineIcon(m), null, Modifier.size(14.dp), tint = dim)
        Text(m.label, Modifier.weight(1f), style = MaterialTheme.typography.labelMedium, color = dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (m.state != MachineState.Connected) Text(m.stateLabel(now), style = MaterialTheme.typography.labelSmall, color = dim)
        when (m.state) {
            MachineState.Connected -> HeadAction(Lucide.Plus, "New space", onNew)
            MachineState.Lost -> HeadAction(Lucide.RefreshCw, "Reconnect ${m.label}", onReconnect)
            else -> {}
        }
        Icon(foldIcon(shut), null, Modifier.size(14.dp), tint = dim)
    }
}

@Composable
private fun HeadAction(icon: Painter, label: String, onClick: () -> Unit) {
    Icon(
        icon,
        label,
        Modifier.size(26.dp).clip(RoundedCornerShape(6.dp)).combinedClickable(onClick = onClick).padding(5.dp),
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** A space being dragged to a new place among its machine's (one at a time). */
private class Reorder {
    var id by mutableStateOf<String?>(null)
    var from by mutableIntStateOf(0)
    var offset by mutableFloatStateOf(0f)

    /** A row's height (px), to tell how many rows the drag went. */
    var row by mutableIntStateOf(1)

    fun target(last: Int) = (from + (offset / row).roundToInt()).coerceIn(0, last)
}

/**
 * A machine's spaces, each worktree under its repo's space as on the desktop
 * (herdr-app mods/spaces), with the git that's unusual about it after its name.
 */
private fun LazyListScope.spaceRows(
    spaces: List<Space>,
    git: Map<String, PlaceGit>,
    terminals: List<Terminal>,
    here: String?,
    vm: LayoutViewModel,
    onOpen: (String) -> Unit,
    reorder: Reorder,
    dense: Boolean = false,
    rail: Boolean = false,
) {
    val rows = nest(spaces) { git[it.id] }
    itemsIndexed(rows, key = { _, it -> it.space.id }) { i, row ->
        val space = row.space
        // Idle says nothing on a space's row.
        val mark = spaceAttention(space, terminals)?.takeIf { it != Attention.Idle }
        // A space opens on whatever in it needs you most.
        val target = terminals.filter { it.space == space.id }
            .maxByOrNull { (it.attention ?: Attention.Idle).priority }?.address
            ?: space.tabs.firstOrNull()?.terminals?.firstOrNull()
        var menu by remember { mutableStateOf(false) }
        var renaming by remember { mutableStateOf(false) }
        // Its checkout, looked up as the menu opens (mods/menu `checkout`), and the worktree dialog open on it.
        var repo by remember { mutableStateOf<Repo?>(null) }
        var worktree by remember { mutableStateOf<Pair<WorktreeAction, Repo>?>(null) }
        LaunchedEffect(menu) { if (menu) repo = vm.repoOf(space.id) }
        val haptic = LocalHapticFeedback.current
        val drag = remember(space.id, i, rows) {
            object : RowDrag {
                override fun start() {
                    haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                    reorder.id = space.id
                    reorder.from = i
                    reorder.offset = 0f
                }

                override fun to(offset: Float) {
                    reorder.offset = offset
                }

                override fun end() {
                    val to = reorder.target(rows.lastIndex)
                    reorder.id = null
                    // Rows are in the order shown; the core's is the spaces'.
                    if (to != i) vm.moveSpace(space.machine, space.id, spaces.indexOfFirst { it.id == rows[to].space.id })
                }

                override fun cancel() {
                    reorder.id = null
                }
            }
        }
        val click = Modifier.combinedClickable(onClickLabel = "Open") { target?.let(onOpen) }
            .rowMenu("Space menu", onMenu = { menu = true }, drag = drag)
        // The dragged row follows the pointer; a line marks where it lands (as the desktop's).
        val dragging = reorder.id == space.id
        val landing = reorder.id != null && !dragging && rows.any { it.space.id == reorder.id } && reorder.target(rows.lastIndex) == i
        val accent = MaterialTheme.colorScheme.primary
        val lifted = MaterialTheme.colorScheme.surfaceContainerHigh
        val place = Modifier
            .onSizeChanged { if (it.height > 0) reorder.row = it.height }
            .zIndex(if (dragging) 1f else 0f)
            .graphicsLayer {
                if (dragging) {
                    translationY = reorder.offset
                    shadowElevation = 6.dp.toPx()
                    shape = RoundedCornerShape(8.dp)
                    clip = true
                }
            }
            .then(if (dragging) Modifier.background(lifted) else Modifier)
            .drawWithContent {
                drawContent()
                if (landing) {
                    val y = if (reorder.from < i) size.height - 1.dp.toPx() else 1.dp.toPx()
                    drawLine(accent, Offset(0f, y), Offset(size.width, y), 2.dp.toPx())
                }
            }
        Box(place.testTag("space-row")) {
            when {
                rail -> RailRow(space.id == here, click, mark = { if (mark != null) StatusMark(mark, compact = true) }) {
                    SpaceAvatar(space.name, space.avatar, 18.dp)
                }
                dense -> SlimRow(space.id == here, click) {
                    if (row.tree) TreeMark(14.dp, indent = 14.dp) else SpaceAvatar(space.name, space.avatar, 18.dp)
                    val name = MaterialTheme.typography.bodyMedium
                    // As the desktop's: the name first, what's unusual about its git after it, dimmed.
                    val git = row.git
                    val tail = tailOf(row)
                    if (git == null || tail == null) {
                        Text(space.name, Modifier.weight(1f), style = name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    } else {
                        Text(space.name, Modifier.weight(1f, fill = false), style = name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        GitTail(git, tail, Modifier.weight(1f), MaterialTheme.typography.labelMedium, end = true)
                    }
                    if (mark != null) StatusMark(mark, compact = true)
                    Text("${space.tabs.size}", style = MaterialTheme.typography.labelMedium.merge(tabular), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                else -> ListItem(
                    headlineContent = { Text(space.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = row.git?.let { git ->
                        tailOf(row)?.let { tail -> { GitTail(git, tail, Modifier, LocalTextStyle.current, end = false) } }
                    },
                    leadingContent = { if (row.tree) TreeMark(20.dp, indent = 16.dp) else SpaceAvatar(space.name, space.avatar) },
                    trailingContent = {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            if (mark != null) StatusMark(mark, compact = true)
                            Text("${space.tabs.size}", style = tabular)
                        }
                    },
                    colors = selected(space.id == here),
                    modifier = click,
                )
            }
            // The desktop's space menu (mods/menu `space`).
            RowMenu(
                menu,
                { menu = false },
                buildList {
                    add(MenuItem("Rename", { Lucide.SquarePen }) { renaming = true })
                    repo?.let { checkout ->
                        add(MenuItem("New worktree", { Lucide.GitBranch }) { worktree = WorktreeAction.New to checkout })
                        add(MenuItem("Open worktree", { Lucide.GitBranch }) { worktree = WorktreeAction.Open to checkout })
                        if (checkout.linked) add(MenuItem("Remove worktree", { Lucide.GitBranch }) { worktree = WorktreeAction.Remove to checkout })
                    }
                    add(MenuItem("Close", { Lucide.X }, close = true) { vm.closeSpace(space.machine, space.id) })
                },
            )
        }
        worktree?.let { (action, checkout) -> WorktreeDialog(action, space, checkout, vm) { worktree = null } }
        if (renaming) {
            RenameDialog("Rename space", space.given, "Empty: named after its folder", { renaming = false }) {
                vm.renameSpace(space.machine, space.id, it)
            }
        }
    }
}

/** In place of a worktree space's avatar, as the desktop's: indented, its branch glyph. */
@Composable
private fun TreeMark(size: Dp, indent: Dp) {
    Icon(Lucide.GitBranch, "Worktree", Modifier.padding(start = indent).size(size), tint = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** What a space's row says about its git ([gitTail]); null when nothing: no repo, or nothing unusual. */
private fun tailOf(row: Nested<Space>): String? {
    val g = row.git ?: return null
    val tail = gitTail(g, row.tree, row.space.name)
    return tail.takeIf { it.isNotEmpty() || g.ahead > 0 || g.behind > 0 }
}

/** A space's repo and branch where they're unusual ([gitTail]), then commits ahead (green) and behind (red) of upstream. */
@Composable
private fun GitTail(g: PlaceGit, tail: String, modifier: Modifier, style: TextStyle, end: Boolean) {
    val dim = MaterialTheme.colorScheme.onSurfaceVariant
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(5.dp, if (end) Alignment.End else Alignment.Start), verticalAlignment = Alignment.CenterVertically) {
        if (tail.isNotEmpty()) Text(tail, Modifier.weight(1f, fill = false), style = style, color = dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (g.ahead > 0) Text("\u2191${g.ahead}", style = style.merge(tabular), color = herdr.ansi[2], maxLines = 1)
        if (g.behind > 0) Text("\u2193${g.behind}", style = style.merge(tabular), color = herdr.ansi[1], maxLines = 1)
    }
}

/**
 * The "new" menu, as the desktop's: the machine's shell, then agents (those
 * not installed are dimmed), then the core's templates. A bottom sheet.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewMenu(title: String, host: String, machine: String?, vm: LayoutViewModel, onClose: () -> Unit, onPick: (Choice) -> Unit) {
    val answer by produceState<Result<List<Choice>>?>(null, host, machine) {
        value = runCatching { vm.choices(host, machine) }
    }
    ModalBottomSheet(onDismissRequest = onClose) {
        Text(
            title,
            Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val a = answer
        LazyColumn(contentPadding = PaddingValues(bottom = 16.dp)) {
            when {
                a == null -> item { ListItem(headlineContent = { Text("Loading…") }) }
                a.isFailure -> item {
                    ListItem(headlineContent = { Text(a.exceptionOrNull()?.message ?: "failed", color = MaterialTheme.colorScheme.error) })
                }
                else -> items(a.getOrThrow(), key = { it.key }) { c ->
                    val missing = c is Choice.Agent && !c.installed
                    val broken = c is Choice.Template && c.error != null
                    // The desktop's icons: its shell glyph, each agent's own; templates keep theirs.
                    val program = when (c) {
                        is Choice.Shell -> programNamed("shell")
                        is Choice.Agent -> programNamed(c.key)
                        is Choice.Template -> null
                    }
                    val dim = MaterialTheme.colorScheme.onSurfaceVariant
                    ListItem(
                        headlineContent = { Text(c.name, maxLines = 1, color = if (missing || broken) dim else MaterialTheme.colorScheme.onSurface) },
                        leadingContent = {
                            if (program != null) ProgramIcon(program, 24.dp, MaterialTheme.colorScheme.onSurface)
                            else Icon(Lucide.FileCode, null)
                        },
                        trailingContent = when {
                            missing -> ({ Text("not installed") })
                            broken -> ({ Text("broken", color = MaterialTheme.colorScheme.error) })
                            else -> null
                        },
                        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                        modifier = Modifier.combinedClickable(enabled = !broken) {
                            onClose()
                            onPick(c)
                        },
                    )
                }
            }
        }
    }
}
