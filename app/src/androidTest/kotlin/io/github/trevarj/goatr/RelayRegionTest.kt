// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Context
import android.content.ContextWrapper
import android.os.Bundle
import android.content.SharedPreferences
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import computer.iroh.IrohAndroid
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Assume.assumeFalse
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test

/** Real native owner and private preferences. No public-network readiness requirement by default. */
class RelayRegionTest {
    @get:Rule val ui=createEmptyComposeRule()
    private val context get()=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
    private fun requireIsolatedInstallation() {
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        assumeFalse("Never adopt an installed protected identity",keys.containsAlias(SecretStore.ENDPOINT_ALIAS))
        for(suffix in listOf(".gcm",".gcm.bak",".gcm.new",".gcm.initialized"))
            assumeFalse("Never adopt installed key material",File(context.noBackupFilesDir,"goatr-keys/${SecretStore.ENDPOINT_ALIAS}$suffix").exists())
        assumeFalse("Another runtime owns the endpoint",GoatrConnection.endpointIsOwned())
    }
    private suspend fun withInstallation(saved:String?=null,block:suspend (Context,SessionStore,SecretStore,GoatrConnection)->Unit) {
        requireIsolatedInstallation()
        IrohAndroid.installAndroidContext(context)
        val base=context
        val prefix="owned-relay-${Uuid.random().value}."
        val isolated=object:ContextWrapper(base) {
            override fun getApplicationContext():Context=this
            override fun getSharedPreferences(name:String,mode:Int)=base.getSharedPreferences(prefix+name,mode)
        }
        val preferences=isolated.getSharedPreferences("local_transport",Context.MODE_PRIVATE)
        if(saved!=null)check(preferences.edit().putString("home_relay_region",saved).commit())
        val name="relay-${Uuid.random().value}.db"
        val store=SessionStore(isolated,name)
        val secrets=SecretStore(isolated)
        val owner=GoatrConnection(isolated,store,secrets)
        try {store.open();block(isolated,store,secrets,owner)}
        finally {
            owner.shutdown()
            // The fixture proved absence before creating anything; never delete another identity.
            owner.forgetInstallation()
            store.close();base.deleteDatabase(name);base.deleteSharedPreferences(prefix+"local_transport")
        }
    }

    @Test fun unavailableSavedRegionRequiresAnExplicitValidatedReset()=runBlocking {
        val unavailable="https://not-a-native-region.invalid/"
        withInstallation(unavailable) {isolated,store,secrets,owner->
            owner.loadHomeRelayRegions()
            val saved=owner.homeRelaySettings.value
            assertFalse(saved.available)
            assertEquals("Unavailable saved region",saved.label(saved.savedUrl))
            try {owner.start();fail("An unavailable saved selection must not bind Automatic")}
            catch(_:IllegalArgumentException) {}
            assertFalse(owner.ownsNetworkCallback());assertFalse(GoatrConnection.endpointIsOwned())
            try {owner.saveHomeRelayRegion(unavailable){true};fail("Unknown selection must be refused")}
            catch(_:IllegalArgumentException) {}
            // Admission must be checked again immediately before the durable effect.
            var admissions=0
            try {owner.saveHomeRelayRegion(null){++admissions<2};fail("Stale settings must not reset the choice")}
            catch(_:IllegalStateException) {}
            assertEquals(2,admissions)
            assertEquals(unavailable,owner.homeRelaySettings.value.savedUrl)
            assertEquals(unavailable,isolated.getSharedPreferences("local_transport",Context.MODE_PRIVATE).getString("home_relay_region",null))
            owner.saveHomeRelayRegion(null){true}
            assertFalse(owner.homeRelaySettings.value.bound)
            assertFalse("Saving alone cannot mint a runtime claim or endpoint",GoatrConnection.endpointIsOwned())
            assertFalse(isolated.getSharedPreferences("local_transport",Context.MODE_PRIVATE).contains("home_relay_region"))
            val reopened=GoatrConnection(isolated,store,secrets)
            try {reopened.loadHomeRelayRegions();assertTrue(reopened.homeRelaySettings.value.available);assertNull(reopened.homeRelaySettings.value.savedUrl)}
            finally {reopened.shutdown()}
        }
    }

    @Test fun cancellationAfterSaveAdmissionRetiresTheOldEndpointAndRecreationRetainsIdentity()=runBlocking {
        withInstallation {isolated,store,secrets,owner->
            val id=withTimeout(20000){owner.start()}
            owner.loadHomeRelayRegions()
            val selected=owner.homeRelaySettings.value.regions.first().url
            try {owner.saveHomeRelayRegion("https://not-a-native-region.invalid/"){true};fail("Invalid choice must not retire the endpoint")}
            catch(_:IllegalArgumentException) {}
            assertTrue(owner.ownsNetworkCallback());assertEquals(id,owner.start())
            val cancellation=Job()
            val admissions=AtomicInteger()
            val saving=launch(cancellation+Dispatchers.IO) {
                owner.saveHomeRelayRegion(selected) {
                    if(admissions.incrementAndGet()==2)cancellation.cancel()
                    true
                }
            }
            try {withTimeout(20000){saving.join()}}finally{cancellation.cancelAndJoin()}
            assertTrue(saving.isCancelled)
            assertEquals(2,admissions.get())
            assertEquals(selected,owner.homeRelaySettings.value.savedUrl)
            assertFalse("Accepted saving must retire even after caller cancellation",owner.ownsNetworkCallback())
            assertFalse(GoatrConnection.endpointIsOwned());assertFalse(owner.homeRelaySettings.value.bound)
            val reopened=GoatrConnection(isolated,store,secrets)
            try {
                assertEquals(selected,reopened.homeRelaySettings.value.savedUrl)
                assertEquals(id,withTimeout(20000){reopened.start()})
                assertEquals(selected,reopened.homeRelaySettings.value.boundUrl)
                assertEquals(id,withTimeout(20000){reopened.recreate()})
                reopened.saveHomeRelayRegion(null){true}
                assertFalse(reopened.ownsNetworkCallback())
                assertEquals(id,withTimeout(20000){reopened.start()})
                assertTrue(reopened.homeRelaySettings.value.bound);assertNull(reopened.homeRelaySettings.value.boundUrl)
            } finally {reopened.shutdown()}
        }
    }

