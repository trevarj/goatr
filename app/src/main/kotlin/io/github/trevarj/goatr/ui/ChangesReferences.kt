// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text as UiText
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/**
 * A presentation-only projection of an already fetched Changes snapshot. It accepts only
 * ordinary, unquoted unified diffs whose paths match the snapshot's authoritative entries.
 */
internal enum class ChangesPatchSource(val label: String) { Staged("staged"), Unstaged("unstaged") }

internal sealed interface ChangesReferenceSelection {
    data class File(val path: String) : ChangesReferenceSelection
    data class Hunk(val source: ChangesPatchSource, val path: String, val hunkIndex: Int) : ChangesReferenceSelection
    data class Line(val source: ChangesPatchSource, val path: String, val hunkIndex: Int, val lineIndex: Int) : ChangesReferenceSelection
}

internal data class ChangesReferencePreview(
    val selection: ChangesReferenceSelection,
    val draftText: String,
    val sourceLabel: String,
    val path: String,
    val lineContext: String,
    val snapshotText: String,
)

internal data class ChangesPatchLine(
    val raw: String,
    val oldLine: Int?,
    val newLine: Int?,
)

internal data class ChangesPatchHunk(
    val header: String,
    val oldStart: Int,
    val oldCount: Int,
    val newStart: Int,
    val newCount: Int,
    val lines: List<ChangesPatchLine>,
) {
    val snapshotText: String get() = buildString {
        append(header)
        lines.forEach { append('\n').append(it.raw) }
    }
}

internal data class ChangesPatchFile(val source: ChangesPatchSource, val path: String, val hunks: List<ChangesPatchHunk>)
internal data class ChangesPatchProjection(val files: List<ChangesPatchFile>, val unavailable: Boolean)

internal fun projectChangesPatch(source: ChangesPatchSource, patch: String, entryPaths: Set<String>): ChangesPatchProjection {
    if (patch.isEmpty()) return ChangesPatchProjection(emptyList(), false)
    if ('\r' in patch) return ChangesPatchProjection(emptyList(), true)
    val lines = patch.split('\n').let { if (it.lastOrNull().isNullOrEmpty()) it.dropLast(1) else it }
    val files = mutableListOf<ChangesPatchFile>()
    var index = 0
    while (index < lines.size) {
        val start = lines[index]
        if (!start.startsWith("diff --git a/")) return ChangesPatchProjection(emptyList(), true)
        val remainder = start.removePrefix("diff --git a/")
        val split = remainder.indexOf(" b/")
        if (split <= 0 || remainder.indexOf(" b/", split + 1) >= 0) return ChangesPatchProjection(emptyList(), true)
        val path = remainder.substring(0, split)
        if (path !in entryPaths || path.contains('"') || path.contains('\\') || path.contains('\t') || remainder.substring(split + 3) != path) return ChangesPatchProjection(emptyList(), true)
        index += 1
        val added = lines.getOrNull(index)?.startsWith("new file mode ") == true
        val deleted = lines.getOrNull(index)?.startsWith("deleted file mode ") == true
        if (added || deleted) index += 1
        if (index >= lines.size || !lines[index].startsWith("index ")) return ChangesPatchProjection(emptyList(), true)
        index += 1
        val oldHeader = if (added) "--- /dev/null" else "--- a/$path"
        val newHeader = if (deleted) "+++ /dev/null" else "+++ b/$path"
        if (added && deleted || index + 1 >= lines.size || lines[index] != oldHeader || lines[index + 1] != newHeader) return ChangesPatchProjection(emptyList(), true)
        index += 2
        val hunks = mutableListOf<ChangesPatchHunk>()
        while (index < lines.size && !lines[index].startsWith("diff --git ")) {
            val parsed = parseChangesHunk(lines, index) ?: return ChangesPatchProjection(emptyList(), true)
            hunks += parsed.first
            if (hunks.size > 1024) return ChangesPatchProjection(emptyList(), true)
            index = parsed.second
        }
        files += ChangesPatchFile(source, path, hunks)
        if (files.size > 128) return ChangesPatchProjection(emptyList(), true)
    }
    return ChangesPatchProjection(files, false)
}

