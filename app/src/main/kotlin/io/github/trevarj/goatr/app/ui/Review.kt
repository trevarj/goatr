package io.github.trevarj.goatr.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.app.ReviewViewModel
import io.github.trevarj.goatr.app.session.*
import kotlinx.coroutines.launch

private enum class Sheet { Files, Bases, History, Editor, Summary, Batch }
private data class Pick(val anchor: ReviewAnchor, val hunk: Int, val first: Int)
private data class Confirmation(val title: String, val detail: String, val action: String, val confirm: () -> Unit)

@Composable
private fun Confirm(value: Confirmation, cancel: () -> Unit, cancelLabel: String = "Cancel") {
    AlertDialog(onDismissRequest = cancel, title = { Text(value.title) }, text = { Text(value.detail) },
        confirmButton = { TextButton({ cancel(); value.confirm() }) { Text(value.action) } },
        dismissButton = { TextButton(cancel) { Text(cancelLabel) } })
}

@Composable
internal fun rememberReviewLeave(vm: ReviewViewModel): (() -> Unit) -> Unit {
    val scope = rememberCoroutineScope()
    var pending by remember(vm) { mutableStateOf<(() -> Unit)?>(null) }
    var flushing by remember(vm) { mutableStateOf(false) }
    pending?.let { exit ->
        Confirm(Confirmation("Could not save", vm.state.value.saveError ?: "Unsaved changes may be lost.", "Leave without saving") {
            vm.leaveWithoutSaving(); exit()
        }, { pending = null }, "Keep editing")
    }
    return { next ->
        if (!flushing && pending == null) {
            flushing = true
            scope.launch {
                try { if (vm.prepareToLeave()) next() else pending = next } finally { flushing = false }
            }
        }
    }
}

private fun ReviewFileKey.label() = "$layer · " + (oldPath?.let { "$it → " } ?: "") + path
private fun ReviewFileKey.rowKey() = "${layer.name}\u0000${oldPath.orEmpty()}\u0000$path"
private fun ReviewAnchor.location() = file.label() + (side?.let { " · $it $startLine" + if (endLine != startLine) "–$endLine" else "" } ?: " · File")
private fun ReviewComparison.label() = if (mode == ReviewMode.WorkingTree) "Working tree" else "Branch · ${baseRef ?: "Choose base"}"
private fun ReviewLine.number(side: ReviewSide) = if (side == ReviewSide.Before) before else after
private fun ReviewRecord.status() = when (phase) {
    ReviewPhase.Draft -> "Draft"
    ReviewPhase.Sent -> if (envelope?.started == false) "Sent; agent start not observed" else "Sent to terminal"
    ReviewPhase.Sending, ReviewPhase.DeliveryUnknown -> "Delivery unknown. Open the agent before sending again."
}

private data class LineBlock(val from: Int, val until: Int, val comments: List<ReviewComment>)
private class FeedIndex(files: List<ReviewFile>, comments: List<ReviewComment>) {
    val fileRows = mutableMapOf<ReviewFileKey, Int>()
    val hunkRows = mutableListOf<Int>()
    val commentRows = mutableMapOf<String, Int>()
    val fileComments = mutableMapOf<ReviewFileKey, List<ReviewComment>>()
    val blocks = mutableMapOf<Pair<ReviewFileKey, Int>, List<LineBlock>>()
    init {
        val byFile = comments.filterNot { it.needsReanchor }.groupBy { it.anchor.file }
        var row = 0
        for (file in files) {
            fileRows[file.key] = row++
            val notes = byFile[file.key].orEmpty()
            fileComments[file.key] = notes.filter { it.anchor.side == null }.onEach { commentRows[it.id] = row++ }
            val ends = notes.filter { it.anchor.side != null }.groupBy { it.anchor.side to it.anchor.endLine }
            file.hunks.forEachIndexed { hi, hunk ->
                hunkRows.add(row++)
                var start = 0
                val runs = buildList {
                    if (ends.isNotEmpty()) hunk.lines.forEachIndexed { li, line ->
                        val before = line.before?.let { ends[ReviewSide.Before to it] }
                        val after = line.after?.let { ends[ReviewSide.After to it] }
                        if (before != null || after != null) {
                            val here = before.orEmpty() + after.orEmpty()
                            add(LineBlock(start, li + 1, here)); row += li + 1 - start
                            here.forEach { commentRows[it.id] = row++ }; start = li + 1
                        }
                    }
                    if (start < hunk.lines.size) {
                        add(LineBlock(start, hunk.lines.size, emptyList())); row += hunk.lines.size - start
                    }
                }
                blocks[file.key to hi] = runs
            }
        }
    }
}