    @Test fun globalSettingsDraftAppliesOnlyOnSaveAndKeepsTheProtectedIdentity()=runBlocking {
        requireIsolatedInstallation()
        val app=context as GoatrApplication
        assumeTrue("Never replace an installation's existing transport choice",app.connectionChoiceIsAutomatic())
        app.runtime.store.open()
        assumeTrue("Never adopt paired host state",app.runtime.store.state.value.hosts.isEmpty())
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var model:GoatrViewModel
        try {
            scenario.onActivity {model=ViewModelProvider(it)[GoatrViewModel::class.java];model.settings()}
            ui.waitUntil(20000){model.foreground&&model.connection.ownsNetworkCallback()}
            val id=model.connection.start()
            ui.onNodeWithTag("home_relay_region").performScrollTo()
            ui.waitUntil(10000){model.connection.homeRelaySettings.value.loaded}
            val selected=model.connection.homeRelaySettings.value.regions.first()
            ui.onNodeWithTag("home_relay_region").performClick()
            ui.onNodeWithTag("home_relay_region_0").performClick()
            ui.onNodeWithTag("home_relay_region").assertTextContains(selected.label)
            assertNull(model.connection.homeRelaySettings.value.savedUrl)
            assertNull(model.connection.homeRelaySettings.value.boundUrl)
            ui.onNodeWithTag("home_relay_save").performScrollTo().performClick()
            ui.waitUntil(25000){!model.busy&&model.connection.homeRelaySettings.value.savedUrl==selected.url&&model.connection.homeRelaySettings.value.bound}
            assertEquals(id,model.connection.start())
            scenario.recreate()
            scenario.onActivity {assertSame(model,ViewModelProvider(it)[GoatrViewModel::class.java])}
            ui.onNodeWithTag("home_relay_region").performScrollTo().assertTextContains(selected.label)
            ui.onNodeWithTag("home_relay_region").performClick()
            ui.onNodeWithTag("home_relay_region_automatic").performClick()
            assertEquals(selected.url,model.connection.homeRelaySettings.value.savedUrl)
            ui.onNodeWithTag("home_relay_save").performScrollTo().performClick()
            ui.waitUntil(25000){!model.busy&&model.connection.homeRelaySettings.value.savedUrl==null&&model.connection.homeRelaySettings.value.bound}
            assertEquals(id,model.connection.start())
            assertNull(model.connection.homeRelaySettings.value.boundUrl)
        } finally {
            scenario.close()
            withTimeout(20000){while(GoatrConnection.endpointIsOwned())delay(20)}
            app.runtime.connection.forgetInstallation()
        }
    }
    private fun GoatrApplication.connectionChoiceIsAutomatic()=
        runtime.connection.homeRelaySettings.value.savedUrl==null && !getSharedPreferences("local_transport",Context.MODE_PRIVATE).contains("home_relay_region")

    @Test fun navigationAndCallerCancellationAfterAcceptedSaveKeepAttentionRuntimeUsable()=runBlocking {
        requireIsolatedInstallation()
        val app=context as GoatrApplication
        assumeTrue("Never replace an installation's existing transport choice",app.connectionChoiceIsAutomatic())
        app.runtime.store.open()
        assumeTrue("Never adopt paired host state",app.runtime.store.state.value.hosts.isEmpty())
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var model:GoatrViewModel
        val preferences=app.getSharedPreferences("local_transport",Context.MODE_PRIVATE)
        try {
            scenario.onActivity {model=ViewModelProvider(it)[GoatrViewModel::class.java];model.settings()}
            ui.waitUntil(20000){model.foreground&&model.connection.ownsNetworkCallback()}
            val id=model.connection.start()
            for(cancelCaller in listOf(false,true)) {
                scenario.onActivity {model.settings()}
                ui.onNodeWithTag("home_relay_region").performScrollTo()
                ui.waitUntil(10000){model.connection.homeRelaySettings.value.loaded}
                val settings=model.connection.homeRelaySettings.value
                val index=settings.regions.indexOfFirst {it.url!=settings.savedUrl}
                ui.onNodeWithTag("home_relay_region").performClick()
                ui.onNodeWithTag("home_relay_region_$index").performClick()
                lateinit var existingChildren:Set<Job>
                scenario.onActivity {existingChildren=model.viewModelScope.coroutineContext[Job]!!.children.toSet()}
                val navigated=AtomicBoolean()
                val cancelled=AtomicBoolean(!cancelCaller)
                val listener=SharedPreferences.OnSharedPreferenceChangeListener {_,key->
                    if(key=="home_relay_region") {
                        // commit posts this on Main before the retiring operation resumes there.
                        model.sessions()
                        if(cancelCaller)model.viewModelScope.coroutineContext[Job]!!.children.singleOrNull {it !in existingChildren}?.let {
                            it.cancel();cancelled.set(true)
                        }
                        navigated.set(true)
                    }
                }
                preferences.registerOnSharedPreferenceChangeListener(listener)
                try {
                    ui.onNodeWithTag("home_relay_save").performScrollTo().performClick()
                    ui.waitUntil(25000){navigated.get()&&!model.busy&&model.screen==Screen.Sessions}
                    assertTrue("The actual accepted-save caller must be cancelled",cancelled.get())
                } finally {preferences.unregisterOnSharedPreferenceChangeListener(listener)}
                // The genuine local endpoint is not a paired desktop. This absent attention
                // lookup must pass runtime readiness and fail locally, without creating/dialing
                // any synthetic host or native session.
                scenario.onActivity {model.openNotification(id,Uuid.random().value)}
                ui.waitUntil(25000){!model.busy&&model.notice=="This notification is no longer retained on this phone."}
                assertTrue(model.connection.ownsNetworkCallback())
                assertEquals(id,model.connection.start())
                assertTrue(app.runtime.store.state.value.hosts.isEmpty())
            }
        } finally {
            scenario.close()
            withTimeout(20000){while(GoatrConnection.endpointIsOwned())delay(20)}
            app.runtime.connection.forgetInstallation()
        }
    }

