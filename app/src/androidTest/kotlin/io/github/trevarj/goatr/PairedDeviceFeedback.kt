// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Activity
import android.app.Application
import android.app.UiAutomation
import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.Process
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import androidx.lifecycle.ViewModelProvider
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import java.io.File
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.StandardOpenOption
import java.security.KeyStore
import java.security.MessageDigest
import java.util.ArrayDeque
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Test

/** One autonomous observation of the existing paired debug installation. */
class PairedDeviceFeedback {
    @Test fun pairedExistingHostReconnectDetails() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val target = instrumentation.targetContext.applicationContext
        val deadline = SystemClock.elapsedRealtime() + DRIVER_TIMEOUT_MILLIS
        var request: FeedbackRequest? = null
        var process = ProcessIdentity(Process.myPid(), 0L)
        var progress: ProgressReporter? = null
        var baseline: PairedSnapshot? = null
        var application: Application? = null
        var lifecycle: MainActivityLifecycle? = null
        var mainHandler: Handler? = null
        var retainedActivity: MainActivity? = null
        var activityOrigin = ActivityOrigin.None
        var callbacksRegistered = false
        var activityCreated = false
        var activityResumed = false
        var launchRequired = false
        var terminalFailure: ConnectionFailure? = null
        var outcome = "blocked"
        var observationCaptured = false
        var driverGuard = DriverGuard.Request
        var signerMatch = false
        var navHostsAck = false
        var hostsDisplayed = false
        var detailsAck = false
        var closed = false
        var closeAttempted = false
        var final: PairedSnapshot? = null
        var blocked = false