@Composable
fun ReviewScreen(vm: ReviewViewModel, spaceId: String, onBack: () -> Unit, onOpenAgent: (String) -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(vm, spaceId) { vm.open(spaceId) }
    DisposableEffect(vm, spaceId) { onDispose { vm.detach(spaceId) } }
    val leave = rememberReviewLeave(vm)
    BackHandler { leave(onBack) }
    if (state.spaceId != spaceId) {
        Column { TextButton({ leave(onBack) }) { Text("Back") }; Text("Loading review…") }
        return
    }
    key(spaceId, state.record?.id) { ReviewBody(vm, state, { leave(onBack) }, { addr -> leave { onOpenAgent(addr) } }, leave) }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class, androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
private fun ReviewBody(vm: ReviewViewModel, state: ReviewUiState, back: () -> Unit, agent: (String) -> Unit, leave: (() -> Unit) -> Unit) {
    val record = state.record
    val snapshot = state.snapshot
    val draft = record?.phase == ReviewPhase.Draft
    val editing = draft && !state.submitting && !state.loading && snapshot?.id == record.snapshotId && state.comparison == record.comparison
    var sheet by rememberSaveable { mutableStateOf<Sheet?>(null) }
    var menu by remember { mutableStateOf(false) }
    var wrap by rememberSaveable { mutableStateOf(true) }
    var confirm by remember { mutableStateOf<Confirmation?>(null) }
    var selection by remember(snapshot?.id) { mutableStateOf<Pick?>(null) }
    var reanchor by remember(snapshot?.id) { mutableStateOf<String?>(null) }
    var hint by remember(snapshot?.id) { mutableStateOf<String?>(null) }
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val feed = remember(state.files, record?.comments) { FeedIndex(state.files, record?.comments.orEmpty()) }
    fun jump(index: Int, header: Boolean = false) {
        scope.launch {
            list.scrollToItem(index)
            if (!header) {
                val pinned = list.layoutInfo.visibleItemsInfo.firstOrNull { it.offset <= 0 && (it.key as? String)?.startsWith("file:") == true }
                if (pinned != null) list.scrollToItem(index, -pinned.size)
            }
        }
    }
    fun close() {
        if (sheet == Sheet.Editor) vm.dismissComposer()
        val summary = sheet == Sheet.Summary && (draft || state.submitting)
        sheet = null
        if (summary) scope.launch { vm.prepareToLeave() }
    }
    val composer = record?.composer
    fun begin(anchor: ReviewAnchor, commentId: String? = reanchor) {
        if (!editing) return
        if (composer == null) { vm.beginComment(anchor, commentId); reanchor = null; selection = null }
        sheet = Sheet.Editor
    }
    fun discardComposer() {
        val discard = { vm.dismissComposer(discard = true); if (sheet == Sheet.Editor) sheet = null }
        if (composer?.text.isNullOrEmpty()) discard() else confirm = Confirmation("Discard unfinished comment?", "The unsaved comment text will be removed.", "Discard", discard)
    }
    fun remove(note: ReviewComment) {
        confirm = Confirmation("Remove comment?", note.anchor.location(), "Remove") { vm.removeComment(note.id) }
    }
    fun delete(r: ReviewRecord) {
        confirm = Confirmation(if (r.phase == ReviewPhase.Draft) "Discard draft?" else "Delete review?", "This removes this review and its comments from this device.", "Delete") { vm.deleteRecord(r.id) }
    }
    fun fresh() {
        if (record?.phase in listOf(ReviewPhase.Sending, ReviewPhase.DeliveryUnknown)) {
            confirm = Confirmation("Create another review?", "The original may already have reached the terminal. Sending the copied feedback could duplicate it.", "Create another review") {
                leave { sheet = null; vm.newDraft(record?.id) }
            }
        } else leave { sheet = null; vm.newDraft() }
    }
    fun compare(c: ReviewComparison) { leave { sheet = null; vm.chooseComparison(c) } }
    LaunchedEffect(composer != null, sheet) { if (sheet == Sheet.Editor && composer == null) sheet = null }
    Column(Modifier.fillMaxSize().imePadding()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(back) { Icon(Lucide.ChevronRight, "Back", Modifier.rotate(180f)) }
            Column(Modifier.weight(1f)) {
                Text(snapshot?.repo?.root ?: record?.repo?.root ?: "Review changes", maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleSmall)
                Text(state.spaceId.orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.labelSmall)
            }
            IconButton(vm::refresh, enabled = !state.submitting && !state.loading && (draft || record == null)) { Icon(Lucide.RefreshCw, "Refresh") }
            IconButton({ sheet = Sheet.Files }) { Icon(Lucide.Layers, "Files") }
            Box {
                IconButton({ menu = true }) { Icon(Lucide.Sliders, "More review options") }
                DropdownMenu(menu, { menu = false }) {
                    DropdownMenuItem(text = { Text("Wrap lines") }, trailingIcon = { Checkbox(checked = wrap, onCheckedChange = null) }, onClick = { wrap = !wrap; menu = false })
                    DropdownMenuItem(text = { Text("Reviews") }, enabled = !state.submitting, onClick = { sheet = Sheet.History; menu = false })
                    DropdownMenuItem(text = { Text("New review") }, enabled = !state.submitting, onClick = { menu = false; fresh() })
                    if (draft) DropdownMenuItem(text = { Text("Discard draft") }, enabled = !state.submitting, onClick = { menu = false; delete(record) })
                }
            }
        }
        ChoiceRow(listOf("Working tree", "Branch"), if (state.comparison.mode == ReviewMode.Branch) 1 else 0, !state.submitting && (draft || record == null)) {
            compare(ReviewComparison(if (it == 0) ReviewMode.WorkingTree else ReviewMode.Branch))
        }
        if (state.comparison.mode == ReviewMode.Branch) {
            TextButton({ sheet = Sheet.Bases }, enabled = !state.submitting && (record == null || draft)) { Text("Choose base · ${state.comparison.baseRef ?: "Locally available refs"}", maxLines = 1, overflow = TextOverflow.Ellipsis) }
            if (snapshot?.comparison == state.comparison) Caption("Merge base ${snapshot.mergeBaseOid?.take(12)} → HEAD ${snapshot.headOid?.take(12)}")
        }
        if (state.loading) { LinearProgressIndicator(Modifier.fillMaxWidth()); Caption("Loading review…") }
        if (!state.connected) Caption("Disconnected · Send unavailable")
        if (state.submitting) Caption("Sending…") else if (record != null && !draft && state.error != record.status()) Caption(record.status())
        ReviewErrors(state)
        if (state.unreadableMetadata) TextButton({ confirm = Confirmation("Delete unreadable reviews?", "This removes the local review history on this device and its snapshots. It cannot be undone.", "Delete") { vm.deleteUnreadableReviews() } }) { Text("Delete unreadable reviews") }
        if (state.workingAgents) Caption("An agent is still working in this Space. Files may continue changing.")
        if (composer != null && draft) Row {
            TextButton({ sheet = Sheet.Editor }, enabled = !state.submitting) { Text("Resume comment") }
            TextButton(::discardComposer, enabled = !state.submitting) { Text("Discard comment") }
        }
        if (reanchor != null) Row(verticalAlignment = Alignment.CenterVertically) {
            Caption("Select a new line range or file", Modifier.weight(1f)); TextButton({ reanchor = null; selection = null }) { Text("Cancel") }
        }
        hint?.let { Caption(it) }
        selection?.let { pick ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Caption("${pick.anchor.side} ${pick.anchor.startLine}–${pick.anchor.endLine}", Modifier.weight(1f))
                TextButton({ begin(pick.anchor) }, enabled = editing) { Text("Comment") }
                TextButton({ selection = null; hint = null }) { Text("Cancel") }
            }
        }
        if (snapshot == null && !state.loading && !state.unreadableMetadata) {
            Caption(if (state.comparison.mode == ReviewMode.Branch && state.comparison.baseRef == null) "Choose a locally available base to load this comparison." else "No review snapshot available.")
        } else if (snapshot != null && state.files.isEmpty()) Caption("No changes in this comparison.")
        BoxWithConstraints(Modifier.fillMaxWidth().weight(1f)) {
            val clearance = maxHeight.coerceAtLeast(96.dp)
            LazyColumn(state = list, contentPadding = PaddingValues(bottom = clearance), modifier = Modifier.fillMaxSize().testTag("review_diff")) {
                for (file in state.files) {
                    val key = file.key.rowKey()
                    stickyHeader(key = "file:$key") {
                        Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh) {
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f).padding(horizontal = 8.dp)) {
                                    Text(file.key.label(), style = MaterialTheme.typography.titleSmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                    Text(file.details(), style = MaterialTheme.typography.labelSmall, maxLines = 3, overflow = TextOverflow.Ellipsis)
                                }
                                if (draft) IconButton({ snapshot?.let { begin(ReviewAnchor(it.id, file.key, null, null, null)) } }, enabled = editing) { Icon(Lucide.SquarePen, "Comment on file") }
                            }
                        }
                    }
                    fun LazyListScope.cards(notes: List<ReviewComment>) {
                        items(notes, key = { "comment:${it.id}" }) { note ->
                            if (record != null) CommentCard(note, record, editing, { begin(note.anchor, note.id) }, { remove(note) })
                        }
                    }
                    cards(feed.fileComments[file.key].orEmpty())
                    file.hunks.forEachIndexed { hi, hunk ->
                        item(key = "hunk:$hi:$key") { Caption(hunk.header, Modifier.background(MaterialTheme.colorScheme.surfaceContainer)) }
                        feed.blocks.getValue(file.key to hi).forEach { block ->
                            items(block.until - block.from, key = { "line:$hi:${it + block.from}:$key" }) { offset ->
                                val line = hunk.lines[offset + block.from]
                                DiffLine(line, wrap, editing, selection?.takeIf { it.hunk == hi && it.anchor.file == file.key }?.anchor,
                                    comment = { side, n -> snapshot?.let { begin(ReviewAnchor(it.id, file.key, side, n, n)) } }) { side, number ->
                                    val id = snapshot?.id ?: return@DiffLine
                                    val old = selection
                                    val sameFile = old != null && old.anchor.file == file.key && old.anchor.side == side
                                    val same = sameFile && old.hunk == hi
                                    val first = if (same) old.first else number
                                    hint = if (sameFile && !same) "Select a range within one displayed hunk." else null
                                    selection = Pick(ReviewAnchor(id, file.key, side, minOf(first, number), maxOf(first, number)), hi, first)
                                }
                            }
                            cards(block.comments)
                        }
                    }
                }
            }
            if (feed.hunkRows.isNotEmpty() && sheet != Sheet.Editor && !WindowInsets.isImeVisible) {
                val current by remember(feed, list) { derivedStateOf {
                    val visible = list.layoutInfo.visibleItemsInfo
                    val pinned = visible.firstOrNull { (it.key as? String)?.startsWith("file:") == true && it.offset <= 0 }
                    val top = visible.firstOrNull { it != pinned && it.offset + it.size > (pinned?.let { p -> p.offset + p.size } ?: 0) }?.index ?: list.firstVisibleItemIndex
                    val found = feed.hunkRows.binarySearch(top)
                    if (found >= 0) found else -found - 2
                } }
                Surface(Modifier.align(Alignment.BottomEnd).padding(8.dp), shape = MaterialTheme.shapes.large, tonalElevation = 4.dp) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        IconButton({ jump(feed.hunkRows[current - 1]) }, enabled = current > 0) { Icon(Lucide.ChevronDown, "Previous hunk", Modifier.rotate(180f)) }
                        Text("${current + 1} / ${feed.hunkRows.size}", style = MaterialTheme.typography.labelMedium)
                        val next = current + 1
                        if (next <= feed.hunkRows.lastIndex) FloatingActionButton({ jump(feed.hunkRows[next]) }, Modifier.padding(start = 4.dp)) { Icon(Lucide.ChevronDown, "Next hunk") }
                        else IconButton({}, enabled = false) { Icon(Lucide.ChevronDown, "Next hunk") }
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            if (record != null) Text(when { state.saveError != null -> "Could not save"; state.saving -> "Saving…"; else -> "Saved" }, Modifier.weight(1f), style = MaterialTheme.typography.labelSmall)
            if (!draft && !state.submitting) record?.envelope?.let { TextButton({ agent(it.recipientAddress) }) { Text("Open agent") } }
            Button({ sheet = Sheet.Summary }, enabled = record != null) { Text("Review (${record?.comments?.size ?: 0})") }
        }
    }
    val shown = sheet
    if (shown != null) ModalBottomSheet(onDismissRequest = ::close, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        val title = when (shown) { Sheet.Files -> "Files"; Sheet.Bases -> "Locally available base refs"; Sheet.History -> "Reviews"; Sheet.Editor -> "Comment"; Sheet.Summary -> "Review summary"; Sheet.Batch -> "Submitted batch" }
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
            IconButton(::close) { Icon(Lucide.X, "Close $title") }
        }
        ReviewErrors(state)
        if (shown == Sheet.Summary) {
            if (state.submitting) Caption("Sending…") else if (!draft && record != null && state.error != record.status()) Caption(record.status())
            if (!state.connected) Caption("Disconnected · Send unavailable")
            if (state.workingAgents) Caption("An agent is still working in this Space. Files may continue changing.")
        }
        key(shown) { LazyColumn(Modifier.fillMaxWidth().imePadding(), contentPadding = PaddingValues(start = 12.dp, end = 12.dp, bottom = 24.dp)) {
            when (shown) {
                Sheet.Files -> {
                    if (state.files.isEmpty()) item { Caption("No files in this comparison.") }
                    items(state.files, key = { it.key.rowKey() }) { file ->
                        ListItem(headlineContent = { Text(file.key.label()) }, supportingContent = { Text(file.details()) }, modifier = Modifier.clickable { sheet = null; jump(feed.fileRows.getValue(file.key), true) })
                    }
                }
                Sheet.Bases -> {
                    if (state.branches.isEmpty()) item { Caption("No locally available branches.") }
                    items(state.branches, key = { it.ref }) { base ->
                        ListItem(headlineContent = { Text(base.ref) }, supportingContent = { Text(base.oid + if (base.suggested) " · Suggested" else "") }, modifier = Modifier.clickable(enabled = !state.submitting) { compare(ReviewComparison(ReviewMode.Branch, base.ref)) })
                    }
                }
                Sheet.History -> {
                    item { TextButton(::fresh, enabled = !state.submitting) { Text("New review") } }
                    if (state.history.isEmpty()) item { Caption("No reviews saved for this Space.") }
                    items(state.history, key = { it.id }) { r ->
                        ListItem(headlineContent = { Text("${r.comparison.label()} · ${r.status()}") }, supportingContent = { Text("${r.repo.root}\n${java.text.DateFormat.getDateTimeInstance().format(java.util.Date(r.updatedAtMs))} · ${r.comments.size} comments") },
                            trailingContent = { IconButton({ delete(r) }, enabled = !state.submitting) { Icon(Lucide.Trash2, "Delete review") } },
                            modifier = Modifier.clickable(enabled = !state.submitting) { leave { sheet = null; vm.showRecord(r.id) } })
                    }
                }
                Sheet.Editor -> if (composer != null) {
                    item { Caption(composer.anchor.location()); Snippet(composer.anchor, state) }
                    item { OutlinedTextField(composer.text, vm::updateComment, Modifier.fillMaxWidth().testTag("review_comment"), enabled = editing, minLines = 2, label = { Text("Comment text") }) }
                    item { Durability(state) }
                    item { Row {
                        Button(vm::saveComment, enabled = editing && composer.text.isNotBlank()) { Text("Save comment") }
                        TextButton(::discardComposer, enabled = !state.submitting) { Text("Discard") }
                    } }
                }
                Sheet.Summary -> if (record != null) {
                    item {
                        Text(record.repo.root, style = MaterialTheme.typography.titleMedium); Caption(record.spaceId); Caption(record.comparison.label())
                        snapshot?.let { Caption("HEAD ${it.headOid ?: "unborn"}" + (it.baseOid?.let { oid -> "\nBase $oid\nMerge base ${it.mergeBaseOid}" } ?: "")) }
                        if (draft) {
                            OutlinedTextField(record.message, vm::updateMessage, Modifier.fillMaxWidth().testTag("review_message"), enabled = editing, minLines = 2, label = { Text("Overall message") }); Durability(state)
                        } else if (record.message.isNotBlank()) Text(record.message)
                    }
                    if (draft && composer != null) item {
                        Caption("Save or discard the unfinished comment before sending.")
                        Row { TextButton({ sheet = Sheet.Editor }, enabled = editing) { Text("Resume comment") }; TextButton(::discardComposer, enabled = editing) { Text("Discard comment") } }
                    }
                    for (outdated in listOf(false, true)) {
                        val notes = record.comments.filter { it.needsReanchor == outdated }
                        if (notes.isNotEmpty()) item { Text(if (outdated) "Outdated · Re-anchor or remove before sending" else "Comments", style = MaterialTheme.typography.titleMedium) }
                        items(notes, key = { it.id }) { note ->
                            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                                Text(note.anchor.location(), style = MaterialTheme.typography.labelMedium); Text(note.text)
                                if (outdated) Snippet(note.anchor, state)
                                Row {
                                    if (!outdated) TextButton({ feed.commentRows[note.id]?.let { sheet = null; jump(it) } }) { Text("Show in diff") }
                                    else if (draft) TextButton({ if (composer != null) sheet = Sheet.Editor else { sheet = null; selection = null; reanchor = note.id } }, enabled = editing) { Text("Re-anchor") }
                                    if (draft) TextButton({ remove(note) }, enabled = editing) { Text("Remove") }
                                }
                            }; HorizontalDivider()
                        }
                    }
                    if (draft) {
                        item { Text("Recipient", style = MaterialTheme.typography.titleMedium); if (state.recipients.isEmpty()) Caption("No reporting agents available in this Space.") }
                        items(state.recipients, key = { it.address }) { r ->
                            val selected = record.recipientAddress == r.address
                            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(selected, enabled = editing && r.canSend && state.connected, role = Role.RadioButton) { vm.chooseRecipient(r.address) }, verticalAlignment = Alignment.CenterVertically) {
                                RadioButton(selected, null, enabled = editing && r.canSend && state.connected); Column(Modifier.padding(start = 8.dp)) { Text("${r.agent} · ${r.name}"); Caption(listOfNotNull(r.tab, r.rawStatus ?: "unknown", r.address).joinToString(" · ")) }
                            }
                        }
                        item { ChoiceRow(listOf("Discuss", "Apply"), record.intent.ordinal, editing) { vm.chooseIntent(ReviewIntent.entries[it]) } }
                        item { Caption("HC pastes and submits to the selected terminal. Readiness can change before input; HC cannot guarantee the reporting agent is still in front. Native blocked-dialog refusal remains enabled.") }
                        item { Button(vm::submit, enabled = editing && state.connected && (record.comments.isNotEmpty() || record.message.isNotBlank()) && composer == null && record.comments.none { it.needsReanchor } && state.recipients.any { it.address == record.recipientAddress && it.canSend }) { Text(if (state.submitting) "Sending…" else "Send") } }
                    } else {
                        item { record.envelope?.let { env ->
                            Caption("${env.recipientAgent} · ${env.recipientAddress} · ${env.intent}")
                            TextButton({ sheet = Sheet.Batch }) { Text("Read submitted batch") }
                            TextButton({ agent(env.recipientAddress) }, enabled = !state.submitting) { Text("Open agent") }
                        }
                            TextButton(::fresh, enabled = !state.submitting) { Text(if (record.phase == ReviewPhase.Sent) "New review" else "Create another review") }
                        }
                    }
                }
                Sheet.Batch -> {
                    val lines = record?.envelope?.batch?.let { batch -> batch.lines() }.orEmpty()
                    items(lines.size) { i -> SelectionContainer { Text(lines[i], fontFamily = GeistMono) } }
                }
            }
        } }
    }
    confirm?.let { Confirm(it, { confirm = null }) }
}

