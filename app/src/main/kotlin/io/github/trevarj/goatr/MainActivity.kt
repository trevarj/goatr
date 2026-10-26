// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.content.Intent
import android.os.Bundle
import android.os.Build
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toArgb
import androidx.core.view.WindowCompat
import androidx.lifecycle.ViewModelProvider
import android.content.pm.ApplicationInfo
import android.os.Looper
import android.os.Process
import androidx.lifecycle.Lifecycle

class MainActivity : ComponentActivity() {
    private val ownerDelegate=lazy {ViewModelProvider(this,ViewModelProvider.AndroidViewModelFactory.getInstance(application))[GoatrViewModel::class.java]}
    private val owner by ownerDelegate
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // Compose selects its own palette; prevent the light platform theme from recoloring it.
            window.decorView.setForceDarkAllowed(false)
        }
        setContent {GoatrTheme(owner.appearance) {
            val background = MaterialTheme.colorScheme.background
            val lightSystemBars = background.luminance() > 0.5f
            SideEffect {
                WindowCompat.getInsetsController(window, window.decorView).apply {
                    isAppearanceLightStatusBars = lightSystemBars
                    isAppearanceLightNavigationBars = lightSystemBars
                }
                @Suppress("DEPRECATION")
                if (Build.VERSION.SDK_INT in Build.VERSION_CODES.O..Build.VERSION_CODES.P) {
                    window.navigationBarColor = background.toArgb()
                }
            }
            GoatrApp(owner) {sensitive->if(sensitive) window.addFlags(WindowManager.LayoutParams.FLAG_SECURE) else window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)}
        }}
        handleAttentionIntent(intent)
    }
    override fun onStart() {super.onStart();owner.foreground(true)}
    override fun onStop() {if(!isChangingConfigurations) owner.foreground(false);super.onStop()}
    /** TEMPORARY opt-in scalar consumer; never dump private UI or initialize an owner here. */
    override fun dump(prefix:String,fd:java.io.FileDescriptor?,writer:java.io.PrintWriter,args:Array<out String>?) {
        val debug=applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        if(debugEligibilityDelegate(debug,args)){super.dump(prefix,fd,writer,args);return}
        if(!debug)return
        val request=DebugEligibilityArgs.parse(args) ?: return
        val out=writer
        val deadline=System.nanoTime()+DEBUG_ELIGIBILITY_EXPIRY_NANOS
        val pidMatched=Process.myPid()==request.pid
        fun activityCurrent()=Looper.myLooper()===Looper.getMainLooper() && !isFinishing && !isDestroyed &&
            lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
        val model=if(pidMatched && activityCurrent() && ownerDelegate.isInitialized())ownerDelegate.value else null
        val publication=model?.connection?.state?.value
        val main=if(model!=null && publication!=null)model.debugEligibilityMain(request,publication) else null
        val expectedHello=publication?.hello?.get(request.host)
        val originalGuard=if(model!=null && expectedHello!=null)model.connection.debugEligibilityOriginalGuard(request.host,expectedHello) else null
        val sample=if(main!=null && main.foreground && main.paired && main.current() &&
            originalGuard?.invoke()==true && publication!=null && System.nanoTime()<deadline)
            debugEligibilityPublished(request,publication) else null
        val resumed=activityCurrent()
        val coherent=sample!=null && main!=null && publication!=null && model!=null &&
            debugEligibilityCoherent(deadline,System.nanoTime(),publication,model.connection.state.value,
                main.current(),resumed,pidMatched)
        val gate=if(coherent)model!!.debugEligibilityGate(request,publication!!,main!!,sample!!) else null
        val line=debugEligibilityLine(request.nonce,pidMatched,resumed,coherent,main,sample,gate)
        // Final main/publication rejoin immediately precedes the sole closed output line.
        val finalCoherent=coherent && originalGuard?.invoke()==true && debugEligibilityCoherent(deadline,System.nanoTime(),publication!!,
            model!!.connection.state.value,main!!.current(),activityCurrent(),pidMatched)
        out.println(if(finalCoherent)line else debugEligibilityLine(request.nonce,pidMatched,activityCurrent(),false,null,null,null))
    }
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleAttentionIntent(intent)
    }
    private fun handleAttentionIntent(intent: Intent?) {
        if(intent?.action!=NotificationCoordinator.ACTION_OPEN_ATTENTION) return
        val host=intent.getStringExtra(NotificationCoordinator.EXTRA_HOST) ?: return
        val event=intent.getStringExtra(NotificationCoordinator.EXTRA_EVENT) ?: return
        val route=intent.data
        if(route?.scheme!="goatr" || route.authority!="attention" || route.pathSegments?.size!=2 || route.pathSegments?.get(0)!=host || route.pathSegments?.get(1)!=event) return
        owner.openNotification(host,event)
    }
}

// TEMPORARY DEBUG verification contract. Selectors are input-only, never record fields.
internal const val DEBUG_ELIGIBILITY_EXPIRY_NANOS=200_000_000L
internal const val DEBUG_ELIGIBILITY_NAMESPACE="--goatr-debug-eligibility-v1"
internal fun debugEligibilityDelegate(debug:Boolean,args:Array<out String>?):Boolean =
    !debug && args?.any {it.startsWith("--goatr-debug-eligibility")}!=true