    @Test fun preAdmissionRefusalPreservesChoiceAndRepairsActualAttentionReadiness()=runBlocking {
        requireIsolatedInstallation()
        val app=context as GoatrApplication
        assumeTrue("Never replace an installation's existing transport choice",app.connectionChoiceIsAutomatic())
        app.runtime.store.open()
        assumeTrue("Never adopt paired host state",app.runtime.store.state.value.hosts.isEmpty())
        val scenario=ActivityScenario.launch(MainActivity::class.java)
        lateinit var model:GoatrViewModel
        try {
            scenario.onActivity {model=ViewModelProvider(it)[GoatrViewModel::class.java];model.settings()}
            ui.waitUntil(20000){model.foreground&&model.connection.homeRelaySettings.value.bound}
            ui.onNodeWithTag("home_relay_region").performScrollTo()
            ui.waitUntil(10000){model.connection.homeRelaySettings.value.loaded}
            ui.waitForIdle() // Zero hosts and the bound endpoint settle the actual startup first.
            val id=model.connection.start()
            val selected=model.connection.homeRelaySettings.value.regions.first().url
            suspend fun whileOwnerAdmissionIsBlocked(block:suspend ()->Unit)=coroutineScope {
                val entered=CompletableDeferred<Unit>()
                val release=java.util.concurrent.CountDownLatch(1)
                val holding=async(Dispatchers.IO) {
                    val failure=try {
                        model.connection.saveHomeRelayRegion(null) {
                            entered.complete(Unit)
                            check(release.await(10,java.util.concurrent.TimeUnit.SECONDS)) {"Owner admission barrier expired"}
                            false // This real owner operation never accepts an effect.
                        }
                        null
                    } catch(error:IllegalStateException) {error}
                    assertEquals("Home relay settings are no longer active",failure?.message)
                }
                try {withTimeout(5000){entered.await()};block()}
                finally {release.countDown();withContext(NonCancellable){withTimeout(15000){holding.await()}}}
            }
            whileOwnerAdmissionIsBlocked {
                scenario.onActivity {
                    model.saveHomeRelayRegion(selected) // Replaces readiness, then waits for the real owner mutex.
                    assertTrue(model.busy)
                    model.sessions() // Original Settings admission is gone before the owner checks it.
                }
            }
            ui.waitUntil(10000){!model.busy}
            assertNull(model.connection.homeRelaySettings.value.savedUrl)
            assertFalse(app.getSharedPreferences("local_transport",Context.MODE_PRIVATE).contains("home_relay_region"))
            assertTrue(model.connection.homeRelaySettings.value.bound)
            assertEquals(id,model.connection.start())
            whileOwnerAdmissionIsBlocked {
                scenario.onActivity {model.openNotification(id,Uuid.random().value)}
                // Drain the actual ordered store writer. Without readiness repair, the failed
                // deferred's stale-attention fallback finishes before this fence and busy=false.
                // With repair, this genuine consumer is still waiting for the owner mutex.
                app.runtime.store.open()
                scenario.onActivity {
                    assertTrue("A refused relay save must not bypass foreground runtime readiness",model.busy)
                    assertNull(model.notice)
                }
            }
            ui.waitUntil(25000){!model.busy&&model.notice=="This notification is no longer retained on this phone."}
            assertTrue(model.connection.ownsNetworkCallback())
            assertEquals(id,model.connection.start())
            assertTrue(app.runtime.store.state.value.hosts.isEmpty())
        } finally {
            scenario.close()
            withTimeout(20000){while(GoatrConnection.endpointIsOwned())delay(20)}
            app.runtime.connection.forgetInstallation()
        }
    }

    /** Explicitly opt in on a fresh isolated install; real Internet relay availability is required. */
    @Test fun networkSmokeObservesConnectedHomeAThenBWithSameIdentityAndRestoresAutomatic()=runBlocking {
        assumeTrue("Opt-in public-network smoke",InstrumentationRegistry.getArguments().getString("goatrRelayRegionNetworkSmoke")=="true")
        withInstallation {isolated,store,secrets,owner->
            owner.loadHomeRelayRegions()
            val regions=owner.homeRelaySettings.value.regions
            val a=regions.first().url
            val b=regions.first {it.url!=a}.url
            owner.saveHomeRelayRegion(a){true}
            val id=withTimeout(20000){owner.start()}
            assertEquals(a,owner.awaitConnectedHomeRelay())
            owner.saveHomeRelayRegion(b){true}
            assertEquals(id,withTimeout(20000){owner.start()})
            assertEquals(b,owner.awaitConnectedHomeRelay())
            owner.shutdown()
            val recreated=GoatrConnection(isolated,store,secrets)
            try {
                assertEquals(b,recreated.homeRelaySettings.value.savedUrl)
                assertEquals(id,withTimeout(20000){recreated.start()})
                assertEquals(b,recreated.awaitConnectedHomeRelay())
                recreated.saveHomeRelayRegion(null){true}
                assertEquals(id,withTimeout(20000){recreated.start()})
                assertNull(recreated.homeRelaySettings.value.boundUrl)
                val automatic=recreated.awaitConnectedHomeRelay()
                InstrumentationRegistry.getInstrumentation().sendStatus(2,Bundle().apply {
                    putString("goatrRelayRegionEndpointId",id);putString("goatrRelayRegionConnectedA",a)
                    putString("goatrRelayRegionConnectedB",b);putString("goatrRelayRegionConnectedAutomatic",automatic)
                })
            } finally {recreated.shutdown()}
        }
    }

