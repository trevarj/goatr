// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import org.junit.Assert.*
import org.junit.Test

/** Actual ZXing QR -> padded/cropped/rotated CameraX luminance decoder. Not camera input proof. */
class QrDecoderTest {
    @Test fun actualDecoderHandlesPaddedPixelStrideCropAllRightAnglesAndBounds() {
        val value="synthetic never-authorized QR decoder sample"
        val side=320
        val qr=QRCodeWriter().encode(value,BarcodeFormat.QR_CODE,side,side,mapOf(EncodeHintType.CHARACTER_SET to "UTF-8",EncodeHintType.MARGIN to 4))
        val width=side+8;val height=side+8;val rowStride=width*2+13
        val bytes=ByteArray(rowStride*height) {255.toByte()}
        for(y in 0 until side) for(x in 0 until side) bytes[(y+3)*rowStride+(x+4)*2]=if(qr[x,y]) 0 else 255.toByte()
        val original=bytes.copyOf()
        for(rotation in listOf(0,90,180,270,-90,450)) assertEquals(value,decodeQrFrame(bytes,width,height,rowStride,rotation,2,4,3,side,side))
        val inverted=ByteArray(bytes.size) {(255-(bytes[it].toInt() and 255)).toByte()}
        val invertedOriginal=inverted.copyOf()
        for(rotation in listOf(0,90,180,270,-90,450)) assertEquals("Inverted native QR polarity must decode at rotation $rotation",value,decodeQrFrame(inverted,width,height,rowStride,rotation,2,4,3,side,side))
        assertArrayEquals(invertedOriginal,inverted)
        assertArrayEquals("Decoder must not mutate an owned CameraX input plane",original,bytes)
        assertNull(decodeQrFrame(bytes,width,height,rowStride,45,2,4,3,side,side))
        assertNull(decodeQrFrame(bytes,width,height,rowStride,0,Int.MAX_VALUE,4,3,side,side))
        assertNull(decodeQrFrame(bytes,width,height,rowStride,0,2,Int.MAX_VALUE,3,side,side))
        assertNull(decodeQrFrame(ByteArray(1),width,height,rowStride,0,2,4,3,side,side))
        assertNull(decodeQrFrame(bytes,0,height,rowStride,0))
    }
}
