// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

private fun removalReason(reason:WorktreeRemoveReason?):String=when(reason){
    WorktreeRemoveReason.TargetOccupied->"Close the checkout’s desktop terminals first."
    WorktreeRemoveReason.TargetDirty->"Tracked, untracked or ignored content prevents safe removal."
    WorktreeRemoveReason.TargetMasked->"Masked index entries prevent safe removal."
    WorktreeRemoveReason.TargetSubmodules->"Checkouts with submodules cannot be removed here."
    WorktreeRemoveReason.TargetChanged,WorktreeRemoveReason.SourceChangedBeforeEffect,WorktreeRemoveReason.SourceChangedAfterEffect->"The original checkout or source changed. Refresh worktrees."
    WorktreeRemoveReason.PreviewUnavailable,WorktreeRemoveReason.CandidateUnavailable,WorktreeRemoveReason.DeadlineBeforeEffect,WorktreeRemoveReason.DeadlineAfterEffect->"The preview expired or is unavailable. Refresh worktrees."
    WorktreeRemoveReason.OperationInProgress->"Another native checkout operation is in progress."
    WorktreeRemoveReason.OperationAlreadyConsumed->"This native operation was already consumed. Inspect its receipt."
    WorktreeRemoveReason.OperationCapacity,WorktreeRemoveReason.InvalidOperation,WorktreeRemoveReason.PlatformUnavailable,WorktreeRemoveReason.WorkerUnavailableBeforeEffect,WorktreeRemoveReason.PreparationPending,WorktreeRemoveReason.PreparationRefused,WorktreeRemoveReason.MutationUncertain,WorktreeRemoveReason.CleanupUnproved,WorktreeRemoveReason.Removed,null->"This checkout is unavailable for removal. Inspect the desktop and refresh."
}

@Composable
internal fun WorktreeRemovalDialog(scope:WorktreeRemovalState,model:GoatrViewModel){
    val preview=scope.context?.preview
    val action=scope.submittedAction?.let {id->model.focusedAction?.takeIf {it.metadata.actionId==id} ?: model.actions[scope.key.host].orEmpty().firstOrNull {it.metadata.actionId==id}}
    AlertDialog(
        onDismissRequest=model::closeWorktreeRemoval,
        title={UiText(if(scope.submittedAction==null)"Remove worktree" else "Worktree removal")},
        text={Column(Modifier.fillMaxWidth().heightIn(max=420.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)){
            when{
                scope.loading->UiText("Checking the original linked checkout…")
                scope.error->UiText("Preview unavailable. Check desktop occupancy and refresh worktrees.")
                preview!=null->{
                    SelectionContainer {UiText(preview.path,fontFamily=FontFamily.Monospace,softWrap=true,modifier=Modifier.fillMaxWidth().testTag("worktree_remove_path"))}
                    preview.branch?.let {UiText("Branch: $it",softWrap=true)}
                    preview.head?.let {UiText("Head: $it",style=MaterialTheme.typography.bodySmall,softWrap=true)}
                    if(scope.submittedAction==null){
                        UiText("Stop external tools using this checkout. Only a clean, closed linked checkout can be removed. The branch is kept.")
                        if(!preview.removable)UiText(removalReason(preview.reason),color=MaterialTheme.colorScheme.error)
                    }
                }
            }
            if(scope.submittedAction!=null){
                if(action!=null)ReceiptPanel(scope.key.host,action,model)
                else {UiText("Waiting for this action’s retained receipt. No automatic retry.");TextButton(onClick={model.inspect(scope.key.host,scope.submittedAction)},enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp)){IconLabel(MaterialGlyph.Refresh,"Check receipt")}}
                if(action?.receipt?.state==ReceiptState.Uncertain)UiText("Inspect the receipt and desktop before starting a separate action.")
            }
        }},
        confirmButton={
            if(scope.submittedAction==null && preview?.removable==true)TextButton(onClick={model.confirmWorktreeRemoval(scope)},enabled=!model.busy,colors=ButtonDefaults.textButtonColors(contentColor=MaterialTheme.colorScheme.error),modifier=Modifier.heightIn(min=48.dp).testTag("worktree_remove_confirm")){IconLabel(MaterialGlyph.Delete,"Remove checkout")}
            else TextButton(onClick=model::refreshWorktrees,enabled=!model.busy,modifier=Modifier.heightIn(min=48.dp).testTag("worktree_remove_refresh")){IconLabel(MaterialGlyph.Refresh,"Refresh worktrees")}
        },
        dismissButton={TextButton(onClick=model::closeWorktreeRemoval,modifier=Modifier.heightIn(min=48.dp).testTag("worktree_remove_cancel")){IconLabel(MaterialGlyph.Close,"Close")}},
        properties=DialogProperties(securePolicy=SecureFlagPolicy.SecureOn),
        modifier=Modifier.testTag("worktree_remove_dialog"),
    )
}
