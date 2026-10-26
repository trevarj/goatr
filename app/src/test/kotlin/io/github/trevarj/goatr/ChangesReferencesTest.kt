// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import org.junit.Assert.*
import org.junit.Test

class ChangesReferencesTest {
    private val staged="diff --git a/file.txt b/file.txt\nindex 1111111..2222222 100644\n--- a/file.txt\n+++ b/file.txt\n@@ -2,2 +2,3 @@\n context\n-before staged\n+after staged\n+new staged\n"
    private val unstaged="diff --git a/file.txt b/file.txt\nindex 2222222..3333333 100644\n--- a/file.txt\n+++ b/file.txt\n@@ -40 +41 @@\n-before worktree\n+after worktree\n"
    @Test fun references_keep_exact_displayed_stage_line_and_hunk_coordinates() {
        val paths=setOf("file.txt")
        val stagedProjection=projectChangesPatch(ChangesPatchSource.Staged,staged,paths)
        val stagedLines=stagedProjection.files.single().hunks.single().lines
        assertEquals(3,stagedLines[1].oldLine);assertNull(stagedLines[1].newLine)
        assertNull(stagedLines[3].oldLine);assertEquals(4,stagedLines[3].newLine)
        val selected=ChangesReferenceSelection.Line(ChangesPatchSource.Staged,"file.txt",0,1)
        val reference=resolveChangesReference(selected,staged,unstaged,paths)!!
        assertEquals(selected,reference.selection);assertEquals("file.txt",reference.path)
        assertEquals(ChangesPatchSource.Staged.label,reference.sourceLabel)
        assertEquals("@@ -2,2 +2,3 @@\n-before staged",reference.snapshotText)
        val worktreeSelection=ChangesReferenceSelection.Line(ChangesPatchSource.Unstaged,"file.txt",0,1)
        val worktree=resolveChangesReference(worktreeSelection,staged,unstaged,paths)!!
        assertEquals(ChangesPatchSource.Unstaged.label,worktree.sourceLabel)
        assertEquals("@@ -40 +41 @@\n+after worktree",worktree.snapshotText)
        val worktreeLine=projectChangesPatch(ChangesPatchSource.Unstaged,unstaged,paths).files.single().hunks.single().lines[1]
        assertNull(worktreeLine.oldLine);assertEquals(41,worktreeLine.newLine)
        val hunk=resolveChangesReference(ChangesReferenceSelection.Hunk(ChangesPatchSource.Staged,"file.txt",0),staged,unstaged,paths)!!
        assertEquals(stagedProjection.files.single().hunks.single().snapshotText,hunk.snapshotText)
    }
    @Test fun binary_rename_and_foreign_patch_rows_never_invent_line_references() {
        for(patch in listOf("diff --git a/file.txt b/file.txt\nindex 1111111..2222222 100644\nBinary files a/file.txt and b/file.txt differ\n",
            "diff --git a/old.txt b/file.txt\nsimilarity index 100%\nrename from old.txt\nrename to file.txt\n",staged.replace("file.txt","foreign.txt"))) {
            assertTrue(projectChangesPatch(ChangesPatchSource.Staged,patch,setOf("file.txt")).unavailable)
            assertNull(resolveChangesReference(ChangesReferenceSelection.Hunk(ChangesPatchSource.Staged,"file.txt",0),patch,"",setOf("file.txt")))
            assertNotNull(resolveChangesReference(ChangesReferenceSelection.File("file.txt"),patch,"",setOf("file.txt")))
        }
        assertNull(resolveChangesReference(ChangesReferenceSelection.File("foreign.txt"),staged,unstaged,setOf("file.txt")))
    }
    @Test fun marker_and_out_of_range_selections_have_no_fabricated_positions() {
        val patch="diff --git a/file.txt b/file.txt\nindex 1111111..2222222 100644\n--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n"
        for(selection in listOf(ChangesReferenceSelection.Line(ChangesPatchSource.Staged,"file.txt",0,2),
            ChangesReferenceSelection.Line(ChangesPatchSource.Staged,"file.txt",0,3),ChangesReferenceSelection.Hunk(ChangesPatchSource.Staged,"file.txt",1))) {
            assertNull(resolveChangesReference(selection,patch,"",setOf("file.txt")))
        }
    }
}
