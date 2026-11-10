// Files for a terminal's program, from the phone. Images (picked, pasted or
// offered by the keyboard): what agents read (PNG, JPEG, GIF, WebP) goes as
// it is; anything else (a camera's HEIC), or anything big, as a JPEG small
// enough to send quickly. Other files go as they are, under their own name,
// up to what one put takes. herdr.core's `put` copies them beside the terminal.

package io.github.trevarj.goatr.app.session

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.provider.OpenableColumns
import herdr.core.PUT_LIMIT
import java.io.ByteArrayOutputStream
import kotlin.math.max

/** A file ready to put: its name (an image's extension saying what it is now) and bytes. */
class Upload(val name: String, val bytes: ByteArray)

/** Sent as they are, up to [KEEP_BYTES]: what agents read. */
private val KEPT = mapOf("image/png" to "png", "image/jpeg" to "jpg", "image/gif" to "gif", "image/webp" to "webp")

/** Bigger than this is made smaller: a phone's link is slow, and agents shrink images anyway. */
private const val KEEP_BYTES = 4 shl 20

/** The long side an image is made smaller to. */
private const val LONG_SIDE = 2048

/** Reads and readies `uri` (blocking: call off the main thread). */
fun readImage(resolver: ContentResolver, uri: Uri, now: Long = System.currentTimeMillis()): Upload {
    val type = resolver.getType(uri)
    val given = about(resolver, uri).first
    val ext = KEPT[type]
    if (ext != null) {
        val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: error("Couldn't read the image")
        if (bytes.size <= KEEP_BYTES) return Upload(named(given, ext, now), bytes)
    }
    return Upload(named(given, "jpg", now), jpeg(resolver, uri))
}

/**
 * Reads `uri` as it is, under its own name (blocking: call off the main
 * thread). One over [PUT_LIMIT] is refused before it's read: herdr-core
 * takes no more in one file, nor in pieces.
 */
fun readFile(resolver: ContentResolver, uri: Uri, now: Long = System.currentTimeMillis()): Upload {
    val (given, size) = about(resolver, uri)
    val name = fileName(given, now)
    if (size != null && size > PUT_LIMIT) error(tooBig(name))
    val bytes = resolver.openInputStream(uri)?.use { it.readAtMost(PUT_LIMIT + 1) } ?: error("Couldn't read $name")
    if (bytes.size > PUT_LIMIT) error(tooBig(name))
    return Upload(name, bytes)
}

/** What a name can't hold to stay one file: slashes either way, NUL. */
private val UNSAFE = Regex("[/\\\\\u0000]")

/** A file's name as given, as one file; one made up from the time when there's none. */
internal fun fileName(given: String?, now: Long): String =
    given?.replace(UNSAFE, "_")?.ifBlank { null } ?: "file-$now"

internal fun tooBig(name: String) = "$name is over ${PUT_LIMIT shr 20} MB, the most herdr-core takes in one file"

/** Up to `limit` bytes (InputStream.readNBytes is API 33). */
private fun java.io.InputStream.readAtMost(limit: Int): ByteArray {
    val out = ByteArrayOutputStream()
    val buffer = ByteArray(64 shl 10)
    while (out.size() < limit) {
        val n = read(buffer, 0, minOf(buffer.size, limit - out.size()))
        if (n < 0) break
        out.write(buffer, 0, n)
    }
    return out.toByteArray()
}

/** `given` with `ext` as its extension; one made up from the time when there's none. */
internal fun named(given: String?, ext: String, now: Long): String {
    val stem = given?.substringBeforeLast('.')?.replace(UNSAFE, "_")?.ifBlank { null } ?: "image-$now"
    return "$stem.$ext"
}

/** Its name and size, as its provider gives them (either may be missing). */
private fun about(resolver: ContentResolver, uri: Uri): Pair<String?, Long?> = runCatching {
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
        if (!c.moveToFirst()) return@use null
        c.getString(0) to (if (c.isNull(1)) null else c.getLong(1))
    }
}.getOrNull() ?: (null to null)

/** The image as a JPEG, its long side at most [LONG_SIDE], under what one put takes. */
private fun jpeg(resolver: ContentResolver, uri: Uri): ByteArray {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(resolver, uri)) { decoder, info, _ ->
        val long = max(info.size.width, info.size.height)
        if (long > LONG_SIDE) {
            val scale = LONG_SIDE.toFloat() / long
            decoder.setTargetSize((info.size.width * scale).toInt().coerceAtLeast(1), (info.size.height * scale).toInt().coerceAtLeast(1))
        }
        // Software: compress() reads its pixels.
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
    }
    try {
        for (quality in listOf(88, 75, 60)) {
            val out = ByteArrayOutputStream()
            bitmap.compress(Bitmap.CompressFormat.JPEG, quality, out)
            if (out.size() <= PUT_LIMIT) return out.toByteArray()
        }
        error("The image is too big to send")
    } finally {
        bitmap.recycle()
    }
}