    /** Individually selected, explicitly admitted PRODUCT smoke; never owns/deletes installation data. */
    @Test fun ownedProductUiObservesConnectedHomeChangesAndRestoresOriginalChoice()=runBlocking {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val arguments=InstrumentationRegistry.getArguments()
        assumeTrue("Opt-in existing-PRODUCT smoke",arguments.getString("goatrRelayRegionOwnedProductSmoke")=="true")
        check(arguments.getString("class")=="io.github.trevarj.goatr.RelayRegionTest#ownedProductUiObservesConnectedHomeChangesAndRestoresOriginalChoice") {"owned_product_individual_selection_required"}
        val expectedUid=arguments.getString("goatrRelayRegionOwnedProductExpectedUid")?.toIntOrNull()?.takeIf {it>0}
            ?: error("owned_product_expected_uid_required")
        val expectedHash=arguments.getString("goatrRelayRegionOwnedProductExpectedApkSha256")
            ?.takeIf {it.matches(Regex("[0-9a-f]{64}"))} ?: error("owned_product_expected_apk_sha256_required")
        val app=context as? GoatrApplication ?: error("owned_product_application_required")
        val packageName="io.github.trevarj.goatr.debug"
        val runtime=app.runtime
        val connection=runtime.connection
        val pid=android.os.Process.myPid()
        fun processEpoch():String {
            val stat=File("/proc/self/stat").readText()
            check(stat.length<=65536&&stat.substringBefore(' ').toIntOrNull()==pid) {"owned_product_process_unavailable"}
            return stat.substring(stat.lastIndexOf(')')+2).trim().split(Regex("\\s+")).getOrNull(19)
                ?.takeIf {it.toLongOrNull()?.let {start->start>0}==true} ?: error("owned_product_process_epoch_unavailable")
        }
        val epoch=processEpoch()
        val source=File(app.applicationInfo.sourceDir).canonicalFile
        val sourceStamp=android.system.Os.stat(source.path)
        val digest=MessageDigest.getInstance("SHA-256")
        source.inputStream().use {input->
            val buffer=ByteArray(65536)
            while(true) {val count=input.read(buffer);if(count<0)break;digest.update(buffer,0,count)}
        }
        check(digest.digest().joinToString("") {"%02x".format(it.toInt() and 255)}==expectedHash) {"owned_product_apk_hash_mismatch"}
        fun requireProduct() {
            check(app.packageName==packageName&&android.os.Process.myUid()==expectedUid&&processEpoch()==epoch) {"owned_product_process_changed"}
            val installed=app.packageManager.getApplicationInfo(packageName,0)
            check(installed.uid==expectedUid&&installed.processName==packageName&&File(installed.sourceDir).canonicalFile==source&&installed.splitSourceDirs.isNullOrEmpty()) {"owned_product_package_changed"}
            val process=app.getSystemService(android.app.ActivityManager::class.java).runningAppProcesses?.singleOrNull {it.pid==pid}
            check(process!=null&&process.uid==expectedUid&&process.processName==packageName) {"owned_product_default_process_required"}
            val stamp=android.system.Os.stat(source.path)
            check(stamp.st_dev==sourceStamp.st_dev&&stamp.st_ino==sourceStamp.st_ino&&stamp.st_size==sourceStamp.st_size&&stamp.st_mtime==sourceStamp.st_mtime) {"owned_product_apk_changed"}
            check(!GoatrConnection.endpointIsOwned()||connection.ownsNetworkCallback()) {"owned_product_foreign_endpoint_owner"}
        }
        requireProduct()
        val keys=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}
        check(keys.containsAlias(SecretStore.ENDPOINT_ALIAS)) {"owned_product_existing_protected_identity_required"}
        runtime.store.open()
        check(runtime.store.state.value.hosts.isEmpty()&&runtime.store.state.value.keyDeletion.isEmpty()) {"owned_product_zero_host_state_required"}
        connection.loadHomeRelayRegions()
        val initial=connection.homeRelaySettings.value
        check(initial.available) {"owned_product_original_region_unavailable"}
        val original=initial.savedUrl
        val preferences=app.getSharedPreferences("local_transport",Context.MODE_PRIVATE)
        check(preferences.getString("home_relay_region",null)==original) {"owned_product_original_selection_mismatch"}
        val choices=initial.regions.filter {it.url!=original}
        check(choices.size>=2) {"owned_product_distinct_regions_unavailable"}
        val a=choices[0].url
        val b=choices[1].url
        val handler=android.os.Handler(android.os.Looper.getMainLooper())
        val actorJob=coroutineContext[Job] ?: error("owned_product_actor_job_missing")
        var callbackLifetime=actorJob
        var lifetimeDeadline=Long.MAX_VALUE
        val monitor=ActivityLifecycleMonitorRegistry.getInstance()
        var observed:MainActivity?=null
        val ownershipAmbiguous=AtomicBoolean(false)
        var observerArmed=false
        val launchIssued=AtomicBoolean(false)
        var activityRetained=false
        val observer=androidx.test.runner.lifecycle.ActivityLifecycleCallback {current,_->
            if(current is MainActivity) {
                if(!launchIssued.get())ownershipAmbiguous.set(true)
                else if(observed==null)observed=current
                else if(observed!==current)ownershipAmbiguous.set(true)
            }
        }
        fun onMain(deadline:Long=android.os.SystemClock.elapsedRealtime()+2000,block:(()->Unit)->Unit) {
            check(android.os.Looper.myLooper()!=android.os.Looper.getMainLooper()) {"owned_product_actor_requires_test_thread"}
            // Snapshot this call's lifetime; cleanup cannot revive an expired actor callback.
            val callbackJob=callbackLifetime
            val callbackDeadline=minOf(deadline,lifetimeDeadline)
            val done=java.util.concurrent.CountDownLatch(1)
            val cancelled=AtomicBoolean(false)
            val failure=java.util.concurrent.atomic.AtomicReference<Throwable?>(null)
            val action=Runnable {
                try {
                    fun live() {
                        check(!cancelled.get()&&callbackJob.isActive&&android.os.SystemClock.elapsedRealtime()<callbackDeadline) {"owned_product_main_callback_expired"}
                    }
                    live()
                    block(::live)
                } catch(error:Throwable) {failure.set(error)}
                finally {done.countDown()}
            }
            check(handler.post(action)) {"owned_product_main_post_failed"}
            try {
                check(done.await(maxOf(0L,callbackDeadline-android.os.SystemClock.elapsedRealtime()),java.util.concurrent.TimeUnit.MILLISECONDS)) {"owned_product_main_callback_timeout"}
                failure.get()?.let {throw it}
            } finally {cancelled.set(true);handler.removeCallbacks(action)}
        }
        lateinit var activity:MainActivity
        lateinit var model:GoatrViewModel
        lateinit var window:android.view.Window
        lateinit var decor:android.view.View
        var token:android.os.IBinder?=null
        var secure=0
        var baseWindow=-1
        var identity:String?=null
        var restorationNeeded=false
        var failed=false
        var restoreFailed=false
        var closeFailed=false
        var restored=false
        var restoredHome:String?=null
        var connectedA=false
        var connectedB=false
        var recreatedB=false
        var stage="startup"
        var failedStage:String?=null
        var failedEvidence:Pair<String,Int>?=null
        var restoreEvidence:Pair<String,Int>?=null
        var closeEvidence:Pair<String,Int>?=null
        var restoreStage="verify_original"
        var closeStage="finish_guard"
        var closeFailedStage:String?=null
        val failureCodes=setOf(
            "owned_product_process_unavailable","owned_product_process_epoch_unavailable","owned_product_process_changed",
            "owned_product_package_changed","owned_product_default_process_required","owned_product_apk_changed","owned_product_foreign_endpoint_owner",
            "owned_product_actor_requires_test_thread","owned_product_main_callback_expired","owned_product_main_post_failed","owned_product_main_callback_timeout",
            "owned_product_activity_ownership_changed","owned_product_activity_changed","owned_product_component_or_model_changed","owned_product_window_changed",
            "owned_product_secure_flag_changed","owned_product_window_focus_changed","owned_product_protected_alias_changed","owned_product_zero_host_state_changed",
            "owned_product_active_window_missing","owned_product_foreign_window","owned_product_scene_not_admitted","owned_product_input_window_changed",
            "owned_product_endpoint_not_bound","owned_product_protected_identity_changed","owned_product_logo_wait_requires_test_thread","owned_product_logo_density_unavailable",
            "owned_product_goat_not_title_centered","owned_product_header_outside_root","owned_product_header_overlap","owned_product_goat_clipped_or_wrong_size",
            "owned_product_logo_region_invalid","owned_product_header_moved_before_copy","owned_product_logo_window_not_current","owned_product_logo_region_outside_window",
            "owned_product_logo_copy_timeout","owned_product_logo_copy_failed","owned_product_header_moved_during_copy","owned_product_logo_png_bound",
            "owned_product_goat_moved_before_output","owned_product_logo_canvas_not_plain","owned_product_logo_foreground_or_gutter_missing","owned_product_header_moved_before_output",
            "owned_product_selected_region_unavailable","owned_product_canonical_selection_not_persisted","owned_product_existing_activity_refused","owned_product_unique_launcher_required",
            "owned_product_launcher_not_admitted","owned_product_launcher_component_changed","owned_product_launch_cancelled","owned_product_launcher_output_unavailable",
            "owned_product_launch_not_admitted",
            "owned_product_endpoint_readiness_timeout",
            "owned_product_popup_return_timeout","owned_product_selection_timeout",
            "owned_product_endpoint_release_timeout",
            "owned_product_launch_activity_not_admitted","owned_product_initial_scene_changed","owned_product_home_a_not_connected","owned_product_home_b_not_connected",
            "owned_product_recreated_identity_changed","owned_product_recreated_home_b_not_connected","owned_product_recreated_b_not_persisted","owned_product_original_home_not_connected",
            "owned_product_original_selection_not_restored","owned_product_cleanup_not_admitted",
        )
        fun failureEvidence(error:Throwable):Pair<String,Int> {
            val code=error.message?.takeIf {it in failureCodes} ?: when(error) {
                is TimeoutCancellationException->"deadline"
                is CancellationException->"cancelled"
                is AssertionError->"assertion"
                is IllegalStateException->"state"
                is SecurityException->"security"
                is java.io.IOException->"io"
                else->"other"
            }
            val line=error.stackTrace.firstOrNull {
                it.fileName=="RelayRegionTest.kt"&&it.lineNumber>0&&
                    (it.className=="io.github.trevarj.goatr.RelayRegionTest"||
                        it.className.startsWith("io.github.trevarj.goatr.RelayRegionTest\$ownedProductUiObservesConnectedHomeChangesAndRestoresOriginalChoice"))
            }?.lineNumber ?: 0
            return code to line
        }
        fun onActivity(popup:Boolean=false,deadline:Long=Long.MAX_VALUE,block:(MainActivity,()->Unit)->Unit) {
            onMain(minOf(deadline,android.os.SystemClock.elapsedRealtime()+2000)) {live->
                requireProduct()
                check(activityRetained&&!ownershipAmbiguous.get()&&observed===activity) {"owned_product_activity_ownership_changed"}
                val current=Stage.values().flatMap {monitor.getActivitiesInStage(it)}.filterIsInstance<MainActivity>().singleOrNull()
                check(current===activity&&monitor.getLifecycleStageOf(activity)==Stage.RESUMED&&activity.application===app&&!activity.isFinishing&&!activity.isDestroyed) {"owned_product_activity_changed"}
                check(activity.componentName==android.content.ComponentName(app,MainActivity::class.java)&&ViewModelProvider(activity)[GoatrViewModel::class.java]===model) {"owned_product_component_or_model_changed"}
                check(activity.window===window&&window.decorView===decor&&decor.isAttachedToWindow&&token!=null&&decor.windowToken===token) {"owned_product_window_changed"}
                check(window.attributes.flags and android.view.WindowManager.LayoutParams.FLAG_SECURE==secure) {"owned_product_secure_flag_changed"}
                check(popup||activity.hasWindowFocus()) {"owned_product_window_focus_changed"}
                check(keys.containsAlias(SecretStore.ENDPOINT_ALIAS)) {"owned_product_protected_alias_changed"}
                check(runtime.store.state.value.hosts.isEmpty()&&runtime.store.state.value.keyDeletion.isEmpty()) {"owned_product_zero_host_state_changed"}
                live()
                block(activity,live)
            }
        }
        fun activeWindow():Int {
            val root=instrumentation.uiAutomation.rootInActiveWindow ?: error("owned_product_active_window_missing")
            return try {
                check(root.packageName?.toString()==packageName) {"owned_product_foreign_window"}
                check(root.windowId>=0) {"owned_product_active_window_missing"}
                root.windowId
            } finally {root.recycle()}
        }
        fun guard(scene:Screen,expectedWindow:Int=baseWindow,popup:Boolean=false,deadline:Long=Long.MAX_VALUE) {
            onActivity(popup,deadline) {_,_->
                check(model.foreground&&!model.busy&&model.screen==scene&&model.host==null&&model.selection==null) {"owned_product_scene_not_admitted"}
            }
            check(activeWindow()==expectedWindow) {"owned_product_input_window_changed"}
        }
        fun click(tag:String,scene:Screen,expectedWindow:Int=baseWindow,popup:Boolean=false,scroll:Boolean=false,deadline:Long=Long.MAX_VALUE) {
            guard(scene,expectedWindow,popup,deadline)
            if(scroll)ui.onNodeWithTag(tag).performScrollTo()
            ui.onAllNodesWithTag(tag).assertCountEquals(1)
            ui.onNodeWithTag(tag).assertIsDisplayed().assertIsEnabled()
            guard(scene,expectedWindow,popup,deadline)
            ui.onNodeWithTag(tag).performClick()
        }
        suspend fun awaitOwnedPopupReturn(popupWindow:Int,previous:Screen,scene:Screen,deadline:Long) {
            val waitingContext=currentCoroutineContext()
            val waitingDeadline=minOf(deadline,lifetimeDeadline)
            while(true) {
                waitingContext.ensureActive()
                check(android.os.SystemClock.elapsedRealtime()<waitingDeadline) {"owned_product_popup_return_timeout"}
                val before=activeWindow()
                check(before==popupWindow||before==baseWindow) {"owned_product_input_window_changed"}
                var ready=false
                // The known popup may still own focus; allow normal Compose frame scheduling, not input.
                onActivity(popup=true,deadline=waitingDeadline) {current,live->
                    check(model.foreground&&!model.busy&&(model.screen==previous||model.screen==scene)&&model.host==null&&model.selection==null) {"owned_product_scene_not_admitted"}
                    live()
                    ready=model.screen==scene&&current.hasWindowFocus()
                    if((before!=baseWindow||!ready)&&ui.mainClock.autoAdvance) {
                        live()
                        val currentWindow=activeWindow()
                        check(currentWindow==popupWindow||currentWindow==baseWindow) {"owned_product_input_window_changed"}
                        live()
                        ui.mainClock.advanceTimeByFrame()
                        live()
                    }
                }
                val after=activeWindow()
                check(after==popupWindow||after==baseWindow) {"owned_product_input_window_changed"}
                if(before==baseWindow&&after==baseWindow&&ready) {
                    guard(scene,deadline=waitingDeadline)
                    waitingContext.ensureActive()
                    check(android.os.SystemClock.elapsedRealtime()<waitingDeadline) {"owned_product_popup_return_timeout"}
                    return
                }
                delay(minOf(20L,maxOf(0L,waitingDeadline-android.os.SystemClock.elapsedRealtime())))
            }
        }
        suspend fun requireIdentity() {
            check(connection.ownsNetworkCallback()&&connection.homeRelaySettings.value.bound) {"owned_product_endpoint_not_bound"}
            check(withTimeout(25000){connection.start()}==identity) {"owned_product_protected_identity_changed"}
        }
        suspend fun observeActualSettingsHeader() {
            check(android.os.Looper.myLooper()!=android.os.Looper.getMainLooper()) {"owned_product_logo_wait_requires_test_thread"}
            guard(Screen.Settings)
            requireIdentity()
            ui.onAllNodesWithTag("titlebar_goat",useUnmergedTree=true).assertCountEquals(1)
            ui.onAllNodesWithText("Settings",useUnmergedTree=true).assertCountEquals(1)
            val goatNode=ui.onNodeWithTag("titlebar_goat",useUnmergedTree=true).assertIsDisplayed()
            val headingNode=ui.onNodeWithText("Settings",useUnmergedTree=true).assertIsDisplayed()
                .assert(SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.Heading))
            val goat=goatNode.fetchSemanticsNode()
            val heading=headingNode.fetchSemanticsNode().boundsInWindow
            val bounds=goat.boundsInWindow
            val root=ui.onRoot(useUnmergedTree=true).fetchSemanticsNode().boundsInWindow
            val menu=ui.onNodeWithTag("navigation_menu").assertIsDisplayed().fetchSemanticsNode().boundsInWindow
            var density=0f
            onActivity {current,_->density=current.resources.displayMetrics.density}
            check(density.isFinite()&&density>0) {"owned_product_logo_density_unavailable"}
            check(kotlin.math.abs(bounds.center.y-heading.center.y)<=0.5f) {"owned_product_goat_not_title_centered"}
            check(bounds.left>=root.left&&bounds.top>=root.top&&bounds.right<=root.right&&bounds.bottom<=root.bottom&&
                heading.left>=root.left&&heading.top>=root.top&&heading.right<=root.right&&heading.bottom<=root.bottom) {"owned_product_header_outside_root"}
            check((bounds.right<=heading.left||bounds.left>=heading.right)&&(bounds.right<=menu.left||bounds.left>=menu.right)) {"owned_product_header_overlap"}
            val whole=goatNode.getUnclippedBoundsInRoot()
            val clipped=goat.boundsInRoot
            check(kotlin.math.abs(clipped.left-whole.left.value*density)<=0.5f&&kotlin.math.abs(clipped.top-whole.top.value*density)<=0.5f&&
                kotlin.math.abs(clipped.right-whole.right.value*density)<=0.5f&&kotlin.math.abs(clipped.bottom-whole.bottom.value*density)<=0.5f&&
                kotlin.math.abs(bounds.width-24*density)<=0.5f&&kotlin.math.abs(bounds.height-24*density)<=0.5f) {"owned_product_goat_clipped_or_wrong_size"}
            val rect=android.graphics.Rect(Math.round(bounds.left),Math.round(bounds.top),Math.round(bounds.right),Math.round(bounds.bottom))
            check(!rect.isEmpty&&rect.width()<=256&&rect.height()<=256) {"owned_product_logo_region_invalid"}
            guard(Screen.Settings)
            check(goatNode.fetchSemanticsNode().boundsInWindow==bounds&&headingNode.fetchSemanticsNode().boundsInWindow==heading) {"owned_product_header_moved_before_copy"}
            val bitmap=android.graphics.Bitmap.createBitmap(rect.width(),rect.height(),android.graphics.Bitmap.Config.ARGB_8888)
            val copied=java.util.concurrent.CountDownLatch(1)
            val copyGate=Any()
            var requested=false
            var completed=false
            var abandoned=false
            var copyResult=-1
            try {
                onActivity {current,live->
                    check(current===activity&&current.window===window&&current.hasWindowFocus()&&model.foreground&&
                        model.screen==Screen.Settings&&model.host==null&&!model.busy&&decor.windowToken===token) {"owned_product_logo_window_not_current"}
                    check(rect.left>=0&&rect.top>=0&&rect.right<=decor.width&&rect.bottom<=decor.height) {"owned_product_logo_region_outside_window"}
                    // Direct, non-null source rectangle and region-sized destination only.
                    // No captureToImage/screenshot/whole-window/crop fallback exists here.
                    synchronized(copyGate) {
                        check(!abandoned) {"owned_product_main_callback_expired"}
                        requested=true
                    }
                    try {
                        live()
                        android.view.PixelCopy.request(window,android.graphics.Rect(rect),bitmap,{result->
                            synchronized(copyGate) {
                                copyResult=result;completed=true
                                if(abandoned)bitmap.recycle()
                            }
                            copied.countDown()
                        },handler)
                    } catch(error:Throwable) {
                        synchronized(copyGate) {completed=true;if(abandoned)bitmap.recycle()}
                        throw error
                    }
                }
                // Only this off-Main test thread waits; the existing Main handler delivers completion.
                check(copied.await(1,java.util.concurrent.TimeUnit.SECONDS)) {"owned_product_logo_copy_timeout"}
                check(copyResult==android.view.PixelCopy.SUCCESS) {"owned_product_logo_copy_failed"}
                guard(Screen.Settings)
                requireIdentity()
                check(goatNode.fetchSemanticsNode().boundsInWindow==bounds&&headingNode.fetchSemanticsNode().boundsInWindow==heading) {"owned_product_header_moved_during_copy"}
                val png=java.io.ByteArrayOutputStream(32768)
                check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,png)&&png.size() in 1..32768) {"owned_product_logo_png_bound"}
                guard(Screen.Settings)
                check(goatNode.fetchSemanticsNode().boundsInWindow==bounds) {"owned_product_goat_moved_before_output"}
                // Infer the plain canvas from its corner, not from resource pixels or a theme palette.
                val background=bitmap.getPixel(0,0)
                fun contrast(pixel:Int)=maxOf(
                    kotlin.math.abs(android.graphics.Color.red(pixel)-android.graphics.Color.red(background)),
                    kotlin.math.abs(android.graphics.Color.green(pixel)-android.graphics.Color.green(background)),
                    kotlin.math.abs(android.graphics.Color.blue(pixel)-android.graphics.Color.blue(background)),
                )
                check(android.graphics.Color.alpha(background)==255&&contrast(bitmap.getPixel(bitmap.width-1,0))<=2&&
                    contrast(bitmap.getPixel(0,bitmap.height-1))<=2&&contrast(bitmap.getPixel(bitmap.width-1,bitmap.height-1))<=2) {"owned_product_logo_canvas_not_plain"}
                var left=bitmap.width;var top=bitmap.height;var right=-1;var bottom=-1
                for(y in 0 until bitmap.height)for(x in 0 until bitmap.width)if(contrast(bitmap.getPixel(x,y))>8) {
                    left=minOf(left,x);top=minOf(top,y);right=maxOf(right,x);bottom=maxOf(bottom,y)
                }
                val gutter=maxOf(1,minOf(bitmap.width,bitmap.height)/24)
                check(right>=left&&bottom>=top&&left>=gutter&&top>=gutter&&right<bitmap.width-gutter&&bottom<bitmap.height-gutter) {"owned_product_logo_foreground_or_gutter_missing"}
                guard(Screen.Settings)
                check(goatNode.fetchSemanticsNode().boundsInWindow==bounds&&headingNode.fetchSemanticsNode().boundsInWindow==heading) {"owned_product_header_moved_before_output"}
                instrumentation.sendStatus(2,Bundle().apply {
                    putString("goatrRelayRegionOwnedProductUiStatus","passed")
                    putInt("goatrRelayRegionOwnedProductPid",pid)
                    putString("goatrRelayRegionOwnedProductProcessStart",epoch)
                    putString("goatrRelayRegionOwnedProductApkSha256",expectedHash)
                    putBoolean("goatrRelayRegionActualHeaderGeometryVerified",true)
                    putBoolean("goatrRelayRegionTitleGoatCanvasVerified",true)
                    putString("goatrRelayRegionTitleGoatPngBase64",android.util.Base64.encodeToString(png.toByteArray(),android.util.Base64.NO_WRAP))
                })
            } finally {
                synchronized(copyGate) {
                    abandoned=true
                    // A timed-out asynchronous copy must not write into a recycled bitmap.
                    // In that case its completion callback above owns the eventual disposal.
                    if(!requested||completed)bitmap.recycle()
                }
            }
        }
        suspend fun applyChoice(url:String?) {
            guard(Screen.Settings)
            val index=connection.homeRelaySettings.value.regions.indexOfFirst {it.url==url}
            check(url==null||index>=0) {"owned_product_selected_region_unavailable"}
            val option=if(url==null)"home_relay_region_automatic" else "home_relay_region_$index"
            click("home_relay_region",Screen.Settings,scroll=true)
            ui.onNodeWithTag(option).assertIsDisplayed()
            val popupWindow=activeWindow().also {check(it!=baseWindow) {"owned_product_input_window_changed"}}
            val selectionDeadline=minOf(android.os.SystemClock.elapsedRealtime()+30000,lifetimeDeadline)
            click(option,Screen.Settings,popupWindow,popup=true,deadline=selectionDeadline)
            awaitOwnedPopupReturn(popupWindow,Screen.Settings,Screen.Settings,selectionDeadline)
            guard(Screen.Settings,deadline=selectionDeadline)
            ui.onNodeWithTag("home_relay_save").performScrollTo()
            guard(Screen.Settings,deadline=selectionDeadline)
            restorationNeeded=true // A click may be accepted even if its acknowledgement is lost.
            click("home_relay_save",Screen.Settings,deadline=selectionDeadline)
            val remaining=selectionDeadline-android.os.SystemClock.elapsedRealtime()
            check(remaining>0) {"owned_product_selection_timeout"}
            ui.waitUntil(remaining) {!model.busy&&connection.homeRelaySettings.value.savedUrl==url&&connection.homeRelaySettings.value.bound}
            guard(Screen.Settings,deadline=selectionDeadline)
            check(if(url==null)!preferences.contains("home_relay_region") else preferences.getString("home_relay_region",null)==url) {"owned_product_canonical_selection_not_persisted"}
            requireIdentity()
        }
        try {
            onMain {live->
                requireProduct()
                check(Stage.values().flatMap {monitor.getActivitiesInStage(it)}.none {it is MainActivity}) {"owned_product_existing_activity_refused"}
                live()
            }
            val component=android.content.ComponentName(app,MainActivity::class.java)
            val launcher=android.content.Intent(android.content.Intent.ACTION_MAIN).addCategory(android.content.Intent.CATEGORY_LAUNCHER).setPackage(packageName)
            val registered=app.packageManager.queryIntentActivities(launcher,android.content.pm.PackageManager.MATCH_DISABLED_COMPONENTS).singleOrNull()?.activityInfo
                ?: error("owned_product_unique_launcher_required")
            check(registered.packageName==packageName&&registered.name==component.className&&registered.enabled&&registered.exported&&
                registered.applicationInfo.enabled&&registered.applicationInfo.uid==expectedUid&&registered.processName==packageName) {"owned_product_launcher_not_admitted"}
            check(component.flattenToString()=="io.github.trevarj.goatr.debug/io.github.trevarj.goatr.MainActivity") {"owned_product_launcher_component_changed"}
            onMain {live->
                requireProduct()
                check(Stage.values().flatMap {monitor.getActivitiesInStage(it)}.none {it is MainActivity}) {"owned_product_existing_activity_refused"}
                live()
                monitor.addLifecycleCallback(observer);observerArmed=true
            }
            val launchDeadline=android.os.SystemClock.elapsedRealtime()+25000
            requireProduct()
            check(actorJob.isActive) {"owned_product_launch_cancelled"}
            check(!ownershipAmbiguous.get()&&launchIssued.compareAndSet(false,true)) {"owned_product_launch_not_admitted"}
            check(!ownershipAmbiguous.get()) {"owned_product_activity_ownership_changed"}
            // Normal registered launcher only; shell text is discarded, never treated as effect proof.
            val descriptor=instrumentation.uiAutomation.executeShellCommand("am start --user 0 -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -n ${component.flattenToString()}")
            android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use {input->
                val drainDeadline=android.os.SystemClock.elapsedRealtime()+1000
                val poll=android.system.StructPollfd().apply {fd=descriptor.fileDescriptor;events=android.system.OsConstants.POLLIN.toShort()}
                val buffer=ByteArray(512)
                var count=0
                while(count<4096) {
                    val remaining=drainDeadline-android.os.SystemClock.elapsedRealtime()
                    if(remaining<=0||android.system.Os.poll(arrayOf(poll),remaining.toInt())==0)break
                    check(poll.revents.toInt() and (android.system.OsConstants.POLLERR or android.system.OsConstants.POLLNVAL)==0) {"owned_product_launcher_output_unavailable"}
                    val read=input.read(buffer,0,minOf(buffer.size,4096-count))
                    if(read<0)break
                    count+=read
                }
            }
            // Passive observation only, under the original shared launch deadline.
            while(true) {
                check(actorJob.isActive) {"owned_product_launch_cancelled"}
                check(android.os.SystemClock.elapsedRealtime()<launchDeadline) {"owned_product_endpoint_readiness_timeout"}
                if(connection.homeRelaySettings.value.bound&&connection.ownsNetworkCallback())break
                delay(minOf(20L,maxOf(0L,launchDeadline-android.os.SystemClock.elapsedRealtime())))
            }
            while(!activityRetained) {
                onMain(launchDeadline) {live->
                    requireProduct()
                    check(!ownershipAmbiguous.get()) {"owned_product_activity_ownership_changed"}
                    val current=Stage.values().flatMap {monitor.getActivitiesInStage(it)}.filterIsInstance<MainActivity>()
                    check(current.size<=1&&(observed==null||current.singleOrNull()===observed)) {"owned_product_activity_ownership_changed"}
                    current.singleOrNull()?.let {candidate->
                        check(candidate===observed&&candidate.application===app&&candidate.componentName==component&&!candidate.isFinishing&&!candidate.isDestroyed) {"owned_product_launch_activity_not_admitted"}
                        val candidateWindow=candidate.window
                        val candidateDecor=candidateWindow.decorView
                        if(monitor.getLifecycleStageOf(candidate)==Stage.RESUMED&&candidateDecor.isAttachedToWindow&&candidateDecor.windowToken!=null&&candidate.hasWindowFocus()) {
                            live()
                            activity=candidate;window=candidateWindow;decor=candidateDecor;token=candidateDecor.windowToken
                            model=ViewModelProvider(candidate)[GoatrViewModel::class.java]
                            secure=window.attributes.flags and android.view.WindowManager.LayoutParams.FLAG_SECURE
                            activityRetained=true
                        }
                    }
                }
                if(!activityRetained)delay(20)
            }
            baseWindow=activeWindow()
            ui.waitUntil(25000){model.foreground&&!model.busy&&connection.ownsNetworkCallback()}
            ui.waitForIdle()
            onActivity {_,_->
                check(model.screen==Screen.Sessions&&model.host==null&&model.selection==null) {"owned_product_initial_scene_changed"}
            }
            guard(Screen.Sessions)
            identity=withTimeout(25000){connection.start()}
            stage="navigate_settings"
            click("navigation_menu",Screen.Sessions)
            ui.onNodeWithTag("navigation_settings_menu").assertIsDisplayed()
            val menuWindow=activeWindow().also {check(it!=baseWindow) {"owned_product_input_window_changed"}}
            val navigationDeadline=android.os.SystemClock.elapsedRealtime()+10000
            click("navigation_settings_menu",Screen.Sessions,menuWindow,popup=true,deadline=navigationDeadline)
            awaitOwnedPopupReturn(menuWindow,Screen.Sessions,Screen.Settings,navigationDeadline)
            stage="actual_settings_header"
            observeActualSettingsHeader()
            stage="select_a"
            applyChoice(a)
            stage="connect_a"
            check(connection.awaitConnectedHomeRelay()==a) {"owned_product_home_a_not_connected"}
            connectedA=true;stage="select_b"
            applyChoice(b)
            stage="connect_b"
            check(connection.awaitConnectedHomeRelay()==b) {"owned_product_home_b_not_connected"}
            connectedB=true;stage="recreate_b"
            guard(Screen.Settings)
            check(withTimeout(30000){connection.recreate()}==identity) {"owned_product_recreated_identity_changed"}
            stage="connect_recreated_b"
            guard(Screen.Settings)
            check(connection.awaitConnectedHomeRelay()==b) {"owned_product_recreated_home_b_not_connected"}
            recreatedB=true
            check(preferences.getString("home_relay_region",null)==b&&connection.homeRelaySettings.value.savedUrl==b) {"owned_product_recreated_b_not_persisted"}
        } catch(error:Throwable) {failed=true;failedStage=stage;failedEvidence=failureEvidence(error)}
        finally {withContext(NonCancellable) {
            try {withTimeout(180000) {
                callbackLifetime=coroutineContext[Job]!!
                lifetimeDeadline=android.os.SystemClock.elapsedRealtime()+180000
                if(restorationNeeded) {
                    restoreStage="select_original"
                    applyChoice(original)
                    restoreStage="connect_original"
                    restoredHome=connection.awaitConnectedHomeRelay()
                    check(original==null||restoredHome==original) {"owned_product_original_home_not_connected"}
                    restoreStage="verify_identity"
                    requireIdentity()
                }
                restoreStage="verify_original"
                check(preferences.getString("home_relay_region",null)==original&&connection.homeRelaySettings.value.savedUrl==original) {"owned_product_original_selection_not_restored"}
                restored=true
            }} catch(error:Throwable) {restoreFailed=true;restoreEvidence=failureEvidence(error)}
            finally {
                try {withTimeout(15000) {
                    callbackLifetime=coroutineContext[Job]!!
                    lifetimeDeadline=android.os.SystemClock.elapsedRealtime()+15000
                    if(launchIssued.get()) {
                        check(activityRetained&&restored&&!restoreFailed) {"owned_product_cleanup_not_admitted"}
                        check(activeWindow()==baseWindow) {"owned_product_input_window_changed"}
                        onActivity {current,live->
                            check(model.foreground&&!model.busy&&(model.screen==Screen.Sessions||model.screen==Screen.Settings)&&model.host==null&&model.selection==null) {"owned_product_scene_not_admitted"}
                            check(preferences.getString("home_relay_region",null)==original&&connection.homeRelaySettings.value.savedUrl==original) {"owned_product_original_selection_not_restored"}
                            live()
                            current.finish()
                        }
                        closeStage="destroy_wait"
                        val destroyDeadline=minOf(android.os.SystemClock.elapsedRealtime()+10000,lifetimeDeadline)
                        val destructionContext=currentCoroutineContext()
                        var destroyed=false
                        while(!destroyed) {
                            // Require global claim absence before guarded destruction observation.
                            while(GoatrConnection.endpointIsOwned()) {
                                destructionContext.ensureActive()
                                check(android.os.SystemClock.elapsedRealtime()<destroyDeadline) {"owned_product_endpoint_release_timeout"}
                                delay(minOf(20L,maxOf(0L,destroyDeadline-android.os.SystemClock.elapsedRealtime())))
                            }
                            destructionContext.ensureActive()
                            onMain(destroyDeadline) {live->
                                requireProduct()
                                check(!ownershipAmbiguous.get()&&observed===activity) {"owned_product_activity_ownership_changed"}
                                val current=Stage.values().flatMap {monitor.getActivitiesInStage(it)}.filterIsInstance<MainActivity>()
                                check(current.all {it===activity}) {"owned_product_activity_ownership_changed"}
                                live()
                                destroyed=activity.isDestroyed&&(current.isEmpty()||monitor.getLifecycleStageOf(activity)==Stage.DESTROYED)
                            }
                            if(!destroyed)delay(minOf(20L,maxOf(0L,destroyDeadline-android.os.SystemClock.elapsedRealtime())))
                        }
                    }
                }} catch(error:Throwable) {closeFailed=true;closeFailedStage=closeStage;closeEvidence=failureEvidence(error)}
                finally {
                    closeStage="observer_remove"
                    if(observerArmed)try {withTimeout(2500) {
                        callbackLifetime=coroutineContext[Job]!!
                        lifetimeDeadline=android.os.SystemClock.elapsedRealtime()+2500
                        onMain {live->live();monitor.removeLifecycleCallback(observer);observerArmed=false}
                    }}
                    catch(error:Throwable) {closeFailed=true;if(closeEvidence==null) {closeFailedStage=closeStage;closeEvidence=failureEvidence(error)}}
                }
            }
        }}
        instrumentation.sendStatus(2,Bundle().apply {
            putString("goatrRelayRegionOwnedProductStatus",if(restoreFailed)"restore_failed" else if(failed||closeFailed)"failed" else "passed")
            putBoolean("goatrRelayRegionOriginalChoiceRestored",restored)
            putBoolean("goatrRelayRegionIdentityUnchanged",!failed&&!restoreFailed&&!closeFailed)
            putBoolean("goatrRelayRegionConnectedAObserved",connectedA)
            putBoolean("goatrRelayRegionConnectedBObserved",connectedB)
            putBoolean("goatrRelayRegionRecreatedBObserved",recreatedB)
            if(connectedA)putString("goatrRelayRegionConnectedA",a)
            if(connectedB)putString("goatrRelayRegionConnectedB",b)
            failedStage?.let {putString("goatrRelayRegionFailedStage",it)}
            failedEvidence?.let {putString("goatrRelayRegionFailureCode",it.first);putInt("goatrRelayRegionFailureLine",it.second)}
            restoreEvidence?.let {
                putString("goatrRelayRegionRestoreFailureCode",it.first);putInt("goatrRelayRegionRestoreFailureLine",it.second)
                putString("goatrRelayRegionRestoreFailedStage",restoreStage)
            }
            closeEvidence?.let {
                putString("goatrRelayRegionCleanupFailureCode",it.first);putInt("goatrRelayRegionCleanupFailureLine",it.second)
                putString("goatrRelayRegionCleanupFailedStage",closeFailedStage)
            }
            restoredHome?.let {putString("goatrRelayRegionRestoredHome",it)}
        })
        check(!restoreFailed&&restored) {"owned_product_original_choice_restore_failed"}
        check(!failed&&!closeFailed) {"owned_product_relay_smoke_failed"}
    }
}
