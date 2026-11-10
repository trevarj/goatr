// Android 17's local network permission: without it the app can't reach a
// host on the LAN (name.local, 192.168.x.x). Asked for the first time a host
// needs it; a host it blocks waits for it instead of retrying.

package io.github.trevarj.goatr.app.session

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import java.net.Inet6Address
import java.net.InetAddress

class LocalNetwork(private val context: Context) {
    private val _allowed = MutableStateFlow(check())
    val allowed: StateFlow<Boolean> = _allowed

    /** Android's answer can change in its settings: read it again (back in front, after asking). */
    fun refresh() {
        _allowed.value = check()
    }

    private fun check() =
        Build.VERSION.SDK_INT < 37 || context.checkSelfPermission(PERMISSION) == PackageManager.PERMISSION_GRANTED

    companion object {
        const val PERMISSION = Manifest.permission.ACCESS_LOCAL_NETWORK
    }
}

/** A host the permission blocks. */
class LocalNetworkBlocked : Exception("Goatr can't reach devices on your local network.")

/** On the local network: a `.local` name, or one that resolves to a private or link-local address. */
suspend fun isLocal(host: String): Boolean = withContext(Dispatchers.IO) {
    if (host.endsWith(".local", ignoreCase = true)) return@withContext true
    val addresses = runCatching { InetAddress.getAllByName(host) }.getOrDefault(emptyArray())
    addresses.any {
        // fc00::/7, IPv6's private range, which isSiteLocalAddress (fec0::/10) misses.
        it.isSiteLocalAddress || it.isLinkLocalAddress || (it is Inet6Address && it.address[0].toInt() and 0xfe == 0xfc)
    }
}
