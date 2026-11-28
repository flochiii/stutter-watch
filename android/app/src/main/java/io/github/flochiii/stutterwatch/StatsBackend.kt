package io.github.flochiii.stutterwatch

import android.content.ComponentName
import android.os.*
import android.system.OsConstants
import org.json.JSONObject
import java.io.File
import java.lang.reflect.Proxy
import java.util.concurrent.*

/** Runs only in an authorized shell/root process. No writable sysfs operations. */
class StatsBackend : AutoCloseable {
    private val pool = Executors.newFixedThreadPool(2)
    private var lastCpu: Pair<Long, Long>? = null
    private var taskId = -1
    private var targetPackage = ""
    private var layer = ""
    private var lastTaskAt = 0L
    private var frameCallback: Any? = null
    private var windowManager: Any? = null
    @Volatile private var fps = Float.NaN
    @Volatile private var fpsAt = 0L
    private var gfxFrames = GfxFrames()
    private var trace: java.lang.Process? = null
    private var traceThread: Thread? = null
    @Volatile private var traceBytes = 0L
    @Volatile private var traceError = ""
    private var traceId = ""

    private fun service(name: String): IBinder = Class.forName("android.os.ServiceManager")
        .getMethod("getService", String::class.java).invoke(null, name) as IBinder

    private fun dump(name: String, vararg arguments: String): String {
        val pipe = ParcelFileDescriptor.createPipe()
        val reader = pool.submit<String> {
            ParcelFileDescriptor.AutoCloseInputStream(pipe[0]).use { input ->
                val bytes = limited(input, 1024 * 1024 + 1)
                require(bytes.size <= 1024 * 1024) { "System dump exceeds limit" }
                bytes.toString(Charsets.UTF_8)
            }
        }
        val writer = pool.submit {
            try { service(name).dump(pipe[1].fileDescriptor, arguments) }
            finally { runCatching { pipe[1].close() } }
        }
        return try { reader.get(2, TimeUnit.SECONDS).also { writer.get(2, TimeUnit.SECONDS) } }
        finally { runCatching { pipe[0].close() }; runCatching { pipe[1].close() }; reader.cancel(true); writer.cancel(true) }
    }

    private fun limited(input: java.io.InputStream, maximum: Int): ByteArray {
        val output = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(8192)
        while (output.size() < maximum) {
            val count = input.read(buffer, 0, minOf(buffer.size, maximum-output.size()))
            if (count < 0) break
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }

    private fun task(requested: String) {
        if (SystemClock.elapsedRealtime() - lastTaskAt < 1500 && requested == targetPackage) return
        lastTaskAt = SystemClock.elapsedRealtime()
        val manager = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
        val info = if (requested.isBlank()) {
            manager.javaClass.methods.first { it.name in setOf("getFocusedRootTaskInfo", "getFocusedStackInfo") && it.parameterCount == 0 }.apply { isAccessible = true }.invoke(manager)
        } else {
            @Suppress("UNCHECKED_CAST") val all = manager.javaClass.methods.first { it.name in setOf("getAllRootTaskInfos", "getAllStackInfos") && it.parameterCount == 0 }.apply { isAccessible = true }.invoke(manager) as List<Any>
            all.firstOrNull { (it.javaClass.getField("topActivity").get(it) as? ComponentName)?.packageName == requested }
        }
        val pkg = info?.let { it.javaClass.getField("topActivity").get(it) as? ComponentName }?.packageName ?: ""
        val id = info?.let { runCatching { it.javaClass.getField("taskId").getInt(it) }.getOrElse { _ -> it.javaClass.getField("stackId").getInt(it) } } ?: -1
        if (pkg != targetPackage || id != taskId) {
            unregisterFps()
            targetPackage = pkg; taskId = id; layer = ""; gfxFrames = GfxFrames()
            if (id >= 0 && Build.VERSION.SDK_INT >= 31) runCatching { registerFps(id) }
        }
    }

    private fun registerFps(task: Int) {
        val manager = Class.forName("android.view.WindowManagerGlobal").getMethod("getWindowManagerService").invoke(null)
        val register = manager.javaClass.methods.first { it.name == "registerTaskFpsCallback" }
        val callbackType = register.parameterTypes[1]
        val descriptor = callbackType.name
        val binder = object : Binder() {
            override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
                if (code == INTERFACE_TRANSACTION) { reply?.writeString(descriptor); return true }
                if (code == FIRST_CALL_TRANSACTION) {
                    data.enforceInterface(descriptor)
                    val value = data.readFloat()
                    if (value.isFinite() && value in 0f..1000f) { fps = value; fpsAt = SystemClock.elapsedRealtime() }
                    return true
                }
                return super.onTransact(code, data, reply, flags)
            }
        }
        val callback = Proxy.newProxyInstance(callbackType.classLoader, arrayOf(callbackType)) { proxy, method, args ->
            when (method.name) {
                "asBinder" -> binder
                "toString" -> "StutterWatchFrameCallback"
                "hashCode" -> System.identityHashCode(proxy)
                "equals" -> proxy === args?.get(0)
                else -> null
            }
        }
        register.apply { isAccessible = true }.invoke(manager, task, callback)
        windowManager = manager; frameCallback = callback
    }
    private fun unregisterFps() {
        val callback = frameCallback; val manager = windowManager
        if (callback != null && manager != null) runCatching { manager.javaClass.methods.first { it.name == "unregisterTaskFpsCallback" }.apply { isAccessible = true }.invoke(manager, callback) }
        frameCallback = null; windowManager = null; fps = Float.NaN; fpsAt = 0L
    }

