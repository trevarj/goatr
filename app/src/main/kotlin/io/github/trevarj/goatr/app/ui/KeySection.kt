// Settings' key section: the phone's public key and how to install it, and
// the key itself: shown (behind the device's lock), or replaced by a new one.
// One key per device, made here: no importing one from elsewhere.

package io.github.trevarj.goatr.app.ui

import android.content.ClipData
import android.os.Build
import android.os.PersistableBundle
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_WEAK
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedCard
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.trevarj.goatr.app.DialogState
import io.github.trevarj.goatr.app.SettingsViewModel
import io.github.trevarj.goatr.app.session.installCommand
import io.github.trevarj.goatr.app.session.shortName
import dev.herdr.ssh.fingerprintOf
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
internal fun KeySection(vm: SettingsViewModel) {
    val context = LocalContext.current
    val clipboard = LocalClipboard.current
    val public by vm.publicKey.collectAsStateWithLifecycle()
    val hosts by vm.hosts.collectAsStateWithLifecycle()
    var copied by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(copied) {
        if (copied != null) {
            delay(1500)
            copied = null
        }
    }
    val scope = rememberCoroutineScope()
    fun copyText(text: String, secret: Boolean) {
        scope.launch { clipboard.setClipEntry(clipOf(text, secret)) }
    }
    fun copy(what: String) {
        val k = public ?: return
        copyText(if (what == "command") installCommand(k) else k, secret = false)
        copied = what
    }
    // What's open: the private key on screen, or the new-key question.
    var shown by remember { mutableStateOf<String?>(null) }
    var renewing by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<String?>(null) }

    SectionTitle("This phone's key", Lucide.KeyRound) {
        val dim = MaterialTheme.colorScheme.onSurfaceVariant
        IconButton(
            {
                val activity = context as? FragmentActivity ?: return@IconButton
                unlock(activity, "Show the private key", { scope.launch { shown = vm.privateKey() } }) { problem = it }
            },
            Modifier.size(32.dp),
        ) { Icon(Lucide.Eye, "Show private key", Modifier.size(18.dp), tint = dim) }
        IconButton({ renewing = true }, Modifier.size(32.dp)) { Icon(Lucide.RefreshCw, "New key", Modifier.size(18.dp), tint = dim) }
    }
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(
            "Run once on each host. The key can only run hc connect, but herdr acts as your user there.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedCard(Modifier.fillMaxWidth()) {
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                SelectionContainer {
                    Text(public ?: "Loading the key…", style = MaterialTheme.typography.bodySmall.copy(fontFamily = GeistMono))
                }
                public?.let {
                    Text(
                        fingerprintOf(it),
                        style = MaterialTheme.typography.labelSmall.copy(fontFamily = GeistMono),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button({ copy("command") }, enabled = public != null) {
                Icon(Lucide.Copy, null, Modifier.size(16.dp))
                Text(if (copied == "command") "Copied" else "Copy install command", Modifier.padding(start = 8.dp))
            }
            FilledTonalButton({ copy("key") }, enabled = public != null) {
                Text(if (copied == "key") "Copied" else "Key only")
            }
        }
        problem?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
    }

    shown?.let { private ->
        AlertDialog(
            onDismissRequest = { shown = null },
            title = { Text("Private key") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        "Anyone with this can log in as you wherever the key is installed.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                    SelectionContainer(Modifier.heightIn(max = 320.dp).verticalScroll(rememberScrollState())) {
                        Text(private, style = MaterialTheme.typography.bodySmall.copy(fontFamily = GeistMono))
                    }
                }
            },
            confirmButton = {
                TextButton({
                    copyText(private, secret = true)
                    shown = null
                }) { Text("Copy") }
            },
            dismissButton = { TextButton({ shown = null }) { Text("Close") } },
        )
    }

    if (renewing) {
        // Every host added here trusts the current key; none will take the new one until it's added there.
        val names = hosts.map(::shortName).distinct()
        val detail = if (names.isEmpty()) "The current key is replaced for good."
        else "${names.joinToString(", ")} ${if (names.size == 1) "trusts" else "trust"} the current key. " +
            "After this, you'll have to add the new public key on ${if (names.size == 1) "it" else "each of them"} again."
        AppDialog(DialogState("Make a new key?", detail, "Replace key", destructive = true) {
            renewing = false
            scope.launch { runCatching { vm.regenerateKey() }.onFailure { problem = it.message } }
        }) { renewing = false }
    }
}

/**
 * Asks for the device's lock (fingerprint, face, PIN, pattern) before [then].
 * No lock set up, or it failed: [refused] says why. Cancelling says nothing.
 */
private fun unlock(activity: FragmentActivity, title: String, then: () -> Unit, refused: (String) -> Unit) {
    // Strong biometrics or the screen lock; before Android 11 that pair can't be asked for, so weak ones count too.
    val allowed = if (Build.VERSION.SDK_INT >= 30) BIOMETRIC_STRONG or DEVICE_CREDENTIAL else BIOMETRIC_WEAK or DEVICE_CREDENTIAL
    if (BiometricManager.from(activity).canAuthenticate(allowed) != BiometricManager.BIOMETRIC_SUCCESS) {
        return refused("Set a screen lock on this device first.")
    }
    val prompt = BiometricPrompt(
        activity,
        ContextCompat.getMainExecutor(activity),
        object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = then()
            override fun onAuthenticationError(code: Int, message: CharSequence) {
                val cancelled = code == BiometricPrompt.ERROR_USER_CANCELED ||
                    code == BiometricPrompt.ERROR_NEGATIVE_BUTTON || code == BiometricPrompt.ERROR_CANCELED
                if (!cancelled) refused(message.toString())
            }
        },
    )
    prompt.authenticate(BiometricPrompt.PromptInfo.Builder().setTitle(title).setAllowedAuthenticators(allowed).build())
}

/** For the clipboard; a [secret] is marked sensitive, so Android doesn't preview it. */
private fun clipOf(text: String, secret: Boolean): ClipEntry {
    val clip = ClipData.newPlainText(if (secret) "SSH private key" else "SSH public key", text)
    if (secret) clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    return ClipEntry(clip)
}
