// The desktop's worktree dialogs (herdr-app mods/worktree), from a git
// space's menu: a new branch's checkout in a new space, a checkout to open
// (its space if one shows it), and removing the one a space is on. Their
// words are the desktop's; git does the work on the space's machine.

package io.github.trevarj.goatr.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.trevarj.goatr.app.LayoutViewModel
import io.github.trevarj.goatr.app.session.Space
import io.github.trevarj.goatr.app.session.reason
import herdr.core.Checkout
import herdr.core.RemoveRisk
import herdr.core.Repo
import herdr.core.ago
import herdr.core.changed
import herdr.core.seen
import herdr.core.tilde
import herdr.core.worktreeRoot
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

enum class WorktreeAction { New, Open, Remove }

/** Runs `block`; what it throws shows in the dialog instead of closing it. */
private suspend fun guarded(onError: (String) -> Unit, block: suspend () -> Unit): Boolean = try {
    block()
    true
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    onError(e.reason)
    false
}

@Composable
fun WorktreeDialog(action: WorktreeAction, space: Space, repo: Repo, vm: LayoutViewModel, onClose: () -> Unit) {
    val home by produceState("") { value = runCatching { vm.home(space) }.getOrDefault("") }
    when (action) {
        WorktreeAction.New -> NewWorktree(space, repo, home, vm, onClose)
        WorktreeAction.Open -> OpenWorktree(space, repo, home, vm, onClose)
        WorktreeAction.Remove -> RemoveWorktree(space, repo, home, vm, onClose)
    }
}

@Composable
private fun NewWorktree(space: Space, repo: Repo, home: String, vm: LayoutViewModel, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    var branch by remember { mutableStateOf("") }
    var error by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    // Ready to type the branch, as the desktop's dialog.
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    val create = {
        if (!busy) scope.launch {
            busy = true
            if (guarded({ error = it }) { vm.newWorktree(space, repo, branch) }) onClose()
            busy = false
        }
    }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("New worktree") },
        text = {
            Column {
                Text(
                    "A new space on a checkout in ${tilde(worktreeRoot(home), home)}/${repo.name}/. " +
                        "A branch that exists is checked out; a new one starts from here.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedTextField(
                    branch,
                    { branch = it },
                    Modifier.padding(top = 12.dp).fillMaxWidth().focusRequester(focus),
                    placeholder = { Text("branch name") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { create() }),
                )
                Problem(error)
            }
        },
        confirmButton = { TextButton({ create() }, enabled = !busy) { Text(if (busy) "working…" else "Create") } },
        dismissButton = { TextButton(onClose) { Text("Cancel") } },
    )
}

@Composable
private fun OpenWorktree(space: Space, repo: Repo, home: String, vm: LayoutViewModel, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    var filter by remember { mutableStateOf("") }
    var error by remember { mutableStateOf("") }
    var all by remember { mutableStateOf<List<Checkout>?>(null) }
    LaunchedEffect(repo) { guarded({ error = it }) { all = vm.checkouts(space, repo) } }
    val list = all.orEmpty()
    val query = filter.trim().lowercase()
    val rows = list.filter { query.isEmpty() || "${it.branch ?: ""} ${it.path}".lowercase().contains(query) }
    val now = System.currentTimeMillis() / 1000
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Open worktree") },
        text = {
            Column {
                OutlinedTextField(
                    filter,
                    { filter = it },
                    Modifier.fillMaxWidth(),
                    placeholder = { Text("filter worktrees") },
                    singleLine = true,
                )
                Text(
                    if (all == null) "…" else if (query.isEmpty()) "${list.size} checkouts" else "${rows.size}/${list.size} checkouts",
                    Modifier.padding(top = 8.dp, bottom = 4.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (all != null && rows.isEmpty()) Text("No matching worktrees.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                LazyColumn(Modifier.heightIn(max = 320.dp)) {
                    items(rows, key = { it.path }) { checkout ->
                        ListItem(
                            headlineContent = { Text(checkout.branch ?: "detached", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            supportingContent = {
                                Text(
                                    listOfNotNull(tilde(checkout.path, home), "main checkout".takeIf { checkout.main }).joinToString(" · "),
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            },
                            trailingContent = { if (checkout.at > 0) Text(ago(checkout.at, now), style = MaterialTheme.typography.labelSmall) },
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                            modifier = Modifier.clickable {
                                scope.launch { if (guarded({ error = it }) { vm.openCheckout(space, checkout, list) }) onClose() }
                            },
                        )
                    }
                }
                Problem(error)
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClose) { Text("Close") } },
    )
}

@Composable
private fun RemoveWorktree(space: Space, repo: Repo, home: String, vm: LayoutViewModel, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    var error by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    // What was at risk when it opened; risk that appears since earns one more warning.
    var shown by remember { mutableStateOf<RemoveRisk?>(null) }
    LaunchedEffect(repo) { guarded({ error = it }) { shown = vm.removeRisk(space, repo) } }
    val remove = {
        val was = shown
        if (!busy && was != null) scope.launch {
            busy = true
            val ok = guarded({ error = it }) {
                val now = vm.removeRisk(space, repo)
                if (changed(was, now)) {
                    shown = seen(was, now)
                    error = "It changed since this opened. Remove again removes it anyway."
                    return@guarded
                }
                // The space closes first, and this dialog with it.
                vm.removeWorktree(space, repo, seen(was, now).force)
                onClose()
            }
            if (!ok) shown = was
            busy = false
        }
    }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text("Remove worktree ${space.name}?") },
        text = {
            Column {
                Text("Closes the space and deletes ${tilde(repo.top, home)}. The branch stays.")
                val risk = shown
                if (risk?.dirty == true) Problem("It has uncommitted changes: they will be lost.")
                if (risk != null && risk.unpushed.isNotEmpty()) {
                    Problem("Submodule ${risk.unpushed.joinToString(", ")} has commits that aren't pushed anywhere: they will be lost.")
                }
                Problem(error)
            }
        },
        confirmButton = {
            TextButton({ remove() }, enabled = !busy && shown != null) {
                Text(if (busy) "working…" else "Remove", color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = { TextButton(onClose) { Text("Cancel") } },
    )
}

@Composable
private fun Problem(text: String) {
    if (text.isNotEmpty()) Text(text, Modifier.padding(top = 8.dp), color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
}
