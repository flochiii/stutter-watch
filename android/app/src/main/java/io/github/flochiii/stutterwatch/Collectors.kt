package io.github.flochiii.stutterwatch

import android.app.ActivityManager
import android.content.*
import android.os.*
import android.os.health.SystemHealthManager
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** API measurements are scoped and optional. A failed sensor never becomes zero. */
class Collectors(private val context: Context) : AutoCloseable {
    private val activity = context.getSystemService(ActivityManager::class.java)
    private val power = context.getSystemService(PowerManager::class.java)
    private val session = UUID.randomUUID().toString()
    private var battery = Intent()
    private var previousCpu = Process.getElapsedCpuTime()
    private var previousWall = SystemClock.elapsedRealtime()
    private var headroomAt = 0L
    private var cpuHeadroom: Float? = null
    private var gpuHeadroom: Float? = null
    private var memoryAt = 0L
    private var ownMemoryBytes: Long? = null
    private val memory = ActivityManager.MemoryInfo()
    private var thermalAt = 0L
    private var thermalHeadroom: Float? = null
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) { if (intent != null) battery = intent }
    }
    init {
        val filter = IntentFilter(Intent.ACTION_BATTERY_CHANGED)
        battery = if (Build.VERSION.SDK_INT >= 33) context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED) ?: Intent()
        else @Suppress("DEPRECATION") (context.registerReceiver(receiver, filter) ?: Intent())
    }
    val interactive get() = power.isInteractive && !context.getSystemService(android.app.KeyguardManager::class.java).isKeyguardLocked

    fun sample(prefs: Preferences, privileged: JSONObject?): JSONObject {
        val elapsed = SystemClock.elapsedRealtime()
        val cpuTime = Process.getElapsedCpuTime()
        val ownCpu = if (elapsed > previousWall) 100.0 * (cpuTime - previousCpu) / (elapsed - previousWall) else null
        previousCpu = cpuTime; previousWall = elapsed
        if (elapsed - memoryAt >= 5000) {
            memoryAt = elapsed
            activity.getMemoryInfo(memory)
            ownMemoryBytes = runCatching { Debug.MemoryInfo().let { Debug.getMemoryInfo(it); it.totalPss.toLong() * 1024 } }.getOrNull()
        }
        val meta = JSONObject().put("provider", privileged?.optString("provider") ?: "standard")
            .put("session_id", session).put("elapsed_realtime_ns", SystemClock.elapsedRealtimeNanos())
            .put("own_cpu_pct", ownCpu ?: JSONObject.NULL)
            .put("own_memory_bytes", ownMemoryBytes ?: JSONObject.NULL)
        val status = JSONObject()
        fun source(name: String, available: Boolean, reason: String = "") {
            status.put(name, JSONObject().put("available", available).put("reason", reason))
        }
        val result = JSONObject().put("ts_ms", System.currentTimeMillis()).put("elapsed_ms", elapsed)
            .put("uptime_ms", SystemClock.uptimeMillis()).put("interactive", interactive)
        if (prefs.memory) {
            result.put("ram_total_bytes", memory.totalMem).put("ram_available_bytes", memory.availMem.coerceAtMost(memory.totalMem))
            meta.put("low_memory", memory.lowMemory).put("low_memory_threshold_bytes", memory.threshold)
        }
        source("memory", prefs.memory, if (prefs.memory) "" else "disabled")
        if (prefs.battery) {
            val level = battery.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = battery.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
            if (level >= 0 && scale > 0) meta.put("battery_pct", level * 100.0 / scale)
            if (battery.hasExtra(BatteryManager.EXTRA_TEMPERATURE)) meta.put("battery_temperature_c", battery.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, 0) / 10.0)
            meta.put("charging", battery.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0)
        }
        source("battery", prefs.battery && meta.has("battery_pct"), if (!prefs.battery) "disabled" else if (meta.has("battery_pct")) "" else "sensor_unavailable")
        if (prefs.thermal) {
            meta.put("thermal_status", power.currentThermalStatus)
            if (Build.VERSION.SDK_INT >= 30 && elapsed - thermalAt >= 10_000) {
                thermalAt = elapsed
                thermalHeadroom = runCatching { power.getThermalHeadroom(0) }.getOrNull()?.takeIf { it.isFinite() }
            }
            thermalHeadroom?.let { meta.put("thermal_headroom", it) }
        }
        source("thermal", prefs.thermal, if (prefs.thermal) "" else "disabled")
        if (Build.VERSION.SDK_INT >= 36 && elapsed - headroomAt >= 5000) {
            headroomAt = elapsed
            val manager = context.getSystemService(SystemHealthManager::class.java)
            cpuHeadroom = if (prefs.cpu) runCatching { manager.getCpuHeadroom(null) }.getOrNull()?.takeIf { it.isFinite() } else null
            gpuHeadroom = if (prefs.gpu) runCatching { manager.getGpuHeadroom(null) }.getOrNull()?.takeIf { it.isFinite() } else null
        }
        if(prefs.cpu) cpuHeadroom?.let { meta.put("cpu_headroom", it) }
        if(prefs.gpu) gpuHeadroom?.let { meta.put("gpu_headroom", it) }
        val localFrequency = if (prefs.cpu) runCatching {
            File("/sys/devices/system/cpu").listFiles()?.asSequence()?.filter { it.name.matches(Regex("cpu\\d+")) }
                ?.mapNotNull { runCatching { it.resolve("cpufreq/scaling_cur_freq").readText().trim().toDouble() / 1000.0 }.getOrNull() }
                ?.maxOrNull()
        }.getOrNull() else null
        localFrequency?.let { meta.put("cpu_frequency_mhz", it) }
        source("cpu", false, if (!prefs.cpu) "disabled" else "system_access_required")
        source("frames", false, if (!prefs.frames) "disabled" else "system_access_required")
        source("gpu", false, if (!prefs.gpu) "disabled" else "device_dependent")
        if (privileged != null) {
            if (prefs.cpu && privileged.has("cpu_pct")) { result.put("cpu_pct", privileged.getDouble("cpu_pct")); source("cpu", true) }
            else if (prefs.cpu) source("cpu", false, privileged.optString("cpu_reason", "warming"))
            for (key in listOf("cpu_frequency_mhz", "gpu_frequency_mhz", "cpu_temperature_c", "gpu_temperature_c", "target_package", "layer", "deadline_misses", "deadline_source", "frame_duration_ms", "fps", "fps_sample_elapsed_ms", "fps_task_id", "frame_source")) {
                if (privileged.has(key)) meta.put(key, privileged.get(key))
            }
            if (prefs.frames && privileged.has("frame_stats")) { result.put("frame_stats", privileged.getString("frame_stats")); source("frames", true) }
            else if (prefs.frames && privileged.has("fps")) source("frames", true)
            else if (prefs.frames) source("frames", false, privileged.optString("frame_reason", "frame_source_unavailable"))
            if (prefs.gpu && (meta.has("gpu_frequency_mhz") || meta.has("gpu_temperature_c"))) source("gpu", true)
        }
        return result.put("android", meta).put("status", status)
    }
    override fun close() { runCatching { context.unregisterReceiver(receiver) } }
}
