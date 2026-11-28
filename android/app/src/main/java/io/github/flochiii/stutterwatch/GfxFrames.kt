package io.github.flochiii.stutterwatch

/** Reads actual frame work, without treating a static window's low FPS as a stall. */
class GfxFrames {
    data class Reading(val misses: Long, val longestMs: Double?)
    private var lastFrame = 0L
    private var lastCount: Long? = null
    private var initialized = false

    fun read(text: String, nowUptimeNs: Long): Reading {
        var columns = emptyList<String>()
        val frames = sortedMapOf<Long, Pair<Double, Boolean?>>()
        for (line in text.lineSequence()) {
            if (line.startsWith("Flags,")) { columns = line.trimEnd(',').split(','); continue }
            if (columns.isEmpty() || !line.startsWith("0,")) continue
            val fields = line.trimEnd(',').split(',')
            fun number(name: String): Long? = fields.getOrNull(columns.indexOf(name))?.toLongOrNull()
            val intended = number("IntendedVsync") ?: continue
            val completed = number("FrameCompleted") ?: continue
            if (intended <= 0 || completed <= intended || completed > nowUptimeNs || nowUptimeNs - completed > 5_000_000_000L) continue
            val duration = (completed - intended) / 1_000_000.0
            if (duration > 10_000) continue
            val deadline = number("FrameDeadline")?.takeIf { it > intended && it < Long.MAX_VALUE }
            frames[intended] = duration to deadline?.let { completed > it }
        }
        val count = Regex("Janky frames:\\s*(\\d+)").find(text)?.groupValues?.get(1)?.toLongOrNull()
        val fresh = frames.filterKeys { it > lastFrame }.values
        val misses = if (fresh.any { it.second != null }) fresh.count { it.second == true }.toLong()
            else if (count != null && lastCount != null && count >= lastCount!!) count - lastCount!! else 0L
        lastCount = count
        lastFrame = maxOf(lastFrame, frames.lastKeyOrNull() ?: 0)
        if (!initialized) { initialized = true; return Reading(0, null) }
        return Reading(misses, fresh.maxOfOrNull { it.first })
    }
    private fun <K, V> java.util.SortedMap<K, V>.lastKeyOrNull(): K? = if (isEmpty()) null else lastKey()
}
