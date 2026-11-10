package io.github.trevarj.goatr.app.session

import kotlin.test.Test
import kotlin.test.assertEquals

class UploadsTest {
    @Test
    fun keepsTheNameWithWhatItIsNow() {
        assertEquals("PXL_20261108.png", named("PXL_20261108.png", "png", 0))
        // A camera's HEIC goes as a JPEG: its name says so.
        assertEquals("IMG_0042.jpg", named("IMG_0042.HEIC", "jpg", 0))
        assertEquals("archive.tar.jpg", named("archive.tar.gz", "jpg", 0))
    }

    @Test
    fun noNameIsMadeUp() {
        assertEquals("image-1700000000000.png", named(null, "png", 1_700_000_000_000))
        assertEquals("image-5.jpg", named(".jpg", "jpg", 5))
    }

    @Test
    fun aNameIsOneFile() {
        assertEquals("a_b.png", named("a/b.png", "png", 0))
    }

    @Test
    fun aFileKeepsItsName() {
        assertEquals("build log (2).txt", fileName("build log (2).txt", 0))
        assertEquals("a_b.csv", fileName("a/b.csv", 0))
        assertEquals("a_b.csv", fileName("a\\b.csv", 0))
        assertEquals("file-7", fileName(null, 7))
        assertEquals("file-7", fileName(" ", 7))
    }

    @Test
    fun tooBigSaysWhatTakesIt() {
        assertEquals("dump.sql is over 8 MB, the most herdr-core takes in one file", tooBig("dump.sql"))
    }
}