private fun parseChangesHunk(lines: List<String>, start: Int): Pair<ChangesPatchHunk, Int>? {
    val match = HUNK_HEADER.matchEntire(lines[start]) ?: return null
    val oldStart = parsePatchPosition(match.groupValues[1]) ?: return null
    val oldCount = parsePatchCount(match.groupValues[2]) ?: return null
    val newStart = parsePatchPosition(match.groupValues[3]) ?: return null
    val newCount = parsePatchCount(match.groupValues[4]) ?: return null
    if (oldCount > Int.MAX_VALUE - oldStart || newCount > Int.MAX_VALUE - newStart) return null
    var oldLine = oldStart
    var newLine = newStart
    val content = mutableListOf<ChangesPatchLine>()
    var index = start + 1
    while (index < lines.size && !lines[index].startsWith("diff --git ") && !lines[index].startsWith("@@ ")) {
        val raw = lines[index]
        when {
            raw == "\\ No newline at end of file" -> content += ChangesPatchLine(raw, null, null)
            raw.startsWith(' ') -> { content += ChangesPatchLine(raw, oldLine, newLine); oldLine += 1; newLine += 1 }
            raw.startsWith('-') -> { content += ChangesPatchLine(raw, oldLine, null); oldLine += 1 }
            raw.startsWith('+') -> { content += ChangesPatchLine(raw, null, newLine); newLine += 1 }
            else -> return null
        }
        if (content.size > 4096) return null
        index += 1
    }
    if (oldLine != oldStart + oldCount || newLine != newStart + newCount) return null
    return ChangesPatchHunk(lines[start], oldStart, oldCount, newStart, newCount, content) to index
}

internal fun resolveChangesReference(
    selection: ChangesReferenceSelection,
    stagedPatch: String,
    unstagedPatch: String,
    entryPaths: Set<String>,
): ChangesReferencePreview? = when (selection) {
    is ChangesReferenceSelection.File -> if (selection.path in entryPaths) ChangesReferencePreview(
        selection = selection,
        draftText = "File reference: ${selection.path}",
        sourceLabel = "snapshot entry",
        path = selection.path,
        lineContext = "File-level reference",
        snapshotText = selection.path,
    ) else null
    is ChangesReferenceSelection.Hunk -> resolvePatchSelection(selection, stagedPatch, unstagedPatch, entryPaths)
    is ChangesReferenceSelection.Line -> resolvePatchSelection(selection, stagedPatch, unstagedPatch, entryPaths)
}

private fun resolvePatchSelection(
    selection: ChangesReferenceSelection,
    stagedPatch: String,
    unstagedPatch: String,
    entryPaths: Set<String>,
): ChangesReferencePreview? {
    val source = when (selection) {
        is ChangesReferenceSelection.Hunk -> selection.source
        is ChangesReferenceSelection.Line -> selection.source
        is ChangesReferenceSelection.File -> return null
    }
    val path = when (selection) {
        is ChangesReferenceSelection.Hunk -> selection.path
        is ChangesReferenceSelection.Line -> selection.path
        is ChangesReferenceSelection.File -> return null
    }
    val hunkIndex = when (selection) {
        is ChangesReferenceSelection.Hunk -> selection.hunkIndex
        is ChangesReferenceSelection.Line -> selection.hunkIndex
        is ChangesReferenceSelection.File -> return null
    }
    val projection = projectChangesPatch(source, if (source == ChangesPatchSource.Staged) stagedPatch else unstagedPatch, entryPaths)
    if (projection.unavailable) return null
    val hunk = projection.files.singleOrNull { it.path == path }?.hunks?.getOrNull(hunkIndex) ?: return null
    val range = "old ${formatRange(hunk.oldStart, hunk.oldCount)}, new ${formatRange(hunk.newStart, hunk.newCount)}"
    return when (selection) {
        is ChangesReferenceSelection.Hunk -> ChangesReferencePreview(
            selection, "Change hunk reference: $path (${source.label}, $range)", source.label, path,
            "Hunk: $range", hunk.snapshotText,
        )
        is ChangesReferenceSelection.Line -> {
            val line = hunk.lines.getOrNull(selection.lineIndex) ?: return null
            if (line.oldLine == null && line.newLine == null) return null
            val lines = listOfNotNull(line.oldLine?.let { "old line $it" }, line.newLine?.let { "new line $it" }).joinToString(", ")
            ChangesReferencePreview(
                selection, "Change line reference: $path (${source.label}, $lines)", source.label, path,
                "$lines; hunk $range", "${hunk.header}\n${line.raw}",
            )
        }
        is ChangesReferenceSelection.File -> null
    }
}

