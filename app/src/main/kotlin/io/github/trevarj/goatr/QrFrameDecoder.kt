// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright 2026 Trevor Arjeski. Adapted 2026-10-08 from MOTD GPL-3.0-or-later,
// f348f44d3b5ca747e3a8751f13a277fd601ffd73 invite/QrFrameDecoder.kt:decodeQrFrame.
// Changes: overflow-safe bounds, reject non-right-angle rotation, clear luminance buffers.
package io.github.trevarj.goatr

import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.MultiFormatReader
import com.google.zxing.BarcodeFormat

private val qrHints=mapOf(
    DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
    DecodeHintType.TRY_HARDER to true,
    DecodeHintType.ALSO_INVERTED to true,
)

fun decodeQrFrame(bytes: ByteArray,width: Int,height: Int,rowStride: Int,rotationDegrees: Int,pixelStride: Int=1,cropLeft: Int=0,cropTop: Int=0,cropWidth: Int=width,cropHeight: Int=height): String? {
    if(width !in 1..4096 || height !in 1..4096 || rowStride<=0 || pixelStride<=0 || cropWidth<=0 || cropHeight<=0 || cropLeft<0 || cropTop<0 || cropLeft.toLong()+cropWidth>width || cropTop.toLong()+cropHeight>height) return null
    val rotation=(rotationDegrees%360+360)%360
    if(rotation%90!=0) return null
    val last=(cropTop.toLong()+cropHeight-1)*rowStride+(cropLeft.toLong()+cropWidth-1)*pixelStride
    if(last !in 0 until bytes.size.toLong()) return null
    val compact=ByteArray(cropWidth*cropHeight)
    for(y in 0 until cropHeight) for(x in 0 until cropWidth) compact[y*cropWidth+x]=bytes[((cropTop.toLong()+y)*rowStride+(cropLeft.toLong()+x)*pixelStride).toInt()]
    val rotated=if(rotation==0) compact else ByteArray(compact.size)
    val rw=if(rotation==90 || rotation==270) cropHeight else cropWidth
    val rh=if(rotation==90 || rotation==270) cropWidth else cropHeight
    if(rotation!=0) for(y in 0 until cropHeight) for(x in 0 until cropWidth) {
        val index=when(rotation) {90->x*cropHeight+cropHeight-y-1;180->compact.lastIndex-(y*cropWidth+x);else->(cropWidth-x-1)*cropHeight+y}
        rotated[index]=compact[y*cropWidth+x]
    }
    return try {
        val source=PlanarYUVLuminanceSource(rotated,rw,rh,0,0,rw,rh,false)
        MultiFormatReader().decode(BinaryBitmap(HybridBinarizer(source)),qrHints).text
    } catch(_: ReaderException) {null} catch(_: IllegalArgumentException) {null}
    finally {compact.fill(0);if(rotated!==compact) rotated.fill(0)}
}
