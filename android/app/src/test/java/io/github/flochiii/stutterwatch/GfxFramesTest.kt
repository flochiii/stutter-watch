package io.github.flochiii.stutterwatch

import org.junit.Assert.*
import org.junit.Test

class GfxFramesTest {
    private fun dump(vararg rows: String) = "Janky frames: 50\nFlags,IntendedVsync,FrameDeadline,FrameCompleted,\n" + rows.joinToString("\n")
    @Test fun oldFramesAndStaticGapsAreNotNewJank() {
        val reader = GfxFrames()
        val old = "0,1000000000,1016000000,1024000000,"
        assertEquals(0L, reader.read(dump(old), 1_100_000_000).misses)
        assertEquals(0L, reader.read(dump(old), 1_200_000_000).misses)
        assertNull(reader.read(dump(old), 1_300_000_000).longestMs)
        val next = reader.read(dump(old, "0,2000000000,2016000000,2024000000,"), 2_100_000_000)
        assertEquals(1L, next.misses)
        assertEquals(24.0, next.longestMs!!, 0.001)
    }
    @Test fun invalidFramesAreIgnoredAndRealLongFrameIsMeasured() {
        val reader = GfxFrames()
        reader.read(dump(), 1_000_000_000)
        val reading = reader.read(dump("0,1000000000,1016000000,1150000000,", "1,1000000000,1016000000,1900000000,", "0,2000000000,2016000000,9223372036854775807,"), 1_200_000_000)
        assertEquals(1L, reading.misses)
        assertEquals(150.0, reading.longestMs!!, 0.001)
    }
}
