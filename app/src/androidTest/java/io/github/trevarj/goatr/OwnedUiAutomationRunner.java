// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr;

import android.app.Activity;
import android.app.Instrumentation;
import android.app.UiAutomation;
import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.AccessibilityServiceInfo;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

import java.io.File;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.HashSet;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.regex.Pattern;
import org.json.JSONObject;
import java.security.KeyFactory;
import java.security.Signature;
import java.security.spec.X509EncodedKeySpec;
import java.util.Collections;

/**
 * Test-package-only native UI owner. Ordinary observer actions never start the product.
 * Delivery helpers use the admitted instrumentation to click one existing owned OS alert;
 * they never substitute a direct component start for that native user interaction.
 */
public final class OwnedUiAutomationRunner extends Instrumentation {
    private Bundle runnerArguments = new Bundle();
    private Request activeRequest;

    @Override
    public void onCreate(Bundle arguments) {
        runnerArguments = arguments == null ? new Bundle() : new Bundle(arguments);
        super.onCreate(arguments);
        start();
    }

    @Override
    public void onStart() {
        final long preflightDeadline = SystemClock.elapsedRealtime() + TIMEOUT_MILLIS;
        RunnerIdentity self = new RunnerIdentity(Process.myPid(), 0L);
        Request request = null;
        StageTracker stage = new StageTracker(Stage.PREFLIGHT);
        Report report = Report.empty(self, "request", stage.stage, ErrorFamily.NONE);
        Observation observation = Observation.empty();
        Acknowledgement acknowledgement = new Acknowledgement();
        boolean expectedScreenObserved = false;
        boolean success = false;
        try {
            stage.stage = Stage.PREFLIGHT;
            self = runnerIdentity();
            report = Report.empty(self, "request", stage.stage, ErrorFamily.NONE);
            request = requiredRequest();
            requireEligible(getContext(), getTargetContext());
            requireBeforeDeadline(preflightDeadline);
            UiAutomation physicalAutomation=request.physical==null?null:getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
            requireBeforeDeadline(preflightDeadline);
            stage.stage = Stage.INTENT;
            File cacheDir = getContext().getCacheDir();
            writeExclusive(cacheDir, intentFile(getContext(), request.runId), intentJson(request, self));
            File releaseDirectory = releaseDirectory(cacheDir, request.runId);
            requireCondition(!releaseDirectory.exists(), "release_marker_existing");
            Bundle processStatus = new Bundle();
            processStatus.putString(PROCESS_STATUS_KEY, processJson(request, self));
            sendStatus(1, processStatus);
            awaitReleaseDirectory(releaseDirectory, SystemClock.elapsedRealtime() + PRE_INPUT_RELEASE_TIMEOUT_MILLIS);
            final long deadline = SystemClock.elapsedRealtime() + TIMEOUT_MILLIS;
            activeRequest = request;
            requirePhysicalEpoch(request);
            requireBeforeDeadline(deadline);
            stage.stage = Stage.AUTOMATION;
            UiAutomation automation = physicalAutomation!=null?physicalAutomation:getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
            InteractiveWindowsService interactiveWindows = needsInteractiveWindows(request.action) ? enableInteractiveWindows(automation) : null;
            try {
            stage.stage = Stage.OBSERVATION;
            if (request.action == Action.BACKGROUND_OWNED_APP || request.action == Action.OPEN_TERMINAL || request.action == Action.OPEN_WORKTREES || request.action == Action.OPEN_CHANGES) {
                exactConversationWindow(automation, request.sessionId, request.sessionGeneration, deadline);
            } else if (request.action == Action.CLOSE_TERMINAL || request.action == Action.OBSERVE_TERMINAL) {
                terminalTargetWindow(automation, request.sessionId, request.sessionGeneration, deadline);
            } else if (request.action == Action.REFRESH_WORKTREES || request.action == Action.CLOSE_WORKTREES || request.action == Action.OBSERVE_WORKTREES) {
                worktreesTargetWindow(automation, request.sessionId, request.sessionGeneration, deadline);
            } else if (request.action == Action.REFRESH_CHANGES || request.action == Action.CLOSE_CHANGES || request.action == Action.OBSERVE_CHANGES) {
                changesTargetWindow(automation, request.sessionId, request.sessionGeneration, deadline);
            }
            Observation before = request.action.isPhysicalAction() || request.physical!=null&&request.action==Action.OBSERVE || request.action.isRouteAction() || request.action == Action.BACKGROUND_OWNED_APP || request.action == Action.OBSERVE_CREATION_PICKER || request.action == Action.CONTINUE_OWNED_CREATION_SERVER || request.action == Action.CONTINUE_OWNED_CREATION_PROJECT
                    ? Observation.empty()
                    : observe(automation, deadline);
            observation = before;
            ActionResult result;
            if (request.action.isPhysicalAction() || request.physical!=null&&request.action.physicalHostNavigation()) {
                result = physicalAction(automation, request, deadline, acknowledgement, stage);
            } else if (request.action.isRouteAction()) {
                result = ownedRouteAction(automation, request, deadline, acknowledgement, stage, observation);
            } else if (request.action == Action.OBSERVE) {
                if(request.physical!=null)physicalObservation(automation,request,deadline);
                expectedScreenObserved = true;
                result = new ActionResult(false, true, before);
            } else if (request.action == Action.OBSERVE_APPEARANCE) {
                result = new ActionResult(false, true, observeAppearance(automation, deadline));
            } else if (request.action == Action.OPEN_SESSION_LIST_SETTINGS) {
                result = clickAndObserve(automation, request.action, deadline, acknowledgement, stage);
            } else if (request.action == Action.OBSERVE_SESSION_LIST_PREFERENCES) {
                result = new ActionResult(false, true, observeSessionListPreferences(automation, deadline));
            } else if (request.action == Action.SET_SESSION_LIST_PREFERENCE) {
                result = setSessionListPreference(automation, request.sessionListPreference, request.sessionListPreferenceValue.booleanValue(), deadline, acknowledgement, stage);
            } else if (request.action == Action.BACKGROUND_OWNED_APP) {
                result = backgroundOwnedApp(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.OBSERVE_CREATION_PICKER) {
                result = observeCreationPicker(automation, deadline);
            } else if (request.action == Action.SELECT_APPEARANCE) {
                result = selectAppearance(automation, request.appearance, deadline, acknowledgement, stage);
            } else if (request.action == Action.SELECT_OWNED_CREATION_SERVER) {
                result = selectOwnedCreationPicker(automation, CREATE_SERVER_TAG, CREATE_SERVER_OPTION_TAG_PREFIX, OWNED_CREATION_SERVER, request.creationServerId, true, deadline, acknowledgement, stage);
            } else if (request.action == Action.SELECT_OWNED_CREATION_PROJECT) {
                result = selectOwnedCreationPicker(automation, CREATE_PROJECT_TAG, CREATE_PROJECT_OPTION_TAG_PREFIX, request.creationProjectLabel, null, false, deadline, acknowledgement, stage);
            } else if (request.action == Action.CONTINUE_OWNED_CREATION_SERVER) {
                result = continueOwnedCreationServer(automation, request.creationServerId, deadline, acknowledgement, stage);
            } else if (request.action == Action.CONTINUE_OWNED_CREATION_PROJECT) {
                result = continueOwnedCreationProject(automation, request.creationProjectLabel, deadline, acknowledgement, stage);
            } else if (request.action == Action.SET_CREATION_NAME) {
                result = setCreationName(automation, request.creationName, request.previousCreationName, deadline, acknowledgement, stage);
            } else if (request.action == Action.SUBMIT_CREATION) {
                result = submitCreation(automation, request.creationName, request.creationServerLabel, request.creationServerId, request.creationProjectLabel, request.excludedReceiptIds, deadline, acknowledgement, stage);
            } else if (request.action == Action.SET_COMPOSER) {
                result = setComposer(automation, request.sessionId, request.sessionGeneration, request.composerText, deadline, acknowledgement, stage);
            } else if (request.action == Action.SUBMIT_PROMPT) {
                result = submitPrompt(automation, request.sessionId, request.sessionGeneration, request.composerText, deadline, acknowledgement, stage);
            } else if (request.action == Action.SELECT_CODEX_PROVIDER) {
                result = selectCodexProvider(automation, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_SESSION) {
                result = openSession(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_HISTORY_DETAILS) {
                result = openHistoryDetails(automation, request.sessionId, request.sessionGeneration, request.historyRecordId, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_HISTORY_DETAILS) {
                result = closeHistoryDetails(automation, request.sessionId, request.sessionGeneration, request.historyRecordId, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_SNIPPETS) {
                result = openScopedComposerSheet(automation, request.sessionId, request.sessionGeneration, COMPOSER_SNIPPETS_TAG, SNIPPET_SCOPE_TAG_PREFIX, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_SNIPPETS) {
                result = closeScopedComposerSheet(automation, request.sessionId, request.sessionGeneration, SNIPPET_SCOPE_TAG_PREFIX, SNIPPET_CLOSE_TAG, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_SWITCHER) {
                result = openScopedComposerSheet(automation, request.sessionId, request.sessionGeneration, SWITCHER_TAG, SWITCHER_SCOPE_TAG_PREFIX, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_SWITCHER) {
                result = closeScopedComposerSheet(automation, request.sessionId, request.sessionGeneration, SWITCHER_SCOPE_TAG_PREFIX, SWITCHER_CLOSE_TAG, deadline, acknowledgement, stage);
            } else if (request.action == Action.REFRESH_CONVERSATION) {
                result = refreshConversation(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.SCROLL_HISTORY_BACKWARD) {
                result = conversationControl(automation, request.sessionId, request.sessionGeneration, CONVERSATION_LIST_TAG, AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD, deadline, acknowledgement, stage);
            } else if (request.action == Action.FOCUS_COMPOSER) {
                result = focusComposer(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.DISMISS_COMPOSER_KEYBOARD) {
                result = dismissComposerKeyboard(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_TERMINAL) {
                result = openTerminal(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_TERMINAL) {
                result = closeTerminal(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_WORKTREES) {
                result = openWorktrees(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.REFRESH_WORKTREES) {
                result = refreshWorktrees(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_WORKTREES) {
                result = closeWorktrees(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_CHANGES) {
                result = openChanges(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.REFRESH_CHANGES) {
                result = refreshChanges(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_CHANGES) {
                result = closeChanges(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.NEW_SNIPPET) {
                result = newSnippet(automation, request.sessionId, request.sessionGeneration, deadline, acknowledgement, stage);
            } else if (request.action == Action.SET_SNIPPET_TITLE || request.action == Action.SET_SNIPPET_BODY) {
                result = setSnippetField(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.SAVE_SNIPPET) {
                result = saveSnippet(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.EDIT_SNIPPET) {
                result = editSnippet(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.INSERT_SNIPPET) {
                result = insertSnippet(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_SNIPPET_DELETE) {
                result = openSnippetDelete(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.CONFIRM_SNIPPET_DELETE) {
                result = confirmSnippetDelete(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.OBSERVE_SNIPPET) {
                result = observeSnippet(automation, request, deadline);
            } else if (request.action == Action.OBSERVE_SNIPPET_DRAFT) {
                result = observeSnippetDraft(automation, request, deadline);
            } else if (request.action == Action.CLEAR_OWNED_SNIPPET_DRAFT) {
                result = clearOwnedSnippetDraft(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.CLOSE_SNIPPET_EDITOR) {
                result = closeSnippetEditor(automation, request, deadline, acknowledgement, stage);
            } else if (request.action == Action.OBSERVE_TERMINAL) {
                if(request.physical!=null)physicalAgentReadOnly(automation,request,deadline);
                expectedScreenObserved = true;
                result = new ActionResult(false, true, before);
            } else if (request.action == Action.OBSERVE_WORKTREES) {
                expectedScreenObserved = true;
                result = new ActionResult(false, true, before);
            } else if (request.action == Action.OBSERVE_CHANGES) {
                expectedScreenObserved = true;
                result = new ActionResult(false, true, before);
            } else if (request.action == Action.INSPECT_RECEIPT) {
                result = inspectReceipt(automation, request.receiptId, deadline, acknowledgement, stage);
            } else if (request.action == Action.OPEN_RECEIPT_DETAILS) {
                result = openReceiptDetails(automation, request.receiptId, deadline, acknowledgement, stage);
            } else if (request.action == Action.PREPARE_SEPARATE_CREATION) {
                result = prepareSeparateCreation(automation, request.receiptId, deadline, acknowledgement, stage);
            } else if (request.action == Action.CONFIRM_SEPARATE_CREATION) {
                result = confirmSeparateCreation(automation, request.receiptId, deadline, acknowledgement, stage);
            } else if (request.action == Action.SCROLL_HOST_SETTINGS) {
                result = scrollHostSettings(automation, deadline, acknowledgement, stage);
            } else if (request.action == Action.SCROLL_SESSIONS) {
                result = scrollSessions(automation, deadline, acknowledgement, stage);
            } else {
                result = clickAndObserve(automation, request.action, deadline, acknowledgement, stage);
            }
            observation = result.observation;
            expectedScreenObserved = result.expectedScreenObserved;
            requireBeforeDeadline(deadline);
            } finally {
                if (interactiveWindows != null) restoreInteractiveWindows(automation, interactiveWindows);
            }
            stage.stage = Stage.OUTCOME;
            report = new Report(request.runId, request.action, "completed", "none", stage.stage, ErrorFamily.NONE, acknowledgement.value, expectedScreenObserved, observation, self);
            writeExclusive(getContext().getCacheDir(), outcomeFile(getContext(), request.runId), report.json());
            success = true;
        } catch (Throwable failure) {
            String blocked = blockedCode(failure);
            if (request != null) {
                report = new Report(request.runId, request.action, "blocked", blocked, stage.stage, errorFamily(failure), acknowledgement.value, expectedScreenObserved, observation, self);
                try {
                    writeExclusive(getContext().getCacheDir(), outcomeFile(getContext(), request.runId), report.json());
                } catch (Throwable ignored) {
                    // Final closed status still reports the original blocked result.
                }
            } else {
                report = Report.empty(self, blocked, stage.stage, errorFamily(failure));
            }
        }
        Bundle finalStatus = new Bundle();
        finalStatus.putString(STATUS_KEY, report.json());
        sendStatus(1, finalStatus);
        finish(success ? Activity.RESULT_OK : Activity.RESULT_CANCELED, finalStatus);
    }

    private Request requiredRequest() {
        requireKnownArguments(runnerArguments);
        String runId = runnerArguments.getString(RUN_ID_ARGUMENT);
        if (runId == null) throw new Blocked("run_id_missing");
        try {
            if (!UUID.fromString(runId).toString().equals(runId)) throw new Blocked("run_id_invalid");
        } catch (IllegalArgumentException exception) {
            throw new Blocked("run_id_invalid");
        }
        String rawAction = runnerArguments.getString(ACTION_ARGUMENT);
        Action action = Action.fromWire(rawAction);
        if (action == null) throw new Blocked("action_invalid");
        String rawAppearance = runnerArguments.getString(APPEARANCE_ARGUMENT);
        UiAppearance appearance = UiAppearance.fromWire(rawAppearance);
        if (action.requiresAppearance()) {
            if (appearance == null) throw new Blocked("appearance_invalid");
        } else if (rawAppearance != null) {
            throw new Blocked("appearance_unexpected");
        }
        String rawSessionListPreference = runnerArguments.getString(SESSION_LIST_PREFERENCE_ARGUMENT);
        SessionListPreference sessionListPreference = SessionListPreference.fromWire(rawSessionListPreference);
        String rawSessionListPreferenceValue = runnerArguments.getString(SESSION_LIST_PREFERENCE_VALUE_ARGUMENT);
        Boolean sessionListPreferenceValue = null;
        if (action.requiresSessionListPreference()) {
            if (sessionListPreference == null) throw new Blocked("session_list_preference_invalid");
            if ("true".equals(rawSessionListPreferenceValue)) sessionListPreferenceValue = true;
            else if ("false".equals(rawSessionListPreferenceValue)) sessionListPreferenceValue = false;
            else throw new Blocked("session_list_preference_value_invalid");
        } else if (rawSessionListPreference != null || rawSessionListPreferenceValue != null) {
            throw new Blocked("session_list_preference_unexpected");
        }
        String creationProjectLabel = runnerArguments.getString(CREATION_PROJECT_LABEL_ARGUMENT);
        if (action.requiresCreationProjectLabel()) {
            if (creationProjectLabel == null || !CREATION_PROJECT_LABEL_PATTERN.matcher(creationProjectLabel).matches()) throw new Blocked("creation_project_label_invalid");
        } else if (!action.allowsOwnedCreationForm() && creationProjectLabel != null) {
            throw new Blocked("creation_project_label_unexpected");
        }
        String creationServerLabel = runnerArguments.getString(CREATION_SERVER_LABEL_ARGUMENT);
        if (action.allowsOwnedCreationForm()) {
            boolean explicitOwnedForm = creationServerLabel != null || creationProjectLabel != null;
            if (explicitOwnedForm && (!OWNED_CREATION_SERVER.equals(creationServerLabel) || creationProjectLabel == null || !CREATION_PROJECT_LABEL_PATTERN.matcher(creationProjectLabel).matches())) {
                throw new Blocked("creation_form_admission_invalid");
            }
        } else if (creationServerLabel != null) {
            throw new Blocked("creation_server_label_unexpected");
        }
        String creationServerId = runnerArguments.getString(CREATION_SERVER_ID_ARGUMENT);
        if (action.allowsCreationServerId()) {
            if (creationServerId != null) requireUuid(creationServerId, "creation_server_id_invalid");
        } else if (creationServerId != null) {
            throw new Blocked("creation_server_id_unexpected");
        }
        String creationName = runnerArguments.getString(CREATION_NAME_ARGUMENT);
        if (action.requiresCreationName()) {
            if (creationName == null || !CREATION_NAME_PATTERN.matcher(creationName).matches()) throw new Blocked("creation_name_invalid");
        } else if (creationName != null) {
            throw new Blocked("creation_name_unexpected");
        }
        String previousCreationName = runnerArguments.getString(PREVIOUS_CREATION_NAME_ARGUMENT);
        if (action == Action.SET_CREATION_NAME) {
            if (previousCreationName != null && !CREATION_NAME_PATTERN.matcher(previousCreationName).matches()) throw new Blocked("previous_creation_name_invalid");
            if (previousCreationName != null && previousCreationName.equals(creationName)) throw new Blocked("previous_creation_name_invalid");
        } else if (previousCreationName != null) {
            throw new Blocked("previous_creation_name_unexpected");
        }
        String composerText = runnerArguments.getString(COMPOSER_TEXT_ARGUMENT);
        if (action.requiresComposerText()) {
            if (composerText == null || !COMPOSER_TEXT_PATTERN.matcher(composerText).matches()) throw new Blocked("composer_text_invalid");
        } else if (composerText != null) {
            throw new Blocked("composer_text_unexpected");
        }
        String sessionId = runnerArguments.getString(SESSION_ID_ARGUMENT);
        if (action.requiresSessionId()) {
            try {
                if (sessionId == null || !UUID.fromString(sessionId).toString().equals(sessionId)) throw new Blocked("session_id_invalid");
            } catch (IllegalArgumentException exception) {
                throw new Blocked("session_id_invalid");
            }
        } else if (sessionId != null) {
            throw new Blocked("session_id_unexpected");
        }
        String sessionGeneration = runnerArguments.getString(SESSION_GENERATION_ARGUMENT);
        if (action.requiresSessionGeneration()) {
            try {
                if (sessionGeneration == null || !UUID.fromString(sessionGeneration).toString().equals(sessionGeneration)) throw new Blocked("session_generation_invalid");
            } catch (IllegalArgumentException exception) {
                throw new Blocked("session_generation_invalid");
            }
        } else if (sessionGeneration != null) {
            throw new Blocked("session_generation_unexpected");
        }
        String historyRecordId = runnerArguments.getString(HISTORY_RECORD_ID_ARGUMENT);
        if (action.requiresHistoryRecordId()) {
            if (historyRecordId == null || !HISTORY_RECORD_ID_PATTERN.matcher(historyRecordId).matches()) throw new Blocked("history_record_id_invalid");
        } else if (historyRecordId != null) {
            throw new Blocked("history_record_id_unexpected");
        }
        String receiptId = runnerArguments.getString(RECEIPT_ID_ARGUMENT);
        if (action.requiresReceiptId()) {
            try {
                if (receiptId == null || !UUID.fromString(receiptId).toString().equals(receiptId)) throw new Blocked("receipt_id_invalid");
            } catch (IllegalArgumentException exception) {
                throw new Blocked("receipt_id_invalid");
            }
        } else if (receiptId != null) {
            throw new Blocked("receipt_id_unexpected");
        }
        String excludedReceiptIds = runnerArguments.getString(EXCLUDED_RECEIPT_IDS_ARGUMENT);
        Set<String> excluded = action == Action.SUBMIT_CREATION ? parseExcludedReceiptIds(excludedReceiptIds) : emptyExcludedReceiptIds(excludedReceiptIds);
        String workflowId = runnerArguments.getString(SNIPPET_WORKFLOW_ID_ARGUMENT);
        if (action.isSnippetAction()) {
            requireUuid(workflowId, "snippet_workflow_invalid");
        } else if (workflowId != null) throw new Blocked("snippet_workflow_unexpected");
        String snippetId = runnerArguments.getString(SNIPPET_ID_ARGUMENT);
        if (action.requiresSnippetId()) {
            requireUuid(snippetId, "snippet_id_invalid");
        } else if (snippetId != null) throw new Blocked("snippet_id_unexpected");
        String rawSnippetVersion = runnerArguments.getString(SNIPPET_VERSION_ARGUMENT);
        SnippetVersion snippetVersion = SnippetVersion.fromWire(rawSnippetVersion);
        if (action.requiresSnippetVersion()) {
            if (snippetVersion == null) throw new Blocked("snippet_version_invalid");
        } else if (rawSnippetVersion != null) throw new Blocked("snippet_version_unexpected");
        String routeHostId=runnerArguments.getString(ROUTE_HOST_ID_ARGUMENT);
        String routeDirect=runnerArguments.getString(ROUTE_DIRECT_ARGUMENT);
        if(action.isRouteAction()) {
            requireCondition(routeHostId!=null && routeHostId.matches("[0-9a-f]{64}"),"route_host_invalid");
            if(action==Action.SET_OWNED_ROUTE_DIRECT || action==Action.SAVE_OWNED_ROUTE_HINTS || action==Action.RECONNECT_OWNED_ROUTE_HOST) requireCondition(validOwnedRouteDirect(routeDirect),"route_direct_invalid");
            else if(routeDirect!=null) requireCondition(action==Action.OBSERVE_OWNED_HOST_ROUTES && validOwnedRouteDirect(routeDirect),"route_direct_invalid");
        } else requireCondition(routeHostId==null && routeDirect==null,"route_arguments_unexpected");
        PhysicalSelectors physical = PhysicalSelectors.parse(runnerArguments, action);
        return new Request(runId, action, appearance, sessionListPreference, sessionListPreferenceValue, creationServerLabel, creationServerId, creationProjectLabel, creationName, previousCreationName, composerText, sessionId, sessionGeneration, historyRecordId, receiptId, excluded, workflowId, snippetId, snippetVersion, routeHostId, routeDirect, physical);
    }

    private static final Set<String> PHYSICAL_KEYS = Collections.unmodifiableSet(new HashSet<>(Arrays.asList(
            "ownedUiHostId","ownedUiShellId","ownedUiShellGeneration","ownedUiProjectId","ownedUiServerId","ownedUiWorkspaceId",
            "ownedUiRequestId","ownedUiRequestAnswer","ownedUiKey","ownedUiSizeField","ownedUiColumns","ownedUiRows",
            "ownedUiScrollOffset","ownedUiScrollDirection","ownedUiDeliveryMode","ownedUiEnabled","ownedUiEventId",
            "ownedUiNotificationCategory","ownedUiNotificationHint","ownedUiHostLabel","ownedUiMvpRunId","ownedUiProductPid",
            "ownedUiProductStart","ownedUiObserverPublicSha256","ownedUiExpectedProductSha256","ownedUiExpectedServiceRequested","ownedUiOwnedStartRunId")));
    private static void requireKnownArguments(Bundle arguments) {
        Set<String> known = new HashSet<>(PHYSICAL_KEYS);
        known.addAll(Arrays.asList(RUN_ID_ARGUMENT,ACTION_ARGUMENT,APPEARANCE_ARGUMENT,SESSION_LIST_PREFERENCE_ARGUMENT,
                SESSION_LIST_PREFERENCE_VALUE_ARGUMENT,CREATION_SERVER_LABEL_ARGUMENT,CREATION_SERVER_ID_ARGUMENT,
                CREATION_PROJECT_LABEL_ARGUMENT,CREATION_NAME_ARGUMENT,PREVIOUS_CREATION_NAME_ARGUMENT,COMPOSER_TEXT_ARGUMENT,
                SESSION_ID_ARGUMENT,SESSION_GENERATION_ARGUMENT,HISTORY_RECORD_ID_ARGUMENT,RECEIPT_ID_ARGUMENT,
                EXCLUDED_RECEIPT_IDS_ARGUMENT,SNIPPET_WORKFLOW_ID_ARGUMENT,SNIPPET_ID_ARGUMENT,SNIPPET_VERSION_ARGUMENT,
                ROUTE_HOST_ID_ARGUMENT,ROUTE_DIRECT_ARGUMENT));
        for(String key:arguments.keySet())requireCondition(known.contains(key),"argument_unknown");
    }
    /** The exact closed parser consumed before private intent/release or any UI effect. */
    static void validatePhysicalArguments(Bundle arguments) {
        OwnedUiAutomationRunner parser=new OwnedUiAutomationRunner();
        parser.runnerArguments=new Bundle(arguments);
        parser.requiredRequest();
    }
    static final class PhysicalSelectors {
        final Map<String,String> values;
        final int productPid;
        final long productStart;
        final String argumentHash;
        private PhysicalSelectors(Map<String,String> values,int pid,long start,String argumentHash) {
            this.values=Collections.unmodifiableMap(values);this.productPid=pid;this.productStart=start;this.argumentHash=argumentHash;
        }
        String get(String key) {return values.get(key);}
        String host() {return get("ownedUiHostId");}
        String shellScope(String prefix) {return prefix+host()+"_"+get("ownedUiShellId")+"_"+get("ownedUiShellGeneration");}
        String marker() {return "GOATR_MVP_"+get("ownedUiMvpRunId");}
        static PhysicalSelectors parse(Bundle arguments,Action action) {
            boolean selected=action.isPhysicalAction();
            for(String key:PHYSICAL_KEYS)selected|=arguments.containsKey(key);
            if(!selected)return null;
            requireCondition(action.isPhysicalAction()||action.physicalLegacyAllowed(),"physical_action_forbidden");
            Set<String> allowed=new HashSet<>(Arrays.asList("ownedUiHostId","ownedUiMvpRunId","ownedUiProductPid","ownedUiProductStart","ownedUiObserverPublicSha256","ownedUiExpectedProductSha256"));
            if(action.shellScoped())allowed.addAll(Arrays.asList("ownedUiShellId","ownedUiShellGeneration"));
            if(action==Action.SELECT_TERMINAL_PROJECT||action==Action.SUBMIT_TERMINAL_CREATION||action==Action.SELECT_TERMINAL_WORKSPACE)
                allowed.addAll(Arrays.asList("ownedUiServerId","ownedUiWorkspaceId"));
            if(action==Action.SELECT_TERMINAL_PROJECT||action==Action.SUBMIT_TERMINAL_CREATION)allowed.add("ownedUiProjectId");
            if(action==Action.SELECT_REQUEST||action==Action.SUBMIT_REQUEST_ANSWER)allowed.add("ownedUiRequestId");
            if(action==Action.SUBMIT_REQUEST_ANSWER)allowed.add("ownedUiRequestAnswer");
            if(action==Action.SEND_SHELL_KEY||action==Action.SET_SHELL_MODIFIER)allowed.add("ownedUiKey");
            if(action==Action.SET_SHELL_SIZE||action==Action.CONFIRM_SHELL_RESIZE)allowed.addAll(Arrays.asList("ownedUiColumns","ownedUiRows"));
            if(action==Action.SET_SHELL_SIZE)allowed.add("ownedUiSizeField");
            if(action==Action.SCROLL_SHELL_HISTORY)allowed.addAll(Arrays.asList("ownedUiScrollOffset","ownedUiScrollDirection"));
            if(action==Action.SELECT_DELIVERY_MODE)allowed.add("ownedUiDeliveryMode");
            if(action==Action.ENABLE_ATTENTION||action==Action.ENABLE_COMPLETED)allowed.add("ownedUiEnabled");
            if(action==Action.START_DELIVERY||action==Action.STOP_DELIVERY)allowed.add("ownedUiExpectedServiceRequested");
            if(action==Action.STOP_DELIVERY)allowed.add("ownedUiOwnedStartRunId");
            if(action==Action.TAP_ATTENTION_NOTIFICATION)allowed.addAll(Arrays.asList("ownedUiEventId","ownedUiNotificationCategory","ownedUiNotificationHint","ownedUiHostLabel"));
            Map<String,String> values=new HashMap<>();
            for(String key:PHYSICAL_KEYS) {
                if(!allowed.contains(key)) {requireCondition(!arguments.containsKey(key),"physical_selector_unexpected");continue;}
                String value=arguments.getString(key);
                requireCondition(value!=null&&!value.isEmpty()&&value.length()<=512,"physical_selector_required");
                values.put(key,value);
            }
            requireCondition(values.get("ownedUiHostId").matches("[0-9a-f]{64}"),"physical_host_invalid");
            for(String key:Arrays.asList("ownedUiMvpRunId","ownedUiShellId","ownedUiShellGeneration","ownedUiProjectId","ownedUiServerId","ownedUiRequestId","ownedUiEventId","ownedUiOwnedStartRunId")) {
                String value=values.get(key);if(value!=null)strictPhysicalUuid(value);
            }
            String workspace=values.get("ownedUiWorkspaceId");
            if(workspace!=null)requireCondition(workspace.matches("[A-Za-z0-9_.:-]{1,128}"),"physical_workspace_invalid");
            requireCondition(values.get("ownedUiObserverPublicSha256").matches("[0-9a-f]{64}"),"physical_observer_invalid");
            requireCondition(values.get("ownedUiExpectedProductSha256").matches("[0-9a-f]{64}"),"physical_artifact_invalid");
            int pid=(int)physicalNumber(values.get("ownedUiProductPid"),1,Integer.MAX_VALUE);
            long start=physicalNumber(values.get("ownedUiProductStart"),1,Long.MAX_VALUE);
            if(values.containsKey("ownedUiColumns")) {
                long columns=physicalNumber(values.get("ownedUiColumns"),1,512),rows=physicalNumber(values.get("ownedUiRows"),1,256);
                requireCondition(columns*rows<=32768,"physical_dimensions_invalid");
            }
            if(values.containsKey("ownedUiSizeField"))requireCondition(Arrays.asList("Columns","Rows").contains(values.get("ownedUiSizeField")),"physical_dimension_field_invalid");
            if(values.containsKey("ownedUiScrollOffset")) {
                physicalNumber(values.get("ownedUiScrollOffset"),0,9_007_199_254_740_991L);
                requireCondition(Arrays.asList("Older","Newer").contains(values.get("ownedUiScrollDirection")),"physical_scroll_invalid");
            }
            if(values.containsKey("ownedUiDeliveryMode"))requireCondition(Arrays.asList("Persistent","Ntfy").contains(values.get("ownedUiDeliveryMode")),"physical_delivery_invalid");
            for(String key:Arrays.asList("ownedUiEnabled","ownedUiNotificationHint","ownedUiExpectedServiceRequested"))
                if(values.containsKey(key))requireCondition(Arrays.asList("true","false").contains(values.get(key)),"physical_boolean_invalid");
            if(values.containsKey("ownedUiHostLabel"))requireCondition(values.get("ownedUiHostLabel").equals("Goatr MVP "+values.get("ownedUiMvpRunId")),"physical_label_invalid");
            if(values.containsKey("ownedUiNotificationCategory"))requireCondition(Arrays.asList("Attention","Completed").contains(values.get("ownedUiNotificationCategory")),"physical_category_invalid");
            if(values.containsKey("ownedUiRequestAnswer"))requireCondition(Arrays.asList("SubmitPrepared","SelectFirst","Reject","Cancel").contains(values.get("ownedUiRequestAnswer")),"physical_answer_invalid");
            if(values.containsKey("ownedUiKey")) {
                String key=values.get("ownedUiKey");
                if(action==Action.SET_SHELL_MODIFIER)requireCondition(Arrays.asList("Ctrl","Alt","Shift","Super").contains(key),"physical_key_invalid");
                else requireCondition(Arrays.asList("Enter","Tab","Escape","Backspace","Delete","Insert","Left","Right","Up","Down","Home","End","PageUp","PageDown","F1","F2","F3","F4","F5","F6","F7","F8","F9","F10","F11","F12").contains(key),"physical_key_invalid");
            }
            return new PhysicalSelectors(values,pid,start,physicalArgumentHash(arguments));
        }
    }
    static String physicalArgumentHash(Bundle arguments) {
        StringBuilder material=new StringBuilder();
        for(String key:new java.util.TreeSet<>(arguments.keySet())) {
            String value=arguments.getString(key);
            requireCondition(value!=null&&key.length()<=128&&value.length()<=4096,"physical_selector_required");
            requireCondition(key.chars().allMatch(character->character>=32&&character<127)&&value.chars().allMatch(character->character>=32&&character<127),"physical_selector_required");
            material.append(key.length()).append(':').append(key).append(value.length()).append(':').append(value);
        }
        requireCondition(material.length()<=65536,"physical_selector_required");
        return digest(material.toString().getBytes(StandardCharsets.US_ASCII));
    }
    private static void strictPhysicalUuid(String value) {
        try {requireCondition(UUID.fromString(value).toString().equals(value),"physical_uuid_invalid");}
        catch(IllegalArgumentException error) {throw new Blocked("physical_uuid_invalid");}
    }
    private static long physicalNumber(String value,long minimum,long maximum) {
        requireCondition(value!=null&&value.matches("0|[1-9][0-9]{0,18}"),"physical_number_invalid");
        try {long result=Long.parseLong(value);requireCondition(result>=minimum&&result<=maximum,"physical_number_invalid");return result;}
        catch(NumberFormatException error) {throw new Blocked("physical_number_invalid");}
    }

    private static Set<String> jsonKeys(JSONObject value) {
        Set<String> keys=new HashSet<>();java.util.Iterator<String> names=value.keys();
        while(names.hasNext())keys.add(names.next());return keys;
    }
    private static byte[] physicalHex(String text,int minimum,int maximum) {
        requireCondition(text!=null&&text.length()%2==0&&text.length()/2>=minimum&&text.length()/2<=maximum&&text.matches("[0-9a-f]+"),"physical_epoch_proof");
        byte[] result=new byte[text.length()/2];for(int i=0;i<result.length;i++)result[i]=(byte)Integer.parseInt(text.substring(i*2,i*2+2),16);return result;
    }
    /** Signed original-product witness, private to this one release directory and action UUID. */
    private JSONObject requirePhysicalEpoch(Request request) {
        if(request==null||request.physical==null)return null;
        try {
            PhysicalSelectors selected=request.physical;
            File directory=releaseDirectory(getContext().getCacheDir().getCanonicalFile(),request.runId);
            File file=new File(directory,"product-epoch.json");
            requireCondition(directory.getCanonicalFile().equals(directory)&&file.getCanonicalFile().equals(file)&&file.length()>=2&&file.length()<=131072,"physical_epoch_proof");
            android.system.StructStat stat=Os.lstat(file.getPath());
            requireCondition(OsConstants.S_ISREG(stat.st_mode)&&stat.st_uid==Process.myUid()&&(stat.st_mode&0777)==0600,"physical_epoch_proof");
            JSONObject witness=new JSONObject(new String(Files.readAllBytes(file.toPath()),StandardCharsets.UTF_8));
            requireCondition(jsonKeys(witness).equals(new HashSet<>(Arrays.asList("public","challenge","signature","request"))),"physical_epoch_proof");
            byte[] point=physicalHex(witness.getString("public"),65,65);
            requireCondition(point[0]==4&&digest(point).equals(selected.get("ownedUiObserverPublicSha256")),"physical_epoch_proof");
            byte[] prefix=new byte[]{0x30,0x59,0x30,0x13,0x06,0x07,0x2a,(byte)0x86,0x48,(byte)0xce,0x3d,0x02,0x01,0x06,0x08,0x2a,(byte)0x86,0x48,(byte)0xce,0x3d,0x03,0x01,0x07,0x03,0x42,0x00};
            byte[] encoded=Arrays.copyOf(prefix,prefix.length+point.length);System.arraycopy(point,0,encoded,prefix.length,point.length);
            byte[] bytes=witness.getString("request").getBytes(StandardCharsets.UTF_8);
            requireCondition(bytes.length>=1&&bytes.length<=65536,"physical_epoch_proof");
            Signature verifier=Signature.getInstance("SHA256withECDSA");
            verifier.initVerify(KeyFactory.getInstance("EC").generatePublic(new X509EncodedKeySpec(encoded)));
            verifier.update("GOATR-STOCK-CONTROL-v1".getBytes(StandardCharsets.US_ASCII));verifier.update((byte)0);
            verifier.update(physicalHex(witness.getString("challenge"),32,32));verifier.update(ByteBuffer.allocate(4).putInt(bytes.length).array());verifier.update(bytes);
            requireCondition(verifier.verify(physicalHex(witness.getString("signature"),8,72)),"physical_epoch_proof");
            JSONObject envelope=new JSONObject(new String(bytes,StandardCharsets.UTF_8));Arrays.fill(bytes,(byte)0);
            requireCondition(jsonKeys(envelope).equals(new HashSet<>(Arrays.asList("method","runId","sequence","report")))
                    &&"physical_report".equals(envelope.getString("method"))&&selected.get("ownedUiMvpRunId").equals(envelope.getString("runId")),"physical_epoch_proof");
            JSONObject report=envelope.getJSONObject("report"),process=report.getJSONObject("process");
            PackageManager packages=getContext().getPackageManager();
            android.content.pm.PackageInfo productInfo=packages.getPackageInfo(PRODUCT_PACKAGE,0),testInfo=packages.getPackageInfo(TEST_PACKAGE,PackageManager.GET_INSTRUMENTATION);
            ApplicationInfo productApp=productInfo.applicationInfo,testApp=testInfo.applicationInfo;
            requireCondition(productApp!=null&&testApp!=null&&productApp.uid!=testApp.uid&&Process.myUid()==testApp.uid
                    &&TEST_PACKAGE.equals(android.app.Application.getProcessName())&&PRODUCT_PACKAGE.equals(productApp.processName)
                    &&TEST_PACKAGE.equals(testApp.processName)&&productInfo.sharedUserId==null&&testInfo.sharedUserId==null
                    &&process.getInt("uid")==productApp.uid&&PRODUCT_PACKAGE.equals(process.getString("name")),"physical_product_epoch");
            requireCondition(testInfo.instrumentation!=null&&testInfo.instrumentation.length==2,"physical_product_epoch");
            for(android.content.pm.InstrumentationInfo instrument:testInfo.instrumentation)requireCondition(instrument.targetProcesses==null,"physical_product_epoch");
            long age=SystemClock.elapsedRealtime()-report.getLong("observedAt");
            requireCondition("epoch".equals(report.getString("kind"))&&request.runId.equals(report.getString("actionRunId"))
                    &&process.getInt("pid")==selected.productPid&&selected.productStart==Long.parseLong(process.getString("start"))
                    &&age>=0&&age<=5000&&selected.get("ownedUiExpectedProductSha256").equals(report.getString("apkSha256"))
                    &&selected.get("ownedUiHostId").equals(report.getJSONObject("snapshot").getString("host")),"physical_product_epoch");
            return report.getJSONObject("snapshot");
        }catch(Blocked refused) {throw refused;}
        catch(Exception error) {throw new Blocked("physical_epoch_proof");}
    }
    private void physicalObservation(UiAutomation automation,Request request,long deadline) {
        JSONObject state=requirePhysicalEpoch(request);
        try {
            JSONObject pairing=state.getJSONObject("pairing");
            if(!pairing.getBoolean("screenPairing")) {
                AccessibilityNodeInfo root=currentProductRoot(automation,deadline);
                try {
                    requireCondition(state.getBoolean("foreground"),"physical_product_epoch");
                    if(!state.isNull("selectedSession"))requireCondition(exactConversationWindow(automation,state.getString("selectedSession"),state.getString("selectedGeneration"),deadline)==root.getWindowId(),"physical_postcondition");
                    else if(!state.isNull("selectedShell"))requireCondition(physicalWindow(automation,"shell_scope_"+request.physical.host()+"_"+state.getString("selectedShell")+"_"+state.getString("selectedShellGeneration"),deadline)==root.getWindowId(),"physical_postcondition");
                }finally{root.recycle();}
                return;
            }
            if(pairing.getBoolean("previewPresent")) {
                requireCondition(pairing.getBoolean("hostMatches")&&pairing.getBoolean("nameMatches")&&pairing.getBoolean("fingerprintMatches"),"physical_pairing_preview");
                NodeRef confirm=uniqueClickableTag(automation,"pair_confirm",deadline,NO_WINDOW);
                confirm.node.recycle();
                try {NodeRef paste=uniqueVisibleTag(automation,"pair_paste",deadline);paste.node.recycle();throw new Blocked("physical_pairing_preview");}
                catch(Blocked absent) {if(!"ui_tag_missing".equals(absent.code))throw absent;}
            }else {
                int window=physicalWindow(automation,"pair_camera",deadline);
                requireTagTextExact(automation,"camera_status","Scanning QR · preview only",deadline);
                NodeRef paste=uniqueVisibleTag(automation,"pair_paste",deadline,window);
                try {CharSequence value=paste.node.getText();requireCondition(value==null||value.length()==0,"physical_pairing_paste_not_empty");}
                finally{paste.node.recycle();}
            }
        }catch(org.json.JSONException malformed){throw new Blocked("physical_product_epoch");}
    }
    /** Inspect ONLY the terminal-root identifier; never properties, text, or descendants. */
    private void requirePhysicalGridIdentifier(UiAutomation automation,int window,long deadline) {
        AccessibilityNodeInfo root=currentProductRoot(automation,deadline);
        if(root.getWindowId()!=window){root.recycle();throw new Blocked("ui_root_changed");}
        ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(root);int discovered=1,grids=0;
        try {
            while(!queue.isEmpty()) {
                AccessibilityNodeInfo node=queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    if(TERMINAL_OUTPUT_TAG.equals(node.getViewIdResourceName())) {requireCondition(++grids==1,"physical_postcondition");continue;}
                    if(isSensitiveContent(node))continue;
                    NodeMetadata meta=metadata(node);requireCondition(PRODUCT_PACKAGE.equals(meta.packageName)&&meta.windowId==window,"ui_root_changed");
                    for(int index=0;index<node.getChildCount();index++) {
                        requireCondition(++discovered<=MAX_NODES,"ui_tree_bound");
                        AccessibilityNodeInfo child=node.getChild(index);if(child!=null)queue.add(child);
                    }
                }finally{node.recycle();}
            }
            requireCondition(grids==1,"physical_postcondition");
        }finally{while(!queue.isEmpty())queue.removeFirst().recycle();}
    }
    private void physicalAgentReadOnly(UiAutomation automation,Request request,long deadline) {
        requirePhysicalEpoch(request);
        int window=terminalTargetWindow(automation,request.sessionId,request.sessionGeneration,deadline);
        requirePhysicalGridIdentifier(automation,window,deadline);
        NodeRef dialog=uniqueVisibleTag(automation,TERMINAL_DIALOG_TAG,deadline,window);
        ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(dialog.node);int discovered=1;
        try {
            while(!queue.isEmpty()) {
                AccessibilityNodeInfo node=queue.removeFirst();
                try {
                    if(isTerminalOutput(node))continue;
                    requireBeforeDeadline(deadline);NodeMetadata meta=metadata(node);
                    requireCondition(PRODUCT_PACKAGE.equals(meta.packageName)&&meta.windowId==window,"ui_root_changed");
                    String tag=meta.viewId;
                    requireCondition(!node.isEditable()&&!meta.supports(AccessibilityNodeInfo.ACTION_SET_TEXT)
                            &&(tag==null||!tag.startsWith("shell_")&&!tag.equals("terminal_focus")&&!tag.equals("terminal_resize")),"physical_agent_not_read_only");
                    if(isSensitiveContent(node))continue;
                    for(int index=0;index<node.getChildCount();index++) {
                        requireCondition(++discovered<=MAX_NODES,"ui_tree_bound");
                        AccessibilityNodeInfo child=node.getChild(index);if(child!=null)queue.add(child);
                    }
                }finally{node.recycle();}
            }
        }finally{while(!queue.isEmpty())queue.removeFirst().recycle();}
    }
    private int physicalWindow(UiAutomation automation,String scope,long deadline) {
        NodeRef node=uniqueVisibleTag(automation,scope,deadline);
        try {return node.windowId;}finally{node.node.recycle();}
    }
    private void awaitPhysicalScope(UiAutomation automation,String scope,long deadline) {
        while(SystemClock.elapsedRealtime()<deadline) {
            try {NodeRef node=uniqueVisibleTag(automation,scope,deadline);node.node.recycle();return;}
            catch(Blocked missing) {if(!"ui_tag_missing".equals(missing.code))throw missing;}
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("physical_postcondition");
    }
    private ActionResult physicalAction(UiAutomation automation,Request request,long deadline,Acknowledgement acknowledgement,StageTracker stage) {
        PhysicalSelectors p=request.physical;requireCondition(p!=null,"physical_selector_required");
        if(request.action==Action.TAP_ATTENTION_NOTIFICATION)return physicalNotificationTap(automation,request,deadline,acknowledgement,stage);
        if(request.action==Action.OPEN_ATTENTION_SHADE) {
            requirePhysicalEpoch(request);requireBeforeDeadline(deadline);stage.stage=Stage.CLICK;
            boolean accepted=automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS);
            acknowledgement.value=accepted;requireCondition(accepted,"ui_global_action_acknowledgement");
            stage.stage=Stage.POSTCONDITION;physicalShadeRoot(automation,deadline).recycle();
            return new ActionResult(true,true,Observation.empty());
        }
        String shell=request.action.shellScoped()?p.shellScope("shell_scope_"):null,scope="scene_host_"+p.host(),after=scope,tag=null,value=null;
        int input=AccessibilityNodeInfo.ACTION_CLICK;
        switch(request.action) {
            case VIEW_SESSIONS:scope=SESSIONS_TAG;tag="host_sessions_"+p.host();break;
            case OPEN_HOST_SETTINGS:scope=SESSIONS_TAG;tag="host_settings_"+p.host();break;
            case OPEN_SHELLS:scope=SESSIONS_TAG;tag="host_terminals_"+p.host();after="shell_list_host_"+p.host();break;
            case OPEN_SHELL: scope="shell_list_host_"+p.host();physicalWindow(automation,"shell_row_target_"+p.get("ownedUiShellId")+"_"+p.get("ownedUiShellGeneration"),deadline);tag="shell_open_"+p.get("ownedUiShellId");after=shell;break;
            case BEGIN_SHELL_CONTROL:scope=shell;tag="shell_begin_control";after=p.shellScope("shell_begin_scope_");break;
            case CONFIRM_SHELL_CONTROL:scope=p.shellScope("shell_begin_scope_");tag="shell_begin_confirm";after=shell;break;
            case SET_SHELL_DRAFT:scope=shell;tag="shell_input_text";input=AccessibilityNodeInfo.ACTION_SET_TEXT;value="echo "+p.marker();break;
            case SEND_SHELL_DRAFT:scope=shell;tag="shell_input_send";requireTagTextExact(automation,"shell_input_text","echo "+p.marker(),deadline);break;
            case SEND_SHELL_KEY:scope=shell;tag="shell_key_"+p.get("ownedUiKey");break;
            case SET_SHELL_MODIFIER:scope=shell;tag="shell_modifier_"+p.get("ownedUiKey");break;
            case OPEN_SHELL_RESIZE:scope=shell;tag="shell_resize";after=p.shellScope("shell_resize_scope_");break;
            case SET_SHELL_SIZE:scope=p.shellScope("shell_resize_scope_");tag="Columns".equals(p.get("ownedUiSizeField"))?"shell_resize_columns":"shell_resize_rows";value=p.get("Columns".equals(p.get("ownedUiSizeField"))?"ownedUiColumns":"ownedUiRows");input=AccessibilityNodeInfo.ACTION_SET_TEXT;break;
            case CONFIRM_SHELL_RESIZE:scope=p.shellScope("shell_resize_scope_");tag="shell_resize_confirm";requireTagTextExact(automation,"shell_resize_columns",p.get("ownedUiColumns"),deadline);requireTagTextExact(automation,"shell_resize_rows",p.get("ownedUiRows"),deadline);after=shell;break;
            case SCROLL_SHELL_HISTORY: {
                scope=shell;JSONObject observed=requirePhysicalEpoch(request);
                try {
                    long offset=observed.getLong("scrollOffset"),maximum=observed.getLong("scrollMaximum"),rows=observed.getLong("scrollViewport");
                    requireCondition(offset>=0&&maximum>=offset&&rows>0,"physical_history_unavailable");
                    long expected="Older".equals(p.get("ownedUiScrollDirection"))?Math.min(maximum,offset+rows):Math.max(0,offset-rows);
                    requireCondition(expected==physicalNumber(p.get("ownedUiScrollOffset"),0,9_007_199_254_740_991L),"physical_history_changed");
                }catch(org.json.JSONException error) {throw new Blocked("physical_history_unavailable");}
                tag="Older".equals(p.get("ownedUiScrollDirection"))?"shell_history_older":"shell_history_newer";break;
            }
            case SHELL_HISTORY_BOTTOM:scope=shell;tag="shell_history_bottom";break;
            case RELEASE_SHELL_CONTROL:scope=shell;tag="shell_release_control";break;
            case OPEN_SHELL_DETAILS:scope=shell;tag="shell_viewer_details";after=p.shellScope("shell_details_scope_");break;
            case OBSERVE_SHELL:int shellWindow=physicalWindow(automation,shell,deadline);requirePhysicalGridIdentifier(automation,shellWindow,deadline);return new ActionResult(false,true,Observation.empty());
            case OPEN_TERMINAL_CREATION:scope="shell_list_host_"+p.host();tag="terminal_create_workspace";after="terminal_workspace_picker_"+p.host();break;
            case SELECT_TERMINAL_WORKSPACE:scope="terminal_workspace_picker_"+p.host();tag="terminal_workspace_"+p.get("ownedUiServerId")+"_"+p.get("ownedUiWorkspaceId");after="terminal_creation_scope_"+p.host()+"_"+p.get("ownedUiServerId")+"_"+p.get("ownedUiWorkspaceId");break;
            case SELECT_TERMINAL_PROJECT:scope="terminal_creation_scope_"+p.host()+"_"+p.get("ownedUiServerId")+"_"+p.get("ownedUiWorkspaceId");physicalWindow(automation,"terminal_creation_source_workspace",deadline);tag="terminal_project_"+p.get("ownedUiProjectId");after=scope;break;
            case SUBMIT_TERMINAL_CREATION:scope="terminal_creation_scope_"+p.host()+"_"+p.get("ownedUiServerId")+"_"+p.get("ownedUiWorkspaceId");physicalWindow(automation,"terminal_creation_source_workspace",deadline);tag="terminal_create_confirm";after=scope;break;
            case STOP_SESSION:physicalWindow(automation,scope,deadline);exactConversationWindow(automation,request.sessionId,request.sessionGeneration,deadline);tag="turn_stop";break;
            case SELECT_REQUEST:physicalWindow(automation,scope,deadline);exactConversationWindow(automation,request.sessionId,request.sessionGeneration,deadline);tag="request_select_"+p.get("ownedUiRequestId");after="request_editor_"+p.get("ownedUiRequestId");break;
            case SUBMIT_REQUEST_ANSWER:
                scope="request_editor_"+p.get("ownedUiRequestId");physicalWindow(automation,"form_request_"+p.get("ownedUiRequestId"),deadline);
                if("SubmitPrepared".equals(p.get("ownedUiRequestAnswer")))tag="form_submit";
                else if("SelectFirst".equals(p.get("ownedUiRequestAnswer"))) {physicalWindow(automation,"form_OmpSelect",deadline);tag="omp_option_0";}
                else if("Reject".equals(p.get("ownedUiRequestAnswer"))) {physicalWindow(automation,"form_CodexForm",deadline);tag="form_decline";}
                else tag="form_cancel";
                after="scene_host_"+p.host();break;
            case SELECT_DELIVERY_MODE:tag="delivery_mode_"+p.get("ownedUiDeliveryMode")+"_"+p.host();break;
            case ENABLE_ATTENTION:tag="delivery_enabled_"+p.host();break;
            case ENABLE_COMPLETED:tag="delivery_completed_"+p.host();break;
            case START_DELIVERY:
            case STOP_DELIVERY: {
                JSONObject state=requirePhysicalEpoch(request);
                try {
                    JSONObject keeper=state.getJSONObject("keeper");
                    requireCondition("Settings".equals(state.getString("screen"))&&p.host().equals(state.getString("selectedHost"))
                            &&keeper.getBoolean("serviceRequested")==Boolean.parseBoolean(p.get("ownedUiExpectedServiceRequested")),"physical_delivery_changed");
                    if(request.action==Action.START_DELIVERY)requireCondition(!keeper.getBoolean("running"),"physical_delivery_already_running");
                    else {
                        requireCondition(keeper.getBoolean("serviceRequested")&&keeper.getBoolean("privateEnabled")&&"Ntfy".equals(keeper.getString("privateMode"))
                                &&keeper.getBoolean("privateVerified")&&!keeper.getBoolean("privateRequiresKeeper"),"physical_cleanup_not_owned");
                        org.json.JSONArray retained=keeper.getJSONArray("retained");
                        for(int i=0;i<retained.length();i++)requireCondition(!retained.getJSONObject(i).getBoolean("requiresKeeper"),"physical_retained_keeper");
                    }
                }catch(org.json.JSONException error) {throw new Blocked("physical_delivery_changed");}
                tag=(request.action==Action.START_DELIVERY?"delivery_start_":"delivery_stop_")+p.host();break;
            }
            case BACKGROUND_PRIVATE_SETTINGS:
                physicalWindow(automation,scope,deadline);physicalWindow(automation,HOST_SETTINGS_PANEL_TAG,deadline);
                requirePhysicalEpoch(request);requireBeforeDeadline(deadline);stage.stage=Stage.CLICK;
                boolean home=automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_HOME);
                acknowledgement.value=home;requireCondition(home,"ui_global_action_acknowledgement");stage.stage=Stage.POSTCONDITION;
                AccessibilityNodeInfo background=automation.getRootInActiveWindow();requireCondition(background!=null,"ui_root_missing");
                try {requireCondition(!PRODUCT_PACKAGE.contentEquals(background.getPackageName()),"physical_postcondition");}finally{background.recycle();}
                return new ActionResult(true,true,Observation.empty());
            default:throw new Blocked("physical_action_forbidden");
        }
        if(after.equals("scene_host_"+p.host())&&request.action.shellScoped())after=scope;
        int window=physicalWindow(automation,scope,deadline);stage.stage=Stage.SELECTION;
        NodeRef selected;
        if(request.action==Action.ENABLE_ATTENTION||request.action==Action.ENABLE_COMPLETED) {
            NodeRef row=uniqueVisibleTag(automation,tag,deadline,window);
            try {selected=uniqueCheckableDescendant(row.node,window,tag,deadline);}finally{row.node.recycle();}
            requireCondition(selected.node.isChecked()!=Boolean.parseBoolean(p.get("ownedUiEnabled")),"physical_precondition_not_fresh");
        }else selected=uniqueActionTag(automation,tag,input,deadline,window);
        boolean accepted;
        try {
            stage.stage=Stage.CLICK;
            boolean toggle=request.action==Action.ENABLE_ATTENTION||request.action==Action.ENABLE_COMPLETED;
            boolean valid=toggle ? selected.node.refresh()&&selected.node.isVisibleToUser()&&selected.node.isEnabled()
                    &&(tag.equals(selected.node.getViewIdResourceName())||hasAncestorTag(selected.node,window,tag,deadline))
                    : revalidateAction(automation,selected,tag,input,deadline);
            requireCondition(valid,"ui_revalidation");
            requireCondition(physicalWindow(automation,scope,deadline)==window,"ui_root_changed");
            if(toggle)requireCondition(selected.node.isChecked()!=Boolean.parseBoolean(p.get("ownedUiEnabled")),"physical_precondition_not_fresh");
            if(request.action==Action.SET_SHELL_MODIFIER)requireCondition(!selected.node.isSelected(),"physical_precondition_not_fresh");
            if(request.action==Action.SUBMIT_TERMINAL_CREATION) {
                NodeRef project=uniqueVisibleTag(automation,"terminal_project_"+p.get("ownedUiProjectId"),deadline,window);
                try {requireCondition(project.node.refresh()&&project.node.isSelected(),"physical_precondition_not_fresh");}finally{project.node.recycle();}
            }
            if(value!=null) {
                CharSequence previous=selected.node.getText();
                if(request.action==Action.SET_SHELL_SIZE) {
                    JSONObject state=requirePhysicalEpoch(request);
                    try {String expected=Long.toString(state.getLong("Columns".equals(p.get("ownedUiSizeField"))?"columns":"rows"));requireCondition(expected.contentEquals(previous),"physical_precondition_not_fresh");}
                    catch(org.json.JSONException error){throw new Blocked("physical_product_epoch");}
                }else requireCondition(previous==null||previous.length()==0,"physical_field_not_empty");
                Bundle text=new Bundle();text.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,value);
                requirePhysicalEpoch(request);requireBeforeDeadline(deadline);accepted=selected.node.performAction(input,text);
            }else {requirePhysicalEpoch(request);requireBeforeDeadline(deadline);accepted=selected.node.performAction(input);}
        }finally{selected.node.recycle();}
        acknowledgement.value=accepted;requireCondition(accepted,input==AccessibilityNodeInfo.ACTION_SET_TEXT?"ui_set_text_acknowledgement":"ui_click_acknowledgement");
        stage.stage=Stage.POSTCONDITION;awaitPhysicalScope(automation,after,deadline);
        if(value!=null)requireTagTextExact(automation,tag,value,deadline);
        return new ActionResult(true,true,Observation.empty());
    }

    private String physicalSystemUiPackage() {
        try {
            android.content.res.Resources resources=android.content.res.Resources.getSystem();
            int id=resources.getIdentifier("config_systemUIServiceComponent","string","android");
            requireCondition(id!=0,"physical_systemui_unqualified");
            android.content.ComponentName component=android.content.ComponentName.unflattenFromString(resources.getString(id));
            requireCondition(component!=null,"physical_systemui_unqualified");
            ApplicationInfo app=getContext().getPackageManager().getApplicationInfo(component.getPackageName(),0);
            requireCondition((app.flags&ApplicationInfo.FLAG_SYSTEM)!=0&&app.uid==1000&&!PRODUCT_PACKAGE.equals(app.packageName)&&!TEST_PACKAGE.equals(app.packageName),"physical_systemui_unqualified");
            return component.getPackageName();
        }catch(PackageManager.NameNotFoundException error) {throw new Blocked("physical_systemui_unqualified");}
    }
    private AccessibilityNodeInfo physicalShadeRoot(UiAutomation automation,long deadline) {
        String system=physicalSystemUiPackage();
        while(SystemClock.elapsedRealtime()<deadline) {
            AccessibilityNodeInfo root=automation.getRootInActiveWindow();
            if(root!=null) {
                if(system.contentEquals(root.getPackageName())&&root.isVisibleToUser()) {
                    AccessibilityWindowInfo window=root.getWindow();
                    boolean qualified;
                    try {qualified=window!=null&&window.getType()==AccessibilityWindowInfo.TYPE_SYSTEM;}
                    finally {if(window!=null)window.recycle();}
                    if(!qualified){root.recycle();throw new Blocked("physical_systemui_unqualified");}
                    return root;
                }
                root.recycle();
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("physical_notification_shade");
    }
    private boolean physicalRowMatches(AccessibilityNodeInfo root,String system,String app,String label,String title,long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(AccessibilityNodeInfo.obtain(root));
        int count=0,apps=0,labels=0,titles=0,closedTitles=0;
        try {
            while(!queue.isEmpty()) {
                AccessibilityNodeInfo node=queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);requireCondition(++count<=MAX_NODES,"ui_node_bound");
                    if(node.isPassword()||!system.contentEquals(node.getPackageName())||node.getWindowId()!=root.getWindowId())return false;
                    CharSequence text=node.getText();
                    if(node.isVisibleToUser()&&text!=null) {
                        if(app.contentEquals(text))apps++;
                        if(label.contentEquals(text))labels++;
                        if(title.contentEquals(text))titles++;
                        if(Arrays.asList("Agent needs attention","Agent finished","Desktop needs attention","Work finished on desktop").contains(text.toString()))closedTitles++;
                    }
                    for(int i=0;i<node.getChildCount();i++) {AccessibilityNodeInfo child=node.getChild(i);if(child!=null)queue.add(child);}
                }finally{node.recycle();}
            }
            return apps>=1&&labels==1&&titles==1&&closedTitles==1;
        }finally{recycleQueue(queue);}
    }
    private NodeRef physicalNotificationRow(AccessibilityNodeInfo root,String system,String app,String label,String title,long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(AccessibilityNodeInfo.obtain(root));
        AccessibilityNodeInfo selected=null;int count=0;
        try {
            while(!queue.isEmpty()) {
                AccessibilityNodeInfo node=queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);requireCondition(++count<=MAX_NODES,"ui_node_bound");
                    requireCondition(system.contentEquals(node.getPackageName())&&!node.isPassword(),"physical_systemui_unqualified");
                    if(node.isVisibleToUser()&&title.contentEquals(node.getText())) {
                        AccessibilityNodeInfo ancestor=AccessibilityNodeInfo.obtain(node);
                        try {
                            for(int depth=0;ancestor!=null&&depth<16;depth++) {
                                if(ancestor.isVisibleToUser()&&ancestor.isEnabled()&&ancestor.isClickable()
                                        &&physicalRowMatches(ancestor,system,app,label,title,deadline)) {
                                    requireCondition(selected==null||selected.equals(ancestor),"physical_notification_row_ambiguous");
                                    if(selected==null)selected=AccessibilityNodeInfo.obtain(ancestor);
                                    break;
                                }
                                AccessibilityNodeInfo next=ancestor.getParent();ancestor.recycle();ancestor=next;
                            }
                        }finally{if(ancestor!=null)ancestor.recycle();}
                    }
                    for(int i=0;i<node.getChildCount();i++) {AccessibilityNodeInfo child=node.getChild(i);if(child!=null)queue.add(child);}
                }finally{node.recycle();}
            }
            requireCondition(selected!=null,"physical_notification_row_missing");
            NodeRef result=new NodeRef(selected,root.getWindowId());selected=null;return result;
        }finally{recycleQueue(queue);if(selected!=null)selected.recycle();}
    }
    private ActionResult physicalNotificationTap(UiAutomation automation,Request request,long deadline,Acknowledgement acknowledgement,StageTracker stage) {
        PhysicalSelectors p=request.physical;String system=physicalSystemUiPackage();
        String app;
        try {app=getContext().getPackageManager().getApplicationInfo(PRODUCT_PACKAGE,0).loadLabel(getContext().getPackageManager()).toString();}
        catch(PackageManager.NameNotFoundException error) {throw new Blocked("physical_systemui_unqualified");}
        boolean hint=Boolean.parseBoolean(p.get("ownedUiNotificationHint")),completed="Completed".equals(p.get("ownedUiNotificationCategory"));
        String title=hint?(completed?"Work finished on desktop":"Desktop needs attention"):(completed?"Agent finished":"Agent needs attention");
        String label=p.get("ownedUiHostLabel")+(hint?" · Open to synchronize":"");
        AccessibilityNodeInfo original=physicalShadeRoot(automation,deadline);
        try {
            stage.stage=Stage.SELECTION;NodeRef row=physicalNotificationRow(original,system,app,label,title,deadline);
            try {
                requirePhysicalEpoch(request);
                AccessibilityNodeInfo current=physicalShadeRoot(automation,deadline);
                try {
                    requireCondition(current.equals(original)&&current.getWindowId()==row.windowId,"ui_root_changed");
                    NodeRef same=physicalNotificationRow(current,system,app,label,title,deadline);
                    try {requireCondition(same.node.equals(row.node)&&row.node.refresh()&&physicalRowMatches(row.node,system,app,label,title,deadline),"ui_revalidation");}
                    finally{same.node.recycle();}
                }finally{current.recycle();}
                stage.stage=Stage.CLICK;requirePhysicalEpoch(request);requireBeforeDeadline(deadline);
                boolean accepted=row.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
                acknowledgement.value=accepted;requireCondition(accepted,"ui_click_acknowledgement");
            }finally{row.node.recycle();}
        }finally{original.recycle();}
        stage.stage=Stage.POSTCONDITION;
        Observation after=awaitExactConversation(automation,request.sessionId,request.sessionGeneration,deadline);
        physicalWindow(automation,"scene_host_"+p.host(),deadline);
        return new ActionResult(true,true,after);
    }

    private void requireUuid(String value, String code) {
        try {
            if (value == null || !UUID.fromString(value).toString().equals(value)) throw new Blocked(code);
        } catch (IllegalArgumentException exception) { throw new Blocked(code); }
    }

    private Set<String> emptyExcludedReceiptIds(String value) {
        if (value != null) throw new Blocked("excluded_receipts_unexpected");
        return new HashSet<>();
    }

    private Set<String> parseExcludedReceiptIds(String value) {
        if (value == null || !RECEIPT_ID_ARRAY_PATTERN.matcher(value).matches()) throw new Blocked("excluded_receipts_invalid");
        Set<String> result = new HashSet<>();
        java.util.regex.Matcher matcher = UUID_PATTERN.matcher(value);
        while (matcher.find()) {
            String uuid = matcher.group();
            try {
                requireCondition(UUID.fromString(uuid).toString().equals(uuid) && result.add(uuid), "excluded_receipts_invalid");
            } catch (IllegalArgumentException exception) {
                throw new Blocked("excluded_receipts_invalid");
            }
            requireCondition(result.size() <= MAX_EXCLUDED_RECEIPTS, "excluded_receipts_invalid");
        }
        return result;
    }

    private static void requireEligible(Context test, Context ownTarget) throws PackageManager.NameNotFoundException {
        requireCondition(TEST_PACKAGE.equals(test.getPackageName()) && TEST_PACKAGE.equals(ownTarget.getPackageName()), "test_package");
        requireCondition(Build.VERSION.SDK_INT >= MIN_API && Build.VERSION.SDK_INT <= MAX_API, "api_bounds");
        ApplicationInfo testInfo = test.getPackageManager().getApplicationInfo(TEST_PACKAGE, 0);
        requireCondition((testInfo.flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0, "test_not_debug");
        ApplicationInfo product = test.getPackageManager().getApplicationInfo(PRODUCT_PACKAGE, 0);
        requireCondition((product.flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0, "product_not_debug");
        Set<String> testSigners = signerDigests(test, TEST_PACKAGE);
        Set<String> productSigners = signerDigests(test, PRODUCT_PACKAGE);
        requireCondition(testSigners.size() == 1 && testSigners.equals(productSigners), "signer_mismatch");
    }

    /** Same admitted test process, not a new runner command or an app-start shortcut. */
    private static UiAutomation deliveryAutomation(Instrumentation instrumentation) throws PackageManager.NameNotFoundException {
        requireCondition(instrumentation instanceof OwnedInstrumentationRunner, "delivery_instrumentation");
        requireCondition(((OwnedInstrumentationRunner) instrumentation).controlIdentity().length == 65, "delivery_instrumentation");
        requireCondition(PRODUCT_PACKAGE.equals(instrumentation.getTargetContext().getPackageName()), "product_package");
        requireEligible(instrumentation.getContext(), instrumentation.getContext());
        requireCondition(Build.VERSION.SDK_INT == 34, "api_bounds");
        return instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
    }

    private static String deliveryHomePackage(Instrumentation instrumentation) {
        android.content.pm.ResolveInfo home = instrumentation.getContext().getPackageManager().resolveActivity(
                new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME), PackageManager.MATCH_DEFAULT_ONLY);
        requireCondition(home != null && home.activityInfo != null && home.activityInfo.packageName != null
                && !PRODUCT_PACKAGE.equals(home.activityInfo.packageName), "home_resolution_invalid");
        return home.activityInfo.packageName;
    }

    /** Identifier-only scan, retaining the existing secret/terminal/document subtree boundary. */
    private static boolean deliveryScene(AccessibilityNodeInfo root, String first, String second, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        int discovered = 1, firstCount = 0, secondCount = 0;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    requireCondition(node.getWindowId() == root.getWindowId(), "ui_root_changed");
                    if (isSensitiveContent(node)) continue;
                    if (node.isVisibleToUser()) {
                        if (first.equals(node.getViewIdResourceName())) firstCount++;
                        if (second != null && second.equals(node.getViewIdResourceName())) secondCount++;
                    }
                    for (int i = 0; i < node.getChildCount(); i++) {
                        requireCondition(discovered++ < MAX_NODES, "ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(i);
                        requireCondition(child != null, "ui_child_missing");
                        queue.addLast(child);
                    }
                } finally { node.recycle(); }
            }
            return firstCount == 1 && (second == null || secondCount == 1);
        } finally { recycleQueue(queue); }
    }

    static void backgroundOwnedDeliveryConversation(Instrumentation instrumentation, String session, String generation) throws PackageManager.NameNotFoundException {
        long deadline = SystemClock.elapsedRealtime() + TIMEOUT_MILLIS;
        UiAutomation automation = deliveryAutomation(instrumentation);
        InteractiveWindowsService windows = enableInteractiveWindows(automation);
        try {
            AccessibilityNodeInfo original = currentProductRoot(automation, deadline);
            try {
                requireCondition(deliveryScene(original, CONVERSATION_LIST_TAG, CONVERSATION_TARGET_TAG_PREFIX + session + "_" + generation, deadline), "conversation_target_mismatch");
                AccessibilityNodeInfo current = currentProductRoot(automation, deadline);
                try { requireCondition(current.equals(original) && original.refresh()
                        && deliveryScene(current, CONVERSATION_LIST_TAG, CONVERSATION_TARGET_TAG_PREFIX + session + "_" + generation, deadline), "ui_root_changed"); }
                finally { current.recycle(); }
                requireBeforeDeadline(deadline);
                requireCondition(automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_HOME), "ui_global_action_acknowledgement");
            } finally { original.recycle(); }
            String home = deliveryHomePackage(instrumentation);
            while (SystemClock.elapsedRealtime() < deadline) {
                AccessibilityNodeInfo root = automation.getRootInActiveWindow();
                if (root != null) {
                    try {
                        if (home.contentEquals(root.getPackageName())) return;
                        requireCondition(PRODUCT_PACKAGE.contentEquals(root.getPackageName()), "ui_root_foreign");
                    } finally { root.recycle(); }
                }
                SystemClock.sleep(POLL_MILLIS);
            }
            throw new Blocked("expected_screen_timeout");
        } finally { restoreInteractiveWindows(automation, windows); }
    }

    private static void requireSingleDeliveryAlert(android.app.NotificationManager manager, android.service.notification.StatusBarNotification selected, String host, String event) {
        requireCondition(host != null && host.matches("[0-9a-f]{64}") && event != null && UUID.fromString(event).toString().equals(event), "notification_identity");
        String tag = "goatr.attention." + host + "." + event;
        requireCondition(PRODUCT_PACKAGE.equals(selected.getPackageName()) && tag.equals(selected.getTag())
                && selected.getUid() == Process.myUid() && selected.getNotification().contentIntent != null, "notification_identity");
        int count = 0, exact = 0;
        for (android.service.notification.StatusBarNotification current : manager.getActiveNotifications()) {
            if (current.getTag() == null || !current.getTag().startsWith("goatr.attention.")) continue;
            count++;
            if (current.getKey().equals(selected.getKey()) && tag.equals(current.getTag())
                    && current.getUid() == selected.getUid() && current.getId() == selected.getId()) exact++;
        }
        requireCondition(count == 1 && exact == 1, "notification_ambiguous");
    }

    /** Reads only public app-name fields, then the one proved-owned row's closed generic title. */
    private static boolean deliveryRowMatches(AccessibilityNodeInfo row, String appLabel, String title, long deadline) {
        int labels = 0, owned = 0;
        for (int pass = 0; pass < 2; pass++) {
            ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
            queue.add(AccessibilityNodeInfo.obtain(row));
            int discovered = 1, titles = 0;
            try {
                while (!queue.isEmpty()) {
                    AccessibilityNodeInfo node = queue.removeFirst();
                    try {
                        requireBeforeDeadline(deadline);
                        requireCondition(node.getWindowId() == row.getWindowId(), "ui_root_changed");
                        if (isSensitiveContent(node)) continue;
                        String id = node.getViewIdResourceName();
                        if (node.isVisibleToUser() && ("android:id/app_name_text".equals(id) || "com.android.systemui:id/app_name_text".equals(id)) && pass == 0) {
                            labels++;
                            if (node.getText() != null && appLabel.contentEquals(node.getText())) owned++;
                        }
                        if (pass == 1 && node.isVisibleToUser() && "android:id/title".equals(id) && node.getText() != null && title.contentEquals(node.getText())) titles++;
                        for (int i = 0; i < node.getChildCount(); i++) {
                            requireCondition(discovered++ < MAX_NODES, "ui_node_bound");
                            AccessibilityNodeInfo child = node.getChild(i);
                            requireCondition(child != null, "ui_child_missing");
                            queue.addLast(child);
                        }
                    } finally { node.recycle(); }
                }
            } finally { recycleQueue(queue); }
            if (pass == 0 && (labels != 1 || owned != 1)) return false;
            if (pass == 1) return titles == 1;
        }
        return false;
    }

    private static NodeRef uniqueDeliveryRow(AccessibilityNodeInfo root, String appLabel, String title, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        AccessibilityNodeInfo selected = null;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    requireCondition(node.getWindowId() == root.getWindowId(), "ui_root_changed");
                    if (isSensitiveContent(node)) continue;
                    String id = node.getViewIdResourceName();
                    if (node.isVisibleToUser() && ("android:id/app_name_text".equals(id) || "com.android.systemui:id/app_name_text".equals(id)) && node.getText() != null && appLabel.contentEquals(node.getText())) {
                        AccessibilityNodeInfo ancestor = AccessibilityNodeInfo.obtain(node);
                        try {
                            for (int depth = 0; ancestor != null && depth < MAX_RECEIPT_ANCESTORS; depth++) {
                                NodeMetadata facts = metadata(ancestor);
                                if (facts.windowId != root.getWindowId()) throw new Blocked("ui_root_changed");
                                if (facts.visible && facts.enabled && facts.clickable && deliveryRowMatches(ancestor, appLabel, title, deadline)) {
                                    requireCondition(selected == null || selected.equals(ancestor), "notification_ambiguous");
                                    if (selected == null) selected = AccessibilityNodeInfo.obtain(ancestor);
                                    break;
                                }
                                AccessibilityNodeInfo parent = ancestor.getParent();
                                ancestor.recycle(); ancestor = parent;
                            }
                        } finally { if (ancestor != null) ancestor.recycle(); }
                    }
                    for (int i = 0; i < node.getChildCount(); i++) {
                        requireCondition(discovered++ < MAX_NODES, "ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(i);
                        requireCondition(child != null, "ui_child_missing");
                        queue.addLast(child);
                    }
                } finally { node.recycle(); }
            }
            requireCondition(selected != null, "notification_row_unavailable");
            NodeRef result = new NodeRef(selected, root.getWindowId()); selected = null; return result;
        } finally { recycleQueue(queue); if (selected != null) selected.recycle(); }
    }

    static void tapOwnedDeliveryNotification(Instrumentation instrumentation, android.app.NotificationManager manager,
            android.service.notification.StatusBarNotification selected, String host, String event) throws PackageManager.NameNotFoundException {
        long deadline = SystemClock.elapsedRealtime() + TIMEOUT_MILLIS;
        UiAutomation automation = deliveryAutomation(instrumentation);
        requireSingleDeliveryAlert(manager, selected, host, event);
        CharSequence raw = selected.getNotification().extras.getCharSequence(android.app.Notification.EXTRA_TITLE);
        requireCondition(raw != null && Arrays.asList("Work finished on desktop", "Desktop needs attention", "Agent finished", "Agent needs attention").contains(raw.toString()), "notification_title_unavailable");
        String title = raw.toString();
        String appLabel = instrumentation.getTargetContext().getApplicationInfo().loadLabel(instrumentation.getTargetContext().getPackageManager()).toString();
        String home = deliveryHomePackage(instrumentation);
        InteractiveWindowsService windows = enableInteractiveWindows(automation);
        AccessibilityNodeInfo originalShade = null;
        try {
            AccessibilityNodeInfo before = automation.getRootInActiveWindow();
            requireCondition(before != null, "ui_root_missing");
            int homeWindow;
            try {
                homeWindow = before.getWindowId();
                boolean shade = "com.android.systemui".contentEquals(before.getPackageName())
                        && deliveryScene(before, "com.android.systemui:id/notification_stack_scroller", null, deadline);
                requireCondition(home.contentEquals(before.getPackageName()) || shade, "ui_root_foreign");
                if (shade) originalShade = AccessibilityNodeInfo.obtain(before);
                requireBeforeDeadline(deadline);
                if (!shade) requireCondition(automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS), "ui_global_action_acknowledgement");
            } finally { before.recycle(); }
            while (SystemClock.elapsedRealtime() < deadline) {
                AccessibilityNodeInfo root = automation.getRootInActiveWindow();
                if (root == null) { SystemClock.sleep(POLL_MILLIS); continue; }
                try {
                    if (originalShade == null && home.contentEquals(root.getPackageName()) && root.getWindowId() == homeWindow) { SystemClock.sleep(POLL_MILLIS); continue; }
                    requireCondition("com.android.systemui".contentEquals(root.getPackageName())
                            && deliveryScene(root, "com.android.systemui:id/notification_stack_scroller", null, deadline), "ui_root_foreign");
                    if (originalShade == null) originalShade = AccessibilityNodeInfo.obtain(root);
                    else requireCondition(originalShade.equals(root), "ui_root_changed");
                    NodeRef row;
                    try { row = uniqueDeliveryRow(root, appLabel, title, deadline); }
                    catch (Blocked unavailable) {
                        if (!"notification_row_unavailable".equals(unavailable.code)) throw unavailable;
                        SystemClock.sleep(POLL_MILLIS); continue; // Observation only, same original shade.
                    }
                    try {
                        requireSingleDeliveryAlert(manager, selected, host, event);
                        AccessibilityNodeInfo current = automation.getRootInActiveWindow();
                        requireCondition(current != null, "ui_root_missing");
                        try {
                            requireCondition(current.equals(root) && current.getWindowId() == row.windowId && "com.android.systemui".contentEquals(current.getPackageName()), "ui_root_changed");
                            NodeRef same = uniqueDeliveryRow(current, appLabel, title, deadline);
                            try { requireCondition(same.node.equals(row.node) && row.node.refresh(), "ui_revalidation"); }
                            finally { same.node.recycle(); }
                        } finally { current.recycle(); }
                        NodeMetadata facts = metadata(row.node);
                        requireCondition(facts.visible && facts.enabled && facts.clickable && facts.windowId == root.getWindowId(), "ui_revalidation");
                        requireSingleDeliveryAlert(manager, selected, host, event);
                        requireCondition(deliveryRowMatches(row.node, appLabel, title, deadline), "ui_revalidation");
                        AccessibilityNodeInfo finalRoot = automation.getRootInActiveWindow();
                        requireCondition(finalRoot != null, "ui_root_missing");
                        try { requireCondition(finalRoot.equals(originalShade) && finalRoot.getWindowId() == row.windowId, "ui_root_changed"); }
                        finally { finalRoot.recycle(); }
                        requireBeforeDeadline(deadline);
                        requireCondition(row.node.performAction(AccessibilityNodeInfo.ACTION_CLICK), "ui_click_acknowledgement");
                        return; // Never retry or substitute an entered click.
                    } finally { row.node.recycle(); }
                } finally { root.recycle(); }
            }
            throw new Blocked("expected_screen_timeout");
        } finally { if (originalShade != null) originalShade.recycle(); restoreInteractiveWindows(automation, windows); }
    }

    private ActionResult clickAndObserve(UiAutomation automation, Action action, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        if (action.tag == null) throw new Blocked("action_tag");
        if (action == Action.NAV_HOSTS || action == Action.NAV_SESSIONS || action == Action.OPEN_SESSION_LIST_SETTINGS) {
            stage.stage = Stage.SELECTION;
            NodeRef menu = uniqueClickableTag(automation, "navigation_menu", deadline);
            boolean menuAccepted;
            try {
                stage.stage = Stage.CLICK;
                requireCondition(revalidate(automation, menu, "navigation_menu", deadline), "ui_revalidation");
                requireBeforeDeadline(deadline);
                menuAccepted = menu.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            } finally {
                menu.node.recycle();
            }
            requireCondition(menuAccepted, "ui_click_acknowledgement");
            stage.stage = Stage.POSTCONDITION;
            while (!tagPresent(automation, action.tag, deadline)) {
                requireBeforeDeadline(deadline);
                SystemClock.sleep(POLL_MILLIS);
            }
        }
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueClickableTag(automation, action.tag, deadline);
        boolean clickAccepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidate(automation, node, action.tag, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            clickAccepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(clickAccepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        if (action.expectedTag == null) throw new Blocked("expected_tag");
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitExpected(automation, action.expectedTag, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult selectAppearance(UiAutomation automation, UiAppearance appearance, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int settingsWindow = observeAppearance(automation, deadline).windowId;
        NodeRef node = uniqueClickableTag(automation, appearance.tag, deadline, settingsWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateAppearanceAction(automation, node, appearance, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitSelectedAppearance(automation, appearance, settingsWindow, deadline));
    }

    private Observation observeSessionListPreferences(UiAutomation automation, long deadline) {
        int windowId = appearanceSettingsWindow(automation, deadline);
        Observation observation = observeAppearance(automation, deadline);
        requireCondition(observation.windowId == windowId, "appearance_settings_window");
        boolean groupByStatus = sessionListPreferenceChecked(automation, SessionListPreference.GROUP_BY_STATUS, windowId, deadline);
        boolean workspaceContext = sessionListPreferenceChecked(automation, SessionListPreference.WORKSPACE_CONTEXT, windowId, deadline);
        boolean hostContext = sessionListPreferenceChecked(automation, SessionListPreference.HOST_CONTEXT, windowId, deadline);
        return observation.withSessionListPreferences(groupByStatus, workspaceContext, hostContext);
    }

    private ActionResult setSessionListPreference(UiAutomation automation, SessionListPreference preference, boolean desired, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        Observation current = observeSessionListPreferences(automation, deadline);
        if (sessionListPreferenceValue(current, preference) == desired) return new ActionResult(false, true, current);
        int windowId = current.windowId;
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueSessionListPreferenceSwitch(automation, preference, windowId, deadline);
        boolean accepted = false;
        boolean alreadyDesired = false;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateSessionListPreferenceSwitch(automation, node, preference, windowId, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            alreadyDesired = node.node.isChecked() == desired;
            if (!alreadyDesired) accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        stage.stage = Stage.POSTCONDITION;
        if (alreadyDesired) return new ActionResult(false, true, awaitSessionListPreference(automation, preference, desired, windowId, deadline));
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        return new ActionResult(true, true, awaitSessionListPreference(automation, preference, desired, windowId, deadline));
    }

    private Observation awaitSessionListPreference(UiAutomation automation, SessionListPreference preference, boolean desired, int windowId, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            Observation observation = observeSessionListPreferences(automation, deadline);
            if (observation.windowId == windowId && sessionListPreferenceValue(observation, preference) == desired) return observation;
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private boolean sessionListPreferenceValue(Observation observation, SessionListPreference preference) {
        if (!observation.sessionListPreferencesObserved) throw new Blocked("ui_tag_missing");
        switch (preference) {
            case GROUP_BY_STATUS: return observation.sessionListGroupByStatus;
            case WORKSPACE_CONTEXT: return observation.sessionListWorkspaceContext;
            case HOST_CONTEXT: return observation.sessionListHostContext;
        }
        throw new Blocked("session_list_preference_invalid");
    }

    private boolean sessionListPreferenceChecked(UiAutomation automation, SessionListPreference preference, int windowId, long deadline) {
        NodeRef node = uniqueSessionListPreferenceSwitch(automation, preference, windowId, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            requireCondition(metadata.checkable && metadata.visible && metadata.enabled && metadata.supports(AccessibilityNodeInfo.ACTION_CLICK), "ui_revalidation");
            return node.node.isChecked();
        } finally {
            node.node.recycle();
        }
    }

    private NodeRef uniqueSessionListPreferenceSwitch(UiAutomation automation, SessionListPreference preference, int expectedWindowId, long deadline) {
        NodeRef row = uniqueVisibleTag(automation, preference.tag, deadline, expectedWindowId);
        try {
            return uniqueCheckableDescendant(row.node, row.windowId, preference.tag, deadline);
        } finally {
            row.node.recycle();
        }
    }

    private NodeRef uniqueCheckableDescendant(AccessibilityNodeInfo root, int windowId, String requiredAncestorTag, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                boolean retained = false;
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                    int childCount = node.getChildCount();
                    for (int index = 0; index < childCount; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered >= MAX_NODES) throw new Blocked("ui_node_bound");
                        discovered++;
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child == null) throw new Blocked("ui_child_missing");
                        queue.addLast(child);
                    }
                    if (metadata.checkable && metadata.visible && metadata.enabled && metadata.supports(AccessibilityNodeInfo.ACTION_CLICK)) {
                        if (!hasAncestorTag(node, windowId, requiredAncestorTag, deadline)) throw new Blocked("ui_revalidation");
                        if (selected != null) throw new Blocked("ui_duplicate");
                        selected = node;
                        retained = true;
                    }
                } finally {
                    if (!retained) node.recycle();
                }
            }
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, windowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    private boolean revalidateSessionListPreferenceSwitch(UiAutomation automation, NodeRef node, SessionListPreference preference, int expectedWindowId, long deadline) {
        if (!node.node.refresh()) return false;
        AccessibilityNodeInfo product = currentProductRoot(automation, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            return product.getWindowId() == expectedWindowId
                    && PRODUCT_PACKAGE.equals(metadata.packageName)
                    && metadata.windowId == expectedWindowId
                    && metadata.checkable
                    && metadata.visible
                    && metadata.enabled
                    && metadata.supports(AccessibilityNodeInfo.ACTION_CLICK)
                    && hasAncestorTag(node.node, expectedWindowId, preference.tag, deadline);
        } finally {
            product.recycle();
        }
    }

    private ActionResult backgroundOwnedApp(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        stage.stage = Stage.CLICK;
        requireBeforeDeadline(deadline);
        boolean accepted = automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_HOME);
        requireCondition(accepted, "ui_global_action_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitOwnedHomeForeground(automation, deadline));
    }

    private Observation awaitOwnedHomeForeground(UiAutomation automation, long deadline) {
        String homePackage = resolvedHomePackage();
        while (SystemClock.elapsedRealtime() < deadline) {
            HomeWindowObservation observation = observeHomeWindows(automation, homePackage, deadline);
            if (observation.complete && observation.homeForeground && !observation.productForeground) {
                return Observation.empty().withHomeObservation(true, false);
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private String resolvedHomePackage() {
        Intent home = new Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME);
        android.content.pm.ResolveInfo resolved = getContext().getPackageManager().resolveActivity(home, PackageManager.MATCH_DEFAULT_ONLY);
        if (resolved == null || resolved.activityInfo == null || resolved.activityInfo.packageName == null || PRODUCT_PACKAGE.equals(resolved.activityInfo.packageName)) throw new Blocked("home_resolution_invalid");
        return resolved.activityInfo.packageName;
    }

    private HomeWindowObservation observeHomeWindows(UiAutomation automation, String homePackage, long deadline) {
        requireBeforeDeadline(deadline);
        List<AccessibilityWindowInfo> windows = automation.getWindows();
        if (windows == null) throw new Blocked("interactive_windows_unavailable");
        boolean complete = true;
        boolean productForeground = false;
        boolean homeForeground = false;
        try {
            for (AccessibilityWindowInfo window : windows) {
                requireBeforeDeadline(deadline);
                if (!window.isActive() && !window.isFocused()) continue;
                AccessibilityNodeInfo root = window.getRoot();
                if (root == null) { complete = false; continue; }
                try {
                    CharSequence packageName = root.getPackageName();
                    if (packageName == null) { complete = false; continue; }
                    if (PRODUCT_PACKAGE.contentEquals(packageName)) productForeground = true;
                    if (homePackage.contentEquals(packageName)) homeForeground = true;
                } finally {
                    root.recycle();
                }
            }
        } finally {
            for (AccessibilityWindowInfo window : windows) window.recycle();
        }
        return new HomeWindowObservation(complete, productForeground, homeForeground);
    }

    private ActionResult selectCodexProvider(UiAutomation automation, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int formWindow = creationSheetWindow(automation, deadline);
        NodeRef node = uniqueCodexProvider(automation, formWindow, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateCodexProvider(automation, node, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitSelectedCodexProvider(automation, formWindow, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult openSession(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String tag = SESSION_TAG_PREFIX + sessionId + "_" + sessionGeneration;
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueClickableTag(automation, tag, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidate(automation, node, tag, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitExactConversation(automation, sessionId, sessionGeneration, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult openHistoryDetails(UiAutomation automation, String sessionId, String sessionGeneration, String recordId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return toggleHistoryDetails(automation, sessionId, sessionGeneration, recordId, HISTORY_DETAILS, true, deadline, acknowledgement, stage);
    }

    private ActionResult closeHistoryDetails(UiAutomation automation, String sessionId, String sessionGeneration, String recordId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return toggleHistoryDetails(automation, sessionId, sessionGeneration, recordId, HIDE_DETAILS, false, deadline, acknowledgement, stage);
    }

    private ActionResult openScopedComposerSheet(UiAutomation automation, String sessionId, String sessionGeneration, String actionTag, String scopePrefix, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String scopeTag = scopedComposerSheetTag(scopePrefix, sessionId, sessionGeneration);
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueActionTag(automation, actionTag, AccessibilityNodeInfo.ACTION_CLICK, deadline, conversationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateConversationAction(automation, node, actionTag, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitScopedComposerSheet(automation, scopeTag, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult closeScopedComposerSheet(UiAutomation automation, String sessionId, String sessionGeneration, String scopePrefix, String closeTag, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String scopeTag = scopedComposerSheetTag(scopePrefix, sessionId, sessionGeneration);
        stage.stage = Stage.SELECTION;
        NodeRef sheet = uniqueVisibleTag(automation, scopeTag, deadline);
        NodeRef node = uniqueDialogAction(sheet.node, sheet.windowId, closeTag, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateDialogAction(automation, node, scopeTag, closeTag, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitExactConversation(automation, sessionId, sessionGeneration, deadline);
        return new ActionResult(true, true, observation);
    }

    /** Select once before input; a disabled visible inline recovery never falls back elsewhere. */
    private ActionResult refreshConversation(UiAutomation automation,String sessionId,String sessionGeneration,long deadline,Acknowledgement acknowledgement,StageTracker stage) {
        stage.stage=Stage.SELECTION;
        int window=exactConversationWindow(automation,sessionId,sessionGeneration,deadline);
        String tag=visibleInventoryRefresh(automation,window,deadline)?CONVERSATION_INVENTORY_REFRESH_TAG:CONVERSATION_REFRESH_TAG;
        return conversationControl(automation,sessionId,sessionGeneration,tag,AccessibilityNodeInfo.ACTION_CLICK,deadline,acknowledgement,stage,window);
    }
    private boolean visibleInventoryRefresh(UiAutomation automation,int window,long deadline) {
        AccessibilityNodeInfo root=currentProductRoot(automation,deadline);
        ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(root);
        int discovered=1,visible=0;
        try {
            while(!queue.isEmpty()) {
                AccessibilityNodeInfo node=queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    NodeMetadata metadata=metadata(node);
                    requireCondition(PRODUCT_PACKAGE.equals(metadata.packageName) && metadata.windowId==window,"ui_root_changed");
                    requireCondition(!node.isPassword() && !PAIRING_SCENE_TAGS.contains(metadata.viewId),"secret_scene");
                    if(isSensitiveContent(node)) continue;
                    if(CONVERSATION_INVENTORY_REFRESH_TAG.equals(metadata.viewId) && metadata.visible) visible++;
                    requireCondition(visible<=1,"ui_duplicate");
                    int children=node.getChildCount();
                    for(int i=0;i<children;i++) {
                        requireBeforeDeadline(deadline);
                        requireCondition(discovered++<MAX_NODES,"ui_node_bound");
                        AccessibilityNodeInfo child=node.getChild(i);
                        requireCondition(child!=null,"ui_child_missing");queue.addLast(child);
                    }
                } finally {node.recycle();}
            }
            return visible==1;
        } finally {recycleQueue(queue);}
    }

    /** Read-only conversation controls still require the original exact Target at admission. */
    private ActionResult conversationControl(UiAutomation automation, String sessionId, String sessionGeneration, String tag, int action, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return conversationControl(automation,sessionId,sessionGeneration,tag,action,deadline,acknowledgement,stage,NO_WINDOW);
    }
    private ActionResult conversationControl(UiAutomation automation,String sessionId,String sessionGeneration,String tag,int action,long deadline,Acknowledgement acknowledgement,StageTracker stage,int originalWindow) {
        stage.stage = Stage.SELECTION;
        int window = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        requireCondition(originalWindow==NO_WINDOW || window==originalWindow,"ui_root_changed");
        NodeRef node = uniqueActionTag(automation, tag, action, deadline, window);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateConversationAction(automation, node, tag, action, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(action);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_action_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation after=awaitExactConversation(automation,sessionId,sessionGeneration,deadline);
        requireCondition(originalWindow==NO_WINDOW || after.windowId==originalWindow,"ui_root_changed");
        return new ActionResult(true,true,after);
    }

    private String scopedComposerSheetTag(String prefix, String sessionId, String sessionGeneration) {
        return prefix + sessionId + "_" + sessionGeneration;
    }

    private Observation awaitScopedComposerSheet(UiAutomation automation, String scopeTag, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                NodeRef sheet = uniqueVisibleTag(automation, scopeTag, deadline);
                sheet.node.recycle();
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private ActionResult focusComposer(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueActionTag(automation, COMPOSER_TAG, AccessibilityNodeInfo.ACTION_CLICK, deadline, conversationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateConversationAction(automation, node, COMPOSER_TAG, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitComposerFocusAndIme(automation, sessionId, sessionGeneration, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult dismissComposerKeyboard(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        requireCondition(hasInputMethodWindow(automation, deadline), "ime_not_present");
        stage.stage = Stage.CLICK;
        // The active root must still be the exact conversation, never a dialog overlay.
        exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        requireCondition(hasInputMethodWindow(automation, deadline), "ime_not_present");
        requireBeforeDeadline(deadline);
        boolean accepted = automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK);
        requireCondition(accepted, "ui_global_action_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitImeDismissed(automation, sessionId, sessionGeneration, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult openTerminal(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueClickableTag(automation, TERMINAL_OPEN_TAG, deadline, conversationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateConversationAction(automation, node, TERMINAL_OPEN_TAG, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitTerminalDialog(automation, sessionId, sessionGeneration, deadline));
    }


    private ActionResult closeTerminal(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int dialogWindow = terminalTargetWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef dialog = uniqueVisibleTag(automation, TERMINAL_DIALOG_TAG, deadline, dialogWindow);
        NodeRef node = uniqueDialogAction(dialog.node, dialog.windowId, TERMINAL_CLOSE_TAG, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateTerminalDialogAction(automation, node, TERMINAL_CLOSE_TAG, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitExactConversation(automation, sessionId, sessionGeneration, deadline));
    }

    private ActionResult openWorktrees(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueClickableTag(automation, WORKTREES_OPEN_TAG, deadline, conversationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateConversationAction(automation, node, WORKTREES_OPEN_TAG, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitWorktreesDialog(automation, sessionId, sessionGeneration, deadline));
    }

    private ActionResult refreshWorktrees(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int dialogWindow = worktreesTargetWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef dialog = uniqueVisibleTag(automation, WORKTREES_DIALOG_TAG, deadline, dialogWindow);
        NodeRef node = uniqueDialogAction(dialog.node, dialog.windowId, WORKTREES_REFRESH_TAG, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateWorktreesDialogAction(automation, node, WORKTREES_REFRESH_TAG, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitWorktreesDialog(automation, sessionId, sessionGeneration, deadline));
    }

    private ActionResult closeWorktrees(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int dialogWindow = worktreesTargetWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef dialog = uniqueVisibleTag(automation, WORKTREES_DIALOG_TAG, deadline, dialogWindow);
        NodeRef node = uniqueDialogAction(dialog.node, dialog.windowId, WORKTREES_CLOSE_TAG, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateWorktreesDialogAction(automation, node, WORKTREES_CLOSE_TAG, sessionId, sessionGeneration, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitExactConversation(automation, sessionId, sessionGeneration, deadline));
    }

    private ActionResult openChanges(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueClickableTag(automation, CHANGES_OPEN_TAG, deadline, conversationWindow);
        boolean accepted;
        try { stage.stage = Stage.CLICK; requireCondition(revalidateConversationAction(automation, node, CHANGES_OPEN_TAG, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline), "ui_revalidation"); requireBeforeDeadline(deadline); accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK); }
        finally { node.node.recycle(); }
        requireCondition(accepted, "ui_click_acknowledgement"); acknowledgement.value = true; stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitChangesDialog(automation, sessionId, sessionGeneration, deadline));
    }

    private ActionResult refreshChanges(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return changesDialogAction(automation, sessionId, sessionGeneration, deadline, acknowledgement, stage, CHANGES_REFRESH_TAG, false);
    }

    private ActionResult closeChanges(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return changesDialogAction(automation, sessionId, sessionGeneration, deadline, acknowledgement, stage, CHANGES_CLOSE_TAG, true);
    }

    private ActionResult changesDialogAction(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage, String tag, boolean closing) {
        stage.stage = Stage.SELECTION;
        int dialogWindow = changesTargetWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef dialog = uniqueVisibleTag(automation, CHANGES_DIALOG_TAG, deadline, dialogWindow);
        NodeRef node = uniqueDialogAction(dialog.node, dialog.windowId, tag, deadline);
        boolean accepted;
        try { stage.stage = Stage.CLICK; requireCondition(revalidateChangesDialogAction(automation, node, tag, sessionId, sessionGeneration, deadline), "ui_revalidation"); requireBeforeDeadline(deadline); accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK); }
        finally { node.node.recycle(); }
        requireCondition(accepted, "ui_click_acknowledgement"); acknowledgement.value=true; stage.stage=Stage.POSTCONDITION;
        return new ActionResult(true, true, closing ? awaitExactConversation(automation, sessionId, sessionGeneration, deadline) : awaitChangesDialog(automation, sessionId, sessionGeneration, deadline));
    }

    private String snippetScopeTag(Request request) { return scopedComposerSheetTag(SNIPPET_SCOPE_TAG_PREFIX, request.sessionId, request.sessionGeneration); }
    private String snippetRowTag(String id) { return SNIPPET_ROW_TAG_PREFIX + id; }
    private String snippetEditorTag(String id) { return SNIPPET_EDITOR_TAG_PREFIX + id; }
    private String snippetText(Request request, boolean title) {
        String compact = request.snippetWorkflowId.replace("-", "");
        if (title) return "Goatr local " + request.snippetWorkflowId.substring(0, 8) + (request.snippetVersion == SnippetVersion.EDITED ? " edited" : "");
        return "GOATR_LOCAL_SNIPPET_" + compact + "_" + request.snippetVersion.wire.toUpperCase(java.util.Locale.ROOT);
    }
    private boolean knownSnippetText(Request request, boolean title, CharSequence value) {
        if (value == null) return true;
        for (SnippetVersion version : SnippetVersion.values()) {
            Request candidate = new Request(request.runId, request.action, request.appearance, request.sessionListPreference, request.sessionListPreferenceValue, request.creationServerLabel, request.creationServerId, request.creationProjectLabel, request.creationName, request.previousCreationName, request.composerText, request.sessionId, request.sessionGeneration, request.historyRecordId, request.receiptId, request.excludedReceiptIds, request.snippetWorkflowId, request.snippetId, version);
            if (snippetText(candidate, title).contentEquals(value)) return true;
        }
        return value.length() == 0;
    }
    private String canonicalSnippetEditorId(String tag) {
        if (tag == null || !tag.startsWith(SNIPPET_EDITOR_TAG_PREFIX)) return null;
        String candidate = tag.substring(SNIPPET_EDITOR_TAG_PREFIX.length());
        try { return UUID.fromString(candidate).toString().equals(candidate) ? candidate : null; }
        catch (IllegalArgumentException exception) { return null; }
    }

    private NodeRef snippetScopedAction(UiAutomation automation, Request request, String tag, int actionId, long deadline) {
        NodeRef scope = uniqueVisibleTag(automation, snippetScopeTag(request), deadline);
        try { return uniqueSnippetChild(scope.node, scope.windowId, tag, actionId, deadline); }
        finally { scope.node.recycle(); }
    }

    private NodeRef snippetEditorField(UiAutomation automation, Request request, String fieldTag, long deadline) {
        NodeRef editor = uniqueVisibleTag(automation, snippetEditorTag(request.snippetId), deadline);
        try {
            requireCondition(hasAncestorTag(editor.node, editor.windowId, snippetScopeTag(request), deadline), "snippet_scope_mismatch");
            return uniqueSnippetChild(editor.node, editor.windowId, fieldTag, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline);
        } finally { editor.node.recycle(); }
    }

    private boolean revalidateSnippetAction(UiAutomation automation, NodeRef node, Request request, String tag, int actionId, long deadline) {
        if (!revalidateAction(automation, node, tag, actionId, deadline)) return false;
        if (!hasAncestorTag(node.node, node.windowId, snippetScopeTag(request), deadline)) return false;
        NodeRef scope = uniqueVisibleTag(automation, snippetScopeTag(request), deadline, node.windowId);
        scope.node.recycle();
        return true;
    }

    private ActionResult newSnippet(UiAutomation automation, String sessionId, String sessionGeneration, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        Request scope = new Request(null, Action.NEW_SNIPPET, null, null, null, null, null, null, null, null, null, sessionId, sessionGeneration, null, null, new HashSet<String>(), "00000000-0000-0000-0000-000000000000", null, null);
        stage.stage = Stage.SELECTION;
        NodeRef node = snippetScopedAction(automation, scope, SNIPPET_NEW_TAG, AccessibilityNodeInfo.ACTION_CLICK, deadline);
        boolean accepted;
        try { stage.stage = Stage.CLICK; requireCondition(revalidateSnippetAction(automation, node, scope, SNIPPET_NEW_TAG, AccessibilityNodeInfo.ACTION_CLICK, deadline), "ui_revalidation"); requireBeforeDeadline(deadline); accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK); }
        finally { node.node.recycle(); }
        requireCondition(accepted, "ui_click_acknowledgement"); acknowledgement.value = true; stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitSnippetEditor(automation, scope, deadline));
    }

    private ActionResult setSnippetField(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        boolean title = request.action == Action.SET_SNIPPET_TITLE;
        String tag = title ? SNIPPET_TITLE_TAG : SNIPPET_BODY_TAG;
        String expected = snippetText(request, title);
        stage.stage = Stage.SELECTION;
        NodeRef node = snippetEditorField(automation, request, tag, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateSnippetAction(automation, node, request, tag, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline) && hasAncestorTag(node.node,node.windowId,snippetEditorTag(request.snippetId),deadline), "ui_revalidation");
            requireCondition(knownSnippetText(request, title, node.node.getText()), "snippet_field_not_owned");
            Bundle arguments = new Bundle(); arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, expected);
            requireBeforeDeadline(deadline); accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments);
        } finally { node.node.recycle(); }
        requireCondition(accepted, "ui_set_text_acknowledgement"); acknowledgement.value = true; stage.stage = Stage.POSTCONDITION;
        return new ActionResult(true, true, awaitSnippetField(automation, request, tag, expected, deadline));
    }

    private ActionResult saveSnippet(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        requireOwnedSnippetEditorPair(automation, request, deadline);
        stage.stage = Stage.SELECTION;
        NodeRef node = snippetScopedAction(automation, request, SNIPPET_SAVE_TAG, AccessibilityNodeInfo.ACTION_CLICK, deadline);
        boolean accepted;
        try { stage.stage = Stage.CLICK; requireCondition(revalidateSnippetAction(automation, node, request, SNIPPET_SAVE_TAG, AccessibilityNodeInfo.ACTION_CLICK, deadline), "ui_revalidation"); requireOwnedSnippetEditorPair(automation, request, deadline); requireBeforeDeadline(deadline); accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK); }
        finally { node.node.recycle(); }
        requireCondition(accepted,"ui_click_acknowledgement"); acknowledgement.value=true; stage.stage=Stage.POSTCONDITION;
        return new ActionResult(true,true,awaitSavedSnippet(automation,request,deadline));
    }

    private ActionResult editSnippet(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) { return snippetRowClick(automation, request, SNIPPET_EDIT_TAG_PREFIX + request.snippetId, true, deadline, acknowledgement, stage); }
    private ActionResult openSnippetDelete(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) { return snippetRowClick(automation, request, SNIPPET_DELETE_TAG_PREFIX + request.snippetId, false, deadline, acknowledgement, stage); }

    private ActionResult snippetRowClick(UiAutomation automation, Request request, String tag, boolean editor, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        requireSnippetRow(automation, request, deadline);
        stage.stage=Stage.SELECTION; NodeRef node=snippetScopedAction(automation,request,tag,AccessibilityNodeInfo.ACTION_CLICK,deadline); boolean accepted;
        try { stage.stage=Stage.CLICK; requireCondition(revalidateSnippetAction(automation,node,request,tag,AccessibilityNodeInfo.ACTION_CLICK,deadline) && snippetRowPresent(automation,request,deadline),"ui_revalidation"); requireBeforeDeadline(deadline); accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK); }
        finally {node.node.recycle();}
        requireCondition(accepted,"ui_click_acknowledgement"); acknowledgement.value=true; stage.stage=Stage.POSTCONDITION;
        return new ActionResult(true,true,editor ? awaitSnippetEditor(automation,request,deadline) : awaitSnippetDeleteDialog(automation,request,deadline));
    }

    private ActionResult insertSnippet(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        SnippetVersion matched=requireOwnedSnippetRowPair(automation,request,deadline); requireSnippetDraftEmpty(automation,request,deadline);
        String expected=snippetText(withSnippetVersion(request,matched),false);
        String tag=SNIPPET_INSERT_TAG_PREFIX+request.snippetId; stage.stage=Stage.SELECTION; NodeRef node=snippetScopedAction(automation,request,tag,AccessibilityNodeInfo.ACTION_CLICK,deadline); boolean accepted;
        try {stage.stage=Stage.CLICK; requireCondition(revalidateSnippetAction(automation,node,request,tag,AccessibilityNodeInfo.ACTION_CLICK,deadline)&&snippetRowPresent(automation,request,deadline),"ui_revalidation"); matched=requireOwnedSnippetRowPair(automation,request,deadline); expected=snippetText(withSnippetVersion(request,matched),false); requireSnippetDraftEmpty(automation,request,deadline); requireBeforeDeadline(deadline); accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);} finally {node.node.recycle();}
        requireCondition(accepted,"ui_click_acknowledgement"); acknowledgement.value=true; stage.stage=Stage.POSTCONDITION;
        return new ActionResult(true,true,awaitExactComposerText(automation,request.sessionId,request.sessionGeneration,expected,deadline));
    }

    private ActionResult confirmSnippetDelete(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        int window=snippetDeleteDialogWindow(automation,request,deadline); NodeRef scope=uniqueVisibleTag(automation,SNIPPET_DELETE_SCOPE_TAG_PREFIX+request.snippetId,deadline,window); NodeRef node;
        try {node=uniqueSnippetChild(scope.node,window,SNIPPET_DELETE_CONFIRM_TAG,AccessibilityNodeInfo.ACTION_CLICK,deadline);} finally {scope.node.recycle();}
        boolean accepted; stage.stage=Stage.SELECTION;
        try {stage.stage=Stage.CLICK; requireCondition(revalidateSnippetDeleteConfirm(automation,node,request,deadline),"ui_revalidation"); requireBeforeDeadline(deadline); accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);} finally {node.node.recycle();}
        requireCondition(accepted,"ui_click_acknowledgement"); acknowledgement.value=true; stage.stage=Stage.POSTCONDITION;
        return new ActionResult(true,true,awaitDeletedSnippet(automation,request,deadline));
    }

    private ActionResult observeSnippet(UiAutomation automation, Request request, long deadline) { requireSnippetKnownState(automation,request,deadline); return new ActionResult(false,true,observe(automation,deadline)); }
    private ActionResult observeSnippetDraft(UiAutomation automation, Request request, long deadline) { requireSnippetScope(automation,request,deadline); requireSnippetDraftMarker(automation,request,deadline); return new ActionResult(false,true,observe(automation,deadline)); }

    private ActionResult clearOwnedSnippetDraft(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String original=snippetText(withSnippetVersion(request,SnippetVersion.ORIGINAL),false); String edited=snippetText(withSnippetVersion(request,SnippetVersion.EDITED),false);
        stage.stage=Stage.SELECTION; int window=exactConversationWindow(automation,request.sessionId,request.sessionGeneration,deadline); NodeRef node=uniqueSetTextTag(automation,COMPOSER_TAG,deadline,window); boolean accepted;
        try {stage.stage=Stage.CLICK; requireCondition(revalidateConversationAction(automation,node,COMPOSER_TAG,AccessibilityNodeInfo.ACTION_SET_TEXT,request.sessionId,request.sessionGeneration,deadline),"ui_revalidation"); requireCondition(original.contentEquals(node.node.getText())||edited.contentEquals(node.node.getText()),"snippet_draft_not_owned"); Bundle arguments=new Bundle();arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,"");requireBeforeDeadline(deadline);accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,arguments);} finally {node.node.recycle();}
        requireCondition(accepted,"ui_set_text_acknowledgement");acknowledgement.value=true;stage.stage=Stage.POSTCONDITION;return new ActionResult(true,true,awaitExactComposerText(automation,request.sessionId,request.sessionGeneration,"",deadline));
    }

    private ActionResult closeSnippetEditor(UiAutomation automation, Request request, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        NodeRef node=snippetScopedAction(automation,request,SNIPPET_EDITOR_BACK_TAG,AccessibilityNodeInfo.ACTION_CLICK,deadline);boolean accepted;stage.stage=Stage.SELECTION;
        try {stage.stage=Stage.CLICK;requireCondition(revalidateSnippetAction(automation,node,request,SNIPPET_EDITOR_BACK_TAG,AccessibilityNodeInfo.ACTION_CLICK,deadline),"ui_revalidation");requireClosableSnippetEditor(automation,request,deadline);requireBeforeDeadline(deadline);accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);} finally {node.node.recycle();}
        requireCondition(accepted,"ui_click_acknowledgement");acknowledgement.value=true;stage.stage=Stage.POSTCONDITION;return new ActionResult(true,true,awaitSnippetListScene(automation,request,deadline));
    }

    private ActionResult toggleHistoryDetails(UiAutomation automation, String sessionId, String sessionGeneration, String recordId, String expectedLabel, boolean expanded, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueHistoryDetailsAction(automation, recordId, expectedLabel, conversationWindow, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateHistoryDetailsAction(automation, node, sessionId, sessionGeneration, recordId, expectedLabel, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitHistoryDetails(automation, sessionId, sessionGeneration, recordId, expanded, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult setComposer(UiAutomation automation, String sessionId, String sessionGeneration, String text, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        NodeRef node = uniqueSetTextTag(automation, COMPOSER_TAG, deadline, conversationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateConversationAction(automation, node, COMPOSER_TAG, AccessibilityNodeInfo.ACTION_SET_TEXT, sessionId, sessionGeneration, deadline), "ui_revalidation");
            CharSequence current = node.node.getText();
            requireCondition(current == null || current.length() == 0, "composer_not_empty");
            Bundle arguments = new Bundle();
            arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text);
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_set_text_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitExactComposerText(automation, sessionId, sessionGeneration, text, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult submitPrompt(UiAutomation automation, String sessionId, String sessionGeneration, String text, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        requireExactComposerText(automation, sessionId, sessionGeneration, text, deadline);
        NodeRef node = uniqueClickableTag(automation, PROMPT_SEND_TAG, deadline, conversationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidatePromptSend(automation, node, sessionId, sessionGeneration, text, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitExactConversation(automation, sessionId, sessionGeneration, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult inspectReceipt(UiAutomation automation, String receiptId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueReceiptInspect(automation, receiptId, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateReceiptInspect(automation, node, receiptId, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitReceiptPanel(automation, receiptId, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult openReceiptDetails(UiAutomation automation, String receiptId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String tag = RECEIPT_DETAILS_TAG_PREFIX + receiptId;
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueReceiptAction(automation, receiptId, tag, ACTION_DETAILS, AccessibilityNodeInfo.ACTION_CLICK, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateReceiptAction(automation, node, receiptId, tag, ACTION_DETAILS, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitReceiptDetails(automation, receiptId, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult prepareSeparateCreation(UiAutomation automation, String receiptId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String tag = SEPARATE_CREATION_PREPARE_PREFIX + receiptId;
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueReceiptAction(automation, receiptId, tag, null, AccessibilityNodeInfo.ACTION_CLICK, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateReceiptAction(automation, node, receiptId, tag, null, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitSeparateCreationDialog(automation, receiptId, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult confirmSeparateCreation(UiAutomation automation, String receiptId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        String dialogTag = SEPARATE_CREATION_DIALOG_PREFIX + receiptId;
        String confirmTag = SEPARATE_CREATION_CONFIRM_PREFIX + receiptId;
        stage.stage = Stage.SELECTION;
        NodeRef dialog = uniqueVisibleTag(automation, dialogTag, deadline);
        NodeRef node;
        try {
            node = uniqueDialogAction(dialog.node, dialog.windowId, confirmTag, deadline);
        } catch (Throwable failure) {
            throw failure;
        }
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateDialogAction(automation, node, dialogTag, confirmTag, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitEmptyCreationForm(automation, deadline);
        return new ActionResult(true, true, observation);
    }

    private static boolean validOwnedRouteDirect(String value) {
        if (value == null || value.length() > 128 || !value.matches("(?:0|[1-9][0-9]{0,2})(?:\\.(?:0|[1-9][0-9]{0,2})){3}:[1-9][0-9]{0,4}")) return false;
        int colon=value.indexOf(':');
        for(String octet:value.substring(0,colon).split("\\.")) if(Integer.parseInt(octet)>255) return false;
        return Integer.parseInt(value.substring(colon+1))<=65535;
    }
    private String routeScopeTag(Request request) {return "host_routes_"+request.routeHostId;}
    private static final class RouteFacts {
        final int window;
        int directCount;
        boolean fieldEmpty, fieldOwned, fieldEnabled, fieldSetText, saveFound, saveEnabled, discardFound, discardEnabled, attemptOwned;
        RouteFacts(int window) {this.window=window;}
        boolean saved() {return directCount==1 && fieldOwned && fieldEnabled && saveFound && !saveEnabled && discardFound && !discardEnabled;}
    }
    /** Complete bounded subtree traversal; only the owned literal is compared, never emitted. */
    private RouteFacts routeFacts(UiAutomation automation,Request request,long deadline) {
        NodeRef scope=uniqueVisibleTag(automation,routeScopeTag(request),deadline);
        ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();
        RouteFacts facts=new RouteFacts(scope.windowId);
        Set<String> seen=new HashSet<>();
        int discovered=1;
        try {
            queue.add(AccessibilityNodeInfo.obtain(scope.node));
            while(!queue.isEmpty()) {
                AccessibilityNodeInfo node=queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    NodeMetadata metadata=metadata(node);
                    requireCondition(PRODUCT_PACKAGE.equals(metadata.packageName) && metadata.windowId==scope.windowId,"ui_root_changed");
                    requireCondition(!isSensitiveContent(node),"secret_scene");
                    String tag=metadata.viewId;
                    if(tag!=null && (tag.startsWith("route_direct_") || "route_save".equals(tag) || "route_discard".equals(tag) || "route_attempt_hints".equals(tag))) {
                        requireCondition(seen.add(tag),"ui_duplicate");
                        if(tag.startsWith("route_direct_")) {
                            facts.directCount++;
                            if("route_direct_0".equals(tag)) {
                                CharSequence text=node.getText();
                                facts.fieldEmpty=text==null || text.length()==0;
                                facts.fieldOwned=request.routeDirect!=null && request.routeDirect.contentEquals(text==null?"":text);
                                facts.fieldEnabled=metadata.enabled;
                                facts.fieldSetText=metadata.visible && metadata.enabled && metadata.setText;
                            }
                        } else if("route_save".equals(tag)) {facts.saveFound=true;facts.saveEnabled=metadata.enabled;}
                        else if("route_discard".equals(tag)) {facts.discardFound=true;facts.discardEnabled=metadata.enabled;}
                        else if(request.routeDirect!=null) {
                            CharSequence text=node.getText();
                            if(text!=null && text.length()<=4096) for(String line:text.toString().split("\n",-1)) if(request.routeDirect.equals(line)) facts.attemptOwned=true;
                        }
                    }
                    int children=node.getChildCount();
                    for(int i=0;i<children;i++) {
                        requireBeforeDeadline(deadline);
                        requireCondition(discovered++<MAX_NODES,"ui_node_bound");
                        AccessibilityNodeInfo child=node.getChild(i);
                        requireCondition(child!=null,"ui_child_missing");
                        queue.addLast(child);
                    }
                } finally {node.recycle();}
            }
            requireCondition(scope.node.refresh(),"ui_revalidation");
            NodeRef current=uniqueVisibleTag(automation,routeScopeTag(request),deadline,scope.windowId);
            try {requireCondition(current.windowId==scope.windowId,"ui_root_changed");} finally {current.node.recycle();}
            return facts;
        } finally {scope.node.recycle();recycleQueue(queue);}
    }
    private void routeObservation(Observation observation,RouteFacts facts) {
        observation.hostRouteScope=true;
        observation.routeDirectOwned=facts.directCount==1 && facts.fieldOwned;
        observation.routeSaved=facts.saved();
        observation.routeReconnectAttempted=facts.saved() && facts.attemptOwned;
    }
    private void requireRouteNode(UiAutomation automation,Request request,NodeRef node,String tag,int action,long deadline) {
        requireCondition(revalidateAction(automation,node,tag,action,deadline) && hasAncestorTag(node.node,node.windowId,routeScopeTag(request),deadline),"ui_revalidation");
        RouteFacts facts=routeFacts(automation,request,deadline);
        requireCondition(facts.window==node.windowId,"ui_root_changed");
    }
    private RouteFacts awaitRouteState(UiAutomation automation,Request request,int originalWindow,int state,long deadline) {
        while(SystemClock.elapsedRealtime()<deadline) {
            try {
                RouteFacts facts=routeFacts(automation,request,deadline);
                requireCondition(originalWindow==NO_WINDOW || facts.window==originalWindow,"ui_root_changed");
                if(state==0 || state==1 && facts.directCount==1 && facts.fieldEmpty && facts.fieldSetText || state==2 && facts.directCount==1 && facts.fieldOwned && facts.fieldSetText || state==3 && facts.saved() || state==4 && facts.saved() && facts.attemptOwned) return facts;
            } catch(Blocked blocked) {if(!"ui_tag_missing".equals(blocked.code)) throw blocked;}
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("route_postcondition_timeout");
    }
    private ActionResult ownedRouteAction(UiAutomation automation,Request request,long deadline,Acknowledgement acknowledgement,StageTracker stage,Observation observation) {
        stage.stage=Stage.SELECTION;
        if(request.action==Action.OPEN_OWNED_HOST_ROUTES) {
            String tag="host_settings_"+request.routeHostId;
            NodeRef node=uniqueClickableTag(automation,tag,deadline);
            int window=node.windowId;
            try {
                stage.stage=Stage.CLICK;
                requireCondition(revalidateAction(automation,node,tag,AccessibilityNodeInfo.ACTION_CLICK,deadline),"ui_revalidation");
                requireBeforeDeadline(deadline);
                boolean accepted=node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
                acknowledgement.value=accepted;
                requireCondition(accepted,"ui_click_acknowledgement");
            } finally {node.node.recycle();}
            stage.stage=Stage.POSTCONDITION;
            routeObservation(observation,awaitRouteState(automation,request,window,0,deadline));
            return new ActionResult(true,true,observation);
        }
        RouteFacts before;
        try {before=routeFacts(automation,request,deadline);}
        catch(Blocked blocked) {if(request.action==Action.OBSERVE_OWNED_HOST_ROUTES && "ui_tag_missing".equals(blocked.code)) return new ActionResult(false,false,observation);throw blocked;}
        routeObservation(observation,before);
        if(request.action==Action.OBSERVE_OWNED_HOST_ROUTES) return new ActionResult(false,true,observation);
        final String tag;
        final int input;
        final int expected;
        if(request.action==Action.ADD_OWNED_ROUTE_DIRECT) {requireCondition(before.directCount==0,"route_direct_not_empty");tag="route_add";input=AccessibilityNodeInfo.ACTION_CLICK;expected=1;}
        else if(request.action==Action.SET_OWNED_ROUTE_DIRECT) {requireCondition(before.directCount==1 && before.fieldEmpty && before.fieldSetText,"route_direct_not_empty");tag="route_direct_0";input=AccessibilityNodeInfo.ACTION_SET_TEXT;expected=2;}
        else if(request.action==Action.SAVE_OWNED_ROUTE_HINTS) {requireCondition(before.directCount==1 && before.fieldOwned && before.fieldEnabled && before.saveFound && before.saveEnabled,"route_direct_not_owned");tag="route_save";input=AccessibilityNodeInfo.ACTION_CLICK;expected=3;}
        else {requireCondition(before.saved() && !before.attemptOwned,"route_reconnect_not_fresh");tag="route_reconnect";input=AccessibilityNodeInfo.ACTION_CLICK;expected=4;}
        NodeRef node=uniqueActionTag(automation,tag,input,deadline,before.window);
        try {
            stage.stage=Stage.CLICK;
            requireRouteNode(automation,request,node,tag,input,deadline);
            RouteFacts current=routeFacts(automation,request,deadline);
            requireCondition(current.window==before.window,"ui_root_changed");
            if(expected==1) requireCondition(current.directCount==0,"route_direct_not_empty");
            else if(expected==2) requireCondition(current.directCount==1 && current.fieldEmpty && current.fieldSetText && (node.node.getText()==null || node.node.getText().length()==0),"route_direct_not_empty");
            else if(expected==3) requireCondition(current.directCount==1 && current.fieldOwned && current.fieldEnabled && current.saveEnabled,"route_direct_not_owned");
            else requireCondition(current.saved() && !current.attemptOwned,"route_reconnect_not_fresh");
            requireRouteNode(automation,request,node,tag,input,deadline);
            requireBeforeDeadline(deadline);
            boolean accepted;
            if(input==AccessibilityNodeInfo.ACTION_SET_TEXT) {
                Bundle arguments=new Bundle();arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,request.routeDirect);
                accepted=node.node.performAction(input,arguments);
            } else accepted=node.node.performAction(input);
            acknowledgement.value=accepted;
            requireCondition(accepted,input==AccessibilityNodeInfo.ACTION_SET_TEXT?"ui_set_text_acknowledgement":"ui_click_acknowledgement");
        } finally {node.node.recycle();}
        stage.stage=Stage.POSTCONDITION;
        RouteFacts after=awaitRouteState(automation,request,before.window,expected,deadline);
        routeObservation(observation,after);
        if(expected==4) observation.routeReconnectAttempted=true;
        return new ActionResult(true,true,observation);
    }

    private ActionResult scrollHostSettings(UiAutomation automation, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int settingsWindow = settingsWindow(automation, deadline);
        NodeRef node = uniqueHostSettingsScrollForward(automation, settingsWindow, deadline);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateHostSettingsScrollForward(automation, node, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitExpected(automation, HOST_SETTINGS_PANEL_TAG, deadline);
        return new ActionResult(true, true, observation);
    }

    private ActionResult scrollSessions(UiAutomation automation, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        NodeRef node = uniqueHostSessionsScrollForward(automation, deadline);
        int sessionsWindow = node.windowId;
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateHostSessionsScrollForward(automation, node, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
            requireCondition(accepted, "ui_click_acknowledgement");
            acknowledgement.value = true;
            stage.stage = Stage.POSTCONDITION;
            requireCondition(revalidateScrolledSessionsContainer(automation, node, deadline), "ui_revalidation");
            Observation observation = observe(automation, deadline);
            requireCondition(observation.windowId == sessionsWindow, "ui_root_changed");
            return new ActionResult(true, true, observation);
        } finally {
            node.node.recycle();
        }
    }

    private ActionResult setCreationName(UiAutomation automation, String name, String previousName, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int creationWindow = previousName == null ? NO_WINDOW : creationSheetWindow(automation, deadline);
        NodeRef node = previousName == null
                ? uniqueSetTextTag(automation, CREATE_NAME_TAG, deadline)
                : uniqueActionTag(automation, CREATE_NAME_TAG, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline, creationWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateAction(automation, node, CREATE_NAME_TAG, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline), "ui_revalidation");
            if (previousName != null) {
                requireCondition(creationSheetWindow(automation, deadline) == node.windowId, "creation_form_window");
                requireCondition(revalidateAction(automation, node, CREATE_NAME_TAG, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline), "ui_revalidation");
            }
            CharSequence current = node.node.getText();
            if (previousName == null) {
                requireCondition(current == null || current.length() == 0, "creation_name_not_empty");
            } else {
                requireCondition(current != null && previousName.contentEquals(current), "creation_name_previous_mismatch");
            }
            Bundle arguments = new Bundle();
            arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, name);
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_set_text_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitTextExact(automation, CREATE_NAME_TAG, name, deadline);
        return new ActionResult(true, true, observation);
    }

    /** Read only the closed picker selectors from product-owned interactive roots. */
    private ActionResult observeCreationPicker(UiAutomation automation, long deadline) {
        CreationPickerObservation picker = new CreationPickerObservation();
        List<AccessibilityWindowInfo> windows = automation.getWindows();
        if (windows == null) throw new Blocked("interactive_windows_unavailable");
        try {
            for (AccessibilityWindowInfo window : windows) {
                requireBeforeDeadline(deadline);
                AccessibilityNodeInfo root = window.getRoot();
                if (root == null) throw new Blocked("interactive_window_root_missing");
                CharSequence packageName = root.getPackageName();
                if (packageName == null || !PRODUCT_PACKAGE.contentEquals(packageName)) {
                    root.recycle();
                    continue;
                }
                if (++picker.productWindowCount > MAX_NODES) {
                    root.recycle();
                    throw new Blocked("ui_node_bound");
                }
                try {
                    observeCreationPickerWindow(root, window.getId(), picker, deadline);
                } finally {
                    root.recycle();
                }
            }
        } finally {
            for (AccessibilityWindowInfo window : windows) window.recycle();
        }
        if (picker.productWindowCount == 0) throw new Blocked("product_window_missing");
        Observation observation = Observation.empty();
        observation.creationPickerTraversalComplete = true;
        observation.creationPickerProductWindowCount = picker.productWindowCount;
        observation.creationCloseTotal = picker.closeTotal;
        observation.creationCloseVisible = picker.closeVisible;
        observation.creationCloseEnabled = picker.closeEnabled;
        observation.creationServerTotal = picker.serverTotal;
        observation.creationServerVisible = picker.serverVisible;
        observation.creationServerOptionTotal = picker.optionTotal;
        observation.creationServerOptionVisible = picker.optionVisible;
        observation.creationServerOptionEnabled = picker.optionEnabled;
        observation.creationServerOptionClickSupported = picker.optionClickSupported;
        observation.creationServerOptionExpectedLabelCount = picker.optionExpectedLabelCount;
        observation.creationServerOptionExpectedLabel = picker.optionExpectedLabelCount > 0;
        if (!picker.sheetWindows.isEmpty() && !picker.optionWindows.isEmpty()) {
            if (picker.sheetWindows.size() != 1 || picker.optionWindows.size() != 1) {
                throw new Blocked("creation_picker_window_ambiguous");
            }
            observation.creationSheetAndOptionDifferentWindows = !picker.sheetWindows.iterator().next().equals(picker.optionWindows.iterator().next());
        }
        return new ActionResult(false, false, observation);
    }

    private void observeCreationPickerWindow(AccessibilityNodeInfo root, int windowId, CreationPickerObservation picker, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    if (NEW_SESSION_CLOSE_TAG.equals(metadata.viewId)) {
                        picker.closeTotal++;
                        if (metadata.visible) picker.closeVisible++;
                        if (metadata.enabled) picker.closeEnabled++;
                        picker.sheetWindows.add(windowId);
                    }
                    if (CREATE_SERVER_TAG.equals(metadata.viewId)) {
                        picker.serverTotal++;
                        if (metadata.visible) picker.serverVisible++;
                    }
                    if (isCreationPickerOptionTag(metadata.viewId, CREATE_SERVER_OPTION_TAG_PREFIX)) {
                        picker.optionTotal++;
                        if (metadata.visible) picker.optionVisible++;
                        if (metadata.enabled) picker.optionEnabled++;
                        if (metadata.clickable) picker.optionClickSupported++;
                        if (creationPickerOptionHasExpectedLabel(node, windowId, deadline)) picker.optionExpectedLabelCount++;
                        picker.optionWindows.add(windowId);
                    }
                    int childCount = node.getChildCount();
                    for (int index = 0; index < childCount; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered++ >= MAX_NODES) throw new Blocked("ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child == null) throw new Blocked("ui_child_missing");
                        queue.addLast(child);
                    }
                } finally {
                    node.recycle();
                }
            }
        } finally {
            recycleQueue(queue);
        }
    }

    private boolean creationPickerOptionHasExpectedLabel(AccessibilityNodeInfo root, int windowId, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    CharSequence text = node.getText();
                    if (text != null && OWNED_CREATION_SERVER.contentEquals(text)) return true;
                    int childCount = node.getChildCount();
                    for (int index = 0; index < childCount; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered++ >= MAX_NODES) throw new Blocked("ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child == null) throw new Blocked("ui_child_missing");
                        queue.addLast(child);
                    }
                } finally {
                    node.recycle();
                }
            }
            return false;
        } finally {
            recycleQueue(queue);
        }
    }

    /** Strict local label traversal for a candidate popup item; incomplete descendants invalidate the whole scan. */
    private boolean creationPickerOptionTextEquals(AccessibilityNodeInfo root, int windowId, String expected, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                    if (isSnippetRowTag(metadata.viewId) || canonicalSnippetEditorId(metadata.viewId) != null) continue;
                    CharSequence text = node.getText();
                    if (text != null && expected.contentEquals(text)) return true;
                    int childCount = node.getChildCount();
                    for (int index = 0; index < childCount; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered++ >= MAX_NODES) throw new Blocked("ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child == null) throw new Blocked("ui_child_missing");
                        queue.addLast(child);
                    }
                } finally {
                    node.recycle();
                }
            }
            return false;
        } finally {
            recycleQueue(queue);
        }
    }

    /** UUID selection is bound to the product's inner identity tag, never the mutable popup index. */
    private boolean creationPickerOptionMatches(AccessibilityNodeInfo root, int windowId, String expectedLabel, String expectedServerId, long deadline) {
        return expectedServerId == null
                ? creationPickerOptionTextEquals(root, windowId, expectedLabel, deadline)
                : nodeOrDescendantTagPresent(root, windowId, creationServerOptionIdTag(expectedServerId), deadline);
    }

    private boolean nodeOrDescendantTagPresent(AccessibilityNodeInfo root, int windowId, String expectedTag, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(AccessibilityNodeInfo.obtain(root));
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                    if (expectedTag.equals(metadata.viewId)) return true;
                    int childCount = node.getChildCount();
                    for (int index = 0; index < childCount; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered++ >= MAX_NODES) throw new Blocked("ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child == null) throw new Blocked("ui_child_missing");
                        queue.addLast(child);
                    }
                } finally {
                    node.recycle();
                }
            }
            return false;
        } finally {
            recycleQueue(queue);
        }
    }

    private ActionResult selectOwnedCreationPicker(UiAutomation automation, String pickerTag, String optionTagPrefix, String expectedLabel, String expectedServerId, boolean server, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int creationWindow = creationSheetWindow(automation, deadline);
        NodeRef picker = uniqueClickableTag(automation, pickerTag, deadline, creationWindow);
        boolean pickerAccepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(creationSheetWindow(automation, deadline) == picker.windowId, "creation_form_window");
            requireCondition(revalidateCreationPicker(automation, picker, pickerTag, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            pickerAccepted = picker.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            picker.node.recycle();
        }
        requireCondition(pickerAccepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.SELECTION;
        NodeRef option = uniqueCreationPickerOption(automation, optionTagPrefix, expectedLabel, expectedServerId, deadline);
        boolean optionAccepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateCreationPickerOption(automation, option, optionTagPrefix, expectedLabel, expectedServerId, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            optionAccepted = option.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            option.node.recycle();
        }
        requireCondition(optionAccepted, "ui_click_acknowledgement");
        stage.stage = Stage.POSTCONDITION;
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                requireCreationPickerSelection(automation, pickerTag, expectedLabel, expectedServerId, creationWindow, deadline);
                Observation observation = observe(automation, deadline);
                requireCondition(observation.windowId == creationWindow && observation.newSession, "creation_form_window");
                if (server) observation.creationServerSelected = true;
                else observation.creationProjectSelected = true;
                return new ActionResult(true, true, observation);
            } catch (Blocked blocked) {
                if (!"creation_form_mismatch".equals(blocked.code) && !"creation_form_window".equals(blocked.code) && !"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("creation_picker_timeout");
    }

    private NodeRef uniqueCreationPickerOption(UiAutomation automation, String optionTagPrefix, String expectedLabel, String expectedServerId, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            NodeRef selected = null;
            boolean retryIncompleteScan = false;
            List<AccessibilityWindowInfo> windows = automation.getWindows();
            if (windows == null) throw new Blocked("interactive_windows_unavailable");
            try {
                for (AccessibilityWindowInfo window : windows) {
                    requireBeforeDeadline(deadline);
                    int popupWindow = window.getId();
                    AccessibilityNodeInfo root = window.getRoot();
                    if (root == null) {
                        retryIncompleteScan = true;
                        break;
                    }
                    CharSequence packageName = root.getPackageName();
                    if (packageName == null || !PRODUCT_PACKAGE.contentEquals(packageName)) {
                        root.recycle();
                        continue;
                    }
                    try {
                        NodeRef candidate = uniqueCreationPickerOptionInRoot(root, popupWindow, optionTagPrefix, expectedLabel, expectedServerId, deadline);
                        if (selected != null) {
                            candidate.node.recycle();
                            throw new Blocked("ui_duplicate");
                        }
                        selected = candidate;
                    } catch (Blocked blocked) {
                        if ("ui_tag_missing".equals(blocked.code)) continue;
                        if ("ui_child_missing".equals(blocked.code)) {
                            retryIncompleteScan = true;
                            break;
                        }
                        throw blocked;
                    }
                }
                if (!retryIncompleteScan && selected != null) {
                    NodeRef result = selected;
                    selected = null;
                    return result;
                }
            } finally {
                if (selected != null) selected.node.recycle();
                for (AccessibilityWindowInfo window : windows) window.recycle();
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("ui_tag_missing");
    }

    private NodeRef uniqueCreationPickerOptionInRoot(AccessibilityNodeInfo root, int windowId, String optionTagPrefix, String expectedLabel, String expectedServerId, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(root);
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                boolean retained = false;
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    if (isCreationPickerOptionTag(metadata.viewId, optionTagPrefix) && creationPickerOptionMatches(node, windowId, expectedLabel, expectedServerId, deadline)) {
                        if (!metadata.visible || !metadata.enabled || !metadata.supports(AccessibilityNodeInfo.ACTION_CLICK)) continue;
                        if (selected != null) throw new Blocked("ui_duplicate");
                        selected = node;
                        retained = true;
                        continue;
                    }
                    int childCount = node.getChildCount();
                    for (int index = 0; index < childCount; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered++ >= MAX_NODES) throw new Blocked("ui_node_bound");
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child == null) throw new Blocked("ui_child_missing");
                        queue.addLast(child);
                    }
                } finally {
                    if (!retained) node.recycle();
                }
            }
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, windowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    private void requireCreationSheetWindow(UiAutomation automation, int creationWindow, long deadline) {
        NodeRef close = uniqueActionTagInWindow(automation, NEW_SESSION_CLOSE_TAG, 0, creationWindow, deadline);
        close.node.recycle();
    }

    private void requireCreationPickerLabel(UiAutomation automation, String pickerTag, String expectedLabel, int creationWindow, long deadline) {
        NodeRef picker = uniqueActionTagInWindow(automation, pickerTag, 0, creationWindow, deadline);
        try {
            requireCondition(nodeOrDescendantTextEquals(picker.node, picker.windowId, expectedLabel, deadline), "creation_form_mismatch");
        } finally {
            picker.node.recycle();
        }
    }

    private void requireCreationPickerSelection(UiAutomation automation, String pickerTag, String expectedLabel, String expectedServerId, int creationWindow, long deadline) {
        if (expectedServerId == null) {
            requireCreationPickerLabel(automation, pickerTag, expectedLabel, creationWindow, deadline);
            return;
        }
        NodeRef selected = uniqueVisibleTag(automation, creationServerSelectedIdTag(expectedServerId), deadline, creationWindow);
        try {
            requireCondition(hasAncestorTag(selected.node, selected.windowId, pickerTag, deadline), "creation_form_mismatch");
            CharSequence text = selected.node.getText();
            requireCondition(text != null && (expectedLabel.contentEquals(text) || text.toString().startsWith(expectedLabel + " \u00b7 ID ")), "creation_form_mismatch");
        } finally {
            selected.node.recycle();
        }
    }

    private boolean revalidateCreationPicker(UiAutomation automation, NodeRef picker, String pickerTag, long deadline) {
        if (!revalidateAction(automation, picker, pickerTag, AccessibilityNodeInfo.ACTION_CLICK, deadline)) return false;
        try {
            return creationSheetWindow(automation, deadline) == picker.windowId;
        } catch (Blocked blocked) {
            return false;
        }
    }

    private boolean revalidateCreationPickerOption(UiAutomation automation, NodeRef option, String optionTagPrefix, String expectedLabel, String expectedServerId, long deadline) {
        if (isSensitiveContent(option.node) || !option.node.refresh()) return false;
        try {
            AccessibilityNodeInfo popupRoot = productWindowRoot(automation, option.windowId, deadline);
            NodeMetadata metadata = metadata(option.node);
            try {
                return PRODUCT_PACKAGE.equals(metadata.packageName)
                    && popupRoot.getWindowId() == option.windowId
                    && metadata.windowId == option.windowId
                    && metadata.visible
                    && metadata.enabled
                    && metadata.supports(AccessibilityNodeInfo.ACTION_CLICK)
                    && isCreationPickerOptionTag(metadata.viewId, optionTagPrefix)
                    && creationPickerOptionMatches(option.node, option.windowId, expectedLabel, expectedServerId, deadline);
            } finally {
                popupRoot.recycle();
            }
        } catch (Blocked blocked) {
            return false;
        }
    }

    /** Continues exactly one already-open owned server picker; it never opens or retries that picker. */
    private ActionResult continueOwnedCreationServer(UiAutomation automation, String expectedServerId, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return continueOwnedCreationPicker(automation, CREATE_SERVER_TAG, CREATE_SERVER_OPTION_TAG_PREFIX, OWNED_CREATION_SERVER, expectedServerId, true, deadline, acknowledgement, stage);
    }

    /** Continues exactly one already-open owned project picker; it never opens or retries that picker. */
    private ActionResult continueOwnedCreationProject(UiAutomation automation, String expectedLabel, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        return continueOwnedCreationPicker(automation, CREATE_PROJECT_TAG, CREATE_PROJECT_OPTION_TAG_PREFIX, expectedLabel, null, false, deadline, acknowledgement, stage);
    }

    private ActionResult continueOwnedCreationPicker(UiAutomation automation, String pickerTag, String optionTagPrefix, String expectedLabel, String expectedServerId, boolean server, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        NodeRef option = uniqueCreationPickerOption(automation, optionTagPrefix, expectedLabel, expectedServerId, deadline);
        boolean optionAccepted;
        try {
            stage.stage = Stage.CLICK;
            requireCondition(revalidateCreationPickerOption(automation, option, optionTagPrefix, expectedLabel, expectedServerId, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            optionAccepted = option.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            option.node.recycle();
        }
        requireCondition(optionAccepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                int sheetWindow = creationSheetWindow(automation, deadline);
                requireCreationPickerSelection(automation, pickerTag, expectedLabel, expectedServerId, sheetWindow, deadline);
                Observation observation = observe(automation, deadline);
                requireCondition(observation.windowId == sheetWindow && observation.newSession, "creation_form_window");
                if (server) observation.creationServerSelected = true;
                else observation.creationProjectSelected = true;
                return new ActionResult(true, true, observation);
            } catch (Blocked blocked) {
                if (!"creation_form_mismatch".equals(blocked.code) && !"creation_form_window".equals(blocked.code) && !"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("creation_picker_timeout");
    }

    private boolean isCreationPickerOptionTag(String tag, String prefix) {
        if (tag == null || !tag.startsWith(prefix)) return false;
        String suffix = tag.substring(prefix.length());
        if (suffix.length() == 0) return false;
        for (int index = 0; index < suffix.length(); index++) if (!Character.isDigit(suffix.charAt(index))) return false;
        return true;
    }

    private String creationServerOptionIdTag(String serverId) {
        return "creation_server_option_id_" + serverId;
    }

    private String creationServerSelectedIdTag(String serverId) {
        return "creation_server_selected_id_" + serverId;
    }

    private ActionResult submitCreation(UiAutomation automation, String name, String serverLabel, String serverId, String projectLabel, Set<String> excludedReceiptIds, long deadline, Acknowledgement acknowledgement, StageTracker stage) {
        stage.stage = Stage.SELECTION;
        int formWindow = creationWindow(automation, deadline);
        requireCreationForm(automation, name, serverLabel, serverId, projectLabel, formWindow, deadline);
        awaitCreationActionAvailable(automation, name, serverLabel, serverId, projectLabel, formWindow, deadline);
        NodeRef node = uniqueClickableTag(automation, SESSION_CREATE_TAG, deadline, formWindow);
        boolean accepted;
        try {
            stage.stage = Stage.CLICK;
            requireCreationForm(automation, name, serverLabel, serverId, projectLabel, formWindow, deadline);
            requireCondition(revalidate(automation, node, SESSION_CREATE_TAG, deadline), "ui_revalidation");
            requireBeforeDeadline(deadline);
            accepted = node.node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
        } finally {
            node.node.recycle();
        }
        requireCondition(accepted, "ui_click_acknowledgement");
        acknowledgement.value = true;
        stage.stage = Stage.POSTCONDITION;
        Observation observation = awaitCreationReceipt(automation, excludedReceiptIds, deadline);
        return new ActionResult(true, true, observation);
    }

    private void awaitCreationActionAvailable(UiAutomation automation, String name, String serverLabel, String serverId, String projectLabel, int formWindow, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            Observation observation = observe(automation, deadline);
            requireCondition(observation.windowId == formWindow, "creation_form_window");
            requireCreationForm(automation, name, serverLabel, serverId, projectLabel, formWindow, deadline);
            if (observation.creationActionAvailable) return;
            if (!"none".equals(observation.creationBlocker) && !"synchronizing".equals(observation.creationBlocker)) {
                throw new Blocked("creation_action_unavailable");
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("creation_action_timeout");
    }

    private int creationWindow(UiAutomation automation, long deadline) {
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        try {
            return root.getWindowId();
        } finally {
            root.recycle();
        }
    }

    private void requireCreationForm(UiAutomation automation, String name, String serverLabel, String serverId, String projectLabel, int formWindow, long deadline) {
        String expectedServer = serverLabel == null ? APPROVED_SERVER : serverLabel;
        String expectedProject = projectLabel == null ? APPROVED_PROJECT : projectLabel;
        requireTagTextExact(automation, CREATE_NAME_TAG, name, formWindow, deadline);
        requireCreationPickerSelection(automation, CREATE_SERVER_TAG, expectedServer, serverId, formWindow, deadline);
        requireTagTextExact(automation, CREATE_PROJECT_TAG, expectedProject, formWindow, deadline);
        requireSelectedProvider(automation, providerForName(name), formWindow, deadline);
    }

    private String providerForName(String name) {
        if (name.startsWith("Goatr phone OMP ")) return PROVIDER_OMP;
        if (name.startsWith("Goatr phone Codex ")) return PROVIDER_CODEX;
        throw new Blocked("creation_name_invalid");
    }

    private Observation awaitExpected(UiAutomation automation, String expectedTag, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            Observation observation = observe(automation, deadline);
            int count = observation.visibleTagCount(expectedTag);
            if (count == 1) return observation;
            if (count > 1) throw new Blocked("ui_duplicate");
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private Observation awaitSelectedAppearance(UiAutomation automation, UiAppearance appearance, int settingsWindow, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            Observation observation = observe(automation, deadline);
            if (observation.windowId == settingsWindow) {
                if ("error".equals(observation.selectedAppearance)) throw new Blocked("appearance_selection_invalid");
                if (appearance.wire.equals(observation.selectedAppearance)) return observation;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("appearance_selection_timeout");
    }

    private Observation observeAppearance(UiAutomation automation, long deadline) {
        int settingsWindow = appearanceSettingsWindow(automation, deadline);
        Observation observation = observe(automation, deadline);
        requireCondition(observation.windowId == settingsWindow, "appearance_settings_window");
        requireCondition(!"unavailable".equals(observation.selectedAppearance) && !"error".equals(observation.selectedAppearance), "appearance_selection_invalid");
        return observation;
    }

    private Observation awaitTerminalDialog(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                terminalTargetWindow(automation, sessionId, sessionGeneration, deadline);
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private String terminalTargetTag(String sessionId, String sessionGeneration) {
        return TERMINAL_TARGET_TAG_PREFIX + sessionId + "_" + sessionGeneration;
    }

    private int terminalTargetWindow(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        NodeRef dialog = uniqueVisibleTag(automation, TERMINAL_DIALOG_TAG, deadline);
        try {
            NodeRef target = uniqueVisibleTag(automation, terminalTargetTag(sessionId, sessionGeneration), deadline, dialog.windowId);
            try {
                requireCondition(hasAncestorTag(target.node, target.windowId, TERMINAL_DIALOG_TAG, deadline), "terminal_target_scope");
                return dialog.windowId;
            } finally {
                target.node.recycle();
            }
        } finally {
            dialog.node.recycle();
        }
    }

    private boolean revalidateTerminalDialogAction(UiAutomation automation, NodeRef node, String tag, String sessionId, String sessionGeneration, long deadline) {
        if (!revalidateDialogAction(automation, node, TERMINAL_DIALOG_TAG, tag, deadline)) return false;
        return terminalTargetWindow(automation, sessionId, sessionGeneration, deadline) == node.windowId;
    }

    private Observation awaitWorktreesDialog(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                worktreesTargetWindow(automation, sessionId, sessionGeneration, deadline);
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private String worktreesTargetTag(String sessionId, String sessionGeneration) {
        return WORKTREES_TARGET_TAG_PREFIX + sessionId + "_" + sessionGeneration;
    }

    private int worktreesTargetWindow(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        NodeRef dialog = uniqueVisibleTag(automation, WORKTREES_DIALOG_TAG, deadline);
        try {
            NodeRef target = uniqueVisibleTag(automation, worktreesTargetTag(sessionId, sessionGeneration), deadline, dialog.windowId);
            try {
                requireCondition(hasAncestorTag(target.node, target.windowId, WORKTREES_DIALOG_TAG, deadline), "worktrees_target_scope");
                return dialog.windowId;
            } finally {
                target.node.recycle();
            }
        } finally {
            dialog.node.recycle();
        }
    }

    private boolean revalidateWorktreesDialogAction(UiAutomation automation, NodeRef node, String tag, String sessionId, String sessionGeneration, long deadline) {
        if (!revalidateDialogAction(automation, node, WORKTREES_DIALOG_TAG, tag, deadline)) return false;
        return worktreesTargetWindow(automation, sessionId, sessionGeneration, deadline) == node.windowId;
    }

    private Observation awaitChangesDialog(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try { changesTargetWindow(automation, sessionId, sessionGeneration, deadline); return observe(automation, deadline); }
            catch (Blocked blocked) { if (!"ui_tag_missing".equals(blocked.code)) throw blocked; }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private String changesTargetTag(String sessionId, String sessionGeneration) { return CHANGES_TARGET_TAG_PREFIX + sessionId + "_" + sessionGeneration; }
    private int changesTargetWindow(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        NodeRef dialog = uniqueVisibleTag(automation, CHANGES_DIALOG_TAG, deadline);
        try {
            NodeRef target = uniqueVisibleTag(automation, changesTargetTag(sessionId, sessionGeneration), deadline, dialog.windowId);
            try { requireCondition(hasAncestorTag(target.node, target.windowId, CHANGES_DIALOG_TAG, deadline), "changes_target_scope"); return dialog.windowId; }
            finally { target.node.recycle(); }
        } finally { dialog.node.recycle(); }
    }
    private boolean revalidateChangesDialogAction(UiAutomation automation, NodeRef node, String tag, String sessionId, String sessionGeneration, long deadline) {
        return revalidateDialogAction(automation, node, CHANGES_DIALOG_TAG, tag, deadline)
                && changesTargetWindow(automation, sessionId, sessionGeneration, deadline) == node.windowId;
    }

    private Observation awaitExactConversation(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private int exactConversationWindow(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        String targetTag = CONVERSATION_TARGET_TAG_PREFIX + sessionId + "_" + sessionGeneration;
        NodeRef conversation = uniqueVisibleTag(automation, CONVERSATION_LIST_TAG, deadline);
        try {
            NodeRef target = uniqueVisibleTag(automation, targetTag, deadline, conversation.windowId);
            try {
                requireBeforeDeadline(deadline);
                return conversation.windowId;
            } finally {
                target.node.recycle();
            }
        } finally {
            conversation.node.recycle();
        }
    }

    private void requireExactComposerText(UiAutomation automation, String sessionId, String sessionGeneration, String expected, long deadline) {
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        requireTagTextExact(automation, COMPOSER_TAG, expected, conversationWindow, deadline);
    }

    private Observation awaitExactComposerText(UiAutomation automation, String sessionId, String sessionGeneration, String expected, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                requireExactComposerText(automation, sessionId, sessionGeneration, expected, deadline);
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code) && !"creation_form_mismatch".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("composer_text_timeout");
    }

    private NodeRef uniqueClickableTag(UiAutomation automation, String tag, long deadline) {
        return uniqueActionTag(automation, tag, AccessibilityNodeInfo.ACTION_CLICK, deadline, NO_WINDOW);
    }

    private NodeRef uniqueSetTextTag(UiAutomation automation, String tag, long deadline) {
        return uniqueActionTag(automation, tag, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline, NO_WINDOW);
    }

    private NodeRef uniqueSetTextTag(UiAutomation automation, String tag, long deadline, int expectedWindowId) {
        return uniqueActionTag(automation, tag, AccessibilityNodeInfo.ACTION_SET_TEXT, deadline, expectedWindowId);
    }

    private NodeRef uniqueVisibleTag(UiAutomation automation, String tag, long deadline) {
        return uniqueActionTag(automation, tag, 0, deadline, NO_WINDOW);
    }

    private NodeRef uniqueVisibleTag(UiAutomation automation, String tag, long deadline, int expectedWindowId) {
        return uniqueActionTag(automation, tag, 0, deadline, expectedWindowId);
    }

    private NodeRef uniqueClickableTag(UiAutomation automation, String tag, long deadline, int expectedWindowId) {
        return uniqueActionTag(automation, tag, AccessibilityNodeInfo.ACTION_CLICK, deadline, expectedWindowId);
    }

    private NodeRef uniqueActionTag(UiAutomation automation, String tag, int actionId, long deadline, int expectedWindowId) {
        AccessibilityNodeInfo root = awaitInitialProductRoot(automation, deadline);
        int windowId = root.getWindowId();
        if (expectedWindowId != NO_WINDOW && windowId != expectedWindowId) {
            root.recycle();
            throw new Blocked("creation_form_window");
        }
        requireBeforeDeadline(deadline);
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(root);
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                requireBeforeDeadline(deadline);
                if (isSensitiveContent(node)) {
                    node.recycle();
                    continue;
                }
                NodeMetadata metadata = metadata(node);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) {
                    node.recycle();
                    throw new Blocked("ui_root_changed");
                }
                if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) {
                    node.recycle();
                    throw new Blocked("secret_scene");
                }
                if ((isSnippetRowTag(metadata.viewId) || canonicalSnippetEditorId(metadata.viewId) != null) && !tag.equals(metadata.viewId)) {
                    node.recycle();
                    continue;
                }
                int childCount = node.getChildCount();
                requireBeforeDeadline(deadline);
                for (int index = 0; index < childCount; index++) {
                    requireBeforeDeadline(deadline);
                    if (discovered >= MAX_NODES) {
                        node.recycle();
                        throw new Blocked("ui_node_bound");
                    }
                    discovered++;
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) queue.addLast(child);
                    requireBeforeDeadline(deadline);
                }
                if (tag.equals(metadata.viewId) && metadata.visible && metadata.enabled && (actionId == 0 || metadata.supports(actionId))) {
                    if (selected != null) {
                        node.recycle();
                        throw new Blocked("ui_duplicate");
                    }
                    selected = node;
                } else {
                    node.recycle();
                }
            }
            requireBeforeDeadline(deadline);
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, windowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    private AccessibilityNodeInfo productWindowRoot(UiAutomation automation, int expectedWindowId, long deadline) {
        requireBeforeDeadline(deadline);
        List<AccessibilityWindowInfo> windows = automation.getWindows();
        if (windows == null) throw new Blocked("interactive_windows_unavailable");
        try {
            for (AccessibilityWindowInfo window : windows) {
                requireBeforeDeadline(deadline);
                if (window.getId() != expectedWindowId) continue;
                AccessibilityNodeInfo root = window.getRoot();
                if (root == null) throw new Blocked("creation_form_window");
                CharSequence packageName = root.getPackageName();
                if (packageName == null || !PRODUCT_PACKAGE.contentEquals(packageName)) {
                    root.recycle();
                    throw new Blocked("ui_root_changed");
                }
                return root;
            }
            throw new Blocked("creation_form_window");
        } finally {
            for (AccessibilityWindowInfo window : windows) window.recycle();
        }
    }

    private NodeRef uniqueActionTagInWindow(UiAutomation automation, String tag, int actionId, int expectedWindowId, long deadline) {
        AccessibilityNodeInfo root = productWindowRoot(automation, expectedWindowId, deadline);
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(root);
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                requireBeforeDeadline(deadline);
                if (isSensitiveContent(node)) {
                    node.recycle();
                    continue;
                }
                NodeMetadata metadata = metadata(node);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != expectedWindowId) {
                    node.recycle();
                    throw new Blocked("ui_root_changed");
                }
                if (tag.equals(metadata.viewId) && metadata.visible && metadata.enabled && (actionId == 0 || metadata.supports(actionId))) {
                    if (selected != null) {
                        node.recycle();
                        throw new Blocked("ui_duplicate");
                    }
                    selected = node;
                    continue;
                }
                int childCount = node.getChildCount();
                for (int index = 0; index < childCount; index++) {
                    requireBeforeDeadline(deadline);
                    if (discovered++ >= MAX_NODES) {
                        node.recycle();
                        throw new Blocked("ui_node_bound");
                    }
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) queue.addLast(child);
                }
                node.recycle();
            }
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, expectedWindowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    /** This localized walker never enters another saved snippet row and never reads text. */
    private NodeRef uniqueSnippetChild(AccessibilityNodeInfo root, int windowId, String tag, int actionId, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>(); queue.add(AccessibilityNodeInfo.obtain(root));
        AccessibilityNodeInfo selected = null; boolean complete = false; int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst(); requireBeforeDeadline(deadline);
                if (isSensitiveContent(node)) { node.recycle(); continue; }
                NodeMetadata metadata = metadata(node); requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) { node.recycle(); throw new Blocked("ui_root_changed"); }
                if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) { node.recycle(); throw new Blocked("secret_scene"); }
                if (isSnippetRowTag(metadata.viewId) && !tag.equals(metadata.viewId) && !tag.endsWith(metadata.viewId.substring(SNIPPET_ROW_TAG_PREFIX.length()))) { node.recycle(); continue; }
                int children=node.getChildCount(); requireBeforeDeadline(deadline);
                for(int index=0;index<children;index++) { requireBeforeDeadline(deadline); if(discovered>=MAX_NODES){node.recycle();throw new Blocked("ui_node_bound");} discovered++; AccessibilityNodeInfo child=node.getChild(index); if(child!=null)queue.addLast(child); }
                if(tag.equals(metadata.viewId)&&metadata.visible&&metadata.enabled&&(actionId==0||metadata.supports(actionId))) { if(selected!=null){node.recycle();throw new Blocked("ui_duplicate");} selected=node; } else node.recycle();
            }
            if(selected==null) throw new Blocked("ui_tag_missing"); complete=true; return new NodeRef(selected,windowId);
        } finally { recycleQueue(queue); if(!complete&&selected!=null) selected.recycle(); }
    }

    private boolean isSnippetRowTag(String tag) {
        if (tag == null || !tag.startsWith(SNIPPET_ROW_TAG_PREFIX) || tag.startsWith(SNIPPET_SCOPE_TAG_PREFIX) || tag.startsWith(SNIPPET_EDITOR_TAG_PREFIX) || tag.startsWith(SNIPPET_DELETE_TAG_PREFIX)) return false;
        try { return UUID.fromString(tag.substring(SNIPPET_ROW_TAG_PREFIX.length())).toString().equals(tag.substring(SNIPPET_ROW_TAG_PREFIX.length())); }
        catch (IllegalArgumentException exception) { return false; }
    }

    private void requireSnippetScope(UiAutomation automation, Request request, long deadline) { NodeRef scope=uniqueVisibleTag(automation,snippetScopeTag(request),deadline);scope.node.recycle(); }
    private void requireSnippetKnownState(UiAutomation automation,Request request,long deadline) { try { requireSnippetScope(automation,request,deadline); if(snippetRowPresent(automation,request,deadline)) return; NodeRef editor=uniqueVisibleTag(automation,snippetEditorTag(request.snippetId),deadline);try{requireCondition(hasAncestorTag(editor.node,editor.windowId,snippetScopeTag(request),deadline),"snippet_scope_mismatch");return;}finally{editor.node.recycle();} } catch(Blocked blocked) { if(!"ui_tag_missing".equals(blocked.code)) throw blocked; snippetDeleteDialogWindow(automation,request,deadline); } }
    private boolean snippetRowPresent(UiAutomation automation, Request request, long deadline) { try { requireSnippetRow(automation,request,deadline);return true; } catch(Blocked blocked) { if("ui_tag_missing".equals(blocked.code)) return false; throw blocked; } }
    private boolean snippetRowTagPresentInScope(UiAutomation automation,Request request,long deadline){NodeRef scope=uniqueVisibleTag(automation,snippetScopeTag(request),deadline);ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(AccessibilityNodeInfo.obtain(scope.node));int discovered=1;try{while(!queue.isEmpty()){AccessibilityNodeInfo node=queue.removeFirst();try{requireBeforeDeadline(deadline);if(isSensitiveContent(node))continue;NodeMetadata metadata=metadata(node);if(!PRODUCT_PACKAGE.equals(metadata.packageName)||metadata.windowId!=scope.windowId)throw new Blocked("ui_root_changed");if(node.isPassword()||PAIRING_SCENE_TAGS.contains(metadata.viewId))throw new Blocked("secret_scene");if(isSnippetRowTag(metadata.viewId)){if(snippetRowTag(request.snippetId).equals(metadata.viewId))return true;continue;}if(canonicalSnippetEditorId(metadata.viewId)!=null)continue;int children=node.getChildCount();for(int index=0;index<children;index++){requireBeforeDeadline(deadline);if(discovered++>=MAX_NODES)throw new Blocked("ui_node_bound");AccessibilityNodeInfo child=node.getChild(index);if(child!=null)queue.addLast(child);}}finally{node.recycle();}}return false;}finally{scope.node.recycle();recycleQueue(queue);}}
    private void requireSnippetRow(UiAutomation automation, Request request, long deadline) { NodeRef row=uniqueVisibleTag(automation,snippetRowTag(request.snippetId),deadline);try { requireCondition(hasAncestorTag(row.node,row.windowId,snippetScopeTag(request),deadline),"snippet_scope_mismatch"); } finally {row.node.recycle();} }
    private void requireSnippetDraftMarker(UiAutomation automation, Request request, long deadline) { NodeRef empty=null;NodeRef present=null;try { empty=uniqueVisibleTag(automation,SNIPPET_DRAFT_EMPTY_TAG,deadline);requireCondition(hasAncestorTag(empty.node,empty.windowId,snippetScopeTag(request),deadline),"snippet_scope_mismatch"); } catch(Blocked missing) { if(!"ui_tag_missing".equals(missing.code)) throw missing; present=uniqueVisibleTag(automation,SNIPPET_DRAFT_PRESENT_TAG,deadline);requireCondition(hasAncestorTag(present.node,present.windowId,snippetScopeTag(request),deadline),"snippet_scope_mismatch"); } finally {if(empty!=null)empty.node.recycle();if(present!=null)present.node.recycle();} }
    private void requireSnippetDraftEmpty(UiAutomation automation, Request request, long deadline) { NodeRef empty=uniqueVisibleTag(automation,SNIPPET_DRAFT_EMPTY_TAG,deadline);try {requireCondition(hasAncestorTag(empty.node,empty.windowId,snippetScopeTag(request),deadline),"snippet_draft_not_empty");}finally{empty.node.recycle();} }
    private Observation awaitSnippetScope(UiAutomation automation, Request request, long deadline) { while(SystemClock.elapsedRealtime()<deadline) { try {requireSnippetScope(automation,request,deadline);return observe(automation,deadline);}catch(Blocked blocked){if(!"ui_tag_missing".equals(blocked.code))throw blocked;}SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout"); }
    private boolean tagPresent(UiAutomation automation,String tag,long deadline) { try {NodeRef node=uniqueVisibleTag(automation,tag,deadline);node.node.recycle();return true;}catch(Blocked blocked){if("ui_tag_missing".equals(blocked.code))return false;throw blocked;} }
    private Observation awaitSnippetListScene(UiAutomation automation,Request request,long deadline) { while(SystemClock.elapsedRealtime()<deadline) { try { NodeRef scope=uniqueVisibleTag(automation,snippetScopeTag(request),deadline);int window=scope.windowId;scope.node.recycle();if(!tagPresent(automation,snippetEditorTag(request.snippetId),deadline)&&!tagPresent(automation,SNIPPET_DELETE_SCOPE_TAG_PREFIX+request.snippetId,deadline))return observe(automation,deadline); } catch(Blocked blocked){if(!"ui_tag_missing".equals(blocked.code))throw blocked;}SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout"); }
    private Observation awaitSavedSnippet(UiAutomation automation,Request request,long deadline) { while(SystemClock.elapsedRealtime()<deadline) { try { Observation result=awaitSnippetListScene(automation,request,deadline);requireOwnedSnippetRowPair(automation,request,deadline);return result;} catch(Blocked blocked){if(!"ui_tag_missing".equals(blocked.code)&&!"snippet_field_not_owned".equals(blocked.code))throw blocked;}SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout"); }
    private Observation awaitDeletedSnippet(UiAutomation automation,Request request,long deadline) { while(SystemClock.elapsedRealtime()<deadline) { Observation result=awaitSnippetListScene(automation,request,deadline);if(!snippetRowTagPresentInScope(automation,request,deadline))return result;SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout"); }
    private Observation awaitSnippetRow(UiAutomation automation, Request request, boolean present, long deadline) { while(SystemClock.elapsedRealtime()<deadline) { if(snippetRowPresent(automation,request,deadline)==present) return observe(automation,deadline);SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout"); }
    private Observation awaitSnippetEditor(UiAutomation automation, Request request, long deadline) { while(SystemClock.elapsedRealtime()<deadline) { try { if(request.snippetId!=null) {NodeRef editor=uniqueVisibleTag(automation,snippetEditorTag(request.snippetId),deadline);try{requireCondition(hasAncestorTag(editor.node,editor.windowId,snippetScopeTag(request),deadline),"snippet_scope_mismatch");}finally{editor.node.recycle();}} else { requireUniqueSnippetEditor(automation,request,deadline); } return observe(automation,deadline); }catch(Blocked blocked){if(!"ui_tag_missing".equals(blocked.code))throw blocked;}SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout"); }
    private void requireUniqueSnippetEditor(UiAutomation automation, Request request, long deadline) { NodeRef scope=uniqueVisibleTag(automation,snippetScopeTag(request),deadline);try { String id=uniqueSnippetEditorId(scope.node,scope.windowId,deadline); requireCondition(id!=null,"ui_tag_missing"); } finally {scope.node.recycle();} }
    private String uniqueSnippetEditorId(AccessibilityNodeInfo root,int windowId,long deadline) { ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(AccessibilityNodeInfo.obtain(root));String id=null;int discovered=1;try{while(!queue.isEmpty()){AccessibilityNodeInfo node=queue.removeFirst();try{requireBeforeDeadline(deadline);if(isSensitiveContent(node))continue;NodeMetadata metadata=metadata(node);if(!PRODUCT_PACKAGE.equals(metadata.packageName)||metadata.windowId!=windowId)throw new Blocked("ui_root_changed");String candidate=canonicalSnippetEditorId(metadata.viewId);if(candidate!=null){if(id!=null)throw new Blocked("ui_duplicate");id=candidate;continue;}if(isSnippetRowTag(metadata.viewId))continue;int children=node.getChildCount();for(int index=0;index<children;index++){if(discovered++>=MAX_NODES)throw new Blocked("ui_node_bound");AccessibilityNodeInfo child=node.getChild(index);if(child!=null)queue.addLast(child);}}finally{node.recycle();}}return id;}finally{recycleQueue(queue);}}
    private Observation awaitSnippetField(UiAutomation automation,Request request,String tag,String expected,long deadline) {while(SystemClock.elapsedRealtime()<deadline){NodeRef node=snippetEditorField(automation,request,tag,deadline);try{if(expected.contentEquals(node.node.getText()))return observe(automation,deadline);}finally{node.node.recycle();}SystemClock.sleep(POLL_MILLIS);}throw new Blocked("snippet_text_timeout");}
    private void requireOwnedSnippetEditorPair(UiAutomation automation,Request request,long deadline){NodeRef title=snippetEditorField(automation,request,SNIPPET_TITLE_TAG,deadline);NodeRef body=null;try{body=snippetEditorField(automation,request,SNIPPET_BODY_TAG,deadline);boolean original=snippetText(withSnippetVersion(request,SnippetVersion.ORIGINAL),true).contentEquals(title.node.getText())&&snippetText(withSnippetVersion(request,SnippetVersion.ORIGINAL),false).contentEquals(body.node.getText());boolean edited=snippetText(withSnippetVersion(request,SnippetVersion.EDITED),true).contentEquals(title.node.getText())&&snippetText(withSnippetVersion(request,SnippetVersion.EDITED),false).contentEquals(body.node.getText());requireCondition(original||edited,"snippet_field_not_owned");}finally{title.node.recycle();if(body!=null)body.node.recycle();}}
    private void requireClosableSnippetEditor(UiAutomation automation,Request request,long deadline){NodeRef title=snippetEditorField(automation,request,SNIPPET_TITLE_TAG,deadline);NodeRef body=null;try{body=snippetEditorField(automation,request,SNIPPET_BODY_TAG,deadline);requireCondition(knownSnippetText(request,true,title.node.getText())&&knownSnippetText(request,false,body.node.getText()),"snippet_field_not_owned");}finally{title.node.recycle();if(body!=null)body.node.recycle();}}
    private SnippetVersion requireOwnedSnippetRowPair(UiAutomation automation,Request request,long deadline){NodeRef row=uniqueVisibleTag(automation,snippetRowTag(request.snippetId),deadline);try{requireCondition(hasAncestorTag(row.node,row.windowId,snippetScopeTag(request),deadline),"snippet_scope_mismatch");for(SnippetVersion version:SnippetVersion.values()){Request candidate=withSnippetVersion(request,version);if(ownedSnippetRowText(row.node,row.windowId,snippetText(candidate,true),deadline)&&ownedSnippetRowText(row.node,row.windowId,snippetText(candidate,false),deadline))return version;}throw new Blocked("snippet_field_not_owned");}finally{row.node.recycle();}}
    private boolean ownedSnippetRowText(AccessibilityNodeInfo root,int windowId,String expected,long deadline){ArrayDeque<AccessibilityNodeInfo> queue=new ArrayDeque<>();queue.add(AccessibilityNodeInfo.obtain(root));int discovered=1;try{while(!queue.isEmpty()){AccessibilityNodeInfo node=queue.removeFirst();try{requireBeforeDeadline(deadline);if(isSensitiveContent(node))continue;NodeMetadata metadata=metadata(node);if(!PRODUCT_PACKAGE.equals(metadata.packageName)||metadata.windowId!=windowId)throw new Blocked("ui_root_changed");if(node.isPassword()||PAIRING_SCENE_TAGS.contains(metadata.viewId))throw new Blocked("secret_scene");CharSequence text=node.getText();if(text!=null&&expected.contentEquals(text))return true;int children=node.getChildCount();for(int index=0;index<children;index++){if(discovered++>=MAX_NODES)throw new Blocked("ui_node_bound");AccessibilityNodeInfo child=node.getChild(index);if(child!=null)queue.addLast(child);}}finally{node.recycle();}}return false;}finally{recycleQueue(queue);}}
    private Request withSnippetVersion(Request request,SnippetVersion version){return new Request(request.runId,request.action,request.appearance,request.sessionListPreference,request.sessionListPreferenceValue,request.creationServerLabel,request.creationServerId,request.creationProjectLabel,request.creationName,request.previousCreationName,request.composerText,request.sessionId,request.sessionGeneration,request.historyRecordId,request.receiptId,request.excludedReceiptIds,request.snippetWorkflowId,request.snippetId,version);}
    private int snippetDeleteDialogWindow(UiAutomation automation,Request request,long deadline){String target=SNIPPET_DELETE_TARGET_TAG_PREFIX+request.sessionId+"_"+request.sessionGeneration;NodeRef scope=uniqueVisibleTag(automation,SNIPPET_DELETE_SCOPE_TAG_PREFIX+request.snippetId,deadline);try{NodeRef marker=uniqueVisibleTag(automation,target,deadline,scope.windowId);marker.node.recycle();return scope.windowId;}finally{scope.node.recycle();}}
    private Observation awaitSnippetDeleteDialog(UiAutomation automation,Request request,long deadline){while(SystemClock.elapsedRealtime()<deadline){try{snippetDeleteDialogWindow(automation,request,deadline);return observe(automation,deadline);}catch(Blocked blocked){if(!"ui_tag_missing".equals(blocked.code))throw blocked;}SystemClock.sleep(POLL_MILLIS);}throw new Blocked("expected_screen_timeout");}
    private boolean revalidateSnippetDeleteConfirm(UiAutomation automation,NodeRef node,Request request,long deadline){if(!revalidateAction(automation,node,SNIPPET_DELETE_CONFIRM_TAG,AccessibilityNodeInfo.ACTION_CLICK,deadline))return false;try{return snippetDeleteDialogWindow(automation,request,deadline)==node.windowId&&hasAncestorTag(node.node,node.windowId,SNIPPET_DELETE_SCOPE_TAG_PREFIX+request.snippetId,deadline);}catch(Blocked blocked){return false;}}

    private NodeRef uniqueHistoryDetailsAction(UiAutomation automation, String recordId, String expectedLabel, int conversationWindow, long deadline) {
        String recordTag = HISTORY_RECORD_TAG_PREFIX + recordId;
        NodeRef record = uniqueVisibleTag(automation, recordTag, deadline, conversationWindow);
        // The subtree traversal takes ownership of this node and recycles it with its queue.
        return uniqueReceiptActionChild(record.node, record.windowId, HISTORY_DETAILS_TAG_PREFIX + recordId, expectedLabel, AccessibilityNodeInfo.ACTION_CLICK, deadline);
    }

    private boolean revalidateHistoryDetailsAction(UiAutomation automation, NodeRef node, String sessionId, String sessionGeneration, String recordId, String expectedLabel, long deadline) {
        if (!revalidateAction(automation, node, HISTORY_DETAILS_TAG_PREFIX + recordId, AccessibilityNodeInfo.ACTION_CLICK, deadline)) return false;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        if (conversationWindow != node.windowId || !hasAncestorTag(node.node, node.windowId, HISTORY_RECORD_TAG_PREFIX + recordId, deadline)) return false;
        return nodeOrDescendantTextEquals(node.node, node.windowId, expectedLabel, deadline);
    }

    private Observation awaitHistoryDetails(UiAutomation automation, String sessionId, String sessionGeneration, String recordId, boolean expanded, long deadline) {
        String expectedLabel = expanded ? HIDE_DETAILS : HISTORY_DETAILS;
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
                NodeRef details = uniqueHistoryDetailsAction(automation, recordId, expectedLabel, conversationWindow, deadline);
                details.node.recycle();
                boolean originalControlPresent = historyOriginalExpandPresent(automation, recordId, conversationWindow, deadline);
                if (!expanded && originalControlPresent) {
                    SystemClock.sleep(POLL_MILLIS);
                    continue;
                }
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private Observation awaitComposerFocusAndIme(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
            NodeRef composer = uniqueVisibleTag(automation, COMPOSER_TAG, deadline, conversationWindow);
            boolean focused;
            try {
                requireCondition(revalidateConversationAction(automation, composer, COMPOSER_TAG, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline), "ui_revalidation");
                focused = composer.node.isFocused();
            } finally {
                composer.node.recycle();
            }
            if (focused && hasInputMethodWindow(automation, deadline)) return observe(automation, deadline);
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("composer_focus_or_ime_missing");
    }

    private Observation awaitImeDismissed(UiAutomation automation, String sessionId, String sessionGeneration, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
            if (!hasInputMethodWindow(automation, deadline)) return observe(automation, deadline);
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("ime_dismissal_missing");
    }

    private boolean hasInputMethodWindow(UiAutomation automation, long deadline) {
        requireBeforeDeadline(deadline);
        List<AccessibilityWindowInfo> windows = automation.getWindows();
        if (windows == null) throw new Blocked("interactive_windows_unavailable");
        try {
            requireBeforeDeadline(deadline);
            for (AccessibilityWindowInfo window : windows) {
                if (window.getType() == AccessibilityWindowInfo.TYPE_INPUT_METHOD) return true;
            }
            return false;
        } finally {
            for (AccessibilityWindowInfo window : windows) window.recycle();
        }
    }

    private static InteractiveWindowsService enableInteractiveWindows(UiAutomation automation) {
        AccessibilityServiceInfo serviceInfo = automation.getServiceInfo();
        if (serviceInfo == null) throw new Blocked("interactive_windows_unavailable");
        int originalFlags = serviceInfo.flags;
        serviceInfo.flags |= AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS;
        automation.setServiceInfo(serviceInfo);
        return new InteractiveWindowsService(serviceInfo, originalFlags);
    }

    private static void restoreInteractiveWindows(UiAutomation automation, InteractiveWindowsService service) {
        service.serviceInfo.flags = service.originalFlags;
        automation.setServiceInfo(service.serviceInfo);
    }

    private boolean needsInteractiveWindows(Action action) {
        return action.isPhysicalAction() || action == Action.BACKGROUND_OWNED_APP || action == Action.FOCUS_COMPOSER || action == Action.DISMISS_COMPOSER_KEYBOARD || action == Action.SELECT_OWNED_CREATION_SERVER || action == Action.SELECT_OWNED_CREATION_PROJECT || action == Action.CONTINUE_OWNED_CREATION_SERVER || action == Action.CONTINUE_OWNED_CREATION_PROJECT || action == Action.OBSERVE_CREATION_PICKER;
    }

    private boolean historyOriginalExpandPresent(UiAutomation automation, String recordId, int conversationWindow, long deadline) {
        NodeRef record = uniqueVisibleTag(automation, HISTORY_RECORD_TAG_PREFIX + recordId, deadline, conversationWindow);
        try {
            NodeRef original = uniqueReceiptActionChild(record.node, record.windowId, ORIGINAL_EXPAND_TAG, null, 0, deadline);
            original.node.recycle();
            return true;
        } catch (Blocked blocked) {
            if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            return false;
        }
    }

    private NodeRef uniqueReceiptInspect(UiAutomation automation, String receiptId, long deadline) {
        return uniqueReceiptAction(automation, receiptId, RECEIPT_INSPECT_TAG, null, AccessibilityNodeInfo.ACTION_CLICK, deadline);
    }

    private NodeRef uniqueReceiptAction(UiAutomation automation, String receiptId, String tag, String expectedText, int actionId, long deadline) {
        NodeRef panel = uniqueVisibleTag(automation, RECEIPT_TAG_PREFIX + receiptId, deadline);
        // The subtree traversal takes ownership of this node and recycles it with its queue.
        return uniqueReceiptActionChild(panel.node, panel.windowId, tag, expectedText, actionId, deadline);
    }

    private NodeRef uniqueReceiptActionChild(AccessibilityNodeInfo panel, int windowId, String tag, String expectedText, int actionId, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(panel);
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                requireBeforeDeadline(deadline);
                if (isSensitiveContent(node)) {
                    node.recycle();
                    continue;
                }
                NodeMetadata metadata = metadata(node);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) {
                    node.recycle();
                    throw new Blocked("ui_root_changed");
                }
                if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) {
                    node.recycle();
                    throw new Blocked("secret_scene");
                }
                int childCount = node.getChildCount();
                requireBeforeDeadline(deadline);
                for (int index = 0; index < childCount; index++) {
                    requireBeforeDeadline(deadline);
                    if (discovered >= MAX_NODES) {
                        node.recycle();
                        throw new Blocked("ui_node_bound");
                    }
                    discovered++;
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) queue.addLast(child);
                    requireBeforeDeadline(deadline);
                }
                if (tag.equals(metadata.viewId) && metadata.visible && metadata.enabled && (actionId == 0 || metadata.supports(actionId))
                        && (expectedText == null || nodeOrDescendantTextEquals(node, windowId, expectedText, deadline))) {
                    if (selected != null) {
                        node.recycle();
                        throw new Blocked("ui_duplicate");
                    }
                    selected = node;
                } else {
                    node.recycle();
                }
            }
            requireBeforeDeadline(deadline);
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, windowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    private boolean revalidateReceiptInspect(UiAutomation automation, NodeRef node, String receiptId, long deadline) {
        return revalidateReceiptAction(automation, node, receiptId, RECEIPT_INSPECT_TAG, null, deadline);
    }

    private boolean revalidateReceiptAction(UiAutomation automation, NodeRef node, String receiptId, String tag, String expectedText, long deadline) {
        if (!node.node.refresh()) return false;
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            requireBeforeDeadline(deadline);
            if (root.getWindowId() != node.windowId
                    || !PRODUCT_PACKAGE.equals(metadata.packageName)
                    || metadata.windowId != node.windowId
                    || !tag.equals(metadata.viewId)
                    || !metadata.visible
                    || !metadata.enabled
                    || !metadata.supports(AccessibilityNodeInfo.ACTION_CLICK)
                    || (expectedText != null && !nodeOrDescendantTextEquals(node.node, node.windowId, expectedText, deadline))
                    || !receiptId.equals(receiptAncestorId(node.node, node.windowId, deadline))) return false;
            NodeRef panel = uniqueVisibleTag(automation, RECEIPT_TAG_PREFIX + receiptId, deadline, node.windowId);
            panel.node.recycle();
            return true;
        } finally {
            root.recycle();
        }
    }

    private Observation awaitReceiptPanel(UiAutomation automation, String receiptId, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            NodeRef panel = uniqueVisibleTag(automation, RECEIPT_TAG_PREFIX + receiptId, deadline);
            panel.node.recycle();
            return observe(automation, deadline);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private Observation awaitReceiptDetails(UiAutomation automation, String receiptId, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            NodeRef panel = uniqueVisibleTag(automation, RECEIPT_TAG_PREFIX + receiptId, deadline);
            try {
                NodeRef exact = uniqueReceiptActionChild(panel.node, panel.windowId, RECEIPT_EXACT_TAG, null, 0, deadline);
                exact.node.recycle();
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private NodeRef uniqueDialogAction(AccessibilityNodeInfo dialog, int windowId, String tag, long deadline) {
        return uniqueReceiptActionChild(dialog, windowId, tag, null, AccessibilityNodeInfo.ACTION_CLICK, deadline);
    }

    private boolean revalidateDialogAction(UiAutomation automation, NodeRef node, String dialogTag, String tag, long deadline) {
        if (!revalidateAction(automation, node, tag, AccessibilityNodeInfo.ACTION_CLICK, deadline)) return false;
        if (!hasAncestorTag(node.node, node.windowId, dialogTag, deadline)) return false;
        NodeRef dialog = uniqueVisibleTag(automation, dialogTag, deadline, node.windowId);
        dialog.node.recycle();
        return true;
    }

    private boolean hasAncestorTag(AccessibilityNodeInfo node, int windowId, String tag, long deadline) {
        AccessibilityNodeInfo parent = node.getParent();
        int depth = 0;
        while (parent != null) {
            try {
                requireBeforeDeadline(deadline);
                if (++depth > MAX_RECEIPT_ANCESTORS) throw new Blocked("ui_node_bound");
                NodeMetadata metadata = metadata(parent);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                if (parent.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                if (tag.equals(metadata.viewId)) {
                    parent.recycle();
                    return true;
                }
                AccessibilityNodeInfo next = parent.getParent();
                parent.recycle();
                parent = next;
            } catch (Throwable failure) {
                parent.recycle();
                throw failure;
            }
        }
        return false;
    }

    private Observation awaitEmptyCreationForm(UiAutomation automation, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                int window = creationSheetWindow(automation, deadline);
                NodeRef name = uniqueVisibleTag(automation, CREATE_NAME_TAG, deadline, window);
                try {
                    CharSequence text = name.node.getText();
                    if (text == null || text.length() == 0) return observe(automation, deadline);
                } finally {
                    name.node.recycle();
                }
            } catch (Blocked blocked) {
                // Await rendering after the single acknowledged action; never click again.
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("creation_text_timeout");
    }

    private Observation awaitSeparateCreationDialog(UiAutomation automation, String receiptId, long deadline) {
        String tag = SEPARATE_CREATION_DIALOG_PREFIX + receiptId;
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                NodeRef dialog = uniqueVisibleTag(automation, tag, deadline);
                dialog.node.recycle();
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"ui_tag_missing".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private int creationSheetWindow(UiAutomation automation, long deadline) {
        NodeRef close = uniqueVisibleTag(automation, NEW_SESSION_CLOSE_TAG, deadline);
        try {
            return close.windowId;
        } finally {
            close.node.recycle();
        }
    }

    private int settingsWindow(UiAutomation automation, long deadline) {
        NodeRef appearance = uniqueVisibleTag(automation, HOST_SETTINGS_PANEL_TAG, deadline);
        try {
            return appearance.windowId;
        } finally {
            appearance.node.recycle();
        }
    }

    /** The three closed controls identify Goatr's local Appearance settings surface. */
    private int appearanceSettingsWindow(UiAutomation automation, long deadline) {
        NodeRef system = uniqueVisibleTag(automation, UiAppearance.SYSTEM.tag, deadline);
        try {
            NodeRef light = uniqueVisibleTag(automation, UiAppearance.LIGHT.tag, deadline, system.windowId);
            try {
                NodeRef dark = uniqueVisibleTag(automation, UiAppearance.DARK.tag, deadline, system.windowId);
                try {
                    return system.windowId;
                } finally {
                    dark.node.recycle();
                }
            } finally {
                light.node.recycle();
            }
        } finally {
            system.node.recycle();
        }
    }

    private NodeRef uniqueHostSettingsScrollForward(UiAutomation automation, int expectedWindowId, long deadline) {
        AccessibilityNodeInfo root = awaitInitialProductRoot(automation, deadline);
        int windowId = root.getWindowId();
        if (windowId != expectedWindowId) {
            root.recycle();
            throw new Blocked("ui_root_changed");
        }
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(root);
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                requireBeforeDeadline(deadline);
                if (isSensitiveContent(node)) {
                    node.recycle();
                    continue;
                }
                NodeMetadata metadata = metadata(node);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) {
                    node.recycle();
                    throw new Blocked("ui_root_changed");
                }
                if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) {
                    node.recycle();
                    throw new Blocked("secret_scene");
                }
                int childCount = node.getChildCount();
                requireBeforeDeadline(deadline);
                for (int index = 0; index < childCount; index++) {
                    requireBeforeDeadline(deadline);
                    if (discovered >= MAX_NODES) {
                        node.recycle();
                        throw new Blocked("ui_node_bound");
                    }
                    discovered++;
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) queue.addLast(child);
                    requireBeforeDeadline(deadline);
                }
                if (metadata.visible && metadata.enabled && metadata.scrollable && metadata.supports(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)) {
                    if (selected != null) {
                        node.recycle();
                        throw new Blocked("ui_duplicate");
                    }
                    selected = node;
                } else {
                    node.recycle();
                }
            }
            requireBeforeDeadline(deadline);
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, windowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    private boolean revalidateHostSettingsScrollForward(UiAutomation automation, NodeRef node, long deadline) {
        if (!node.node.refresh()) return false;
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            requireBeforeDeadline(deadline);
            if (root.getWindowId() != node.windowId
                    || !PRODUCT_PACKAGE.equals(metadata.packageName)
                    || metadata.windowId != node.windowId
                    || !metadata.visible
                    || !metadata.enabled
                    || !metadata.scrollable
                    || !metadata.supports(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)) return false;
            NodeRef appearance = uniqueVisibleTag(automation, HOST_SETTINGS_PANEL_TAG, deadline, node.windowId);
            appearance.node.recycle();
            return true;
        } finally {
            root.recycle();
        }
    }

    /** Select only the outer Sessions scroll container by its refresh-anchor ancestry. */
    private NodeRef uniqueHostSessionsScrollForward(UiAutomation automation, long deadline) {
        NodeRef anchor = uniqueVisibleTag(automation, INVENTORY_REFRESH_TAG, deadline);
        AccessibilityNodeInfo current = anchor.node;
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        try {
            for (int ancestors = 0; ancestors < MAX_RECEIPT_ANCESTORS; ancestors++) {
                requireBeforeDeadline(deadline);
                AccessibilityNodeInfo parent = current.getParent();
                requireBeforeDeadline(deadline);
                if (current != anchor.node && current != selected) current.recycle();
                current = parent;
                if (current == null) break;
                NodeMetadata metadata = metadata(current);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != anchor.windowId) throw new Blocked("ui_root_changed");
                if (current.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                if (metadata.visible && metadata.enabled && metadata.scrollable && metadata.supports(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)) {
                    if (selected != null) throw new Blocked("ui_duplicate");
                    selected = current;
                }
            }
            if (current != null) throw new Blocked("ui_node_bound");
            requireBeforeDeadline(deadline);
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, anchor.windowId);
        } finally {
            anchor.node.recycle();
            if (current != null && current != selected) current.recycle();
            if (!complete && selected != null) selected.recycle();
        }
    }

    private boolean revalidateHostSessionsScrollForward(UiAutomation automation, NodeRef node, long deadline) {
        if (!node.node.refresh()) return false;
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        NodeRef anchor = null;
        try {
            NodeMetadata metadata = metadata(node.node);
            requireBeforeDeadline(deadline);
            if (root.getWindowId() != node.windowId
                    || !PRODUCT_PACKAGE.equals(metadata.packageName)
                    || metadata.windowId != node.windowId
                    || !metadata.visible
                    || !metadata.enabled
                    || !metadata.scrollable
                    || !metadata.supports(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)) return false;
            anchor = uniqueVisibleTag(automation, INVENTORY_REFRESH_TAG, deadline, node.windowId);
            return uniqueScrollAncestor(anchor.node, node.node, node.windowId, deadline);
        } finally {
            if (anchor != null) anchor.node.recycle();
            root.recycle();
        }
    }

    private boolean revalidateScrolledSessionsContainer(UiAutomation automation, NodeRef node, long deadline) {
        if (!node.node.refresh()) return false;
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            requireBeforeDeadline(deadline);
            return root.getWindowId() == node.windowId
                    && PRODUCT_PACKAGE.equals(metadata.packageName)
                    && metadata.windowId == node.windowId
                    && metadata.visible
                    && metadata.enabled;
        } finally {
            root.recycle();
        }
    }

    private boolean uniqueScrollAncestor(AccessibilityNodeInfo anchor, AccessibilityNodeInfo expected, int windowId, long deadline) {
        AccessibilityNodeInfo current = anchor;
        boolean expectedFound = false;
        int scrollableCount = 0;
        try {
            for (int ancestors = 0; ancestors < MAX_RECEIPT_ANCESTORS; ancestors++) {
                requireBeforeDeadline(deadline);
                AccessibilityNodeInfo parent = current.getParent();
                requireBeforeDeadline(deadline);
                if (current != anchor) current.recycle();
                current = parent;
                if (current == null) break;
                NodeMetadata metadata = metadata(current);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) return false;
                if (current.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) return false;
                if (metadata.visible && metadata.enabled && metadata.scrollable && metadata.supports(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD)) {
                    scrollableCount++;
                    if (current.equals(expected)) expectedFound = true;
                }
            }
            return current == null && expectedFound && scrollableCount == 1;
        } finally {
            if (current != null && current != anchor) current.recycle();
        }
    }

    private NodeRef uniqueCodexProvider(UiAutomation automation, int expectedWindowId, long deadline) {
        AccessibilityNodeInfo root = awaitInitialProductRoot(automation, deadline);
        int windowId = root.getWindowId();
        if (windowId != expectedWindowId) {
            root.recycle();
            throw new Blocked("creation_form_window");
        }
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(root);
        AccessibilityNodeInfo selected = null;
        boolean complete = false;
        int discovered = 1;
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                requireBeforeDeadline(deadline);
                if (isSensitiveContent(node)) {
                    node.recycle();
                    continue;
                }
                NodeMetadata metadata = metadata(node);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) {
                    node.recycle();
                    throw new Blocked("ui_root_changed");
                }
                if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) {
                    node.recycle();
                    throw new Blocked("secret_scene");
                }
                int childCount = node.getChildCount();
                requireBeforeDeadline(deadline);
                for (int index = 0; index < childCount; index++) {
                    requireBeforeDeadline(deadline);
                    if (discovered >= MAX_NODES) {
                        node.recycle();
                        throw new Blocked("ui_node_bound");
                    }
                    discovered++;
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) queue.addLast(child);
                    requireBeforeDeadline(deadline);
                }
                if (CREATE_PROVIDER_TAG.equals(metadata.viewId) && metadata.visible && metadata.enabled && metadata.checkable && metadata.supports(AccessibilityNodeInfo.ACTION_CLICK) && nodeOrDescendantTextEquals(node, windowId, PROVIDER_CODEX, deadline)) {
                    if (selected != null) {
                        node.recycle();
                        throw new Blocked("ui_duplicate");
                    }
                    if (metadata.selected) {
                        node.recycle();
                        throw new Blocked("ui_revalidation");
                    }
                    selected = node;
                } else {
                    node.recycle();
                }
            }
            requireBeforeDeadline(deadline);
            if (selected == null) throw new Blocked("ui_tag_missing");
            complete = true;
            return new NodeRef(selected, windowId);
        } finally {
            recycleQueue(queue);
            if (!complete && selected != null) selected.recycle();
        }
    }

    private boolean revalidateCodexProvider(UiAutomation automation, NodeRef node, long deadline) {
        if (!node.node.refresh()) return false;
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            requireBeforeDeadline(deadline);
            return root.getWindowId() == node.windowId
                    && PRODUCT_PACKAGE.equals(metadata.packageName)
                    && metadata.windowId == node.windowId
                    && CREATE_PROVIDER_TAG.equals(metadata.viewId)
                    && metadata.visible
                    && metadata.enabled
                    && metadata.checkable
                    && !metadata.selected
                    && metadata.supports(AccessibilityNodeInfo.ACTION_CLICK)
                    && nodeOrDescendantTextEquals(node.node, node.windowId, PROVIDER_CODEX, deadline);
        } finally {
            root.recycle();
        }
    }

    private Observation awaitSelectedCodexProvider(UiAutomation automation, int formWindow, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            Observation observation = observe(automation, deadline);
            requireCondition(observation.windowId == formWindow && observation.newSession, "creation_form_window");
            if (PROVIDER_CODEX.equals(observation.selectedProvider)) return observation;
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("expected_screen_timeout");
    }

    private Observation observe(UiAutomation automation, long deadline) {
        AccessibilityNodeInfo root = awaitInitialProductRoot(automation, deadline);
        int windowId = root.getWindowId();
        requireBeforeDeadline(deadline);
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        queue.add(root);
        int discovered = 1;
        int hosts = 0;
        int sessions = 0;
        int conversations = 0;
        int appearanceSystem = 0;
        int hostSettingsPanels = 0;
        int appearanceLight = 0;
        int appearanceDark = 0;
        int selectedAppearances = 0;
        String selectedAppearance = "unavailable";
        int newSession = 0;
        int inventoryRefresh = 0;
        boolean connectedDirect = false;
        boolean connectedRelay = false;
        boolean offline = false;
        int creationNames = 0;
        boolean creationNameEmpty = false;
        boolean creationDetailsShown = false;
        int creationActions = 0;
        int selectedProviders = 0;
        String selectedProvider = "none";
        String creationBlocker = "none";
        boolean terminalDialog = false;
        boolean terminalOutput = false;
        boolean terminalLoading = false;
        boolean terminalError = false;
        boolean terminalCloseEnabled = false;
        int worktreesDialogs = 0;
        int changesDialogs = 0;
        int snippetScopes = 0;
        int snippetEditors = 0;
        int snippetDeletes = 0;
        int snippetDraftMarkers = 0;
        int snippetSaveControls = 0;
        boolean snippetSaveVisible = false;
        boolean snippetSaveEnabled = false;
        boolean snippetSaveAction = false;
        String snippetState = "unavailable";
        String snippetId = null;
        String snippetDraft = "unavailable";
        Map<String, ReceiptEvidence> receipts = new HashMap<>();
        try {
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                requireBeforeDeadline(deadline);
                if (isTerminalOutput(node)) {
                    terminalOutput = true;
                    node.recycle();
                    continue;
                }
                if (isWorktreesSensitive(node)) {
                    node.recycle();
                    continue;
                }
                if (isChangesSensitive(node)) {
                    node.recycle();
                    continue;
                }
                // Do not inspect or traverse repository-derived Worktrees dialog values.
                if (WORKTREES_DIALOG_TAG.equals(node.getViewIdResourceName())) {
                    worktreesDialogs++;
                    node.recycle();
                    continue;
                }
                // The complete Changes sheet is repository-derived. Observe only that it is open.
                if (CHANGES_DIALOG_TAG.equals(node.getViewIdResourceName())) {
                    changesDialogs++;
                    node.recycle();
                    continue;
                }
                NodeMetadata metadata = metadata(node);
                requireBeforeDeadline(deadline);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) {
                    node.recycle();
                    throw new Blocked("ui_root_changed");
                }
                if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) {
                    node.recycle();
                    throw new Blocked("secret_scene");
                }
                if (SNIPPET_SAVE_TAG.equals(metadata.viewId)) {
                    snippetSaveControls++;
                    snippetSaveVisible = metadata.visible;
                    snippetSaveEnabled = metadata.enabled;
                    snippetSaveAction = metadata.clickable;
                    node.recycle();
                    continue;
                }
                String editorId = canonicalSnippetEditorId(metadata.viewId);
                if (editorId != null) { snippetEditors++; snippetState = "editor"; snippetId = editorId; node.recycle(); continue; }
                if (isSnippetRowTag(metadata.viewId)) { node.recycle(); continue; }
                if (metadata.visible) {
                    if (metadata.viewId != null && metadata.viewId.startsWith(SNIPPET_SCOPE_TAG_PREFIX)) { snippetScopes++; snippetState = "list"; }
                    if (SNIPPET_DRAFT_EMPTY_TAG.equals(metadata.viewId)) { snippetDraftMarkers++; snippetDraft = "empty"; }
                    if (SNIPPET_DRAFT_PRESENT_TAG.equals(metadata.viewId)) { snippetDraftMarkers++; snippetDraft = "present"; }
                    if (metadata.viewId != null && metadata.viewId.startsWith(SNIPPET_DELETE_SCOPE_TAG_PREFIX)) {
                        String candidate = metadata.viewId.substring(SNIPPET_DELETE_SCOPE_TAG_PREFIX.length()); requireUuid(candidate, "ui_tag_missing"); snippetDeletes++; snippetState = "delete"; snippetId = candidate;
                    }
                    if (HOSTS_TAG.equals(metadata.viewId)) hosts++;
                    if (SESSIONS_TAG.equals(metadata.viewId)) sessions++;
                    if (CONVERSATION_LIST_TAG.equals(metadata.viewId)) conversations++;
                    if (APPEARANCE_SYSTEM_TAG.equals(metadata.viewId)) appearanceSystem++;
                    if (HOST_SETTINGS_PANEL_TAG.equals(metadata.viewId)) hostSettingsPanels++;
                    UiAppearance appearance = UiAppearance.fromTag(metadata.viewId);
                    if (appearance != null) {
                        if (appearance == UiAppearance.LIGHT) appearanceLight++;
                        if (appearance == UiAppearance.DARK) appearanceDark++;
                        if (metadata.selected) {
                            selectedAppearances++;
                            selectedAppearance = appearance.wire;
                        }
                    }
                    if (NEW_SESSION_CLOSE_TAG.equals(metadata.viewId)) newSession++;
                    if (INVENTORY_REFRESH_TAG.equals(metadata.viewId)) inventoryRefresh++;
                    if (TERMINAL_DIALOG_TAG.equals(metadata.viewId)) terminalDialog = true;
                    if (TERMINAL_LOADING_TAG.equals(metadata.viewId)) terminalLoading = true;
                    if (TERMINAL_ERROR_TAG.equals(metadata.viewId)) terminalError = true;
                    if (TERMINAL_CLOSE_TAG.equals(metadata.viewId)) terminalCloseEnabled = metadata.enabled && metadata.clickable;
                    CharSequence text = node.getText();
                    if (text != null) {
                        if (CONNECTED_DIRECT.contentEquals(text)) connectedDirect = true;
                        if (CONNECTED_RELAY.contentEquals(text)) connectedRelay = true;
                        if (OFFLINE.contentEquals(text)) offline = true;
                    }
                    if (CREATE_NAME_TAG.equals(metadata.viewId)) {
                        creationNames++;
                        creationNameEmpty = text == null || text.length() == 0;
                    }
                    if (CREATE_SETUP_DETAILS_TAG.equals(metadata.viewId) && text != null && HIDE_DETAILS.contentEquals(text)) creationDetailsShown = true;
                    if (SESSION_CREATE_TAG.equals(metadata.viewId) && metadata.enabled && metadata.clickable) creationActions++;
                    if (CREATE_PROVIDER_TAG.equals(metadata.viewId) && metadata.selected) {
                        selectedProviders++;
                        selectedProvider = nodeOrDescendantTextEquals(node, windowId, PROVIDER_OMP, deadline) ? PROVIDER_OMP : nodeOrDescendantTextEquals(node, windowId, PROVIDER_CODEX, deadline) ? PROVIDER_CODEX : "other";
                    }
                    if (CREATE_BLOCKING_REASON_TAG.equals(metadata.viewId) && "none".equals(creationBlocker)) creationBlocker = creationBlockerFamily(text);
                    String uuid = receiptUuidForTag(metadata.viewId);
                    if (uuid != null) {
                        receiptEvidence(receipts, uuid).panel = true;
                    }
                    if (RECEIPT_STATE_TAG.equals(metadata.viewId)) {
                        String receipt = receiptAncestorId(node, windowId, deadline);
                        if (receipt != null) receiptEvidence(receipts, receipt).state = receiptStateFamily(text);
                    }
                    if (text != null && NEW_CONVERSATION_STATUS.contentEquals(text)) {
                        String receipt = receiptAncestorId(node, windowId, deadline);
                        if (receipt != null) receiptEvidence(receipts, receipt).heading = true;
                    }
                }
                int childCount = node.getChildCount();
                requireBeforeDeadline(deadline);
                for (int index = 0; index < childCount; index++) {
                    requireBeforeDeadline(deadline);
                    if (discovered >= MAX_NODES) {
                        node.recycle();
                        throw new Blocked("ui_node_bound");
                    }
                    discovered++;
                    AccessibilityNodeInfo child = node.getChild(index);
                    if (child != null) queue.addLast(child);
                    requireBeforeDeadline(deadline);
                }
                node.recycle();
            }
        } finally {
            recycleQueue(queue);
        }
        requireBeforeDeadline(deadline);
        if (hosts > 1 || sessions > 1 || conversations > 1 || appearanceSystem > 1 || appearanceLight > 1 || appearanceDark > 1 || newSession > 1 || inventoryRefresh > 1 || creationNames > 1 || selectedProviders > 1 || snippetScopes > 1 || snippetEditors > 1 || snippetDeletes > 1 || snippetDraftMarkers > 1 || snippetSaveControls > 1 || worktreesDialogs > 1 || changesDialogs > 1) throw new Blocked("ui_duplicate");
        if (hostSettingsPanels > 1) throw new Blocked("ui_duplicate");
        int appearanceControls = appearanceSystem + appearanceLight + appearanceDark;
        if (appearanceControls != 0 && (appearanceControls != UiAppearance.values().length || selectedAppearances != 1)) selectedAppearance = "error";
        String terminalState = !terminalDialog ? "hidden" : terminalLoading ? "loading" : terminalError ? "error" : terminalOutput ? "ready" : "error";
        Observation result = new Observation(windowId, hosts == 1, sessions == 1, conversations == 1, appearanceSystem == 1, selectedAppearance, newSession == 1, inventoryRefresh == 1, connectedDirect, connectedRelay, offline, creationNames == 1, creationNameEmpty, creationDetailsShown, creationActions == 1, selectedProvider, creationBlocker, null, "none", receipts, terminalDialog, terminalState, terminalCloseEnabled, worktreesDialogs == 1, changesDialogs == 1);
        result.snippetState = snippetScopes == 0 && snippetDeletes == 0 ? "unavailable" : snippetState;
        result.snippetId = snippetId;
        result.snippetDraft = snippetScopes == 1 ? snippetDraft : "unavailable";
        result.snippetSaveState = snippetSaveControls == 0
                ? (snippetScopes == 0 && snippetEditors == 0 && snippetDeletes == 0 ? "unavailable" : "missing")
                : !snippetSaveVisible ? "hidden"
                : !snippetSaveEnabled ? "disabled"
                : !snippetSaveAction ? "no_action"
                : "ready";
        result.productForeground = true;
        result.hostSettingsPanel = hostSettingsPanels == 1;
        return result;
    }

    private AccessibilityNodeInfo awaitInitialProductRoot(UiAutomation automation, long deadline) {
        String last = "ui_root_missing";
        while (SystemClock.elapsedRealtime() < deadline) {
            AccessibilityNodeInfo root = automation.getRootInActiveWindow();
            requireBeforeDeadline(deadline);
            if (root == null) {
                last = "ui_root_missing";
                SystemClock.sleep(POLL_MILLIS);
                continue;
            }
            CharSequence packageName = root.getPackageName();
            if (packageName == null || !PRODUCT_PACKAGE.contentEquals(packageName)) {
                root.recycle();
                last = "ui_root_foreign";
                SystemClock.sleep(POLL_MILLIS);
                continue;
            }
            return root;
        }
        throw new Blocked(last);
    }

    /** Final click validation never waits for a new root or retries an action. */
    private static AccessibilityNodeInfo currentProductRoot(UiAutomation automation, long deadline) {
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = automation.getRootInActiveWindow();
        requireBeforeDeadline(deadline);
        if (root == null) throw new Blocked("ui_root_missing");
        CharSequence packageName = root.getPackageName();
        if (packageName == null || !PRODUCT_PACKAGE.contentEquals(packageName)) {
            root.recycle();
            throw new Blocked("ui_root_foreign");
        }
        return root;
    }

    private boolean revalidate(UiAutomation automation, NodeRef node, String tag, long deadline) {
        return revalidateAction(automation, node, tag, AccessibilityNodeInfo.ACTION_CLICK, deadline);
    }

    private boolean revalidateAction(UiAutomation automation, NodeRef node, String tag, int actionId, long deadline) {
        requirePhysicalEpoch(activeRequest);
        if(activeRequest!=null&&activeRequest.physical!=null&&!activeRequest.action.isPhysicalAction()&&!activeRequest.action.physicalHostNavigation()
                &&activeRequest.action!=Action.NAV_SESSIONS&&activeRequest.action!=Action.NAV_HOSTS&&activeRequest.action!=Action.OBSERVE) {
            JSONObject state=requirePhysicalEpoch(activeRequest);
            requireCondition(activeRequest.physical.host().equals(state.optString("selectedHost","")),"physical_product_epoch");
        }
        if (isSensitiveContent(node.node)) return false;
        if (!node.node.refresh()) return false;
        requireBeforeDeadline(deadline);
        AccessibilityNodeInfo root = currentProductRoot(automation, deadline);
        try {
            NodeMetadata metadata = metadata(node.node);
            requireBeforeDeadline(deadline);
            return root.getWindowId() == node.windowId
                    && PRODUCT_PACKAGE.equals(metadata.packageName)
                    && metadata.windowId == node.windowId
                    && tag.equals(metadata.viewId)
                    && metadata.visible
                    && metadata.enabled
                    && metadata.supports(actionId);
        } finally {
            root.recycle();
        }
    }

    private boolean revalidateAppearanceAction(UiAutomation automation, NodeRef node, UiAppearance appearance, long deadline) {
        if (!revalidateAction(automation, node, appearance.tag, AccessibilityNodeInfo.ACTION_CLICK, deadline)) return false;
        return appearanceSettingsWindow(automation, deadline) == node.windowId;
    }

    private boolean revalidateConversationAction(UiAutomation automation, NodeRef node, String tag, int actionId, String sessionId, String sessionGeneration, long deadline) {
        if (!revalidateAction(automation, node, tag, actionId, deadline)) return false;
        int conversationWindow = exactConversationWindow(automation, sessionId, sessionGeneration, deadline);
        requireBeforeDeadline(deadline);
        return conversationWindow == node.windowId;
    }

    private boolean revalidatePromptSend(UiAutomation automation, NodeRef node, String sessionId, String sessionGeneration, String text, long deadline) {
        if (!revalidateConversationAction(automation, node, PROMPT_SEND_TAG, AccessibilityNodeInfo.ACTION_CLICK, sessionId, sessionGeneration, deadline)) return false;
        requireExactComposerText(automation, sessionId, sessionGeneration, text, deadline);
        requireBeforeDeadline(deadline);
        return true;
    }

    private void requireTagTextExact(UiAutomation automation, String tag, String expected, long deadline) {
        requireTagTextExact(automation, tag, expected, NO_WINDOW, deadline);
    }

    private void requireTagTextExact(UiAutomation automation, String tag, String expected, int expectedWindowId, long deadline) {
        NodeRef node = uniqueVisibleTag(automation, tag, deadline, expectedWindowId);
        try {
            if (node.node.isPassword()) throw new Blocked("secret_scene");
            requireCondition(nodeOrDescendantTextEquals(node.node, node.windowId, expected, deadline), "creation_form_mismatch");
        } finally {
            node.node.recycle();
        }
    }

    private Observation awaitTextExact(UiAutomation automation, String tag, String expected, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                requireTagTextExact(automation, tag, expected, deadline);
                return observe(automation, deadline);
            } catch (Blocked blocked) {
                if (!"creation_form_mismatch".equals(blocked.code)) throw blocked;
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("creation_text_timeout");
    }

    private boolean nodeOrDescendantTextEquals(AccessibilityNodeInfo root, int windowId, String expected, long deadline) {
        ArrayDeque<AccessibilityNodeInfo> queue = new ArrayDeque<>();
        int discovered = 1;
        try {
            if (isSensitiveContent(root)) return false;
            NodeMetadata rootMetadata = metadata(root);
            if (isSnippetRowTag(rootMetadata.viewId) || canonicalSnippetEditorId(rootMetadata.viewId) != null) return false;
            CharSequence rootText = root.getText();
            if (rootText != null && expected.contentEquals(rootText)) return true;
            int rootChildren = root.getChildCount();
            for (int index = 0; index < rootChildren; index++) {
                requireBeforeDeadline(deadline);
                if (discovered >= MAX_NODES) throw new Blocked("ui_node_bound");
                discovered++;
                AccessibilityNodeInfo child = root.getChild(index);
                if (child != null) queue.addLast(child);
            }
            while (!queue.isEmpty()) {
                AccessibilityNodeInfo node = queue.removeFirst();
                try {
                    requireBeforeDeadline(deadline);
                    if (isSensitiveContent(node)) continue;
                    NodeMetadata metadata = metadata(node);
                    if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                    if (node.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                    if (isSnippetRowTag(metadata.viewId) || canonicalSnippetEditorId(metadata.viewId) != null) continue;
                    CharSequence text = node.getText();
                    if (text != null && expected.contentEquals(text)) return true;
                    int children = node.getChildCount();
                    for (int index = 0; index < children; index++) {
                        requireBeforeDeadline(deadline);
                        if (discovered >= MAX_NODES) throw new Blocked("ui_node_bound");
                        discovered++;
                        AccessibilityNodeInfo child = node.getChild(index);
                        if (child != null) queue.addLast(child);
                    }
                } finally {
                    node.recycle();
                }
            }
            return false;
        } finally {
            recycleQueue(queue);
        }
    }

    private void requireSelectedProvider(UiAutomation automation, String expected, int formWindow, long deadline) {
        Observation observation = observe(automation, deadline);
        requireCondition(observation.windowId == formWindow, "creation_form_window");
        requireCondition(expected.equals(observation.selectedProvider), "creation_provider_mismatch");
    }

    private Observation awaitCreationReceipt(UiAutomation automation, Set<String> excludedReceiptIds, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            Observation observation = observe(automation, deadline);
            ReceiptEvidence selected = null;
            String selectedId = null;
            for (Map.Entry<String, ReceiptEvidence> entry : observation.receipts.entrySet()) {
                ReceiptEvidence evidence = entry.getValue();
                if (excludedReceiptIds.contains(entry.getKey()) || !evidence.panel || !evidence.heading || "none".equals(evidence.state) || "other".equals(evidence.state)) continue;
                if (selected != null) throw new Blocked("ui_duplicate");
                selected = evidence;
                selectedId = entry.getKey();
            }
            if (!observation.newSession && selected != null) return observation.withReceipt(selectedId, selected.state);
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("creation_receipt_timeout");
    }

    private ReceiptEvidence receiptEvidence(Map<String, ReceiptEvidence> receipts, String id) {
        ReceiptEvidence evidence = receipts.get(id);
        if (evidence == null) {
            evidence = new ReceiptEvidence();
            receipts.put(id, evidence);
        }
        return evidence;
    }

    private String receiptAncestorId(AccessibilityNodeInfo node, int windowId, long deadline) {
        AccessibilityNodeInfo parent = node.getParent();
        int depth = 0;
        while (parent != null) {
            try {
                requireBeforeDeadline(deadline);
                if (++depth > MAX_RECEIPT_ANCESTORS) throw new Blocked("ui_node_bound");
                NodeMetadata metadata = metadata(parent);
                if (!PRODUCT_PACKAGE.equals(metadata.packageName) || metadata.windowId != windowId) throw new Blocked("ui_root_changed");
                if (parent.isPassword() || PAIRING_SCENE_TAGS.contains(metadata.viewId)) throw new Blocked("secret_scene");
                String id = receiptUuidForTag(metadata.viewId);
                if (id != null) {
                    parent.recycle();
                    return id;
                }
                AccessibilityNodeInfo next = parent.getParent();
                parent.recycle();
                parent = next;
            } catch (Throwable failure) {
                parent.recycle();
                throw failure;
            }
        }
        return null;
    }

    private String creationBlockerFamily(CharSequence text) {
        if (text == null) return "other";
        if ("Reconnect before creating a session.".contentEquals(text)) return "reconnect";
        if ("Wait for the phone to finish synchronizing or its current operation.".contentEquals(text)) return "synchronizing";
        if ("Choose a server.".contentEquals(text) || "The selected server is not online.".contentEquals(text)) return "server";
        if ("Refresh the selected server's desktop source before creating a session.".contentEquals(text)) return "source";
        if ("The selected server is not ready to create OMP sessions.".contentEquals(text) || "The selected server is not ready to create Codex sessions.".contentEquals(text)) return "backend";
        if ("Choose an available project.".contentEquals(text)) return "project";
        if ("Enter a session name between 1 and 128 characters.".contentEquals(text)) return "name";
        return "other";
    }

    private String receiptUuidForTag(String tag) {
        if (tag == null || !tag.startsWith(RECEIPT_TAG_PREFIX)) return null;
        String candidate = tag.substring(RECEIPT_TAG_PREFIX.length());
        try {
            return UUID.fromString(candidate).toString().equals(candidate) ? candidate : null;
        } catch (IllegalArgumentException exception) {
            return null;
        }
    }

    private String receiptStateFamily(CharSequence text) {
        if (text == null) return "other";
        if ("Preparing".contentEquals(text)) return "reserved";
        if ("Sending".contentEquals(text)) return "dispatching";
        if ("Sent to desktop".contentEquals(text)) return "dispatched";
        if ("Action accepted".contentEquals(text)) return "succeeded";
        if ("Action failed · inspect details".contentEquals(text)) return "failed";
        if ("Outcome unknown · check status".contentEquals(text)) return "uncertain";
        return "other";
    }

    private static NodeMetadata metadata(AccessibilityNodeInfo node) {
        CharSequence packageName = node.getPackageName();
        boolean clickable = false;
        boolean setText = false;
        boolean scrollForward = false;
        boolean scrollBackward = false;
        boolean checkable = node.isCheckable();
        boolean selected = node.isSelected() || (checkable && node.isChecked());
        List<AccessibilityNodeInfo.AccessibilityAction> actions = node.getActionList();
        for (AccessibilityNodeInfo.AccessibilityAction action : actions) {
            if (action.getId() == AccessibilityNodeInfo.ACTION_CLICK) {
                clickable = true;
            }
            if (action.getId() == AccessibilityNodeInfo.ACTION_SET_TEXT) setText = true;
            if (action.getId() == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD) scrollForward = true;
            if (action.getId() == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD) scrollBackward = true;
        }
        return new NodeMetadata(
                packageName == null ? null : packageName.toString(),
                node.getViewIdResourceName(),
                node.getWindowId(),
                node.isVisibleToUser(),
                node.isEnabled(),
                clickable,
                setText,
                node.isScrollable(),
                scrollForward,
                scrollBackward,
                checkable,
                selected
        );
    }

    /** This tag is a hard privacy boundary: only its identifier is inspected. */
    private static boolean isTerminalOutput(AccessibilityNodeInfo node) {
        return TERMINAL_OUTPUT_TAG.equals(node.getViewIdResourceName());
    }

    /** Worktree values are repository-derived and are never read by automation. */
    private static boolean isWorktreesSensitive(AccessibilityNodeInfo node) {
        String tag = node.getViewIdResourceName();
        return WORKTREES_LIST_TAG.equals(tag) || WORKTREES_ENTRY_TAG.equals(tag);
    }

    /** Changes contain paths, revision names and patches; automation never reads their subtree. */
    private static boolean isChangesSensitive(AccessibilityNodeInfo node) {
        String tag = node.getViewIdResourceName();
        return CHANGES_BODY_TAG.equals(tag) || CHANGES_ENTRY_TAG.equals(tag) || CHANGES_STAGED_PATCH_TAG.equals(tag) || CHANGES_UNSTAGED_PATCH_TAG.equals(tag);
    }

    /** Shared privacy boundary for every walker that could otherwise read descendants. */
    private static boolean isSensitiveContent(AccessibilityNodeInfo node) {
        return node.isPassword()
                || PAIRING_SCENE_TAGS.contains(node.getViewIdResourceName())
                || isTerminalOutput(node)
                || isWorktreesSensitive(node)
                || isChangesSensitive(node);
    }

    @SuppressWarnings("deprecation")
    private static Set<String> signerDigests(Context context, String packageName) throws PackageManager.NameNotFoundException {
        android.content.pm.PackageInfo info = context.getPackageManager().getPackageInfo(packageName, PackageManager.GET_SIGNATURES);
        Set<String> digests = new HashSet<>();
        if (info.signatures != null) {
            for (android.content.pm.Signature signature : info.signatures) digests.add(digest(signature.toByteArray()));
        }
        return digests;
    }

    private RunnerIdentity runnerIdentity() throws IOException {
        int pid = Process.myPid();
        String stat = new String(Files.readAllBytes(new File("/proc/self/stat").toPath()), StandardCharsets.UTF_8);
        int firstSpace = stat.indexOf(' ');
        requireCondition(firstSpace > 0 && Integer.parseInt(stat.substring(0, firstSpace)) == pid, "runner_pid");
        int close = stat.lastIndexOf(')');
        requireCondition(close >= 0, "runner_start");
        String[] fields = stat.substring(close + 2).trim().split("\\s+");
        requireCondition(fields.length > 19, "runner_start");
        long start = Long.parseLong(fields[19]);
        requireCondition(start > 0L, "runner_start");
        return new RunnerIdentity(pid, start);
    }

    private static void requireBeforeDeadline(long deadline) {
        if (SystemClock.elapsedRealtime() >= deadline) throw new Blocked("deadline");
    }

    private File intentFile(Context context, String runId) {
        return new File(context.getCacheDir(), "owned-ui-automation-" + runId + ".intent.json");
    }

    private File outcomeFile(Context context, String runId) {
        return new File(context.getCacheDir(), "owned-ui-automation-" + runId + ".outcome.json");
    }

    private File releaseDirectory(File cacheDir, String runId) {
        return new File(cacheDir, "owned-ui-release-" + runId);
    }

    private void awaitReleaseDirectory(File releaseDirectory, long deadline) {
        while (SystemClock.elapsedRealtime() < deadline) {
            try {
                if (OsConstants.S_ISDIR(Os.lstat(releaseDirectory.getPath()).st_mode)) return;
                throw new Blocked("release_marker_invalid");
            } catch (ErrnoException error) {
                if (error.errno != OsConstants.ENOENT) throw new Blocked("release_marker_invalid");
            }
            SystemClock.sleep(POLL_MILLIS);
        }
        throw new Blocked("release_timeout");
    }

    private void writeExclusive(File cacheDir, File file, String content) throws IOException {
        requireCondition(cacheDir.equals(file.getParentFile()), "report_path");
        byte[] bytes = content.getBytes(StandardCharsets.UTF_8);
        try (java.nio.channels.SeekableByteChannel channel = Files.newByteChannel(file.toPath(), StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE)) {
            ByteBuffer buffer = ByteBuffer.wrap(bytes);
            while (buffer.hasRemaining()) channel.write(buffer);
            if (channel instanceof FileChannel) ((FileChannel) channel).force(true);
        } finally {
            Arrays.fill(bytes, (byte) 0);
        }
    }

    private String intentJson(Request request, RunnerIdentity self) {
        String creationName = request.creationName == null ? "null" : "\"" + request.creationName + "\"";
        String creationServerId = request.creationServerId == null ? "null" : "\"" + request.creationServerId + "\"";
        String routeArguments=request.routeHostId==null ? "null" : "\""+digest((request.routeHostId+"\n"+(request.routeDirect==null?"":request.routeDirect)).getBytes(StandardCharsets.UTF_8))+"\"";
        String routeClass=request.routeHostId==null ? "none" : request.routeDirect==null ? "host_scope" : "literal_ipv4";
        String routeIntent=request.routeHostId==null ? "" : ",\"routeArgumentClass\":\""+routeClass+"\",\"routeArgumentHash\":"+routeArguments;
        String physicalIntent=request.physical==null?"":",\"physicalArgumentHash\":\""+request.physical.argumentHash+"\"";
        return "{\"phase\":\"intent\",\"runId\":\"" + request.runId + "\",\"action\":\"" + request.action.wire + "\",\"creationName\":" + creationName + ",\"creationServerId\":" + creationServerId + routeIntent + physicalIntent + ",\"excludedReceiptCount\":" + request.excludedReceiptIds.size() + ",\"runnerPid\":" + self.pid + ",\"runnerStart\":" + self.start + "}";
    }

    private String processJson(Request request, RunnerIdentity self) {
        return "{\"phase\":\"process\",\"runId\":\"" + request.runId + "\",\"action\":\"" + request.action.wire + "\",\"runnerPid\":" + self.pid + ",\"runnerStart\":" + self.start + "}";
    }

    private String blockedCode(Throwable failure) {
        String code = failure instanceof Blocked ? ((Blocked) failure).code : failure.getMessage();
        if("argument_unknown".equals(code))return "request";
        if(Arrays.asList("physical_action_forbidden","physical_selector_unexpected","physical_selector_required","physical_host_invalid",
                "physical_uuid_invalid","physical_workspace_invalid","physical_observer_invalid","physical_artifact_invalid",
                "physical_dimensions_invalid","physical_dimension_field_invalid","physical_scroll_invalid","physical_delivery_invalid",
                "physical_boolean_invalid","physical_label_invalid","physical_category_invalid","physical_answer_invalid","physical_key_invalid",
                "physical_number_invalid","physical_epoch_proof","physical_product_epoch","physical_postcondition","physical_history_unavailable",
                "physical_history_changed","physical_delivery_changed","physical_delivery_already_running","physical_cleanup_not_owned",
                "physical_retained_keeper","physical_precondition_not_fresh","physical_field_not_empty","physical_systemui_unqualified",
                "physical_notification_shade","physical_notification_row_ambiguous","physical_notification_row_missing",
                "physical_pairing_preview","physical_pairing_paste_not_empty","physical_agent_not_read_only").contains(code))return code;
        if ("run_id_missing".equals(code) || "run_id_invalid".equals(code) || "action_invalid".equals(code) || "appearance_invalid".equals(code) || "appearance_unexpected".equals(code) || "session_list_preference_invalid".equals(code) || "session_list_preference_value_invalid".equals(code) || "session_list_preference_unexpected".equals(code) || "creation_project_label_invalid".equals(code) || "creation_project_label_unexpected".equals(code) || "creation_server_id_invalid".equals(code) || "creation_server_id_unexpected".equals(code) || "creation_name_invalid".equals(code) || "creation_name_unexpected".equals(code) || "previous_creation_name_invalid".equals(code) || "previous_creation_name_unexpected".equals(code) || "composer_text_invalid".equals(code) || "composer_text_unexpected".equals(code) || "session_id_invalid".equals(code) || "session_id_unexpected".equals(code) || "session_generation_invalid".equals(code) || "session_generation_unexpected".equals(code) || "history_record_id_invalid".equals(code) || "history_record_id_unexpected".equals(code) || "receipt_id_invalid".equals(code) || "receipt_id_unexpected".equals(code) || "excluded_receipts_invalid".equals(code) || "excluded_receipts_unexpected".equals(code) || "snippet_workflow_invalid".equals(code) || "snippet_workflow_unexpected".equals(code) || "snippet_id_invalid".equals(code) || "snippet_id_unexpected".equals(code) || "snippet_version_invalid".equals(code) || "snippet_version_unexpected".equals(code)) return "request";
        if ("route_host_invalid".equals(code) || "route_direct_invalid".equals(code) || "route_arguments_unexpected".equals(code)) return "request";
        if ("route_direct_not_empty".equals(code) || "route_direct_not_owned".equals(code) || "route_reconnect_not_fresh".equals(code) || "route_postcondition_timeout".equals(code)) return code;
        if ("runner_pid".equals(code) || "runner_start".equals(code)) return "runner_identity";
        if ("test_package".equals(code) || "api_bounds".equals(code) || "test_not_debug".equals(code) || "product_not_debug".equals(code) || "signer_mismatch".equals(code)) return "eligibility";
        if ("deadline".equals(code)) return "deadline";
        if ("ui_root_missing".equals(code)) return "ui_root_missing";
        if ("interactive_window_root_missing".equals(code)) return "interactive_window_root_missing";
        if ("product_window_missing".equals(code)) return "product_window_missing";
        if ("ui_child_missing".equals(code)) return "ui_child_missing";
        if ("creation_picker_window_ambiguous".equals(code)) return "creation_picker_window_ambiguous";
        if ("ui_root_foreign".equals(code)) return "ui_root_foreign";
        if ("ui_root_changed".equals(code) || "appearance_settings_window".equals(code)) return "ui_root_changed";
        if ("ui_node_bound".equals(code)) return "ui_node_bound";
        if ("ui_duplicate".equals(code)) return "ui_duplicate";
        if ("ui_tag_missing".equals(code)) return "ui_tag_missing";
        if ("ui_revalidation".equals(code)) return "ui_revalidation";
        if ("ui_click_acknowledgement".equals(code)) return "ui_click_acknowledgement";
        if ("ui_global_action_acknowledgement".equals(code)) return "ui_global_action_acknowledgement";
        if ("home_resolution_invalid".equals(code)) return "home_resolution_invalid";
        if ("appearance_selection_invalid".equals(code)) return "appearance_selection_invalid";
        if ("appearance_selection_timeout".equals(code)) return "appearance_selection_timeout";
        if ("expected_screen_timeout".equals(code)) return "expected_screen_timeout";
        if ("creation_name_not_empty".equals(code)) return "creation_name_not_empty";
        if ("creation_name_previous_mismatch".equals(code)) return "creation_name_previous_mismatch";
        if ("composer_not_empty".equals(code)) return "composer_not_empty";
        if ("creation_form_mismatch".equals(code)) return "creation_form_mismatch";
        if ("creation_provider_mismatch".equals(code)) return "creation_provider_mismatch";
        if ("creation_text_timeout".equals(code)) return "creation_text_timeout";
        if ("creation_picker_timeout".equals(code)) return "creation_picker_timeout";
        if ("creation_picker_unavailable".equals(code)) return "creation_picker_unavailable";
        if ("composer_text_timeout".equals(code)) return "composer_text_timeout";
        if ("snippet_text_timeout".equals(code)) return "snippet_text_timeout";
        if ("snippet_scope_mismatch".equals(code)) return "snippet_scope_mismatch";
        if ("snippet_field_not_owned".equals(code) || "snippet_draft_not_owned".equals(code)) return "snippet_not_owned";
        if ("creation_receipt_timeout".equals(code)) return "creation_receipt_timeout";
        if ("ui_set_text_acknowledgement".equals(code)) return "ui_set_text_acknowledgement";
        if ("secret_scene".equals(code)) return "secret_scene";
        if ("release_marker_existing".equals(code)) return "release_marker_existing";
        if ("release_marker_invalid".equals(code)) return "release_marker_invalid";
        if ("release_timeout".equals(code)) return "release_timeout";
        if ("report_path".equals(code)) return "report";
        return "other";
    }

    private ErrorFamily errorFamily(Throwable failure) {
        if (failure instanceof SecurityException) return ErrorFamily.SECURITY;
        if (failure instanceof IOException) return ErrorFamily.IO;
        if (failure instanceof LinkageError) return ErrorFamily.LINKAGE;
        if (failure instanceof IllegalStateException) return ErrorFamily.ILLEGAL_STATE;
        if (failure instanceof NullPointerException) return ErrorFamily.NULL_VALUE;
        if (failure instanceof UnsupportedOperationException) return ErrorFamily.UNSUPPORTED;
        return ErrorFamily.OTHER;
    }

    private static String digest(byte[] bytes) {
        try {
            byte[] value = MessageDigest.getInstance("SHA-256").digest(bytes);
            StringBuilder result = new StringBuilder(value.length * 2);
            for (byte item : value) result.append(String.format("%02x", item & 255));
            return result.toString();
        } catch (NoSuchAlgorithmException exception) {
            throw new Blocked("signer_digest");
        }
    }

    private static void recycleQueue(ArrayDeque<AccessibilityNodeInfo> queue) {
        while (!queue.isEmpty()) queue.removeFirst().recycle();
    }

    private static void requireCondition(boolean condition, String code) {
        if (!condition) throw new Blocked(code);
    }

    private static final class Request {
        final String runId;
        final Action action;
        final UiAppearance appearance;
        final SessionListPreference sessionListPreference;
        final Boolean sessionListPreferenceValue;
        final String creationServerLabel;
        final String creationServerId;
        final String creationProjectLabel;
        final String creationName;
        final String previousCreationName;
        final String composerText;
        final String sessionId;
        final String sessionGeneration;
        final String historyRecordId;
        final String receiptId;
        final Set<String> excludedReceiptIds;
        final String snippetWorkflowId;
        final String snippetId;
        final SnippetVersion snippetVersion;
        final String routeHostId;
        final String routeDirect;
        final PhysicalSelectors physical;
        Request(String runId, Action action, UiAppearance appearance, SessionListPreference sessionListPreference, Boolean sessionListPreferenceValue, String creationServerLabel, String creationServerId, String creationProjectLabel, String creationName, String previousCreationName, String composerText, String sessionId, String sessionGeneration, String historyRecordId, String receiptId, Set<String> excludedReceiptIds, String snippetWorkflowId, String snippetId, SnippetVersion snippetVersion) {this(runId,action,appearance,sessionListPreference,sessionListPreferenceValue,creationServerLabel,creationServerId,creationProjectLabel,creationName,previousCreationName,composerText,sessionId,sessionGeneration,historyRecordId,receiptId,excludedReceiptIds,snippetWorkflowId,snippetId,snippetVersion,null,null,null);}
        Request(String runId, Action action, UiAppearance appearance, SessionListPreference sessionListPreference, Boolean sessionListPreferenceValue, String creationServerLabel, String creationServerId, String creationProjectLabel, String creationName, String previousCreationName, String composerText, String sessionId, String sessionGeneration, String historyRecordId, String receiptId, Set<String> excludedReceiptIds, String snippetWorkflowId, String snippetId, SnippetVersion snippetVersion,String routeHostId,String routeDirect,PhysicalSelectors physical) {this.physical=physical;this.routeHostId=routeHostId;this.routeDirect=routeDirect;this.runId=runId;this.action=action;this.appearance=appearance;this.sessionListPreference=sessionListPreference;this.sessionListPreferenceValue=sessionListPreferenceValue;this.creationServerLabel=creationServerLabel;this.creationServerId=creationServerId;this.creationProjectLabel=creationProjectLabel;this.creationName=creationName;this.previousCreationName=previousCreationName;this.composerText=composerText;this.sessionId=sessionId;this.sessionGeneration=sessionGeneration;this.historyRecordId=historyRecordId;this.receiptId=receiptId;this.excludedReceiptIds=excludedReceiptIds;this.snippetWorkflowId=snippetWorkflowId;this.snippetId=snippetId;this.snippetVersion=snippetVersion;}
    }

    private static final class RunnerIdentity {
        final int pid;
        final long start;
        RunnerIdentity(int pid, long start) { this.pid = pid; this.start = start; }
    }

    private static final class NodeRef {
        final AccessibilityNodeInfo node;
        final int windowId;
        NodeRef(AccessibilityNodeInfo node, int windowId) { this.node = node; this.windowId = windowId; }
    }

    private static final class NodeMetadata {
        final String packageName;
        final String viewId;
        final int windowId;
        final boolean visible;
        final boolean enabled;
        final boolean clickable;
        final boolean setText;
        final boolean scrollable;
        final boolean scrollForward;
        final boolean scrollBackward;
        final boolean checkable;
        final boolean selected;
        NodeMetadata(String packageName, String viewId, int windowId, boolean visible, boolean enabled, boolean clickable, boolean setText, boolean scrollable, boolean scrollForward, boolean scrollBackward, boolean checkable, boolean selected) {
            this.packageName = packageName;
            this.viewId = viewId;
            this.windowId = windowId;
            this.visible = visible;
            this.enabled = enabled;
            this.clickable = clickable;
            this.setText = setText;
            this.scrollable = scrollable;
            this.scrollForward = scrollForward;
            this.scrollBackward = scrollBackward;
            this.checkable = checkable;
            this.selected = selected;
        }
        boolean supports(int actionId) {
            return actionId == AccessibilityNodeInfo.ACTION_CLICK ? clickable
                    : actionId == AccessibilityNodeInfo.ACTION_SET_TEXT ? setText
                    : actionId == AccessibilityNodeInfo.ACTION_SCROLL_FORWARD ? scrollForward
                    : actionId == AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD && scrollBackward;
        }
    }

    private static final class ActionResult {
        final boolean acknowledged;
        final boolean expectedScreenObserved;
        final Observation observation;
        ActionResult(boolean acknowledged, boolean expectedScreenObserved, Observation observation) {
            this.acknowledged = acknowledged;
            this.expectedScreenObserved = expectedScreenObserved;
            this.observation = observation;
        }
    }

    private static final class HomeWindowObservation {
        final boolean complete;
        final boolean productForeground;
        final boolean homeForeground;
        HomeWindowObservation(boolean complete, boolean productForeground, boolean homeForeground) {
            this.complete = complete;
            this.productForeground = productForeground;
            this.homeForeground = homeForeground;
        }
    }

    private static final class Acknowledgement {
        boolean value;
    }

    private static final class StageTracker {
        Stage stage;
        StageTracker(Stage stage) { this.stage = stage; }
    }

    private static final class ReceiptEvidence {
        boolean panel;
        boolean heading;
        String state = "none";
    }

    private static final class CreationPickerObservation {
        int productWindowCount;
        int closeTotal;
        int closeVisible;
        int closeEnabled;
        int serverTotal;
        int serverVisible;
        int optionTotal;
        int optionVisible;
        int optionEnabled;
        int optionClickSupported;
        int optionExpectedLabelCount;
        final Set<Integer> sheetWindows = new HashSet<>();
        final Set<Integer> optionWindows = new HashSet<>();
    }

    private static final class Observation {
        final int windowId;
        final boolean hosts;
        final boolean sessions;
        final boolean conversation;
        final boolean appearanceSystem;
        boolean hostSettingsPanel;
        boolean hostRouteScope;
        boolean routeDirectOwned;
        boolean routeSaved;
        boolean routeReconnectAttempted;
        final String selectedAppearance;
        final boolean newSession;
        final boolean inventoryRefresh;
        final boolean connectedDirect;
        final boolean connectedRelay;
        final boolean offline;
        final boolean creationName;
        final boolean creationNameEmpty;
        final boolean creationDetailsShown;
        final boolean creationActionAvailable;
        final String selectedProvider;
        final String creationBlocker;
        final String receiptId;
        final String receiptState;
        final Map<String, ReceiptEvidence> receipts;
        final boolean terminalDialog;
        final String terminalState;
        final boolean terminalCloseEnabled;
        final boolean worktreesDialog;
        final boolean changesDialog;
        boolean creationServerSelected;
        boolean creationProjectSelected;
        boolean creationPickerTraversalComplete;
        int creationPickerProductWindowCount;
        int creationCloseTotal;
        int creationCloseVisible;
        int creationCloseEnabled;
        int creationServerTotal;
        int creationServerVisible;
        int creationServerOptionTotal;
        int creationServerOptionVisible;
        int creationServerOptionEnabled;
        int creationServerOptionClickSupported;
        int creationServerOptionExpectedLabelCount;
        boolean creationServerOptionExpectedLabel;
        boolean creationSheetAndOptionDifferentWindows;
        String snippetState;
        String snippetId;
        String snippetDraft;
        String snippetSaveState;
        boolean homeObserved;
        boolean productForeground;
        boolean sessionListPreferencesObserved;
        boolean sessionListGroupByStatus;
        boolean sessionListWorkspaceContext;
        boolean sessionListHostContext;
        Observation(int windowId, boolean hosts, boolean sessions, boolean conversation, boolean appearanceSystem, String selectedAppearance, boolean newSession, boolean inventoryRefresh, boolean connectedDirect, boolean connectedRelay, boolean offline, boolean creationName, boolean creationNameEmpty, boolean creationDetailsShown, boolean creationActionAvailable, String selectedProvider, String creationBlocker, String receiptId, String receiptState, Map<String, ReceiptEvidence> receipts, boolean terminalDialog, String terminalState, boolean terminalCloseEnabled, boolean worktreesDialog, boolean changesDialog) {
            this.windowId = windowId;
            this.hosts = hosts;
            this.sessions = sessions;
            this.conversation = conversation;
            this.appearanceSystem = appearanceSystem;
            this.selectedAppearance = selectedAppearance;
            this.newSession = newSession;
            this.inventoryRefresh = inventoryRefresh;
            this.connectedDirect = connectedDirect;
            this.connectedRelay = connectedRelay;
            this.offline = offline;
            this.creationName = creationName;
            this.creationNameEmpty = creationNameEmpty;
            this.creationDetailsShown = creationDetailsShown;
            this.creationActionAvailable = creationActionAvailable;
            this.selectedProvider = selectedProvider;
            this.creationBlocker = creationBlocker;
            this.receiptId = receiptId;
            this.receiptState = receiptState;
            this.receipts = receipts;
            this.terminalDialog = terminalDialog;
            this.terminalState = terminalState;
            this.terminalCloseEnabled = terminalCloseEnabled;
            this.worktreesDialog = worktreesDialog;
            this.changesDialog = changesDialog;
            this.creationServerSelected = false;
            this.creationProjectSelected = false;
            this.creationPickerTraversalComplete = false;
            this.creationPickerProductWindowCount = 0;
            this.creationCloseTotal = 0;
            this.creationCloseVisible = 0;
            this.creationCloseEnabled = 0;
            this.creationServerTotal = 0;
            this.creationServerVisible = 0;
            this.creationServerOptionTotal = 0;
            this.creationServerOptionVisible = 0;
            this.creationServerOptionEnabled = 0;
            this.creationServerOptionClickSupported = 0;
            this.creationServerOptionExpectedLabelCount = 0;
            this.creationServerOptionExpectedLabel = false;
            this.creationSheetAndOptionDifferentWindows = false;
            this.snippetState = "unavailable";
            this.snippetId = null;
            this.snippetDraft = "unavailable";
            this.snippetSaveState = "unavailable";
            this.homeObserved = false;
            this.productForeground = false;
            this.sessionListPreferencesObserved = false;
            this.sessionListGroupByStatus = false;
            this.sessionListWorkspaceContext = false;
            this.sessionListHostContext = false;
        }
        static Observation empty() { return new Observation(NO_WINDOW, false, false, false, false, "unavailable", false, false, false, false, false, false, false, false, false, "none", "none", null, "none", new HashMap<String, ReceiptEvidence>(), false, "hidden", false, false, false); }
        Observation withReceipt(String receiptId, String receiptState) { Observation result = new Observation(windowId, hosts, sessions, conversation, appearanceSystem, selectedAppearance, newSession, inventoryRefresh, connectedDirect, connectedRelay, offline, creationName, creationNameEmpty, creationDetailsShown, creationActionAvailable, selectedProvider, creationBlocker, receiptId, receiptState, receipts, terminalDialog, terminalState, terminalCloseEnabled, worktreesDialog, changesDialog); result.hostSettingsPanel=hostSettingsPanel; result.creationServerSelected=creationServerSelected; result.creationProjectSelected=creationProjectSelected; result.creationPickerTraversalComplete=creationPickerTraversalComplete; result.creationPickerProductWindowCount=creationPickerProductWindowCount; result.creationCloseTotal=creationCloseTotal; result.creationCloseVisible=creationCloseVisible; result.creationCloseEnabled=creationCloseEnabled; result.creationServerTotal=creationServerTotal; result.creationServerVisible=creationServerVisible; result.creationServerOptionTotal=creationServerOptionTotal; result.creationServerOptionVisible=creationServerOptionVisible; result.creationServerOptionEnabled=creationServerOptionEnabled; result.creationServerOptionClickSupported=creationServerOptionClickSupported; result.creationServerOptionExpectedLabelCount=creationServerOptionExpectedLabelCount; result.creationServerOptionExpectedLabel=creationServerOptionExpectedLabel; result.creationSheetAndOptionDifferentWindows=creationSheetAndOptionDifferentWindows; result.snippetState=snippetState; result.snippetId=snippetId; result.snippetDraft=snippetDraft; result.snippetSaveState=snippetSaveState; result.homeObserved=homeObserved; result.productForeground=productForeground; result.sessionListPreferencesObserved=sessionListPreferencesObserved; result.sessionListGroupByStatus=sessionListGroupByStatus; result.sessionListWorkspaceContext=sessionListWorkspaceContext; result.sessionListHostContext=sessionListHostContext; return result; }
        Observation withHomeObservation(boolean homeObserved, boolean productForeground) { this.homeObserved = homeObserved; this.productForeground = productForeground; return this; }
        Observation withSessionListPreferences(boolean groupByStatus, boolean workspaceContext, boolean hostContext) { this.sessionListPreferencesObserved = true; this.sessionListGroupByStatus = groupByStatus; this.sessionListWorkspaceContext = workspaceContext; this.sessionListHostContext = hostContext; return this; }
        int visibleTagCount(String tag) {
            if (HOSTS_TAG.equals(tag)) return hosts ? 1 : 0;
            if (SESSIONS_TAG.equals(tag)) return sessions ? 1 : 0;
            if (CONVERSATION_LIST_TAG.equals(tag)) return conversation ? 1 : 0;
            if (APPEARANCE_SYSTEM_TAG.equals(tag)) return appearanceSystem ? 1 : 0;
            if (HOST_SETTINGS_PANEL_TAG.equals(tag)) return hostSettingsPanel ? 1 : 0;
            if (NEW_SESSION_CLOSE_TAG.equals(tag)) return newSession ? 1 : 0;
            if (INVENTORY_REFRESH_TAG.equals(tag)) return inventoryRefresh ? 1 : 0;
            return 0;
        }
    }

    private static final class Report {
        final String runId;
        final Action action;
        final String outcome;
        final String guard;
        final Stage stage;
        final ErrorFamily errorFamily;
        final boolean acknowledged;
        final boolean expectedScreenObserved;
        final Observation observation;
        final RunnerIdentity self;
        Report(String runId, Action action, String outcome, String guard, Stage stage, ErrorFamily errorFamily, boolean acknowledged, boolean expectedScreenObserved, Observation observation, RunnerIdentity self) {
            this.runId = runId;
            this.action = action;
            this.outcome = outcome;
            this.guard = guard;
            this.stage = stage;
            this.errorFamily = errorFamily;
            this.acknowledged = acknowledged;
            this.expectedScreenObserved = expectedScreenObserved;
            this.observation = observation;
            this.self = self;
        }
        static Report empty(RunnerIdentity self, String guard, Stage stage, ErrorFamily errorFamily) {
            return new Report(null, null, "blocked", guard, stage, errorFamily, false, false, Observation.empty(), self);
        }
        String json() {
            String run = runId == null ? "null" : "\"" + runId + "\"";
            String actionValue = action == null ? "null" : "\"" + action.wire + "\"";
            String receipt = observation.receiptId == null ? "null" : "\"" + observation.receiptId + "\"";
            String snippet = observation.snippetId == null ? "null" : "\"" + observation.snippetId + "\"";
            return "{\"runId\":" + run + ",\"action\":" + actionValue + ",\"outcome\":\"" + outcome + "\",\"guard\":\"" + guard + "\",\"stage\":\"" + stage.wire + "\",\"errorFamily\":\"" + errorFamily.wire + "\",\"acknowledged\":" + acknowledged + ",\"expectedScreenObserved\":" + expectedScreenObserved + ",\"selectedAppearance\":\"" + observation.selectedAppearance + "\",\"snippetState\":\"" + observation.snippetState + "\",\"snippetId\":" + snippet + ",\"snippetDraft\":\"" + observation.snippetDraft + "\",\"snippetSaveState\":\"" + observation.snippetSaveState + "\",\"hosts\":" + observation.hosts + ",\"sessions\":" + observation.sessions + ",\"newSession\":" + observation.newSession + ",\"connectedDirect\":" + observation.connectedDirect + ",\"connectedRelay\":" + observation.connectedRelay + ",\"offline\":" + observation.offline + ",\"creationName\":" + observation.creationName + ",\"creationNameEmpty\":" + observation.creationNameEmpty + ",\"creationDetailsShown\":" + observation.creationDetailsShown + ",\"creationActionAvailable\":" + observation.creationActionAvailable + ",\"creationServerSelected\":" + observation.creationServerSelected + ",\"creationProjectSelected\":" + observation.creationProjectSelected + ",\"creationPickerTraversalComplete\":" + observation.creationPickerTraversalComplete + ",\"creationPickerProductWindowCount\":" + observation.creationPickerProductWindowCount + ",\"creationCloseTotal\":" + observation.creationCloseTotal + ",\"creationCloseVisible\":" + observation.creationCloseVisible + ",\"creationCloseEnabled\":" + observation.creationCloseEnabled + ",\"creationServerTotal\":" + observation.creationServerTotal + ",\"creationServerVisible\":" + observation.creationServerVisible + ",\"creationServerOptionTotal\":" + observation.creationServerOptionTotal + ",\"creationServerOptionVisible\":" + observation.creationServerOptionVisible + ",\"creationServerOptionEnabled\":" + observation.creationServerOptionEnabled + ",\"creationServerOptionClickSupported\":" + observation.creationServerOptionClickSupported + ",\"creationServerOptionExpectedLabelCount\":" + observation.creationServerOptionExpectedLabelCount + ",\"creationServerOptionExpectedLabel\":" + observation.creationServerOptionExpectedLabel + ",\"creationSheetAndOptionDifferentWindows\":" + observation.creationSheetAndOptionDifferentWindows + ",\"selectedProvider\":\"" + observation.selectedProvider + "\",\"creationBlocker\":\"" + observation.creationBlocker + "\",\"receiptId\":" + receipt + ",\"receiptState\":\"" + observation.receiptState + "\",\"terminalDialog\":" + observation.terminalDialog + ",\"terminalState\":\"" + observation.terminalState + "\",\"terminalCloseEnabled\":" + observation.terminalCloseEnabled + ",\"worktreesDialog\":" + observation.worktreesDialog + ",\"changesDialog\":" + observation.changesDialog + ",\"homeObserved\":" + observation.homeObserved + ",\"productForeground\":" + observation.productForeground + ",\"sessionListPreferencesObserved\":" + observation.sessionListPreferencesObserved + ",\"sessionListGroupByStatus\":" + observation.sessionListGroupByStatus + ",\"sessionListWorkspaceContext\":" + observation.sessionListWorkspaceContext + ",\"sessionListHostContext\":" + observation.sessionListHostContext + ",\"hostRouteScope\":" + observation.hostRouteScope + ",\"routeDirectOwned\":" + observation.routeDirectOwned + ",\"routeSaved\":" + observation.routeSaved + ",\"routeReconnectAttempted\":" + observation.routeReconnectAttempted + ",\"runnerPid\":" + self.pid + ",\"runnerStart\":" + self.start + "}";
        }
    }

    private static final class Blocked extends RuntimeException {
        final String code;
        Blocked(String code) { super(code); this.code = code; }
    }

    private static final class InteractiveWindowsService {
        final AccessibilityServiceInfo serviceInfo;
        final int originalFlags;
        InteractiveWindowsService(AccessibilityServiceInfo serviceInfo, int originalFlags) {
            this.serviceInfo = serviceInfo;
            this.originalFlags = originalFlags;
        }
    }

    private enum Action {
        OBSERVE("Observe", null, null),
        OBSERVE_APPEARANCE("ObserveAppearance", null, null),
        OBSERVE_SESSION_LIST_PREFERENCES("ObserveSessionListPreferences", null, null),
        OPEN_SESSION_LIST_SETTINGS("OpenSessionListSettings", NAV_SETTINGS_TAG, APPEARANCE_SYSTEM_TAG),
        SET_SESSION_LIST_PREFERENCE("SetSessionListPreference", null, null),
        BACKGROUND_OWNED_APP("BackgroundOwnedApp", null, null),
        OBSERVE_CREATION_PICKER("ObserveCreationPicker", null, null),
        SELECT_APPEARANCE("SelectAppearance", null, null),
        NAV_SESSIONS("NavSessions", NAV_SESSIONS_TAG, SESSIONS_TAG),
        NAV_HOSTS("NavHosts", NAV_HOSTS_TAG, HOSTS_TAG),
        RECONNECT_HOST("ReconnectHost", "host_reconnect", HOSTS_TAG),
        OPEN_OWNED_HOST_ROUTES("OpenOwnedHostRoutes", null, null),
        ADD_OWNED_ROUTE_DIRECT("AddOwnedRouteDirect", null, null),
        SET_OWNED_ROUTE_DIRECT("SetOwnedRouteDirect", null, null),
        SAVE_OWNED_ROUTE_HINTS("SaveOwnedRouteHints", null, null),
        RECONNECT_OWNED_ROUTE_HOST("ReconnectOwnedRouteHost", null, null),
        OBSERVE_OWNED_HOST_ROUTES("ObserveOwnedHostRoutes", null, null),
        OPEN_HOST_SETTINGS("OpenHostSettings", HOST_SETTINGS_TAG, HOST_SETTINGS_PANEL_TAG),
        TOGGLE_HOST_DIAGNOSTICS("ToggleHostDiagnostics", HOST_DIAGNOSTICS_TAG, HOST_SETTINGS_PANEL_TAG),
        SCROLL_HOST_SETTINGS("ScrollHostSettings", null, null),
        SCROLL_SESSIONS("ScrollSessions", null, null),
        VIEW_SESSIONS("ViewSessions", HOST_SESSIONS_TAG, INVENTORY_REFRESH_TAG),
        REFRESH_SESSIONS("RefreshSessions", INVENTORY_REFRESH_TAG, INVENTORY_REFRESH_TAG),
        OPEN_NEW_SESSION("OpenNewSession", NEW_SESSION_TAG, NEW_SESSION_CLOSE_TAG),
        CLOSE_NEW_SESSION("CloseNewSession", NEW_SESSION_CLOSE_TAG, INVENTORY_REFRESH_TAG),
        SELECT_OWNED_CREATION_SERVER("SelectOwnedCreationServer", null, null),
        SELECT_OWNED_CREATION_PROJECT("SelectOwnedCreationProject", null, null),
        CONTINUE_OWNED_CREATION_SERVER("ContinueOwnedCreationServer", null, null),
        CONTINUE_OWNED_CREATION_PROJECT("ContinueOwnedCreationProject", null, null),
        SET_CREATION_NAME("SetCreationName", null, null),
        SUBMIT_CREATION("SubmitCreation", null, null),
        SET_COMPOSER("SetComposer", null, null),
        SUBMIT_PROMPT("SubmitPrompt", null, null),
        SELECT_CODEX_PROVIDER("SelectCodexProvider", null, null),
        OPEN_SESSION("OpenSession", null, null),
        OPEN_HISTORY_DETAILS("OpenHistoryDetails", null, null),
        CLOSE_HISTORY_DETAILS("CloseHistoryDetails", null, null),
        OPEN_SNIPPETS("OpenSnippets", null, null),
        CLOSE_SNIPPETS("CloseSnippets", null, null),
        OPEN_SWITCHER("OpenSwitcher", null, null),
        CLOSE_SWITCHER("CloseSwitcher", null, null),
        REFRESH_CONVERSATION("RefreshConversation", null, null),
        SCROLL_HISTORY_BACKWARD("ScrollHistoryBackward", null, null),
        FOCUS_COMPOSER("FocusComposer", null, null),
        DISMISS_COMPOSER_KEYBOARD("DismissComposerKeyboard", null, null),
        OPEN_TERMINAL("OpenTerminal", null, null),
        CLOSE_TERMINAL("CloseTerminal", null, null),
        OBSERVE_TERMINAL("ObserveTerminal", null, null),
        OPEN_WORKTREES("OpenWorktrees", null, null),
        REFRESH_WORKTREES("RefreshWorktrees", null, null),
        CLOSE_WORKTREES("CloseWorktrees", null, null),
        OBSERVE_WORKTREES("ObserveWorktrees", null, null),
        OPEN_CHANGES("OpenChanges", null, null),
        REFRESH_CHANGES("RefreshChanges", null, null),
        CLOSE_CHANGES("CloseChanges", null, null),
        OBSERVE_CHANGES("ObserveChanges", null, null),
        NEW_SNIPPET("NewSnippet", null, null),
        SET_SNIPPET_TITLE("SetSnippetTitle", null, null),
        SET_SNIPPET_BODY("SetSnippetBody", null, null),
        SAVE_SNIPPET("SaveSnippet", null, null),
        EDIT_SNIPPET("EditSnippet", null, null),
        INSERT_SNIPPET("InsertSnippet", null, null),
        OPEN_SNIPPET_DELETE("OpenSnippetDelete", null, null),
        CONFIRM_SNIPPET_DELETE("ConfirmSnippetDelete", null, null),
        OBSERVE_SNIPPET("ObserveSnippet", null, null),
        OBSERVE_SNIPPET_DRAFT("ObserveSnippetDraft", null, null),
        CLEAR_OWNED_SNIPPET_DRAFT("ClearOwnedSnippetDraft", null, null),
        CLOSE_SNIPPET_EDITOR("CloseSnippetEditor", null, null),
        INSPECT_RECEIPT("InspectReceipt", null, null),
        OPEN_RECEIPT_DETAILS("OpenReceiptDetails", null, null),
        PREPARE_SEPARATE_CREATION("PrepareSeparateCreation", null, null),
        CONFIRM_SEPARATE_CREATION("ConfirmSeparateCreation", null, null),
        OPEN_SHELLS("OpenShells",null,null),OPEN_SHELL("OpenShell",null,null),
        BEGIN_SHELL_CONTROL("BeginShellControl",null,null),CONFIRM_SHELL_CONTROL("ConfirmShellControl",null,null),
        SET_SHELL_DRAFT("SetShellDraft",null,null),SEND_SHELL_DRAFT("SendShellDraft",null,null),SEND_SHELL_KEY("SendShellKey",null,null),
        OPEN_SHELL_RESIZE("OpenShellResize",null,null),SET_SHELL_SIZE("SetShellSize",null,null),CONFIRM_SHELL_RESIZE("ConfirmShellResize",null,null),
        SCROLL_SHELL_HISTORY("ScrollShellHistory",null,null),SHELL_HISTORY_BOTTOM("ShellHistoryBottom",null,null),
        RELEASE_SHELL_CONTROL("ReleaseShellControl",null,null),OBSERVE_SHELL("ObserveShell",null,null),
        OPEN_TERMINAL_CREATION("OpenTerminalCreation",null,null),SELECT_TERMINAL_PROJECT("SelectTerminalProject",null,null),
        SUBMIT_TERMINAL_CREATION("SubmitTerminalCreation",null,null),STOP_SESSION("StopSession",null,null),
        SELECT_REQUEST("SelectRequest",null,null),SUBMIT_REQUEST_ANSWER("SubmitRequestAnswer",null,null),
        SELECT_DELIVERY_MODE("SelectDeliveryMode",null,null),ENABLE_ATTENTION("EnableAttention",null,null),
        ENABLE_COMPLETED("EnableCompleted",null,null),START_DELIVERY("StartDelivery",null,null),
        TAP_ATTENTION_NOTIFICATION("TapAttentionNotification",null,null),STOP_DELIVERY("StopDelivery",null,null),
        OPEN_ATTENTION_SHADE("OpenAttentionShade",null,null),BACKGROUND_PRIVATE_SETTINGS("BackgroundPrivateSettings",null,null),
        OPEN_SHELL_DETAILS("OpenShellDetails",null,null),SELECT_TERMINAL_WORKSPACE("SelectTerminalWorkspace",null,null),
        SET_SHELL_MODIFIER("SetShellModifier",null,null);
        final String wire;
        final String tag;
        final String expectedTag;
        Action(String wire, String tag, String expectedTag) { this.wire = wire; this.tag = tag; this.expectedTag = expectedTag; }
        static Action fromWire(String wire) {
            if (wire == null) return null;
            for (Action action : values()) if (action.wire.equals(wire)) return action;
            return null;
        }
        boolean isRouteAction() {return this==OPEN_OWNED_HOST_ROUTES || this==ADD_OWNED_ROUTE_DIRECT || this==SET_OWNED_ROUTE_DIRECT || this==SAVE_OWNED_ROUTE_HINTS || this==RECONNECT_OWNED_ROUTE_HOST || this==OBSERVE_OWNED_HOST_ROUTES;}
        boolean isPhysicalAction() {return ordinal()>=OPEN_SHELLS.ordinal();}
        boolean physicalAgentAction() {return this==STOP_SESSION||this==SELECT_REQUEST||this==SUBMIT_REQUEST_ANSWER||this==TAP_ATTENTION_NOTIFICATION;}
        boolean shellScoped() {return isPhysicalAction()&&ordinal()>=OPEN_SHELL.ordinal()&&ordinal()<=OBSERVE_SHELL.ordinal()||this==OPEN_SHELL_DETAILS||this==SET_SHELL_MODIFIER;}
        boolean physicalHostNavigation() {return this==OPEN_HOST_SETTINGS||this==VIEW_SESSIONS;}
        boolean physicalLegacyAllowed() {return Arrays.asList(OBSERVE,NAV_SESSIONS,NAV_HOSTS,OPEN_HOST_SETTINGS,VIEW_SESSIONS,REFRESH_SESSIONS,OPEN_NEW_SESSION,CLOSE_NEW_SESSION,OBSERVE_CREATION_PICKER,SELECT_OWNED_CREATION_SERVER,SELECT_OWNED_CREATION_PROJECT,CONTINUE_OWNED_CREATION_SERVER,CONTINUE_OWNED_CREATION_PROJECT,SET_CREATION_NAME,SUBMIT_CREATION,SELECT_CODEX_PROVIDER,OPEN_SESSION,SET_COMPOSER,SUBMIT_PROMPT,FOCUS_COMPOSER,DISMISS_COMPOSER_KEYBOARD,OPEN_TERMINAL,OBSERVE_TERMINAL,CLOSE_TERMINAL,BACKGROUND_OWNED_APP).contains(this);}
        boolean requiresCreationName() {
            return this == SET_CREATION_NAME || this == SUBMIT_CREATION;
        }
        boolean requiresCreationProjectLabel() {
            return this == SELECT_OWNED_CREATION_PROJECT || this == CONTINUE_OWNED_CREATION_PROJECT;
        }
        boolean allowsOwnedCreationForm() {
            return this == SUBMIT_CREATION;
        }
        boolean allowsCreationServerId() {
            return this == SELECT_OWNED_CREATION_SERVER || this == CONTINUE_OWNED_CREATION_SERVER || this == SUBMIT_CREATION;
        }
        boolean requiresAppearance() {
            return this == SELECT_APPEARANCE;
        }
        boolean requiresSessionId() {
            if(physicalAgentAction())return true;
            return this == OPEN_SESSION || this == BACKGROUND_OWNED_APP || this == SET_COMPOSER || this == SUBMIT_PROMPT || this == OPEN_HISTORY_DETAILS || this == CLOSE_HISTORY_DETAILS || this == OPEN_SNIPPETS || this == CLOSE_SNIPPETS || this == OPEN_SWITCHER || this == CLOSE_SWITCHER || this == REFRESH_CONVERSATION || this == SCROLL_HISTORY_BACKWARD || this == FOCUS_COMPOSER || this == DISMISS_COMPOSER_KEYBOARD || this == OPEN_TERMINAL || this == CLOSE_TERMINAL || this == OBSERVE_TERMINAL || this == OPEN_WORKTREES || this == REFRESH_WORKTREES || this == CLOSE_WORKTREES || this == OBSERVE_WORKTREES || this == OPEN_CHANGES || this == REFRESH_CHANGES || this == CLOSE_CHANGES || this == OBSERVE_CHANGES || isSnippetAction();
        }
        boolean requiresSessionGeneration() {
            if(physicalAgentAction())return true;
            return this == OPEN_SESSION || this == BACKGROUND_OWNED_APP || this == SET_COMPOSER || this == SUBMIT_PROMPT || this == OPEN_HISTORY_DETAILS || this == CLOSE_HISTORY_DETAILS || this == OPEN_SNIPPETS || this == CLOSE_SNIPPETS || this == OPEN_SWITCHER || this == CLOSE_SWITCHER || this == REFRESH_CONVERSATION || this == SCROLL_HISTORY_BACKWARD || this == FOCUS_COMPOSER || this == DISMISS_COMPOSER_KEYBOARD || this == OPEN_TERMINAL || this == CLOSE_TERMINAL || this == OBSERVE_TERMINAL || this == OPEN_WORKTREES || this == REFRESH_WORKTREES || this == CLOSE_WORKTREES || this == OBSERVE_WORKTREES || this == OPEN_CHANGES || this == REFRESH_CHANGES || this == CLOSE_CHANGES || this == OBSERVE_CHANGES || isSnippetAction();
        }
        boolean requiresHistoryRecordId() {
            return this == OPEN_HISTORY_DETAILS || this == CLOSE_HISTORY_DETAILS;
        }
        boolean requiresSessionListPreference() { return this == SET_SESSION_LIST_PREFERENCE; }
        boolean requiresComposerText() {
            return this == SET_COMPOSER || this == SUBMIT_PROMPT;
        }
        boolean requiresReceiptId() {
            return this == INSPECT_RECEIPT || this == OPEN_RECEIPT_DETAILS || this == PREPARE_SEPARATE_CREATION || this == CONFIRM_SEPARATE_CREATION;
        }
        boolean isSnippetAction() { return ordinal() >= NEW_SNIPPET.ordinal() && ordinal() <= CLOSE_SNIPPET_EDITOR.ordinal(); }
        boolean requiresSnippetId() { return isSnippetAction() && this != NEW_SNIPPET; }
        boolean requiresSnippetVersion() { return this == SET_SNIPPET_TITLE || this == SET_SNIPPET_BODY; }
    }

    private enum SnippetVersion {
        ORIGINAL("Original"), EDITED("Edited");
        final String wire;
        SnippetVersion(String wire) { this.wire = wire; }
        static SnippetVersion fromWire(String wire) { for (SnippetVersion value : values()) if (value.wire.equals(wire)) return value; return null; }
    }

    private enum UiAppearance {
        SYSTEM("System", "appearance_System"),
        LIGHT("Light", "appearance_Light"),
        DARK("Dark", "appearance_Dark");
        final String wire;
        final String tag;
        UiAppearance(String wire, String tag) { this.wire = wire; this.tag = tag; }
        static UiAppearance fromWire(String wire) {
            if (wire == null) return null;
            for (UiAppearance appearance : values()) if (appearance.wire.equals(wire)) return appearance;
            return null;
        }
        static UiAppearance fromTag(String tag) {
            for (UiAppearance appearance : values()) if (appearance.tag.equals(tag)) return appearance;
            return null;
        }
    }

    private enum SessionListPreference {
        GROUP_BY_STATUS("groupByStatus", SESSION_LIST_GROUP_BY_STATUS_TAG),
        WORKSPACE_CONTEXT("workspaceContext", SESSION_LIST_WORKSPACE_CONTEXT_TAG),
        HOST_CONTEXT("hostContext", SESSION_LIST_HOST_CONTEXT_TAG);
        final String wire;
        final String tag;
        SessionListPreference(String wire, String tag) { this.wire = wire; this.tag = tag; }
        static SessionListPreference fromWire(String wire) {
            if (wire == null) return null;
            for (SessionListPreference preference : values()) if (preference.wire.equals(wire)) return preference;
            return null;
        }
    }

    private enum Stage {
        PREFLIGHT("preflight"),
        INTENT("intent"),
        AUTOMATION("automation"),
        OBSERVATION("observation"),
        SELECTION("selection"),
        CLICK("click"),
        POSTCONDITION("postcondition"),
        OUTCOME("outcome");
        final String wire;
        Stage(String wire) { this.wire = wire; }
    }

    private enum ErrorFamily {
        NONE("none"),
        SECURITY("security"),
        IO("io"),
        LINKAGE("linkage"),
        ILLEGAL_STATE("illegalstate"),
        NULL_VALUE("null"),
        UNSUPPORTED("unsupported"),
        OTHER("other");
        final String wire;
        ErrorFamily(String wire) { this.wire = wire; }
    }

    private static final String ROUTE_HOST_ID_ARGUMENT = "ownedUiRouteHostId";
    private static final String ROUTE_DIRECT_ARGUMENT = "ownedUiRouteDirect";
    private static final String RUN_ID_ARGUMENT = "ownedUiRunId";
    private static final String ACTION_ARGUMENT = "ownedUiAction";
    private static final String APPEARANCE_ARGUMENT = "ownedUiAppearance";
    private static final String SESSION_LIST_PREFERENCE_ARGUMENT = "ownedUiSessionListPreference";
    private static final String SESSION_LIST_PREFERENCE_VALUE_ARGUMENT = "ownedUiSessionListPreferenceValue";
    private static final String CREATION_SERVER_LABEL_ARGUMENT = "ownedUiCreationServerLabel";
    private static final String CREATION_SERVER_ID_ARGUMENT = "ownedUiCreationServerId";
    private static final String CREATION_PROJECT_LABEL_ARGUMENT = "ownedUiCreationProjectLabel";
    private static final String CREATION_NAME_ARGUMENT = "ownedUiCreationName";
    private static final String PREVIOUS_CREATION_NAME_ARGUMENT = "ownedUiPreviousCreationName";
    private static final String COMPOSER_TEXT_ARGUMENT = "ownedUiComposerText";
    private static final String SESSION_ID_ARGUMENT = "ownedUiSessionId";
    private static final String SESSION_GENERATION_ARGUMENT = "ownedUiSessionGeneration";
    private static final String HISTORY_RECORD_ID_ARGUMENT = "ownedUiHistoryRecordId";
    private static final String RECEIPT_ID_ARGUMENT = "ownedUiReceiptId";
    private static final String EXCLUDED_RECEIPT_IDS_ARGUMENT = "ownedUiExcludedReceiptIds";
    private static final String SNIPPET_WORKFLOW_ID_ARGUMENT = "ownedUiSnippetWorkflowId";
    private static final String SNIPPET_ID_ARGUMENT = "ownedUiSnippetId";
    private static final String SNIPPET_VERSION_ARGUMENT = "ownedUiSnippetVersion";
    private static final String STATUS_KEY = "goatrOwnedUiAutomation";
    private static final String PROCESS_STATUS_KEY = "goatrOwnedUiProcess";
    private static final String PRODUCT_PACKAGE = "io.github.trevarj.goatr.debug";
    private static final String TEST_PACKAGE = "io.github.trevarj.goatr.debug.test";
    private static final int MIN_API = 26;
    private static final int MAX_API = 36;
    private static final long TIMEOUT_MILLIS = 20_000L;
    private static final long PRE_INPUT_RELEASE_TIMEOUT_MILLIS = 150_000L;
    private static final long POLL_MILLIS = 100L;
    private static final int MAX_NODES = 512;
    private static final int MAX_EXCLUDED_RECEIPTS = 100;
    private static final int MAX_RECEIPT_ANCESTORS = 32;
    private static final int NO_WINDOW = -1;
    private static final String NAV_SESSIONS_TAG = "navigation_sessions_menu";
    private static final String NAV_HOSTS_TAG = "navigation_hosts_menu";
    private static final String NAV_SETTINGS_TAG = "navigation_settings_menu";
    private static final String SESSION_LIST_GROUP_BY_STATUS_TAG = "settings_session_list_group_by_status";
    private static final String SESSION_LIST_WORKSPACE_CONTEXT_TAG = "settings_session_list_workspace_context";
    private static final String SESSION_LIST_HOST_CONTEXT_TAG = "settings_session_list_host_context";
    private static final String HOST_SESSIONS_TAG = "host_sessions";
    private static final String INVENTORY_REFRESH_TAG = "inventory_refresh";
    private static final String NEW_SESSION_TAG = "new_session";
    private static final String NEW_SESSION_CLOSE_TAG = "new_session_close";
    private static final String HOSTS_TAG = "connections_list";
    private static final String SESSIONS_TAG = "sessions_console";
    private static final String HOST_SETTINGS_TAG = "host_settings";
    private static final String HOST_DIAGNOSTICS_TAG = "host_diagnostics";
    private static final String APPEARANCE_SYSTEM_TAG = "appearance_System";
    private static final String HOST_SETTINGS_PANEL_TAG = "settings_overview";
    private static final String CONVERSATION_LIST_TAG = "conversation_list";
    private static final String CONVERSATION_TARGET_TAG_PREFIX = "conversation_target_";
    private static final String SESSION_TAG_PREFIX = "session_";
    private static final String HISTORY_RECORD_TAG_PREFIX = "history_record_";
    private static final String HISTORY_DETAILS_TAG_PREFIX = "history_details_";
    private static final String ORIGINAL_EXPAND_TAG = "original_expand";
    private static final String COMPOSER_TAG = "composer";
    private static final String PROMPT_SEND_TAG = "prompt_send";
    private static final String COMPOSER_SNIPPETS_TAG = "composer_snippets";
    private static final String CONVERSATION_INVENTORY_REFRESH_TAG = "conversation_inventory_refresh";
    private static final String CONVERSATION_REFRESH_TAG = "conversation_refresh";
    private static final String SWITCHER_TAG = "switcher";
    private static final String SNIPPET_SCOPE_TAG_PREFIX = "snippet_scope_";
    private static final String SWITCHER_SCOPE_TAG_PREFIX = "switcher_scope_";
    private static final String SNIPPET_CLOSE_TAG = "snippet_close";
    private static final String SNIPPET_NEW_TAG = "snippet_new";
    private static final String SNIPPET_EDITOR_TAG_PREFIX = "snippet_editor_";
    private static final String SNIPPET_TITLE_TAG = "snippet_title";
    private static final String SNIPPET_BODY_TAG = "snippet_body";
    private static final String SNIPPET_SAVE_TAG = "snippet_save";
    private static final String SNIPPET_EDITOR_BACK_TAG = "snippet_editor_back";
    private static final String SNIPPET_ROW_TAG_PREFIX = "snippet_";
    private static final String SNIPPET_INSERT_TAG_PREFIX = "snippet_insert_";
    private static final String SNIPPET_EDIT_TAG_PREFIX = "snippet_edit_";
    private static final String SNIPPET_DELETE_TAG_PREFIX = "snippet_delete_";
    private static final String SNIPPET_DELETE_SCOPE_TAG_PREFIX = "snippet_delete_scope_";
    private static final String SNIPPET_DELETE_TARGET_TAG_PREFIX = "snippet_delete_target_";
    private static final String SNIPPET_DELETE_CONFIRM_TAG = "snippet_delete_confirm";
    private static final String SNIPPET_DRAFT_EMPTY_TAG = "snippet_draft_empty";
    private static final String SNIPPET_DRAFT_PRESENT_TAG = "snippet_draft_present";
    private static final String SWITCHER_CLOSE_TAG = "switcher_close";
    private static final String TERMINAL_OPEN_TAG = "terminal_open";
    private static final String TERMINAL_DIALOG_TAG = "terminal_dialog";
    private static final String TERMINAL_TARGET_TAG_PREFIX = "terminal_target_";
    private static final String TERMINAL_OUTPUT_TAG = "terminal_grid";
    private static final String TERMINAL_CLOSE_TAG = "terminal_close";
    private static final String TERMINAL_LOADING_TAG = "terminal_loading";
    private static final String TERMINAL_ERROR_TAG = "terminal_error";
    private static final String WORKTREES_OPEN_TAG = "worktrees_open";
    private static final String WORKTREES_DIALOG_TAG = "worktrees_dialog";
    private static final String WORKTREES_TARGET_TAG_PREFIX = "worktrees_target_";
    private static final String WORKTREES_CLOSE_TAG = "worktrees_close";
    private static final String WORKTREES_REFRESH_TAG = "worktrees_refresh";
    private static final String WORKTREES_LIST_TAG = "worktrees_list";
    private static final String WORKTREES_ENTRY_TAG = "worktrees_entry";
    private static final String CHANGES_OPEN_TAG = "changes_open";
    private static final String CHANGES_DIALOG_TAG = "changes_dialog";
    private static final String CHANGES_TARGET_TAG_PREFIX = "changes_target_";
    private static final String CHANGES_CLOSE_TAG = "changes_close";
    private static final String CHANGES_REFRESH_TAG = "changes_refresh";
    private static final String CHANGES_BODY_TAG = "changes_body";
    private static final String CHANGES_ENTRY_TAG = "changes_entry";
    private static final String CHANGES_STAGED_PATCH_TAG = "changes_staged_patch";
    private static final String CHANGES_UNSTAGED_PATCH_TAG = "changes_unstaged_patch";
    private static final String CONNECTED_DIRECT = "Connected directly";
    private static final String CONNECTED_RELAY = "Connected through relay";
    private static final String OFFLINE = "Offline";
    private static final String CREATE_NAME_TAG = "create_name";
    private static final String CREATE_PROVIDER_TAG = "create_provider";
    private static final String CREATE_SERVER_TAG = "create_server";
    private static final String CREATE_PROJECT_TAG = "create_project";
    private static final String CREATE_SERVER_OPTION_TAG_PREFIX = "create_server_";
    private static final String CREATE_PROJECT_OPTION_TAG_PREFIX = "create_project_";
    private static final String CREATE_SETUP_DETAILS_TAG = "create_setup_details";
    private static final String CREATE_BLOCKING_REASON_TAG = "create_blocking_reason";
    private static final String SESSION_CREATE_TAG = "session_create";
    private static final String RECEIPT_STATE_TAG = "receipt_state";
    private static final String RECEIPT_INSPECT_TAG = "receipt_inspect";
    private static final String RECEIPT_TAG_PREFIX = "receipt_";
    private static final String RECEIPT_DETAILS_TAG_PREFIX = "receipt_details_";
    private static final String RECEIPT_EXACT_TAG = "receipt_exact";
    private static final String SEPARATE_CREATION_PREPARE_PREFIX = "separate_creation_prepare_";
    private static final String SEPARATE_CREATION_DIALOG_PREFIX = "separate_creation_";
    private static final String SEPARATE_CREATION_CONFIRM_PREFIX = "separate_creation_confirm_";
    private static final String ACTION_DETAILS = "Action details";
    private static final String HISTORY_DETAILS = "Details";
    private static final String PROVIDER_OMP = "OMP";
    private static final String PROVIDER_CODEX = "Codex";
    private static final String APPROVED_SERVER = "default";
    private static final String OWNED_CREATION_SERVER = "Configured Herdr";
    private static final String APPROVED_PROJECT = "goatr";
    private static final String HIDE_DETAILS = "Hide details";
    private static final String NEW_CONVERSATION_STATUS = "New conversation status";
    private static final Pattern CREATION_NAME_PATTERN = Pattern.compile("Goatr phone (OMP|Codex) [a-f0-9]{8}");
    private static final Pattern CREATION_PROJECT_LABEL_PATTERN = Pattern.compile("(?:Goatr feature repo [a-f0-9]{8}|goatr)");
    private static final Pattern COMPOSER_TEXT_PATTERN = Pattern.compile("Reply only with GOATR_MOBILE_(OMP|CODEX)_[a-f0-9]{32}");
    private static final Pattern HISTORY_RECORD_ID_PATTERN = Pattern.compile("[A-Za-z0-9._:-]{1,128}");
    private static final Pattern UUID_PATTERN = Pattern.compile("[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}");
    private static final Pattern RECEIPT_ID_ARRAY_PATTERN = Pattern.compile("\\[\\s*(?:\\\"[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}\\\"\\s*(?:,\\s*\\\"[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}\\\"\\s*)*)?\\]");
    private static final Set<String> PAIRING_SCENE_TAGS = new HashSet<>(Arrays.asList("pair_authority", "pair_retry", "pair_cancel"));
}
