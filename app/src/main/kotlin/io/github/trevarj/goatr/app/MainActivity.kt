// Four places: what needs you, where things are, the terminal you're in, and
// settings, as Navigation Compose destinations under a Material navigation
// bar (a tablet shows them on one screen instead: WideApp.kt). The bar steps aside while typing, so the terminal keeps its height.
// goatr://agents, goatr://spaces, goatr://settings and
// goatr://terminal?address=<machine/t1> open them from outside (goatr-dev:// for debug builds).

package io.github.trevarj.goatr.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.LocalActivity
import androidx.activity.compose.setContent
import android.view.InputDevice
import android.view.KeyEvent
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.core.util.Consumer
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavDestination.Companion.hasRoute
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navDeepLink
import androidx.navigation.toRoute
import io.github.trevarj.goatr.app.session.agents
import io.github.trevarj.goatr.app.ui.AgentsScreen
import io.github.trevarj.goatr.app.ui.HerdrTheme
import io.github.trevarj.goatr.app.ui.LoadingScreen
import io.github.trevarj.goatr.app.ui.Lucide
import io.github.trevarj.goatr.app.ui.SettingsScreen
import io.github.trevarj.goatr.app.ui.SpacesScreen
import io.github.trevarj.goatr.app.ui.TerminalHost
import io.github.trevarj.goatr.app.ui.TerminalScreen
import io.github.trevarj.goatr.app.ui.herdr
import herdr.core.Attention
import kotlinx.serialization.Serializable
import android.graphics.Color as AndroidColor

@Serializable
object Agents

@Serializable
object Spaces

/** `address`: a terminal to show (from a link); without it, the one last opened. */
@Serializable
data class Terminal(val address: String? = null)

@Serializable
object Settings

private data class Destination(val route: Any, val label: String, val icon: @Composable () -> Painter)

/** Each tab's icon: the bar and the screen's title show the same one. */
private val TABS = listOf(
    Destination(Agents, "Agents", { Lucide.Bot }),
    Destination(Spaces, "Spaces", { Lucide.Layers }),
    Destination(Terminal(), "Terminal", { Lucide.SquareTerminal }),
    Destination(Settings, "Settings", { Lucide.Settings }),
)

// A FragmentActivity (still a ComponentActivity) for BiometricPrompt.
class MainActivity : FragmentActivity() {
    /** The terminals, while the app shows them: a hardware keyboard types into the one on screen. */
    var terminals: TerminalHost? = null
    private var pairingInvitation by mutableStateOf<String?>(null)

    private fun takePairing(intent: Intent?) {
        val uri = intent?.data ?: return
        if (uri.scheme == BuildConfig.SCHEME && uri.host == "pair") {
            // The shared invitation grammar is release-independent. Merely opening previews; never pairs.
            pairingInvitation = uri.buildUpon().scheme("goatr").build().toString().takeIf { it.length <= 4096 }
            intent.data = null
        }
    }

    override fun onNewIntent(intent: Intent) {
        takePairing(intent)
        super.onNewIntent(intent)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // Wherever focus is (a tab, the sidebar), a hardware keyboard types into
        // the terminal on screen. A focused text field keeps its keys, and the
        // soft keyboard's already reach the view it types into.
        val hardware = event.device?.isVirtual == false &&
            event.source and InputDevice.SOURCE_KEYBOARD == InputDevice.SOURCE_KEYBOARD
        if (hardware && currentFocus?.onCheckIsTextEditor() != true && terminals?.takeHardwareKey(event) == true) return true
        return super.dispatchKeyEvent(event)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        takePairing(intent)
        setContent {
            val main: MainViewModel = appViewModel()
            val prefs by main.prefs.collectAsStateWithLifecycle()
            val loaded by main.loaded.collectAsStateWithLifecycle()
            // Android's answer can change in its settings: read it again each time the app comes back.
            LifecycleResumeEffect(Unit) {
                main.refreshLocalNetwork()
                onPauseOrDispose {}
            }
            HerdrTheme(prefs.theme) {
                val light = MaterialTheme.colorScheme.background.luminance() > 0.5f
                LaunchedEffect(light) {
                    val bars = if (light) SystemBarStyle.light(AndroidColor.TRANSPARENT, AndroidColor.TRANSPARENT)
                    else SystemBarStyle.dark(AndroidColor.TRANSPARENT)
                    enableEdgeToEdge(bars, bars)
                }
                LaunchedEffect(prefs.fullscreen) {
                    val bars = WindowCompat.getInsetsController(window, window.decorView)
                    if (prefs.fullscreen) {
                        bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                        bars.hide(WindowInsetsCompat.Type.systemBars())
                    } else {
                        bars.show(WindowInsetsCompat.Type.systemBars())
                    }
                }
                // Test tags as resource ids: what end-to-end flows (e2e/) find things by.
                Surface(Modifier.fillMaxSize().semantics { testTagsAsResourceId = true }) { if (loaded) App(main) }
                pairingInvitation?.let { raw ->
                    io.github.trevarj.goatr.app.ui.PairDesktopDialog(
                        invitation = raw, onDismiss = { pairingInvitation = null },
                    )
                }
            }
        }
    }