private fun ReviewFile.details() = listOfNotNull(status, when {
    previewUnavailable != null -> previewUnavailable
    conflict -> "Unmerged · preview unavailable"
    submodule -> "Submodule · nested changes not shown"
    binary -> "Binary · no inline preview"
    beforeMode != afterMode -> "${beforeMode ?: "—"} → ${afterMode ?: "—"}"
    hunks.isEmpty() -> "No inline changes"
    else -> null
}).joinToString(" · ")

@Composable
private fun Caption(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier.padding(horizontal = 8.dp, vertical = 2.dp), style = MaterialTheme.typography.bodySmall)
}

@Composable
private fun ReviewErrors(s: ReviewUiState) {
    s.error?.let { Text(it, Modifier.padding(horizontal = 8.dp), color = MaterialTheme.colorScheme.error) }
    s.saveError?.let { Text("Could not save: $it", Modifier.padding(horizontal = 8.dp), color = MaterialTheme.colorScheme.error) }
}

@Composable
private fun Durability(s: ReviewUiState) = Caption((if (s.saveError != null) "Could not save" else if (s.saving) "Saving…" else "Saved") + " · Closing the process before a save completes can lose unsaved text.")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChoiceRow(options: List<String>, selected: Int, enabled: Boolean, choose: (Int) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
        options.forEachIndexed { i, label -> SegmentedButton(i == selected, { if (i != selected) choose(i) }, SegmentedButtonDefaults.itemShape(i, options.size), enabled = enabled) { Text(label) } }
    }
}