    @Synchronized fun sample(requested: String, frames: Boolean, gpu: Boolean): String {
        require(requested.isEmpty() || requested.matches(Regex("[A-Za-z][A-Za-z0-9_.]{1,199}")))
        val result = JSONObject().put("provider", if (Process.myUid() == 0) "root" else "shizuku")
        runCatching {
            val values = File("/proc/stat").bufferedReader().use { it.readLine() }.trim().split(Regex("\\s+")).drop(1).map(String::toLong)
            if (values.size >= 4) {
                val total = values.take(8).sum(); val idle = values[3] + values.getOrElse(4) { 0 }
                lastCpu?.let { (oldTotal, oldIdle) -> if (total > oldTotal && idle >= oldIdle) result.put("cpu_pct", (100.0 * (1.0 - (idle-oldIdle).toDouble()/(total-oldTotal))).coerceIn(0.0,100.0)) }
                lastCpu = total to idle
            }
        }.onFailure { result.put("cpu_reason", "system_source_unavailable") }
        if (gpu) {
            runCatching { File("/sys/class/kgsl/kgsl-3d0/devfreq/cur_freq").readText().trim().toDouble()/1_000_000.0 }.getOrNull()?.let { result.put("gpu_frequency_mhz", it) }
        }
        if (frames) {
            runCatching { task(requested) }.onFailure { result.put("frame_reason", "task_source_unavailable") }
            result.put("target_package", targetPackage).put("fps_task_id", taskId)
            if (fps.isFinite() && SystemClock.elapsedRealtime()-fpsAt <= 3000) {
                result.put("fps", fps).put("fps_sample_elapsed_ms", fpsAt).put("frame_source", "android_task_fps")
            }
            if (targetPackage.isNotBlank()) {
                if (!fps.isFinite() || SystemClock.elapsedRealtime()-fpsAt > 3000) runCatching {
                    if (layer.isEmpty()) {
                        val layers = dump("SurfaceFlinger", "--list").lines().filter { it.contains(targetPackage) && !it.contains("Snapshot") }
                        layer = layers.firstOrNull { it.contains("SurfaceView") } ?: layers.firstOrNull { it.contains("BLAST") } ?: layers.lastOrNull().orEmpty()
                    }
                    if (layer.isNotBlank()) result.put("frame_stats", dump("SurfaceFlinger", "--latency", layer)).put("layer", layer)
                }
                runCatching {
                    val stats = dump("gfxinfo", targetPackage, "framestats")
                    val reading = gfxFrames.read(stats, SystemClock.uptimeMillis() * 1_000_000)
                    result.put("deadline_misses", reading.misses).put("deadline_source", "gfxinfo")
                    reading.longestMs?.let { result.put("frame_duration_ms", it) }
                }
            }
            if (!result.has("fps") && !result.has("frame_stats")) result.put("frame_reason", if (targetPackage.isBlank()) "no_active_task" else "frame_source_unavailable")
        } else { unregisterFps(); taskId = -1; gfxFrames = GfxFrames() }
        return result.toString()
    }