private fun formatRange(start: Int, count: Int): String = if (count == 1) start.toString() else "$start,$count"
private fun parsePatchPosition(value: String): Int? = value.toIntOrNull()?.takeIf { it >= 0 }
private fun parsePatchCount(value: String): Int? = if (value.isEmpty()) 1 else value.toIntOrNull()?.takeIf { it >= 0 }
private val HUNK_HEADER = Regex("^@@ -(\\d+)(?:,(\\d+))? \\+(\\d+)(?:,(\\d+))? @@(?: .*)?$")

@Composable
internal fun ChangesPatchReferenceSections(
    stagedPatch: String,
    unstagedPatch: String,
    entryPaths: Set<String>,
    onReference: (ChangesReferenceSelection) -> Unit,
) {
    ChangesPatchSource.entries.forEach { source ->
        val patch = if (source == ChangesPatchSource.Staged) stagedPatch else unstagedPatch
        val projection = projectChangesPatch(source, patch, entryPaths)
        GoatrSection("${source.label.replaceFirstChar {it.uppercase()}} patch")
        when {
            patch.isEmpty() -> UiText("No ${source.label} patch", style = MaterialTheme.typography.bodySmall)
            projection.unavailable -> UiText(
                "Line and hunk references are unavailable for this raw native patch.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            else -> ChangesPatchReferencePicker(source, projection.files, onReference)
        }
        var rawDetails by remember(source,patch) {mutableStateOf(false)}
        DetailDisclosure(rawDetails,{rawDetails=!rawDetails},"changes_${source.label}_details")
        if(rawDetails) UiText(
            if(patch.isEmpty()) "No ${source.label} patch" else patch,
            style=MaterialTheme.typography.bodySmall,
            fontFamily=FontFamily.Monospace,
            softWrap=true,
            modifier=Modifier.testTag("changes_${source.label}_patch"),
        )
    }
}

private data class ChangesHunkChoice(val path: String, val hunkIndex: Int, val hunk: ChangesPatchHunk)

@Composable
private fun ChangesPatchReferencePicker(
    source: ChangesPatchSource,
    files: List<ChangesPatchFile>,
    onReference: (ChangesReferenceSelection) -> Unit,
) {
    val choices = files.flatMap { file -> file.hunks.mapIndexed { index, hunk -> ChangesHunkChoice(file.path, index, hunk) } }
    if (choices.isEmpty()) {
        UiText("This patch has no line hunks to reference.", style = MaterialTheme.typography.bodySmall)
        return
    }
    var hunkPage by remember(files) { mutableStateOf(0) }
    var expanded by remember(files) { mutableStateOf<ChangesHunkChoice?>(null) }
    val pageSize = 8
    val pageStart = (hunkPage * pageSize).coerceAtMost(((choices.size - 1) / pageSize) * pageSize)
    val page = choices.subList(pageStart, minOf(pageStart + pageSize, choices.size))
    UiText("Choose a snapshot hunk", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    var previousFile:String?=null
    page.forEach { choice ->
        if(previousFile!=choice.path) GoatrSection(choice.path,"${source.label} file")
        previousFile=choice.path
        val hunk = choice.hunk
        GroupedSurface(Modifier.testTag("changes_${source.label}_hunk")) {
            UiText("Hunk ${choice.hunkIndex+1}",style=MaterialTheme.typography.titleSmall)
            UiText("Old ${formatRange(hunk.oldStart, hunk.oldCount)} · new ${formatRange(hunk.newStart, hunk.newCount)}", style = MaterialTheme.typography.bodySmall)
            FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                TextButton(onClick={onReference(ChangesReferenceSelection.Hunk(source,choice.path,choice.hunkIndex))},modifier=Modifier.heightIn(min=48.dp).testTag("changes_reference_hunk")) {IconLabel(MaterialGlyph.Add,"Add hunk reference")}
                TextButton(onClick={expanded=if(expanded==choice) null else choice},modifier=Modifier.heightIn(min=48.dp).testTag("changes_reference_lines")) {IconLabel(MaterialGlyph.ExpandMore,if(expanded==choice) "Hide line choices" else "Choose a line")}
            }
            if(expanded==choice) ChangesLineReferencePicker(source,choice,onReference)
        }
    }
    if (choices.size > pageSize) Row(Modifier.fillMaxWidth()) {
        TextButton(
            onClick = { hunkPage -= 1 }, enabled = pageStart > 0,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("changes_hunks_previous"),
        ) { IconLabel(MaterialGlyph.Back, "Previous hunks") }
        TextButton(
            onClick = { hunkPage += 1 }, enabled = pageStart + pageSize < choices.size,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("changes_hunks_next"),
        ) { IconLabel(MaterialGlyph.Add, "Next hunks") }
    }
}

@Composable
private fun ChangesLineReferencePicker(
    source: ChangesPatchSource,
    choice: ChangesHunkChoice,
    onReference: (ChangesReferenceSelection) -> Unit,
) {
    val selectable = choice.hunk.lines.mapIndexedNotNull { index, line -> if (line.oldLine == null && line.newLine == null) null else index to line }
    if (selectable.isEmpty()) {
        UiText("This hunk has no selectable line positions.", style = MaterialTheme.typography.bodySmall)
        return
    }
    var linePage by remember(choice) { mutableStateOf(0) }
    val pageSize = 12
    val pageStart = (linePage * pageSize).coerceAtMost(((selectable.size - 1) / pageSize) * pageSize)
    selectable.subList(pageStart, minOf(pageStart + pageSize, selectable.size)).forEach { (lineIndex, line) ->
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            UiText("Old ${line.oldLine?.toString() ?: "—"} · new ${line.newLine?.toString() ?: "—"}",style=MaterialTheme.typography.labelMedium)
            UiText(line.raw, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace, softWrap = true)
            TextButton(
                onClick = { onReference(ChangesReferenceSelection.Line(source, choice.path, choice.hunkIndex, lineIndex)) },
                modifier = Modifier.heightIn(min = 48.dp).testTag("changes_reference_line"),
            ) { IconLabel(MaterialGlyph.Add, "Add line reference") }
        }
    }
    if (selectable.size > pageSize) Row(Modifier.fillMaxWidth()) {
        TextButton(
            onClick = { linePage -= 1 }, enabled = pageStart > 0,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("changes_lines_previous"),
        ) { IconLabel(MaterialGlyph.Back, "Previous lines") }
        TextButton(
            onClick = { linePage += 1 }, enabled = pageStart + pageSize < selectable.size,
            modifier = Modifier.weight(1f).heightIn(min = 48.dp).testTag("changes_lines_next"),
        ) { IconLabel(MaterialGlyph.Add, "Next lines") }
    }
}