@Composable
private fun DiffLine(line: ReviewLine, wrap: Boolean, editable: Boolean, selected: ReviewAnchor?, comment: (ReviewSide, Int) -> Unit, select: (ReviewSide, Int) -> Unit) {
    val colors = MaterialTheme.colorScheme
    val bg = when (line.kind) { ReviewLineKind.Addition -> colors.tertiaryContainer; ReviewLineKind.Deletion -> colors.errorContainer; ReviewLineKind.Context -> colors.surface }
    val fg = when (line.kind) { ReviewLineKind.Addition -> colors.onTertiaryContainer; ReviewLineKind.Deletion -> colors.onErrorContainer; ReviewLineKind.Context -> colors.onSurface }
    Row(Modifier.fillMaxWidth().background(bg), verticalAlignment = Alignment.Top) {
        for (side in ReviewSide.entries) {
            val n = line.number(side)
            val on = n != null && selected?.side == side && n >= (selected.startLine ?: 0) && n <= (selected.endLine ?: 0)
            Box(Modifier.widthIn(min = 48.dp).heightIn(min = 48.dp).then(if (on) Modifier.background(colors.primaryContainer) else Modifier)
                .then(if (n == null) Modifier else Modifier.semantics {
                    contentDescription = "$side line $n"
                    if (editable) customActions = listOf(CustomAccessibilityAction("Comment") { comment(side, n); true })
                }.clickable(enabled = editable, onClickLabel = "Select range") { select(side, n) }), contentAlignment = Alignment.Center) {
                Text(n?.toString().orEmpty(), textAlign = TextAlign.End, fontFamily = GeistMono, style = MaterialTheme.typography.labelSmall, color = if (on) colors.onPrimaryContainer else fg)
            }
        }
        Box(Modifier.weight(1f).then(if (wrap) Modifier else Modifier.horizontalScroll(rememberScrollState())).padding(vertical = 8.dp, horizontal = 4.dp)) {
            val mark = when (line.kind) { ReviewLineKind.Addition -> "+"; ReviewLineKind.Deletion -> "−"; ReviewLineKind.Context -> " " }
            Text("$mark ${line.text}" + if (line.noFinalNewline) "\n\\ No newline at end of file" else "", softWrap = wrap, style = MaterialTheme.typography.bodySmall, fontFamily = GeistMono, color = fg)
        }
    }
}