    @Synchronized fun startTrace(destination: ParcelFileDescriptor, full: Boolean): String {
        check(trace?.isAlive != true) { "A trace is already active" }
        traceId = "stutter-watch-${Process.myPid()}-${SystemClock.elapsedRealtime()}"
        traceBytes = 0; traceError = ""
        val scheduling = if (full) "ftrace_events: \"sched/sched_switch\" ftrace_events: \"sched/sched_waking\"" else ""
        val config = """
            unique_session_name: "$traceId"
            duration_ms: 60000
            buffers { size_kb: 8192 fill_policy: RING_BUFFER }
            write_into_file: true
            file_write_period_ms: 1000
            max_file_size_bytes: 16777216
            incremental_state_config { clear_period_ms: 5000 }
            data_sources { config { name: "android.surfaceflinger.frametimeline" } }
            data_sources { config { name: "linux.sys_stats" sys_stats_config { stat_period_ms: 1000 meminfo_period_ms: 1000 cpufreq_period_ms: 1000 } } }
            data_sources { config { name: "linux.process_stats" process_stats_config { scan_all_processes_on_start: true proc_stats_poll_ms: 1000 } } }
            data_sources { config { name: "linux.ftrace" ftrace_config { ftrace_events: "power/cpu_frequency" ftrace_events: "power/suspend_resume" $scheduling } } }
        """.trimIndent()
        val process = ProcessBuilder("/system/bin/perfetto", "--txt", "-c", "-", "-o", "-").start()
        trace = process
        process.outputStream.use { it.write(config.toByteArray()) }
        Thread({ process.errorStream.use { input -> traceError = limited(input,2048).toString(Charsets.UTF_8) } }, "trace-errors").start()
        traceThread = Thread({
            runCatching {
                ParcelFileDescriptor.AutoCloseOutputStream(destination).use { output -> process.inputStream.use { input ->
                    val buffer = ByteArray(32768)
                    while (true) {
                        val count = input.read(buffer); if (count < 0) break
                        if (traceBytes + count > 17L*1024*1024) { signalTrace(process); error("Trace exceeds limit") }
                        output.write(buffer,0,count); traceBytes += count
                    }
                } }
            }.onFailure { traceError = it.message.orEmpty() }
        }, "trace-output").also { it.start() }
        return traceStatus()
    }
    private fun signalTrace(process: java.lang.Process) {
        val pid = runCatching { process.javaClass.getMethod("pid").invoke(process) as Long }.getOrNull()?.toInt()
            ?: runCatching { process.javaClass.getDeclaredField("pid").apply { isAccessible=true }.getInt(process) }.getOrNull()
        if (pid != null) Process.sendSignal(pid, OsConstants.SIGINT) else process.destroy()
    }
    @Synchronized fun stopTrace(): String {
        trace?.let { if (it.isAlive) { signalTrace(it); if (!it.waitFor(5,TimeUnit.SECONDS)) it.destroy() } }
        traceThread?.join(2000)
        return traceStatus()
    }
    fun traceStatus(): String = JSONObject().put("id",traceId).put("running",trace?.isAlive==true)
        .put("bytes",traceBytes).put("detail",traceError).put("exit",trace?.takeIf { !it.isAlive }?.exitValue() ?: JSONObject.NULL).toString()
    override fun close() { stopTrace(); unregisterFps(); pool.shutdownNow() }
}

open class StatsBinder : IStats.Stub() {
    private val backend = StatsBackend()
    override fun attach(client: IBinder) { client.linkToDeath({ destroy() },0) }
    override fun sample(target: String, frames: Boolean, gpu: Boolean) = backend.sample(target,frames,gpu)
    override fun startTrace(output: ParcelFileDescriptor, full: Boolean) = backend.startTrace(output,full)
    override fun stopTrace() = backend.stopTrace()
    override fun traceStatus() = backend.traceStatus()
    override fun destroy() { backend.close() }
}

class ShizukuStatsService : StatsBinder()
