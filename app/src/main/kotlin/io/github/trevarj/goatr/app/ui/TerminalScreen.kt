// One terminal, full screen: its space on top, the space's tabs, then the live
// terminal edge to edge; tap it to type (its key bar rides the keyboard).

package io.github.trevarj.goatr.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PrimaryScrollableTabRow
import androidx.compose.material3.Surface
import androidx.compose.material3.TabRowDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.app.TerminalViewModel
import io.github.trevarj.goatr.app.session.Connection
import io.github.trevarj.goatr.app.session.Prefs
import io.github.trevarj.goatr.app.session.Session
import io.github.trevarj.goatr.app.session.Space
import io.github.trevarj.goatr.app.session.Tab
import io.github.trevarj.goatr.app.session.Terminal
import io.github.trevarj.goatr.app.session.agents
import io.github.trevarj.goatr.app.session.localOf
import herdr.core.Attention

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalScreen(
    vm: TerminalViewModel,
    host: TerminalHost,
    prefs: Prefs,
    onOpen: (String) -> Unit,
    onBack: () -> Unit,
    /** The space's bar on top; the tablet's sidebar already says where you are. */
    header: Boolean = true,
) {
    val session by vm.session.collectAsStateWithLifecycle()
    val current by vm.current.collectAsStateWithLifecycle()
    val dialog by vm.dialog.collectAsStateWithLifecycle()
    LaunchedEffect(vm) { vm.opened.collect(onOpen) }
    AppDialog(dialog, vm::dismiss)

    // The one last opened (or asked for by a link, which may name it as its
    // core does, `beta/t1`, without the host).
    val asked = current?.let { w -> session.terminals.find { it.address == w } ?: session.terminals.find { localOf(it.address) == w } }
    // What was on screen, to tell a closed terminal from one not here yet: its
    // host still answers (it has machines) but no longer lists it.
    var shown by remember { mutableStateOf<Terminal?>(null) }
    val was = shown
    val gone = asked == null && was != null && (current == null || current == was.address) &&
        session.terminals.none { it.address == was.address } && session.machines.any { it.host == was.host }
    LaunchedEffect(gone) {
        if (!gone) return@LaunchedEffect
        // Another tab of its space, else back: never off to another space.
        val next = session.spaces.find { it.id == was.space }?.tabs.orEmpty()
            .flatMap { it.terminals }.firstOrNull { a -> session.terminals.any { it.address == a } }
        if (next != null) vm.select(next) else {
            shown = null
            onBack()
        }
    }
    // Nothing opened yet, or the last one's host hasn't answered: whatever needs
    // you most, from the first host that does, without waiting for the others.
    val address = asked?.address
        ?: if (gone) "" else session.agents().firstOrNull()?.address
            ?: session.terminals.firstOrNull()?.address
            ?: ""
    val terminal = session.terminals.find { it.address == address }
    // Once on screen it's the one opened: a host answering later doesn't take it away.
    LaunchedEffect(address) { if (asked == null && terminal != null) vm.select(address) }
    SideEffect { if (terminal != null && terminal.address != shown?.address) shown = terminal }
    val space = session.spaces.find { it.id == terminal?.space }
    // Where it runs: the machine it names, else its host's own.
    val machine = terminal?.let { t ->
        session.machines.find { it.host == t.host && (if (t.machine == null) it.name == null else it.label == t.machine) }
    }

    // What's on screen counts as seen: a done agent stops asking for attention.
    DisposableEffect(address) {
        vm.setVisible(if (address.isEmpty()) emptyList() else listOf(address))
        onDispose { vm.setVisible(emptyList()) }
    }
    // A terminal that's gone (its tab closed) frees its view once there's nothing to show.
    LaunchedEffect(address) { if (address.isEmpty()) host.release() }

    Column(Modifier.fillMaxSize()) {
        // Compact: the terminal gets the height.
        if (header) TopAppBar(
            title = {
                Text(
                    space?.name ?: terminal?.name ?: "No terminal",
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Medium,
                )
            },
            navigationIcon = {
                // The space's mark when it has one, else the program's tile: never both.
                Box(Modifier.padding(start = 16.dp, end = 4.dp)) {
                    if (space != null) SpaceAvatar(space.name, space.avatar, 20.dp)
                    else if (terminal != null) ProgramIcon(terminal.program, 20.dp)
                    else Icon(Lucide.SquareTerminal, null, Modifier.size(20.dp))
                }
            },
            actions = { machine?.let { MachineBadge(it) } },
            expandedHeight = 44.dp,
        )

        // The system's pickers: no permission, only what's picked is shared.
        val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { host.sendImages(address, it) }
        val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { host.sendFiles(address, it) }
        val attach = Attach(
            photos = { photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
            files = { files.launch(arrayOf("*/*")) },
        )
        if (space != null) TabStrip(space, session, address, vm, attach.takeIf { address.isNotEmpty() })

        // Lifted by exactly what the keyboard covers, so the terminal's key bar
        // sits right on top of it.
        Surface(Modifier.weight(1f).fillMaxWidth().imePadding(), color = herdr.background) {
            if (address.isNotEmpty()) {
                val colors = herdr
                val state by host.state.collectAsStateWithLifecycle()
                val sending by host.sending.collectAsStateWithLifecycle()
                Box(Modifier.fillMaxSize()) {
                    // A view per terminal: another tab's address makes (or reuses) its own.
                    key(address) {
                        AndroidView(
                            factory = { host.show(address) },
                            update = { host.style(prefs, colors) },
                            modifier = Modifier.fillMaxSize(),
                        )
                    }
                    val label = sending ?: when (val s = state) {
                        PaneState.Live -> null
                        PaneState.Waiting -> "connecting…"
                        PaneState.Attaching -> "attaching…"
                        is PaneState.Ended -> s.why
                        is PaneState.Failed -> s.why
                    }
                    if (label != null) {
                        Surface(
                            Modifier.align(Alignment.TopEnd).padding(8.dp),
                            shape = MaterialTheme.shapes.small,
                            color = MaterialTheme.colorScheme.surfaceContainerHigh,
                        ) {
                            Text(
                                label,
                                Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                style = MaterialTheme.typography.labelSmall.copy(fontFamily = GeistMono),
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            } else {
                Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                    if (session.connection == Connection.Connected || session.connection == Connection.Idle) {
                        Text("No terminal open.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    } else {
                        Hosts(vm)
                    }
                }
            }
        }
    }
}

/** Nothing to show yet: every host, each connecting on its own, and how it's going. */
@Composable
private fun Hosts(vm: TerminalViewModel) {
    val hosts by vm.hosts.collectAsStateWithLifecycle()
    val states by vm.hostStates.collectAsStateWithLifecycle()
    Column(Modifier.widthIn(max = 320.dp)) {
        Text("Connecting", Modifier.padding(bottom = 12.dp), style = MaterialTheme.typography.titleMedium)
        HostRows(hosts.filter { !it.off }, states, vm::reconnect)
    }
}

/**
 * The space's tabs, above the terminal; scrolls sideways when they don't fit.
 * + opens the new menu; × (or a long press on any tab) closes, asking first
 * when that closes the space.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TabStrip(space: Space, session: Session, address: String, vm: TerminalViewModel, attach: Attach?) {
    var menu by rememberSaveable { mutableStateOf(false) }
    // The tab whose menu is open, and the one being renamed.
    var menuFor by remember { mutableStateOf<String?>(null) }
    var renaming by remember { mutableStateOf<Tab?>(null) }
    val shown = space.tabs.indexOfFirst { address in it.terminals }
    fun close(tab: String) = vm.closeTab(space.machine, space.id, tab)
    // + belongs to the bar: the tabs' background behind it, their rule under both.
    Column(Modifier.background(TabRowDefaults.primaryContainerColor)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            PrimaryScrollableTabRow(
                selectedTabIndex = shown.coerceAtLeast(0),
                modifier = Modifier.weight(1f),
                edgePadding = 0.dp,
                divider = {},
            ) {
                space.tabs.forEachIndexed { i, tab ->
                    val lead = session.terminals.find { it.address == tab.terminals.firstOrNull() }
                    val on = i == shown
                    // A tab of our own (Material's has no long press), with the same semantics;
                    // a long press or a right click opens the desktop's tab menu.
                    Box(
                        Modifier.heightIn(min = 40.dp).widthIn(max = 224.dp).testTag("tab")
                            .semantics { selected = on }
                            .combinedClickable(role = Role.Tab) { tab.terminals.firstOrNull()?.let(vm::select) }
                            .rowMenu("Tab menu", onMenu = { menuFor = tab.id }),
                        contentAlignment = Alignment.Center,
                    ) {
                        RowMenu(
                            menuFor == tab.id,
                            { menuFor = null },
                            listOf(
                                MenuItem("Rename", { Lucide.SquarePen }) { renaming = tab },
                                MenuItem("Close", { Lucide.X }, close = true) { close(tab.id) },
                            ),
                        )
                        Row(
                            Modifier.padding(start = 16.dp, end = if (on) 0.dp else 16.dp),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            // As the desktop's tabs: what runs there, its name unless the tab
                            // was named, then a spinner while working or a dot when it wants you.
                            lead?.let { ProgramIcon(it.program, 16.dp, MaterialTheme.colorScheme.onSurfaceVariant) }
                            val auto = tab.name.isEmpty() || Regex("""tb\d+""").matches(tab.name)
                            Text(
                                if (auto) lead?.program?.name ?: "Terminal" else tab.name,
                                Modifier.weight(1f, fill = false),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            when (val a = lead?.attention) {
                                Attention.Working -> StatusMark(a)
                                Attention.Blocked, Attention.Done ->
                                    Box(Modifier.size(6.dp).clip(CircleShape).background(tone(a)).semantics { contentDescription = SECTION.getValue(a) })
                                else -> {}
                            }
                            if (on) IconButton({ close(tab.id) }, Modifier.size(32.dp)) { Icon(Lucide.X, "Close tab", Modifier.size(16.dp)) }
                        }
                    }
                }
            }
            // Beside +, on the phone and the tablet alike: photos or files for the terminal's program.
            attach?.let { AttachButton(it) }
            IconButton({ menu = true }, Modifier.size(40.dp)) { Icon(Lucide.Plus, "New tab") }
        }
        HorizontalDivider()
    }
    renaming?.let { tab ->
        // An automatic name (empty, or the desktop's `tbN`) starts empty.
        val given = tab.name.takeUnless { it.isEmpty() || Regex("""tb\d+""").matches(it) } ?: ""
        RenameDialog("Rename tab", given, "Empty: its number", { renaming = null }) {
            vm.renameTab(space.machine, tab.id, it)
        }
    }
    if (menu) {
        val after = space.tabs.getOrNull(shown)?.id
        NewMenu("New tab in ${space.name}", space.host, space.machine, vm, onClose = { menu = false }) {
            vm.newTab(space.machine, space.id, after, address, it)
        }
    }
}

/** The pickers that send to the terminal's program: Android's photo picker, and its files. */
private class Attach(val photos: () -> Unit, val files: () -> Unit)

/** Photos or files for the terminal's program, from a menu. */
@Composable
private fun AttachButton(attach: Attach) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton({ open = true }, Modifier.size(40.dp)) { Icon(Lucide.Paperclip, "Attach", Modifier.size(20.dp)) }
        RowMenu(
            open,
            { open = false },
            listOf(
                MenuItem("Photos", { Lucide.ImagePlus }, onClick = attach.photos),
                MenuItem("Files", { Lucide.File }, onClick = attach.files),
            ),
        )
    }
}