@Composable
private fun CommentCard(note: ReviewComment, record: ReviewRecord, editing: Boolean, edit: () -> Unit, remove: () -> Unit) {
    var expanded by rememberSaveable(note.id) { mutableStateOf(false) }
    Surface(Modifier.fillMaxWidth().padding(6.dp), shape = MaterialTheme.shapes.medium, color = MaterialTheme.colorScheme.secondaryContainer) {
        Column(Modifier.padding(8.dp)) {
            Column(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(onClickLabel = if (expanded) "Collapse comment" else "Expand comment") { expanded = !expanded }) {
                Text(note.anchor.location(), style = MaterialTheme.typography.labelMedium)
                Text(record.envelope?.let { "${record.status()} · ${it.recipientAgent} · ${it.intent}" } ?: "Draft", style = MaterialTheme.typography.labelSmall)
                Text(note.text, maxLines = if (expanded) Int.MAX_VALUE else 2, overflow = TextOverflow.Ellipsis)
            }
            if (record.phase == ReviewPhase.Draft) Row { TextButton(edit, enabled = editing) { Text("Edit") }; TextButton(remove, enabled = editing) { Text("Delete") } }
        }
    }
}

@Composable
private fun Snippet(anchor: ReviewAnchor, state: ReviewUiState) {
    val files = if (anchor.snapshotId == state.snapshot?.id) state.files else state.oldFiles[anchor.snapshotId].orEmpty()
    val file = remember(files, anchor.file) { files.firstOrNull { it.key == anchor.file } }
    val side = anchor.side
    val first = anchor.startLine
    val last = anchor.endLine
    if (file == null || side == null || first == null || last == null) {
        Caption(file?.details() ?: "Original preview unavailable")
        return
    }
    val lines = remember(file, anchor) {
        val hunk = file.hunks.firstOrNull { h ->
            val start = if (side == ReviewSide.Before) h.beforeStart else h.afterStart
            val count = if (side == ReviewSide.Before) h.beforeCount else h.afterCount
            first >= start && last < start.toLong() + count
        }
        val range = (first.toLong() - 3)..(last.toLong() + 3)
        hunk?.lines?.filter { line -> line.number(side)?.let { it.toLong() in range } == true }
    }
    if (lines.isNullOrEmpty()) Caption("Original preview unavailable")
    else LazyColumn(Modifier.fillMaxWidth().heightIn(max = 164.dp)) {
        items(lines.size) { i ->
            val line = lines[i]
            val n = line.number(side)!!
            Text("${if (n in first..last) ">" else " "} $n  ${line.text}", style = MaterialTheme.typography.bodySmall, fontFamily = GeistMono)
        }
    }
}