internal class DebugEligibilityArgs private constructor(val nonce:String,val pid:Int,val host:String,val server:String,val incarnation:String,val project:String) {
    companion object {
        fun parse(args:Array<out String>?):DebugEligibilityArgs? {
            if(args==null || args.size!=7 || args.any {it.length>64} || args[0]!=DEBUG_ELIGIBILITY_NAMESPACE)return null
            fun uuid(value:String,nonce:Boolean=false):Boolean=try {
                java.util.UUID.fromString(value).let {it.toString()==value && it.variant()==2 && (!nonce || it.version()==4)}
            } catch(_:IllegalArgumentException){false}
            if(!uuid(args[1],true) || args[2].isEmpty() || args[2].length>10 || args[2][0]=='0' ||
                args[2].any {it !in '0'..'9'} || args[3].length!=64 || args[3].any {it !in '0'..'9' && it !in 'a'..'f'} ||
                !uuid(args[4]) || !uuid(args[5]) || !uuid(args[6]))return null
            val pid=args[2].toIntOrNull()?.takeIf {it>0} ?: return null
            return DebugEligibilityArgs(args[1],pid,args[3],args[4],args[5],args[6])
        }
    }
}
internal class DebugEligibilityMain(val foreground:Boolean,val busy:Boolean,val ongoing:Boolean?,val paired:Boolean,val current:()->Boolean)
internal class DebugEligibilityConnection(
    val helloPresent:Boolean,val inventoryPresent:Boolean,val scopeMatched:Boolean,
    val inventoryReady:Boolean,val mutationReady:Boolean,
    val serverOnline:Boolean?,val sourceCurrent:Boolean?,val projectAvailable:Boolean?,
    val ompBackendReady:Boolean?,val codexBackendReady:Boolean?,val ompCanCreate:Boolean?,val codexCanCreate:Boolean?,
) {
    // Only a positive recovery fact follows from this publication; false mutation readiness is ambiguous.
    val receiptsRecovered:Boolean? get()=if(mutationReady)true else null
}
/** Published scalars only: internal invalidation may precede the next publication. */
internal fun debugEligibilityPublished(request:DebugEligibilityArgs,publication:ConnectionView):DebugEligibilityConnection? {
    val hello=publication.hello[request.host]?.takeIf {it.hostId.value==request.host} ?: return null
    val inventory=publication.inventory[request.host] ?: return null
    val ready=publication.inventoryReady[request.host] ?: return null
    val mutation=publication.mutationReady[request.host] ?: return null
    val server=inventory.servers.singleOrNull {it.serverId.value==request.server} ?: return null
    val graph=inventory.hierarchy.singleOrNull {it.serverId.value==request.server} ?: return null
    if(graph.serverIncarnation.value!=request.incarnation)return null
    val project=inventory.projects.singleOrNull {it.projectId.value==request.project} ?: return null
    return DebugEligibilityConnection(
        helloPresent=true,inventoryPresent=true,scopeMatched=true,inventoryReady=ready,mutationReady=mutation,
        serverOnline=server.connectivity==ServerConnectivity.Online,
        sourceCurrent=hostSourceCurrent(ready,inventory,server.serverId),projectAvailable=project.available,
        ompBackendReady=hello.backends.singleOrNull {it.provider==Provider.Omp}?.let {it.availability==Availability.Ready},
        codexBackendReady=hello.backends.singleOrNull {it.provider==Provider.Codex}?.let {it.availability==Availability.Ready},
        ompCanCreate=server.canCreate.omp,codexCanCreate=server.canCreate.codex,
    )
}
internal fun debugEligibilityCoherent(deadline:Long,now:Long,before:Any,after:Any,mainCurrent:Boolean,activityCurrent:Boolean,pidMatched:Boolean)=
    now<deadline && before===after && mainCurrent && activityCurrent && pidMatched
internal fun debugEligibilityAnd(common:Boolean?,online:Boolean?,current:Boolean?,project:Boolean?,canCreate:Boolean?):Boolean? =
    if(common==false || online==false || current==false || project==false || canCreate==false)false
    else if(common==true && online==true && current==true && project==true && canCreate==true)true else null
internal fun debugEligibilityLine(nonce:String,pidMatched:Boolean,resumed:Boolean,coherent:Boolean,main:DebugEligibilityMain?,sample:DebugEligibilityConnection?,gate:Boolean?):String=buildString(2048) {
    val c=sample.takeIf {coherent};val m=main.takeIf {coherent}
    append("GOATR_DEBUG_ELIGIBILITY_V1 {\"v\":1,\"nonce\":\"").append(nonce).append("\",\"observed\":").append(c!=null)
    append(",\"coherent\":").append(coherent).append(",\"expectedPidMatched\":").append(pidMatched)
    append(",\"scopeMatched\":").append(c?.scopeMatched==true && m?.paired==true)
    append(",\"scopeUnproved\":").append(c?.scopeMatched!=true || m?.paired!=true)
    fun field(name:String,value:Boolean?){append(",\"").append(name).append("Known\":").append(value!=null);append(",\"").append(name).append("\":").append(value==true)}
    field("resumed",resumed);field("foreground",m?.foreground);field("busy",m?.busy);field("ongoingCreation",m?.ongoing)
    field("helloPresent",c?.helloPresent);field("inventoryPresent",c?.inventoryPresent)
    field("inventoryReady",c?.inventoryReady);field("mutationReady",c?.mutationReady);field("receiptsRecovered",c?.receiptsRecovered)
    field("serverOnline",c?.serverOnline);field("sourceCurrent",c?.sourceCurrent);field("projectAvailable",c?.projectAvailable)
    fun row(provider:String,backend:Boolean?,canCreate:Boolean?){
        append("{\"provider\":\"").append(provider).append('"')
        field("backendReady",backend);field("providerCanCreate",canCreate)
        field("commonAttemptEligible",if(coherent)debugEligibilityAnd(gate,c?.serverOnline,c?.sourceCurrent,c?.projectAvailable,canCreate) else null)
        append('}')
    }
    append(",\"providers\":[");row("omp",c?.ompBackendReady,c?.ompCanCreate);append(',');row("codex",c?.codexBackendReady,c?.codexCanCreate);append("]}")
}
