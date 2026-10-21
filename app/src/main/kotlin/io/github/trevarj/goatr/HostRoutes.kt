// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import java.net.URI

/** Phone-local discovery hints. The paired endpoint ID remains the only peer identity. */
data class SavedHostRoutes(val hostId: String, val revision: Long = 0, val direct: List<String> = emptyList(), val relay: String? = null)
data class RouteAttemptView(val revision: Long, val direct: List<String>, val relay: String?, val supplied: Boolean = false)
data class SelectedIrohPath(val id: String, val remoteAddr: String, val isIp: Boolean, val isRelay: Boolean)
class RouteValidation(message: String) : IllegalArgumentException(message)

internal object HostRoutes {
    fun validate(directHints: List<String>, relayHint: String?): Pair<List<String>, String?> {
        if(directHints.size>8) throw RouteValidation("Use at most eight direct addresses.")
        val addresses=directHints.map {direct(it)}
        if(addresses.distinct().size!=addresses.size) throw RouteValidation("Remove duplicate direct addresses.")
        return addresses to relayHint?.let {relay(it)}
    }
    private fun invalid(): Nothing = throw RouteValidation("Use a literal IPv4:port or [IPv6]:port, with port 1–65535.")
    fun direct(value: String): String {
        if(value.isEmpty() || value.length>128 || value.any {it.isWhitespace() || it.isISOControl()} || '%' in value) invalid()
        val ip: String; val portText: String; val isV6: Boolean
        if(value.startsWith('[')) {
            val close=value.indexOf(']')
            if(close<2 || value.getOrNull(close+1)!=':') invalid()
            ip=value.substring(1,close);portText=value.substring(close+2);isV6=true
        } else {
            if(value.count {it==':'}!=1) invalid()
            ip=value.substringBefore(':');portText=value.substringAfter(':');isV6=false
        }
        if(portText.isEmpty() || portText.any {it !in '0'..'9'}) invalid()
        val port=portText.toIntOrNull()?.takeIf {it in 1..65535} ?: invalid()
        return if(isV6) "[${ipv6(ip)}]:$port" else "${ipv4(ip).joinToString(".")}:$port"
    }
    private fun ipv4(value: String): List<Int> {
        val parts=value.split('.')
        if(parts.size!=4) invalid()
        return parts.map {part->
            if(part.isEmpty() || part.length>3 || part.any {it !in '0'..'9'} || (part.length>1 && part[0]=='0')) invalid()
            part.toIntOrNull()?.takeIf {it in 0..255} ?: invalid()
        }
    }
    private fun ipv6(value: String): String {
        if(value.isEmpty() || value.count {it==':'}<2) invalid()
        fun groups(part: String): List<Int> {
            if(part.isEmpty()) return emptyList()
            val pieces=part.split(':');val result=mutableListOf<Int>()
            pieces.forEachIndexed {index,piece->
                if('.' in piece) {
                    if(index!=pieces.lastIndex) invalid()
                    val ip=ipv4(piece);result+=ip[0]*256+ip[1];result+=ip[2]*256+ip[3]
                } else {
                    if(piece.isEmpty() || piece.length>4 || piece.any {it !in '0'..'9' && it !in 'a'..'f' && it !in 'A'..'F'}) invalid()
                    result+=piece.toInt(16)
                }
            }
            return result
        }
        val compressed=value.indexOf("::")
        val words=if(compressed>=0) {
            if(value.indexOf("::",compressed+2)>=0) invalid()
            val leftText=value.substring(0,compressed)
            if('.' in leftText) invalid()
            val left=groups(leftText);val right=groups(value.substring(compressed+2))
            val missing=8-left.size-right.size
            if(missing<1) invalid()
            left+List(missing) {0}+right
        } else groups(value).also {if(it.size!=8) invalid()}
        var bestStart=-1;var bestLength=1;var index=0
        while(index<8) {
            if(words[index]!=0) {index++;continue}
            val start=index;while(index<8 && words[index]==0) index++
            if(index-start>bestLength) {bestStart=start;bestLength=index-start}
        }
        if(bestStart<0) return words.joinToString(":") {it.toString(16)}
        val left=words.take(bestStart).joinToString(":") {it.toString(16)}
        val right=words.drop(bestStart+bestLength).joinToString(":") {it.toString(16)}
        return "$left::$right"
    }
    fun relay(value: String): String {
        fun invalidRelay(): Nothing = throw RouteValidation("Use an HTTPS relay URL with no credentials, query, fragment, or path.")
        if(value.isEmpty() || value.length>2048 || value.any {it.isWhitespace() || it.isISOControl() || it.code>127}) invalidRelay()
        val uri=try {URI(value)} catch(_: Exception) {invalidRelay()}
        if(!uri.scheme.equals("https",ignoreCase=true) || uri.isOpaque || uri.host.isNullOrEmpty() || uri.rawUserInfo!=null || uri.rawQuery!=null || uri.rawFragment!=null || uri.rawPath !in listOf("","/") || (uri.port!=-1 && uri.port !in 1..65535)) invalidRelay()
        return try {URI("https",null,uri.host.lowercase(),if(uri.port==443) -1 else uri.port,"/",null,null).toASCIIString()} catch(_: Exception) {invalidRelay()}
    }
    fun matches(path: SelectedIrohPath, hint: String, isRelay: Boolean): Boolean = runCatching {
        if(isRelay) path.isRelay && relay(path.remoteAddr)==hint else path.isIp && direct(path.remoteAddr)==hint
    }.getOrDefault(false)
}