    override fun onStart() {
        super.onStart()
        // Back in front: a dropped connection tries again at once.
        container.live.foreground()
        container.notifier.foreground.value = true
    }

    override fun onStop() {
        super.onStop()
        container.notifier.foreground.value = false
    }
}

/** Switches tabs the Material way: one copy of each, its state saved and restored. */
private fun NavHostController.tab(route: Any) {
    val start = graph.findStartDestination().id
    // The first tab is the root: going there is going back down to it. What
    // `restoreState` keeps under its id is the tab that was above it.
    if (route == Agents) {
        popBackStack(start, inclusive = false)
        return
    }
    navigate(route) {
        popUpTo(start) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}

private inline fun <reified T : Any> links(path: String) = listOf(navDeepLink<T>(basePath = "${BuildConfig.SCHEME}://$path"))

/** A link while the app is open (it's single-task) comes as a new intent. */
@Composable
internal fun OnNewIntent(onIntent: (Intent) -> Unit) {
    val activity = LocalActivity.current as ComponentActivity
    val current by rememberUpdatedState(onIntent)
    DisposableEffect(activity) {
        val links = Consumer<Intent> { current(it) }
        activity.addOnNewIntentListener(links)
        onDispose { activity.removeOnNewIntentListener(links) }
    }
}

/** The opening screen while it lasts: once a run, until the first host is in (or you move on). */
@Composable
fun Opening(main: MainViewModel) {
    val started by main.started.collectAsStateWithLifecycle()
    if (started) return
    val hosts by main.hosts.collectAsStateWithLifecycle()
    val states by main.hostStates.collectAsStateWithLifecycle()
    Surface(Modifier.fillMaxSize()) {
        LoadingScreen(hosts, states, main::reconnect, main::start)
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun App(main: MainViewModel) {
    val nav = rememberNavController()
    val context = LocalContext.current
    val session by main.session.collectAsStateWithLifecycle()
    val prefs by main.prefs.collectAsStateWithLifecycle()

    // The live terminal belongs to the activity, not the screen: leaving the
    // Terminal tab keeps its grid and its stream.
    val scope = rememberCoroutineScope()
    val host = remember { TerminalHost(context, scope, main::client, main::sizeOf) }
    DisposableEffect(host) {
        val activity = context as? MainActivity
        activity?.terminals = host
        onDispose {
            if (activity?.terminals === host) activity.terminals = null
            host.release()
        }
    }

    // A tablet (Material's expanded width): one screen with a sidebar instead of tabs.
    if (LocalConfiguration.current.screenWidthDp >= 840) return WideApp(main, host, prefs)

    val openTerminal: (String) -> Unit = { address ->
        main.select(address)
        nav.tab(Terminal())
    }
    OnNewIntent { nav.handleDeepLink(it) }
    val entry by nav.currentBackStackEntryAsState()
    Scaffold(
        bottomBar = {
            if (!WindowInsets.isImeVisible) {
                val needs = session.agents().count { it.attention == Attention.Blocked }
                NavigationBar {
                    for (d in TABS) {
                        val on = entry?.destination?.hierarchy?.any { it.hasRoute(d.route::class) } == true
                        NavigationBarItem(
                            modifier = Modifier.semantics { contentDescription = d.label },
                            selected = on,
                            onClick = {
                                main.start()
                                nav.tab(d.route)
                            },
                            label = { Text(d.label) },
                            icon = {
                                BadgedBox(badge = {
                                    if (d.route == Agents && needs > 0) {
                                        Badge(containerColor = herdr.blocked, contentColor = herdr.background) { Text("$needs") }
                                    }
                                }) { Icon(d.icon(), null) }
                            },
                        )
                    }
                }
            }
        },
    ) { inner ->
        Box(Modifier.padding(inner).consumeWindowInsets(inner)) {
            NavHost(nav, startDestination = Agents) {
                composable<Agents>(deepLinks = links<Agents>("agents")) {
                    AgentsScreen(appViewModel(), openTerminal) { nav.tab(Settings) }
                }
                composable<Spaces>(deepLinks = links<Spaces>("spaces")) {
                    SpacesScreen(appViewModel(), openTerminal)
                }
                composable<Terminal>(deepLinks = links<Terminal>("terminal")) { e ->
                    // A link names its terminal once; after that, the tab shows whatever is opened.
                    // Picked while composing (not in an effect), so the screen's first frame already shows it.
                    val asked = e.toRoute<Terminal>().address
                    remember(asked) { asked?.also(main::select) }
                    TerminalScreen(appViewModel(), host, prefs, openTerminal, onBack = { nav.popBackStack() })
                }
                composable<Settings>(deepLinks = links<Settings>("settings")) {
                    SettingsScreen(appViewModel())
                }
            }
            // The opening screen over the tabs' content; the bar stays usable.
            Opening(main)
        }
    }
}
