package io.github.flochiii.stutterwatch

import android.content.*
import android.content.pm.PackageManager
import android.os.*
import com.topjohnwu.superuser.Shell
import com.topjohnwu.superuser.ipc.RootService
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import rikka.shizuku.Shizuku

class HelperConnection(private val context: Context) {
    @Volatile var remote: IStats? = null
        private set
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutex = Mutex()
    private val token = Binder()
    private var current = Access.Standard
    private val args = Shizuku.UserServiceArgs(ComponentName(context, ShizukuStatsService::class.java))
        .daemon(false).processNameSuffix("stats").version(BuildConfig.VERSION_CODE)
    private var connection: ServiceConnection? = null
    private var timeout: Job? = null
    @Volatile private var retryAt = 0L
    @Volatile private var binding = false
    @Volatile private var closed = false

    /** Remote initialization and cleanup never run on the UI thread. */
    suspend fun connect(access: Access, prompt: Boolean) = mutex.withLock {
        if (closed) return@withLock
        detach()
        current = access
        retryAt = SystemClock.elapsedRealtime() + 60_000
        if (access == Access.Standard) {
            MonitorRuntime.update { it.copy(helper = "standard") }
            return@withLock
        }
        val allowed = when (access) {
            Access.Shizuku -> runCatching {
                Shizuku.pingBinder() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
            }.getOrDefault(false)
            Access.Root -> prompt || Shell.isAppGrantedRoot() == true
            else -> false
        }
        if (!allowed) {
            MonitorRuntime.update { it.copy(helper = if (access == Access.Root) "root_required" else "shizuku_required") }
            return@withLock
        }
        binding = true
        MonitorRuntime.update { it.copy(helper = "connecting", detail = "") }
        val next = object : ServiceConnection {
            override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
                val callback = this
                scope.launch {
                    mutex.withLock {
                        if (connection !== callback || closed) return@withLock
                        runCatching {
                            requireNotNull(service)
                            val client = IStats.Stub.asInterface(service)
                            client.attach(token)
                            service.linkToDeath({ lost(service) }, 0)
                            remote = client
                            binding = false
                            timeout?.cancel(); timeout = null
                            MonitorRuntime.update { it.copy(helper = if (access == Access.Root) "root" else "shizuku", detail = "") }
                        }.onFailure { failure("helper_start_failed", it) }
                    }
                }
            }
            override fun onServiceDisconnected(name: ComponentName?) = lost(null)
            override fun onBindingDied(name: ComponentName?) = lost(null)
            override fun onNullBinding(name: ComponentName?) = lost(null)
            private fun lost(service: IBinder?) {
                val callback = this
                scope.launch {
                    mutex.withLock {
                        if (connection === callback && (service == null || remote?.asBinder() === service)) failure("access_lost", null)
                    }
                }
            }
        }
        connection = next
        runCatching {
            withContext(Dispatchers.Main) {
                if (access == Access.Shizuku) Shizuku.bindUserService(args, next)
                else RootService.bind(Intent(context, RootStatsService::class.java), next)
            }
        }.onFailure { failure("helper_start_failed", it) }
        if (binding) timeout = scope.launch {
            delay(35_000)
            mutex.withLock { if (connection === next && binding) failure("helper_start_failed", null) }
        }
    }

    /** Retry only previously authorized access; never reopen a root prompt in the background. */
    fun maintain(access: Access) {
        if (!closed && access != Access.Standard && remote == null && !binding && SystemClock.elapsedRealtime() >= retryAt) {
            retryAt = SystemClock.elapsedRealtime() + 60_000
            scope.launch { connect(access, false) }
        }
    }
    fun accessLost(error: Throwable) { scope.launch { mutex.withLock { failure("access_lost", error) } } }
    private suspend fun failure(reason: String, error: Throwable?) {
        detach()
        MonitorRuntime.update { it.copy(helper = reason, detail = error?.javaClass?.simpleName.orEmpty()) }
    }
    private suspend fun detach() {
        val timer = timeout; timeout = null
        if (timer !== currentCoroutineContext()[Job]) timer?.cancel()
        val old = remote; remote = null
        val oldConnection = connection; connection = null
        binding = false
        if (oldConnection != null) withContext(Dispatchers.Main) {
            when (current) {
                Access.Shizuku -> runCatching { Shizuku.unbindUserService(args, oldConnection, true) }
                Access.Root -> runCatching { RootService.unbind(oldConnection) }
                else -> Unit
            }
        }
        runCatching { old?.destroy() }
    }
    suspend fun close() {
        mutex.withLock { closed = true; detach() }
        scope.cancel()
    }
}
