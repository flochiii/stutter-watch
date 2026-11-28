package io.github.flochiii.stutterwatch

import android.app.ActivityManager
import android.app.Application
import android.app.ApplicationExitInfo
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.content.ContextCompat
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject
import java.io.IOException

enum class Access { Standard, Shizuku, Root }
enum class Energy { Eco, Full }

data class Preferences(
    val access: Access = Access.Standard,
    val energy: Energy = Energy.Eco,
    val language: String = "en",
    val autostart: Boolean = true,
    val desired: Boolean = false,
    val paused: Boolean = false,
    val overlay: Boolean = false,
    val overlayCpu: Boolean = true,
    val overlayRam: Boolean = true,
    val overlayFps: Boolean = true,
    val memory: Boolean = true,
    val battery: Boolean = true,
    val thermal: Boolean = true,
    val cpu: Boolean = true,
    val gpu: Boolean = false,
    val frames: Boolean = true,
    val detailed: Boolean = false,
    val target: String = "",
    val budgetMb: Int = 500,
    val retentionHours: Int = 24,
)

class PreferenceStore(private val context: Context) {
    private val prefs = context.getSharedPreferences("monitor", Context.MODE_PRIVATE)
    fun read() = Preferences(
        access = runCatching { Access.valueOf(prefs.getString("access", "Standard")!!) }.getOrDefault(Access.Standard),
        energy = runCatching { Energy.valueOf(prefs.getString("energy", "Eco")!!) }.getOrDefault(Energy.Eco),
        language = prefs.getString("language", "en") ?: "en",
        autostart = prefs.getBoolean("autostart", true), desired = prefs.getBoolean("desired", false), paused = prefs.getBoolean("paused", false),
        overlay = prefs.getBoolean("overlay", false), overlayCpu = prefs.getBoolean("overlayCpu", true), overlayRam = prefs.getBoolean("overlayRam", true), overlayFps = prefs.getBoolean("overlayFps", true),
        memory = prefs.getBoolean("memory", true), battery = prefs.getBoolean("battery", true), thermal = prefs.getBoolean("thermal", true),
        cpu = prefs.getBoolean("cpu", true), gpu = prefs.getBoolean("gpu", false), frames = prefs.getBoolean("frames", true), detailed = prefs.getBoolean("detailed", false),
        target = prefs.getString("target", "") ?: "", budgetMb = prefs.getInt("budgetMb", 500), retentionHours = prefs.getInt("retentionHours", 24),
    )
    fun save(value: Preferences) {
        prefs.edit().apply {
            putString("access", value.access.name); putString("energy", value.energy.name); putString("language", value.language)
            putBoolean("autostart", value.autostart); putBoolean("desired", value.desired); putBoolean("paused", value.paused)
            putBoolean("overlay", value.overlay); putBoolean("overlayCpu", value.overlayCpu); putBoolean("overlayRam", value.overlayRam); putBoolean("overlayFps", value.overlayFps)
            putBoolean("memory", value.memory); putBoolean("battery", value.battery); putBoolean("thermal", value.thermal)
            putBoolean("cpu", value.cpu); putBoolean("gpu", value.gpu); putBoolean("frames", value.frames); putBoolean("detailed", value.detailed)
            putString("target", value.target); putInt("budgetMb", value.budgetMb); putInt("retentionHours", value.retentionHours)
        }.apply()
        MonitorRuntime.preferences.value = value
    }
    fun respectUserStop() {
        if (Build.VERSION.SDK_INT < 30) return
        val exit = context.getSystemService(ActivityManager::class.java)
            .getHistoricalProcessExitReasons(context.packageName, 0, 4).firstOrNull() ?: return
        if (exit.timestamp > prefs.getLong("lastExit", 0)) {
            if (exit.reason == ApplicationExitInfo.REASON_USER_REQUESTED) save(read().copy(desired = false, paused = false))
            prefs.edit().putLong("lastExit", exit.timestamp).apply()
        }
    }
}

object NativeCore {
    init { System.loadLibrary("stutter_android") }
    @JvmStatic external fun exchange(command: String, payload: String): String
    fun request(command: String, payload: String = ""): JSONObject {
        val result = JSONObject(exchange(command, payload))
        if (result.has("error")) throw IOException(result.getString("error"))
        return result
    }
    fun open(context: Context) {
        configure(context,"open")
    }
    fun configure(context: Context, command: String="configure") {
        val preferences = PreferenceStore(context).read()
        request(command, JSONObject().put("directory", context.filesDir.resolve("recordings").absolutePath)
            .put("budget_mb", preferences.budgetMb).put("retention_hours", preferences.retentionHours).toString())
    }
}

data class ViewState(
    val running: Boolean = false,
    val status: String = "stopped",
    val snapshot: JSONObject = JSONObject(),
    val incidents: org.json.JSONArray = org.json.JSONArray(),
    val incident: JSONObject? = null,
    val detail: String = "",
    val helper: String = "standard",
)

object MonitorRuntime {
    val preferences = MutableStateFlow(Preferences())
    private val mutable = MutableStateFlow(ViewState())
    val view = mutable.asStateFlow()
    val io = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    fun update(transform: (ViewState) -> ViewState) { mutable.update(transform) }
    private var incidentsAt = 0L
    fun refresh(force: Boolean = false) {
        val visible = force || mutable.subscriptionCount.value > 0
        val snapshot = NativeCore.request(if(visible) "snapshot" else "latest")
        val now = android.os.SystemClock.elapsedRealtime()
        val incidents = if(visible && (force || now-incidentsAt >= 5000)) {
            incidentsAt = now
            org.json.JSONArray(NativeCore.exchange("incidents", ""))
        } else mutable.value.incidents
        update { it.copy(snapshot = snapshot, incidents = incidents) }
    }
    fun load(context: Context) = io.launch {
        runCatching { NativeCore.open(context); refresh(true) }.onFailure { e -> update { it.copy(detail = e.message ?: "", status = "recorder_error") } }
    }
    fun selectIncident(id: String) = io.launch {
        runCatching { NativeCore.request("incident", JSONObject.quote(id)) }
            .onSuccess { data -> update { it.copy(incident = data) } }
            .onFailure { e -> update { it.copy(detail = e.message ?: "") } }
    }
    fun action(context: Context, action: String, manual: Boolean = false) {
        val intent = Intent(context, RecordingService::class.java).setAction(action).putExtra("manual_access", manual)
        if (action == RecordingService.START) ContextCompat.startForegroundService(context, intent)
        else if (view.value.running || action == RecordingService.STOP) context.startService(intent)
    }
}

class MonitorApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        val store = PreferenceStore(this)
        store.respectUserStop()
        MonitorRuntime.preferences.value = store.read()
    }
}