        try {
            request = requiredRequest()
            process = processIdentity()
            progress = ProgressReporter(instrumentation, target, request.runId, process)
            progress.emit(PHASE_METHOD_ENTRY)
            requireBeforeDeadline(deadline)
            requireOwnedDebugTarget(instrumentation, target, request.signerSha256)
            signerMatch = true
            require(instrumentation is OwnedInstrumentationRunner) { "feedback_runner_required" }

            progress.emit(PHASE_PREFLIGHT)
            val initialBaseline = preflightExistingPairing(target)
            baseline = initialBaseline
            requireBeforeDeadline(deadline)

            progress.emit(PHASE_LAUNCH)
            val targetApplication = target.applicationContext as? Application ?: throw FeedbackBlocked("target_application_required")
            val feedbackRequest = request ?: throw FeedbackBlocked("request_unavailable")
            application = targetApplication
            val handler = Handler(Looper.getMainLooper())
            mainHandler = handler
            when (val existing = queryResumedMainActivity(handler, targetApplication, process, deadline)) {
                MainActivityLookup.Missing -> {
                    val registeredLifecycle = MainActivityLifecycle(targetApplication, feedbackRequest.runId, process)
                    targetApplication.registerActivityLifecycleCallbacks(registeredLifecycle)
                    callbacksRegistered = true
                    lifecycle = registeredLifecycle
                    // Do not adopt a lifecycle transition between the zero-activity query and arming root's launch.
                    require(queryResumedMainActivity(handler, targetApplication, process, deadline) === MainActivityLookup.Missing) { "main_activity_changed_before_launch" }
                    launchRequired = true
                    emitLaunchArmed(instrumentation, target, feedbackRequest.runId, process, deadline)
                    val original = awaitOriginalActivity(registeredLifecycle, deadline)
                    retainedActivity = original
                    activityOrigin = ActivityOrigin.FreshCreated
                    val owner = awaitOriginalOwner(registeredLifecycle, deadline)
                    requireExactResumedActivity(handler, targetApplication, process, original, owner, deadline)
                    activityCreated = registeredLifecycle.createdObserved.get()
                    activityResumed = registeredLifecycle.resumedObserved.get()
                    progress.emit(PHASE_OWNER)
                    require(awaitOwnerBaseline(owner, initialBaseline, deadline) == initialBaseline) { "owner_store_baseline_mismatch" }
                    observeAndNavigate(progress,instrumentation,target,feedbackRequest,owner,initialBaseline,handler,targetApplication,process,original,deadline,activityOrigin) { failure, observedOutcome ->
                        terminalFailure=failure; outcome=observedOutcome; observationCaptured=true
                    }.also { result ->
                        terminalFailure=result.failure; outcome=result.outcome; signerMatch=true; navHostsAck=result.navHostsAck; hostsDisplayed=result.hostsDisplayed; detailsAck=result.detailsAck
                    }
                }
                is MainActivityLookup.Existing -> {
                    retainedActivity = existing.activity
                    activityOrigin = ActivityOrigin.ExistingResumed
                    activityResumed = true
                    progress.emit(PHASE_OWNER)
                    require(awaitOwnerBaseline(existing.owner, initialBaseline, deadline) == initialBaseline) { "owner_store_baseline_mismatch" }
                    observeAndNavigate(progress,instrumentation,target,feedbackRequest,existing.owner,initialBaseline,handler,targetApplication,process,existing.activity,deadline,activityOrigin) { failure, observedOutcome ->
                        terminalFailure=failure; outcome=observedOutcome; observationCaptured=true
                    }.also { result ->
                        terminalFailure=result.failure; outcome=result.outcome; signerMatch=true; navHostsAck=result.navHostsAck; hostsDisplayed=result.hostsDisplayed; detailsAck=result.detailsAck
                    }
                }
            }
            driverGuard = DriverGuard.None
        } catch (error: Throwable) {
            blocked = true
            if (!observationCaptured) outcome = "blocked"
            driverGuard = classify(error)
        } finally {
            if (activityOrigin == ActivityOrigin.FreshCreated && !closed && !closeAttempted && retainedActivity != null && lifecycle?.original?.get() === retainedActivity) {
                try {
                    requireBeforeDeadline(deadline)
                    closeAttempted = true
                    finishOriginalActivity(lifecycle!!, mainHandler ?: throw FeedbackBlocked("main_handler_required"), deadline)
                    closed = true
                    requireBeforeDeadline(deadline)
                } catch (_: Throwable) {
                    blocked = true
                    if (!observationCaptured) outcome = "blocked"
                    if (driverGuard == DriverGuard.None) driverGuard = DriverGuard.Cleanup
                }
            }
            if (callbacksRegistered) lifecycle?.let { registered -> application?.unregisterActivityLifecycleCallbacks(registered) }
            lifecycle?.let { registered ->
                activityCreated = registered.createdObserved.get()
                activityResumed = registered.resumedObserved.get()
            }
            if (baseline != null) {
                final = runCatching {
                    requireBeforeDeadline(deadline)
                    preflightExistingPairing(target).also { requireBeforeDeadline(deadline) }
                }.getOrNull()
                if (final != baseline) {
                    blocked = true
                    if (!observationCaptured) outcome = "blocked"
                    if (driverGuard == DriverGuard.None) driverGuard = DriverGuard.OwnerState
                }
            }
            runCatching { progress?.emit(PHASE_FINAL) }.onFailure {
                blocked = true
                if (!observationCaptured) outcome = "blocked"
                if (driverGuard == DriverGuard.None) driverGuard = DriverGuard.Progress
            }
            val finalReport = FeedbackReport(
                outcome = outcome,
                driverGuard = driverGuard,
                failure = terminalFailure,
                hostCount = final?.hostCount ?: 0,
                exactHostMatch = baseline != null && final?.hostId == baseline?.hostId,
                uiHosts = hostsDisplayed,
                navHostsAck = navHostsAck,
                uiDetails = detailsAck,
                pairedMetadataUnchanged = baseline != null && final?.metadataDigest == baseline?.metadataDigest,
                keyAliasDigestUnchanged = baseline != null && final?.keyAliasDigest == baseline?.keyAliasDigest,
                endpointAliasPresent = final?.endpointAliasPresent ?: false,
                deviceIdPresent = final?.deviceIdPresent ?: false,
                signerMatch = signerMatch,
                closed = closed,
                callbacksRegistered = callbacksRegistered,
                created = activityCreated,
                resumed = activityResumed,
                activityOrigin = activityOrigin,
                launchRequired = launchRequired,
                pid = process.pid,
                start = process.start,
                capInhZero = process.capInhZero,
                capPrmZero = process.capPrmZero,
                capEffZero = process.capEffZero,
                capAmbZero = process.capAmbZero,
                runId = request?.runId ?: "invalid",
            )
            var reportJson = finalReport.json()
            if (request != null) {
                try {
                    writeExclusive(target.cacheDir, finalOutcomeFile(target, request.runId), reportJson)
                } catch (_: Throwable) {
                    blocked = true
                    reportJson = finalReport.copy(outcome = "blocked", driverGuard = DriverGuard.Report).json()
                }
            }
            instrumentation.sendStatus(1, Bundle().apply { putString("goatrPairedDeviceFeedback", reportJson) })
        }
        if (blocked) throw AssertionError("PairedDeviceFeedback blocked; inspect the sanitized feedback record")
    }

    private fun requiredRequest(): FeedbackRequest {
        val arguments = InstrumentationRegistry.getArguments()
        val forbidden = arguments.keySet().any { name ->
            val normalized = name.lowercase()
            normalized.contains("fixture") || normalized.contains("stockcontrol") ||
                normalized.contains("reviewcontrol") || normalized.contains("expectedhost") || normalized == "hostid"
        }
        require(!forbidden) { "feedback_disallows_fixture_or_control_arguments" }
        val runId = Uuid(arguments.getString(RUN_ID_ARGUMENT) ?: throw FeedbackBlocked("run_id_required")).value
        val signer = arguments.getString(SIGNER_ARGUMENT)?.lowercase() ?: throw FeedbackBlocked("signer_required")
        require(signer.matches(SIGNER_SHA256)) { "signer_hash_required" }
        return FeedbackRequest(runId, signer)
    }

    private fun requireOwnedDebugTarget(instrumentation: android.app.Instrumentation, target: Context, expectedSigner: String) {
        require(target.packageName == TARGET_PACKAGE) { "target_package_required" }
        require(instrumentation.context.packageName == TEST_PACKAGE) { "test_package_required" }
        require(target.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) { "debug_target_required" }
        require(signerDigests(target, target.packageName) == setOf(expectedSigner)) { "target_signer_mismatch" }
        require(signerDigests(target, instrumentation.context.packageName) == setOf(expectedSigner)) { "test_signer_mismatch" }
    }

    @Suppress("DEPRECATION")
    private fun signerDigests(context: Context, packageName: String): Set<String> =
        context.packageManager.getPackageInfo(packageName, PackageManager.GET_SIGNATURES).signatures
            ?.map { signature -> digest(signature.toByteArray()) }
            ?.toSet()
            .orEmpty()

    /** Read the existing private database only; this never opens a SessionStore or creates identity state. */
    private fun preflightExistingPairing(context: Context): PairedSnapshot {
        val database = context.getDatabasePath(DATABASE_NAME)
        require(database.isFile) { "paired_database_required" }
        val metadata = SQLiteDatabase.openDatabase(database.path, null, SQLiteDatabase.OPEN_READONLY).use { db ->
            val hosts = db.rawQuery("SELECT metadata FROM hosts ORDER BY host", null).use { rows ->
                buildList { while (rows.moveToNext()) add(Protocol.strictJson.decodeFromString<HostMetadata>(rows.getString(0))) }
            }
            val deletionCount = db.rawQuery("SELECT count(*) FROM key_deletion", null).use { rows ->
                require(rows.moveToFirst()) { "key_deletion_query_failed" }
                rows.getLong(0)
            }
            require(hosts.size == 1) { "exactly_one_paired_host_required" }
            require(deletionCount == 0L) { "pending_key_deletion_required" }
            hosts.single()
        }
        require(metadata.deviceId != null) { "paired_device_id_required" }
        val aliases = keyAliasSnapshot()
        require(aliases.endpointPresent && aliases.names.containsAll(metadata.keyAliases)) { "paired_key_alias_required" }
        return PairedSnapshot(metadata.hostId, metadataDigest(metadata), aliases.digest, 1, true, true)
    }

    private fun awaitOriginalActivity(lifecycle: MainActivityLifecycle, deadline: Long): MainActivity {
        awaitLifecycle(lifecycle, lifecycle.created, "activity_create_timeout", deadline)
        return lifecycle.original.get() ?: throw FeedbackBlocked("original_activity_missing")
    }

    private fun awaitOriginalOwner(lifecycle: MainActivityLifecycle, deadline: Long): GoatrViewModel {
        awaitLifecycle(lifecycle, lifecycle.resumed, "activity_resume_timeout", deadline)
        return lifecycle.owner.get() ?: throw FeedbackBlocked("owner_unavailable")
    }

    /** Runs the lifecycle monitor query only on the main looper, without an instrumentation idle barrier. */
    private fun queryResumedMainActivity(handler: Handler, targetApplication: Application, process: ProcessIdentity, deadline: Long): MainActivityLookup {
        requireBeforeDeadline(deadline)
        val reply = AtomicReference<MainActivityLookup?>(null)
        val failure = AtomicReference<Throwable?>(null)
        val acknowledged = CountDownLatch(1)
        require(handler.post {
            try {
                requireBeforeDeadline(deadline)
                require(processMatches(process)) { "instrumentation_process_changed" }
                val activities = Stage.values().flatMap { stage ->
                    ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(stage).map { activity -> stage to activity }
                }.filter { (_, activity) -> activity is MainActivity }
                if (activities.isEmpty()) {
                    reply.set(MainActivityLookup.Missing)
                } else {
                    require(activities.size == 1) { "main_activity_ambiguous" }
                    val (stage, activity) = activities.single()
                    val main = activity as MainActivity
                    require(stage == Stage.RESUMED) { "main_activity_not_resumed" }
                    require(main.application === targetApplication && !main.isFinishing && !main.isDestroyed) { "main_activity_not_current" }
                    reply.set(MainActivityLookup.Existing(main, ViewModelProvider(main)[GoatrViewModel::class.java]))
                }
            } catch (error: Throwable) {
                failure.set(error)
            } finally {
                acknowledged.countDown()
            }
        }) { "main_activity_query_post_failed" }
        val remaining = deadline - SystemClock.elapsedRealtime()
        if (!acknowledged.await(remaining, TimeUnit.MILLISECONDS)) throw FeedbackBlocked("main_activity_query_timeout")
        failure.get()?.let { throw it }
        requireBeforeDeadline(deadline)
        return reply.get() ?: throw FeedbackBlocked("main_activity_query_unavailable")
    }

    private fun requireExactResumedActivity(handler: Handler, targetApplication: Application, process: ProcessIdentity, expected: MainActivity, expectedOwner: GoatrViewModel, deadline: Long) {
        val current = queryResumedMainActivity(handler, targetApplication, process, deadline)
        require(current is MainActivityLookup.Existing && current.activity === expected && current.owner === expectedOwner) { "main_activity_replaced" }
    }

    private fun finishOriginalActivity(lifecycle: MainActivityLifecycle, handler: Handler, deadline: Long) {
        val original = lifecycle.original.get() ?: throw FeedbackBlocked("original_activity_missing")
        require(lifecycle.finishRequested.compareAndSet(false, true)) { "activity_finish_repeated" }
        requireBeforeDeadline(deadline)
        require(handler.post {
            if (SystemClock.elapsedRealtime() >= deadline) lifecycle.reject("activity_finish_expired")
            else if (lifecycle.original.get() === original) original.finish()
            else lifecycle.reject("original_activity_replaced")
        }) { "activity_finish_post_failed" }
        awaitLifecycle(lifecycle, lifecycle.destroyed, "activity_destroy_timeout", deadline)
    }

    private fun awaitLifecycle(lifecycle: MainActivityLifecycle, latch: CountDownLatch, timeout: String, deadline: Long) {
        lifecycle.failure.get()?.let { throw FeedbackBlocked(it) }
        requireBeforeDeadline(deadline)
        val remaining = deadline - SystemClock.elapsedRealtime()
        if (!latch.await(remaining, TimeUnit.MILLISECONDS)) throw FeedbackBlocked(timeout)
        lifecycle.failure.get()?.let { throw FeedbackBlocked(it) }
        requireBeforeDeadline(deadline)
    }

    private fun awaitOwnerBaseline(model: GoatrViewModel, baseline: PairedSnapshot, deadline: Long): PairedSnapshot {
        while (SystemClock.elapsedRealtime() < deadline) {
            ownerSnapshotOrNull(model)?.takeIf { it == baseline }?.let { return it }
            SystemClock.sleep(POLL_MILLIS)
        }
        throw FeedbackBlocked("owner_store_load_timeout")
    }

    private fun ownerSnapshot(model: GoatrViewModel): PairedSnapshot {
        val state = model.store.state.value
        require(state.hosts.size == 1 && state.keyDeletion.isEmpty()) { "owner_paired_state_required" }
        val metadata = state.hosts.single()
        require(metadata.deviceId != null) { "owner_device_id_required" }
        val aliases = keyAliasSnapshot()
        require(aliases.endpointPresent && aliases.names.containsAll(metadata.keyAliases)) { "owner_key_alias_required" }
        return PairedSnapshot(metadata.hostId, metadataDigest(metadata), aliases.digest, 1, true, true)
    }

    private fun ownerSnapshotOrNull(model: GoatrViewModel): PairedSnapshot? = runCatching { ownerSnapshot(model) }.getOrNull()

    private fun observeExistingConnection(model: GoatrViewModel, host: String, deadline: Long): ConnectionView {
        val observationDeadline = minOf(deadline - CLOSE_RESERVE_MILLIS, SystemClock.elapsedRealtime() + RECONNECT_TIMEOUT_MILLIS)
        while (SystemClock.elapsedRealtime() < observationDeadline) {
            val view = model.connection.state.value
            if (view.connectionFailures.containsKey(host) || view.hello.containsKey(host) && view.inventory.containsKey(host)) return view
            SystemClock.sleep(POLL_MILLIS)
        }
        throw FeedbackBlocked("connection_observation_timeout")
    }

    private fun observeAndNavigate(
        progress: ProgressReporter,
        instrumentation: android.app.Instrumentation,
        target: Context,
        request: FeedbackRequest,
        model: GoatrViewModel,
        baseline: PairedSnapshot,
        handler: Handler,
        targetApplication: Application,
        process: ProcessIdentity,
        activity: MainActivity,
        deadline: Long,
        origin: ActivityOrigin,
        onObserved: (ConnectionFailure?, String) -> Unit,
    ): DriverObservation {
        progress.emit(PHASE_OBSERVATION)
        val observation = observeExistingConnection(model, baseline.hostId, deadline)
        val failure = observation.connectionFailures[baseline.hostId]
        val outcome = when {
            failure != null && origin == ActivityOrigin.ExistingResumed -> "existing_closed_failure"
            failure != null -> "closed_failure"
            origin == ActivityOrigin.ExistingResumed -> "existing_hello_inventory"
            else -> "hello_inventory"
        }
        onObserved(failure, outcome)
        requireExactResumedActivity(handler, targetApplication, process, activity, model, deadline)
        progress.emit(PHASE_NAVIGATION)
        val automation = instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        val uiOwner = UiOwner(handler, targetApplication, process, activity, model)
        invokeWithExclusiveRecord(automation, target, request.runId, "navigation_menu", deadline, null, uiOwner)
        awaitOwnedVisible(automation, "navigation_hosts_menu", uiOwner, deadline)
        val navHostsAck = invokeWithExclusiveRecord(automation, target, request.runId, "navigation_hosts_menu", deadline, null, uiOwner)
        val hostsDisplayed = awaitOwnedVisible(automation, "connections_list", uiOwner, deadline)
        requireExactResumedActivity(handler, targetApplication, process, activity, model, deadline)
        progress.emit(PHASE_DETAILS)
        val detailsAck = if (failure != null) {
            invokeWithExclusiveRecord(automation, target, request.runId, "connection_failure_details", deadline, failure, uiOwner)
        } else false
        require(ownerSnapshot(model) == baseline) { "owner_store_changed" }
        requireBeforeDeadline(deadline)
        return DriverObservation(failure, outcome, navHostsAck, hostsDisplayed, detailsAck)
    }

    private fun invokeWithExclusiveRecord(automation: UiAutomation, context: Context, runId: String, tag: String, deadline: Long, closedFailure: ConnectionFailure?, uiOwner: UiOwner): Boolean {
        requireBeforeDeadline(deadline)
        writeExclusive(context.cacheDir, actionIntentFile(context, runId, tag), actionIntentJson(runId, tag))
        requireBeforeDeadline(deadline)
        val found = uniqueOwnedNode(automation, tag, uiOwner, deadline)
        val acknowledged = try {
            require(revalidate(automation, found, tag, uiOwner, deadline)) { "ui_node_revalidation_failed" }
            requireBeforeDeadline(deadline)
            found.node.performAction(AccessibilityNodeInfo.ACTION_CLICK)
        } finally {
            found.node.recycle()
        }
        require(acknowledged) { "ui_callback_not_acknowledged" }
        val closedPhaseObserved = closedFailure?.let { failure ->
            awaitAuthoredDiagnostic(automation, expectedDiagnostic(failure.phase), uiOwner, deadline)
        } ?: false
        requireBeforeDeadline(deadline)
        writeExclusive(context.cacheDir, actionOutcomeFile(context, runId, tag), actionOutcomeJson(runId, tag, acknowledged, closedPhaseObserved))
        requireBeforeDeadline(deadline)
        return acknowledged && (closedFailure == null || closedPhaseObserved)
    }

    private fun awaitOwnedVisible(automation: UiAutomation, tag: String, uiOwner: UiOwner, deadline: Long): Boolean {
        while (SystemClock.elapsedRealtime() < deadline) {
            if (ownedVisibleNodeCount(automation, tag, uiOwner, deadline) == 1) return true
            SystemClock.sleep(POLL_MILLIS)
        }
        throw FeedbackBlocked("owned_ui_poll_timeout")
    }

    private fun awaitAuthoredDiagnostic(automation: UiAutomation, expected: String, uiOwner: UiOwner, deadline: Long): Boolean {
        while (SystemClock.elapsedRealtime() < deadline) {
            if (ownedVisibleTextCount(automation, expected, uiOwner, deadline) == 1) return true
            SystemClock.sleep(POLL_MILLIS)
        }
        throw FeedbackBlocked("authored_diagnostic_timeout")
    }

    /** Bounded metadata-only traversal of the active own-package root; no text or content is read. */
    private fun uniqueOwnedNode(automation: UiAutomation, tag: String, uiOwner: UiOwner, deadline: Long): OwnedNode {
        val root = awaitOwnRoot(automation, uiOwner, deadline)
        val rootWindow = root.windowId
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        var retained: AccessibilityNodeInfo? = null
        var complete = false
        var count = 0
        try {
            while (queue.isNotEmpty()) {
                val node = queue.removeFirst()
                if (++count > MAX_NODES) {
                    node.recycle()
                    throw FeedbackBlocked("ui_node_bound")
                }
                val metadata = nodeMetadata(node)
                if (metadata.packageName != TARGET_PACKAGE || metadata.windowId != rootWindow) {
                    node.recycle()
                    throw FeedbackBlocked("ui_root_changed")
                }
                repeat(node.childCount) { index -> node.getChild(index)?.let(queue::addLast) }
                if (metadata.viewId == resourceId(tag)) {
                    if (retained != null) {
                        node.recycle()
                        throw FeedbackBlocked("accessibility_ambiguous_node")
                    }
                    retained = node
                } else node.recycle()
            }
            val node = retained ?: throw FeedbackBlocked("accessibility_node_missing")
            complete = true
            return OwnedNode(node, rootWindow)
        } finally {
            while (queue.isNotEmpty()) queue.removeFirst().recycle()
            if (!complete) retained?.recycle()
        }
    }

    private fun ownedVisibleNodeCount(automation: UiAutomation, tag: String, uiOwner: UiOwner, deadline: Long): Int {
        val root = awaitOwnRoot(automation, uiOwner, deadline)
        val rootWindow = root.windowId
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        var count = 0
        var matches = 0
        try {
            while (queue.isNotEmpty()) {
                val node = queue.removeFirst()
                if (++count > MAX_NODES) {
                    node.recycle()
                    throw FeedbackBlocked("ui_node_bound")
                }
                val metadata = nodeMetadata(node)
                if (metadata.packageName != TARGET_PACKAGE || metadata.windowId != rootWindow) {
                    node.recycle()
                    throw FeedbackBlocked("ui_root_changed")
                }
                if (metadata.viewId == resourceId(tag) && metadata.visible) {
                    matches++
                    if (matches > 1) {
                        node.recycle()
                        throw FeedbackBlocked("accessibility_ambiguous_node")
                    }
                }
                repeat(node.childCount) { index -> node.getChild(index)?.let(queue::addLast) }
                node.recycle()
            }
        } finally {
            while (queue.isNotEmpty()) queue.removeFirst().recycle()
        }
        return matches
    }

    /** Match one finite, authored diagnostic without retaining or emitting arbitrary UI text. */
    private fun ownedVisibleTextCount(automation: UiAutomation, expected: String, uiOwner: UiOwner, deadline: Long): Int {
        val root = awaitOwnRoot(automation, uiOwner, deadline)
        val rootWindow = root.windowId
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        var count = 0
        var matches = 0
        try {
            while (queue.isNotEmpty()) {
                val node = queue.removeFirst()
                if (++count > MAX_NODES) {
                    node.recycle()
                    throw FeedbackBlocked("ui_node_bound")
                }
                val metadata = nodeMetadata(node)
                if (metadata.packageName != TARGET_PACKAGE || metadata.windowId != rootWindow) {
                    node.recycle()
                    throw FeedbackBlocked("ui_root_changed")
                }
                if (metadata.visible && node.text?.toString() == expected) {
                    matches++
                    if (matches > 1) {
                        node.recycle()
                        throw FeedbackBlocked("accessibility_ambiguous_text")
                    }
                }
                repeat(node.childCount) { index -> node.getChild(index)?.let(queue::addLast) }
                node.recycle()
            }
        } finally {
            while (queue.isNotEmpty()) queue.removeFirst().recycle()
        }
        return matches
    }

    /** Waits only for an owned active root; no node is retained across root/window changes. */
    private fun awaitOwnRoot(automation: UiAutomation, uiOwner: UiOwner, deadline: Long): AccessibilityNodeInfo {
        var last = "ui_root_missing"
        while (SystemClock.elapsedRealtime() < deadline) {
            requireExactResumedActivity(uiOwner.handler, uiOwner.application, uiOwner.process, uiOwner.activity, uiOwner.model, deadline)
            val root = automation.rootInActiveWindow
            requireBeforeDeadline(deadline)
            if (root == null) {
                last = "ui_root_missing"
                SystemClock.sleep(POLL_MILLIS)
                continue
            }
            if (root.packageName?.toString() != TARGET_PACKAGE) {
                root.recycle()
                last = "ui_root_foreign"
                SystemClock.sleep(POLL_MILLIS)
                continue
            }
            return root
        }
        throw FeedbackBlocked(last)
    }

    /** The final click revalidation must use the current root immediately, never wait or retry. */
    private fun immediateOwnRoot(automation: UiAutomation, uiOwner: UiOwner, deadline: Long): AccessibilityNodeInfo {
        requireExactResumedActivity(uiOwner.handler, uiOwner.application, uiOwner.process, uiOwner.activity, uiOwner.model, deadline)
        val root = automation.rootInActiveWindow
        requireBeforeDeadline(deadline)
        if (root == null) throw FeedbackBlocked("ui_root_missing")
        if (root.packageName?.toString() != TARGET_PACKAGE) {
            root.recycle()
            throw FeedbackBlocked("ui_root_foreign")
        }
        return root
    }

    private fun nodeMetadata(node: AccessibilityNodeInfo): NodeMetadata = NodeMetadata(
        packageName = node.packageName?.toString(),
        viewId = node.viewIdResourceName,
        windowId = node.windowId,
        visible = node.isVisibleToUser,
        enabled = node.isEnabled,
        click = node.actionList.any { it.id == AccessibilityNodeInfo.ACTION_CLICK },
    )

    private fun revalidate(automation: UiAutomation, found: OwnedNode, tag: String, uiOwner: UiOwner, deadline: Long): Boolean {
        if (!found.node.refresh()) return false
        requireBeforeDeadline(deadline)
        val root = immediateOwnRoot(automation, uiOwner, deadline)
        return try {
            val metadata = nodeMetadata(found.node)
            root.windowId == found.windowId && metadata.packageName == TARGET_PACKAGE &&
                metadata.viewId == resourceId(tag) && metadata.windowId == found.windowId &&
                metadata.visible && metadata.enabled && metadata.click
        } finally {
            root.recycle()
        }
    }

    /** Compose exposes testTagsAsResourceId as the raw test tag, not a package resource ID. */
    private fun resourceId(tag: String): String = tag

    private fun expectedDiagnostic(phase: ConnectionFailurePhase): String = "Reconnect stopped while ${when (phase) {
        ConnectionFailurePhase.Dial -> "connecting to the desktop"
        ConnectionFailurePhase.OpenStream -> "opening the desktop connection"
        ConnectionFailurePhase.Hello -> "confirming the desktop connection"
        ConnectionFailurePhase.Snapshot -> "loading desktop sessions"
        ConnectionFailurePhase.Reader -> "reading the desktop connection"
        ConnectionFailurePhase.Recovery -> "refreshing desktop sessions"
    }}."

    private fun metadataDigest(host: HostMetadata): String = digest(
        listOf(host.hostId, host.name, host.deviceId?.value.orEmpty(), *host.keyAliases.sorted().toTypedArray())
            .joinToString("\u0000")
            .toByteArray(StandardCharsets.UTF_8)
    )

    private fun keyAliasSnapshot(): KeyAliasSnapshot {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val names = buildList {
            val aliases = keyStore.aliases()
            while (aliases.hasMoreElements()) add(aliases.nextElement())
        }.sorted().toSet()
        return KeyAliasSnapshot(
            names = names,
            digest = digest(names.joinToString("\u0000").toByteArray(StandardCharsets.UTF_8)),
            endpointPresent = SecretStore.ENDPOINT_ALIAS in names,
        )
    }

    private fun processIdentity(): ProcessIdentity {
        val pid = Process.myPid()
        val stat = File("/proc/self/stat").readText()
        require(stat.substringBefore(' ').toInt() == pid) { "instrumentation_pid_mismatch" }
        val start = stat.substring(stat.lastIndexOf(')') + 2).trim().split(Regex("\\s+"))[19].toLong()
        val capabilitySets = File("/proc/self/status").useLines { lines ->
            lines.mapNotNull { line ->
                val key = line.substringBefore(':')
                line.substringAfter(':', "").trim().takeIf { key in CAPABILITY_FIELDS }?.let { key to it }
            }.toMap()
        }
        require(start > 0L && capabilitySets.keys == CAPABILITY_FIELDS) { "instrumentation_identity_unavailable" }
        val zero = capabilitySets.mapValues { (_, value) -> value.all { it == '0' } }
        return ProcessIdentity(pid, start, zero.getValue("CapInh"), zero.getValue("CapPrm"), zero.getValue("CapEff"), zero.getValue("CapAmb"))
    }

    private fun processMatches(expected: ProcessIdentity): Boolean {
        if (Process.myPid() != expected.pid) return false
        val stat = File("/proc/self/stat").readText()
        if (stat.substringBefore(' ').toIntOrNull() != expected.pid) return false
        return stat.substring(stat.lastIndexOf(')') + 2).trim().split(Regex("\\s+"))[19].toLongOrNull() == expected.start
    }

    private fun requireBeforeDeadline(deadline: Long) {
        if (SystemClock.elapsedRealtime() >= deadline) throw FeedbackBlocked("driver_deadline")
    }

    private fun actionIntentFile(context: Context, runId: String, tag: String) = File(context.cacheDir, "paired-device-feedback-$runId.$tag.intent.json")
    private fun actionOutcomeFile(context: Context, runId: String, tag: String) = File(context.cacheDir, "paired-device-feedback-$runId.$tag.outcome.json")
    private fun finalOutcomeFile(context: Context, runId: String) = File(context.cacheDir, "paired-device-feedback-$runId.final.outcome.json")
    private fun progressFile(context: Context, runId: String, index: Int) = File(context.cacheDir, "paired-device-feedback-$runId.phase$index.json")
    private fun launchArmedFile(context: Context, runId: String) = File(context.cacheDir, "paired-device-feedback-$runId.launch_armed.json")

    /** Root consumes this one-shot handshake before its own single ordinary activity launch. */
    private fun emitLaunchArmed(instrumentation: android.app.Instrumentation, context: Context, runId: String, process: ProcessIdentity, deadline: Long) {
        requireBeforeDeadline(deadline)
        val json = buildJsonObject {
            put("runId", runId)
            put("pid", process.pid)
            put("start", process.start)
            put("phase", "launch_armed")
            put("absoluteElapsedDeadline", deadline)
            put("launchRequired", true)
        }.toString()
        writeExclusive(context.cacheDir, launchArmedFile(context, runId), json)
        instrumentation.sendStatus(1, Bundle().apply { putString("goatrPairedFeedbackProgress", json) })
    }

    private fun actionIntentJson(runId: String, tag: String): String = buildJsonObject {
        put("phase", "before_ui_action")
        put("action", tag)
        put("runId", runId)
        put("nativeMutation", false)
    }.toString()

    private fun actionOutcomeJson(runId: String, tag: String, acknowledged: Boolean, closedPhaseObserved: Boolean): String = buildJsonObject {
        put("phase", "after_ui_action")
        put("action", tag)
        put("runId", runId)
        put("onClickAcknowledged", acknowledged)
        put("closedPhaseObserved", closedPhaseObserved)
        put("nativeMutation", false)
    }.toString()

    private fun writeExclusive(cacheDir: File, file: File, text: String) {
        require(file.parentFile == cacheDir) { "feedback_cache_path_required" }
        val bytes = text.toByteArray(StandardCharsets.UTF_8)
        try {
            Files.newByteChannel(file.toPath(), StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).use { channel ->
                val buffer = ByteBuffer.wrap(bytes)
                while (buffer.hasRemaining()) channel.write(buffer)
                (channel as? FileChannel)?.force(true)
            }
        } finally {
            bytes.fill(0)
        }
    }

    private fun classify(error: Throwable): DriverGuard {
        val code = (error as? FeedbackBlocked)?.code ?: error.message
        return code?.let(::guardFor) ?: when (error) {
            is SecurityException -> DriverGuard.Security
            is AssertionError -> DriverGuard.UiOther
            else -> DriverGuard.Other
        }
    }

    /** Maps only finite source literals; raw exception text is neither emitted nor retained. */
    private fun guardFor(code: String): DriverGuard = when (code) {
        "driver_deadline" -> DriverGuard.Deadline
        "main_activity_query_timeout", "main_activity_query_post_failed", "main_activity_query_unavailable" -> DriverGuard.MainQuery
        "main_activity_ambiguous" -> DriverGuard.MainAmbiguous
        "main_activity_not_resumed" -> DriverGuard.MainStage
        "main_activity_not_current", "original_activity_not_current" -> DriverGuard.MainNotCurrent
        "main_activity_changed_before_launch" -> DriverGuard.MainChanged
        "main_activity_replaced", "original_activity_replaced", "original_activity_missing" -> DriverGuard.MainReplaced
        "unexpected_main_activity" -> DriverGuard.MarkerMismatch
        "activity_create_timeout" -> DriverGuard.CreateTimeout
        "activity_resume_timeout" -> DriverGuard.ResumeTimeout
        "activity_destroy_timeout", "activity_finish_expired", "activity_finish_post_failed", "activity_finish_repeated", "main_handler_required" -> DriverGuard.Cleanup
        "owner_unavailable", "owner_replaced" -> DriverGuard.OwnerUnavailable
        "owner_store_load_timeout", "owner_store_baseline_mismatch", "owner_store_changed", "owner_paired_state_required", "owner_device_id_required", "owner_key_alias_required" -> DriverGuard.OwnerState
        "connection_observation_timeout" -> DriverGuard.ConnectionObservation
        "accessibility_node_missing" -> DriverGuard.UiMissing
        "accessibility_ambiguous_node", "accessibility_ambiguous_text" -> DriverGuard.UiAmbiguous
        "ui_root_missing" -> DriverGuard.UiRootMissing
        "ui_root_foreign" -> DriverGuard.UiRootForeign
        "ui_root_changed" -> DriverGuard.UiRootChanged
        "ui_node_bound" -> DriverGuard.UiNodeBound
        "ui_node_revalidation_failed" -> DriverGuard.UiRevalidation
        "ui_callback_not_acknowledged" -> DriverGuard.UiClickAcknowledgement
        "owned_ui_poll_timeout" -> DriverGuard.UiVisibleTimeout
        "authored_diagnostic_timeout" -> DriverGuard.UiDiagnosticTimeout
        "progress_phase_invalid", "progress_phase_order", "progress_final_repeated" -> DriverGuard.Progress
        "feedback_cache_path_required" -> DriverGuard.Report
        "run_id_required", "signer_required", "signer_hash_required", "feedback_disallows_fixture_or_control_arguments" -> DriverGuard.Request
        "target_package_required", "test_package_required", "debug_target_required", "target_signer_mismatch", "test_signer_mismatch", "feedback_runner_required", "instrumentation_identity_unavailable", "instrumentation_pid_mismatch", "instrumentation_process_changed", "target_application_required" -> DriverGuard.Target
        "paired_database_required", "exactly_one_paired_host_required", "pending_key_deletion_required", "paired_device_id_required", "paired_key_alias_required", "key_deletion_query_failed" -> DriverGuard.Pairing
        else -> DriverGuard.Other
    }

    private fun digest(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes)
        .joinToString("") { "%02x".format(it.toInt() and 255) }

    private data class FeedbackRequest(val runId: String, val signerSha256: String)
    private data class KeyAliasSnapshot(val names: Set<String>, val digest: String, val endpointPresent: Boolean)
    private data class PairedSnapshot(
        val hostId: String,
        val metadataDigest: String,
        val keyAliasDigest: String,
        val hostCount: Int,
        val endpointAliasPresent: Boolean,
        val deviceIdPresent: Boolean,
    )
    private data class ProcessIdentity(
        val pid: Int,
        val start: Long,
        val capInhZero: Boolean = false,
        val capPrmZero: Boolean = false,
        val capEffZero: Boolean = false,
        val capAmbZero: Boolean = false,
    )
    private data class UiOwner(
        val handler: Handler,
        val application: Application,
        val process: ProcessIdentity,
        val activity: MainActivity,
        val model: GoatrViewModel,
    )

    private sealed interface MainActivityLookup {
        object Missing : MainActivityLookup
        data class Existing(val activity: MainActivity, val owner: GoatrViewModel) : MainActivityLookup
    }

    private data class DriverObservation(
        val failure: ConnectionFailure?,
        val outcome: String,
        val navHostsAck: Boolean,
        val hostsDisplayed: Boolean,
        val detailsAck: Boolean,
    )

    private enum class ActivityOrigin(val wire: String) {
        None("none"),
        ExistingResumed("existing_resumed"),
        FreshCreated("fresh_created"),
    }

    /** Closed report vocabulary; raw thrown error text never leaves the test process. */
    private enum class DriverGuard(val wire: String) {
        None("none"),
        Request("request"),
        Target("target"),
        Pairing("pairing"),
        Security("security"),
        Deadline("deadline"),
        MainQuery("main_query"),
        MainAmbiguous("main_ambiguous"),
        MainStage("main_stage"),
        MainNotCurrent("main_not_current"),
        MainChanged("main_changed"),
        MainReplaced("main_replaced"),
        MarkerMismatch("marker_mismatch"),
        CreateTimeout("create_timeout"),
        ResumeTimeout("resume_timeout"),
        OwnerUnavailable("owner_unavailable"),
        OwnerState("owner_state"),
        ConnectionObservation("connection_observation"),
        UiMissing("ui_missing"),
        UiAmbiguous("ui_ambiguous"),
        UiRootMissing("ui_root_missing"),
        UiRootForeign("ui_root_foreign"),
        UiRootChanged("ui_root_changed"),
        UiNodeBound("ui_node_bound"),
        UiRevalidation("ui_revalidation"),
        UiClickAcknowledgement("ui_click_acknowledgement"),
        UiVisibleTimeout("ui_visible_timeout"),
        UiDiagnosticTimeout("ui_diagnostic_timeout"),
        UiOther("ui_other"),
        Cleanup("cleanup"),
        Progress("progress"),
        Report("report"),
        Other("other"),
    }

    /** Main-thread callbacks only identify this explicitly marked activity and publish its existing owner. */
    private inner class MainActivityLifecycle(
        private val targetApplication: Application,
        private val marker: String,
        private val process: ProcessIdentity,
    ) : Application.ActivityLifecycleCallbacks {
        val original = AtomicReference<MainActivity?>(null)
        val owner = AtomicReference<GoatrViewModel?>(null)
        val created = CountDownLatch(1)
        val resumed = CountDownLatch(1)
        val destroyed = CountDownLatch(1)
        val finishRequested = AtomicBoolean(false)
        val failure = AtomicReference<String?>(null)
        val createdObserved = AtomicBoolean(false)
        val resumedObserved = AtomicBoolean(false)

        fun reject(code: String) {
            if (failure.compareAndSet(null, code)) {
                created.countDown()
                resumed.countDown()
                destroyed.countDown()
            }
        }

        override fun onActivityCreated(activity: Activity, state: Bundle?) {
            if (activity !is MainActivity) return
            if (!processMatches(process) || activity.application !== targetApplication) {
                reject("unexpected_main_activity")
                return
            }
            if (activity.intent?.getStringExtra(RUN_MARKER_EXTRA) != marker) {
                reject("unexpected_main_activity")
                return
            }
            val current = original.get()
            when {
                current == null && original.compareAndSet(null, activity) -> {
                    createdObserved.set(true)
                    created.countDown()
                }
                current !== activity -> reject("original_activity_replaced")
            }
        }

        override fun onActivityResumed(activity: Activity) {
            val originalActivity = original.get() ?: return
            if (activity !== originalActivity) return
            if (!processMatches(process) || activity.isFinishing || activity.isDestroyed) {
                reject("original_activity_not_current")
                return
            }
            val model = try {
                ViewModelProvider(originalActivity)[GoatrViewModel::class.java]
            } catch (_: Throwable) {
                reject("owner_unavailable")
                return
            }
            val current = owner.get()
            when {
                current == null && owner.compareAndSet(null, model) -> { resumedObserved.set(true); resumed.countDown() }
                current === model -> { resumedObserved.set(true); resumed.countDown() }
                else -> reject("owner_replaced")
            }
        }

        override fun onActivityDestroyed(activity: Activity) {
            if (activity === original.get()) destroyed.countDown()
        }
        override fun onActivityStarted(activity: Activity) = Unit
        override fun onActivityPaused(activity: Activity) = Unit
        override fun onActivityStopped(activity: Activity) = Unit
        override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
    }

    private data class NodeMetadata(val packageName: String?, val viewId: String?, val windowId: Int, val visible: Boolean, val enabled: Boolean, val click: Boolean)
    private data class OwnedNode(val node: AccessibilityNodeInfo, val windowId: Int)
    private class FeedbackBlocked(val code: String) : IllegalStateException(code)

    private inner class ProgressReporter(
        private val instrumentation: android.app.Instrumentation,
        private val context: Context,
        private val runId: String,
        private val process: ProcessIdentity,
    ) {
        private var index = 0
        private var finalEmitted = false
        fun emit(phase: String) {
            require(index < PROGRESS_PHASES.size) { "progress_phase_invalid" }
            if (phase == PHASE_FINAL) {
                require(!finalEmitted) { "progress_final_repeated" }
                finalEmitted = true
            } else {
                require(!finalEmitted && phase == PROGRESS_PHASES[index]) { "progress_phase_order" }
            }
            index++
            val json = buildJsonObject {
                put("runId", runId)
                put("pid", process.pid)
                put("start", process.start)
                put("phase", phase)
                put("entered", true)
            }.toString()
            writeExclusive(context.cacheDir, progressFile(context, runId, index), json)
            instrumentation.sendStatus(1, Bundle().apply { putString("goatrPairedFeedbackProgress", json) })
        }
    }

    private data class FeedbackReport(
        val outcome: String,
        val driverGuard: DriverGuard,
        val failure: ConnectionFailure?,
        val hostCount: Int,
        val exactHostMatch: Boolean,
        val uiHosts: Boolean,
        val navHostsAck: Boolean,
        val uiDetails: Boolean,
        val pairedMetadataUnchanged: Boolean,
        val keyAliasDigestUnchanged: Boolean,
        val endpointAliasPresent: Boolean,
        val deviceIdPresent: Boolean,
        val signerMatch: Boolean,
        val closed: Boolean,
        val callbacksRegistered: Boolean,
        val created: Boolean,
        val resumed: Boolean,
        val activityOrigin: ActivityOrigin,
        val launchRequired: Boolean,
        val pid: Int,
        val start: Long,
        val capInhZero: Boolean,
        val capPrmZero: Boolean,
        val capEffZero: Boolean,
        val capAmbZero: Boolean,
        val runId: String,
    ) {
        private fun JsonObjectBuilder.putClosed(name: String, value: String?) {
            if (value == null) put(name, JsonNull) else put(name, value)
        }

        private fun JsonObjectBuilder.putClosed(name: String, value: Boolean?) {
            if (value == null) put(name, JsonNull) else put(name, value)
        }

        fun json(): String = buildJsonObject {
            put("outcome", outcome)
            put("driverGuard", driverGuard.wire)
            putClosed("phase", failure?.phase?.name?.lowercase())
            putClosed("category", failure?.category?.name?.lowercase())
            putClosed("errorCode", failure?.code?.name?.lowercase())
            putClosed("transmission", failure?.helloTransmissionCompleted)
            putClosed("receive", failure?.helloReceivePhase?.name?.lowercase())
            putClosed("nativeRead", failure?.helloNativeReadPhase?.name?.lowercase())
            put("hostCount", hostCount)
            put("exactHostMatch", exactHostMatch)
            put("uiHosts", uiHosts)
            put("navHostsAck", navHostsAck)
            put("uiDetails", uiDetails)
            put("pairedMetadataUnchanged", pairedMetadataUnchanged)
            put("keyAliasDigestUnchanged", keyAliasDigestUnchanged)
            put("endpointAliasPresent", endpointAliasPresent)
            put("deviceIdPresent", deviceIdPresent)
            put("signerMatch", signerMatch)
            put("closed", closed)
            put("callbacksRegistered", callbacksRegistered)
            put("created", created)
            put("resumed", resumed)
            put("activityOrigin", activityOrigin.wire)
            put("launchRequired", launchRequired)
            put("pid", pid)
            put("start", start)
            put("capInhZero", capInhZero)
            put("capPrmZero", capPrmZero)
            put("capEffZero", capEffZero)
            put("capAmbZero", capAmbZero)
            put("runId", runId)
        }.toString()
    }

    private companion object {
        const val RUN_ID_ARGUMENT = "feedbackRunId"
        const val SIGNER_ARGUMENT = "feedbackExpectedSignerSha256"
        const val TARGET_PACKAGE = "io.github.trevarj.goatr.debug"
        const val TEST_PACKAGE = "io.github.trevarj.goatr.debug.test"
        const val RUN_MARKER_EXTRA = "io.github.trevarj.goatr.feedback.RUN_MARKER"
        const val DATABASE_NAME = "goatr-private.db"
        const val DRIVER_TIMEOUT_MILLIS = 60_000L
        const val RECONNECT_TIMEOUT_MILLIS = 45_000L
        const val CLOSE_RESERVE_MILLIS = 5_000L
        const val POLL_MILLIS = 50L
        const val MAX_NODES = 512
        val CAPABILITY_FIELDS = setOf("CapInh", "CapPrm", "CapEff", "CapAmb")
        val SIGNER_SHA256 = Regex("[0-9a-f]{64}")
        const val PHASE_METHOD_ENTRY = "method_entry"
        const val PHASE_PREFLIGHT = "preflight"
        const val PHASE_LAUNCH = "launch"
        const val PHASE_OWNER = "owner"
        const val PHASE_OBSERVATION = "observation"
        const val PHASE_NAVIGATION = "navigation"
        const val PHASE_DETAILS = "details"
        const val PHASE_FINAL = "final"
        val PROGRESS_PHASES = listOf(PHASE_METHOD_ENTRY, PHASE_PREFLIGHT, PHASE_LAUNCH, PHASE_OWNER, PHASE_OBSERVATION, PHASE_NAVIGATION, PHASE_DETAILS, PHASE_FINAL)
    }
}
