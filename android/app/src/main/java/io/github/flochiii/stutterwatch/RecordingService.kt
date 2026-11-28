package io.github.flochiii.stutterwatch

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.os.*
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import org.json.JSONObject

class RecordingService : Service() {
    companion object {
        const val START="start"
        const val STOP="stop"
        const val PAUSE="pause"
        const val MARK="mark"
        const val CONFIGURE="configure"
        const val CHANNEL="recording"
    }
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.IO)
    private lateinit var collector: Collectors
    private lateinit var helper: HelperConnection
    private lateinit var overlay: Overlay
    private lateinit var traces: TraceController
    private var loop: Job?=null
    private var screenOffAt=0L
    private var sleeping=false
    private var closing=false
    private val wake=Channel<Unit>(Channel.CONFLATED)
    private var connectedAccess=Access.Standard
    private var notificationState=""
    private val screenReceiver=object : BroadcastReceiver() {
        override fun onReceive(context: Context?,intent: Intent?) { wake.trySend(Unit) }
    }
    override fun onCreate() {
        super.onCreate()
        collector=Collectors(this); helper=HelperConnection(this); overlay=Overlay(this); traces=TraceController(this)
        val filter=IntentFilter().apply {addAction(Intent.ACTION_SCREEN_ON);addAction(Intent.ACTION_SCREEN_OFF);addAction(Intent.ACTION_USER_PRESENT)}
        if(Build.VERSION.SDK_INT>=33) registerReceiver(screenReceiver,filter,Context.RECEIVER_NOT_EXPORTED)
        else @Suppress("DEPRECATION") registerReceiver(screenReceiver,filter)
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel(CHANNEL,"Stutter Watch",NotificationManager.IMPORTANCE_LOW).apply { setSound(null,null) })
    }
    override fun onBind(intent: Intent?): IBinder?=null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val store=PreferenceStore(this)
        val action=intent?.action
        if(action==STOP) {
            store.save(store.read().copy(desired=false,paused=false)); closing=true
            loop?.cancel()
            scope.launch { runCatching { traces.stop(helper.remote); NativeCore.request("close") }; withContext(Dispatchers.Main) { stopForeground(STOP_FOREGROUND_REMOVE); stopSelf() } }
            MonitorRuntime.update { it.copy(running=false,status="stopped") }
            return START_NOT_STICKY
        }
        if(action==START) store.save(store.read().copy(desired=true,paused=if(intent?.getBooleanExtra("manual_access",false)==true) false else store.read().paused))
        val prefs=store.read()
        if(!prefs.desired) { stopSelf(); return START_NOT_STICKY }
        if(Build.VERSION.SDK_INT>=34) startForeground(1,notification(),ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(1,notification())
        MonitorRuntime.update {it.copy(running=true)}
        wake.trySend(Unit)
        if(action==PAUSE) {
            store.save(prefs.copy(paused=!prefs.paused))
            scope.launch { runCatching { traces.stop(helper.remote); NativeCore.request("gap","paused") } }
        }
        if(action==MARK) scope.launch { runCatching { NativeCore.request("mark"); traces.protect(); MonitorRuntime.refresh() } }
        if(loop==null || loop?.isActive!=true) {
            loop=scope.launch { runRecorder(intent?.getBooleanExtra("manual_access",false)==true) }
        } else if(action==CONFIGURE) scope.launch {
            runCatching {
                NativeCore.configure(this@RecordingService)
                val access=store.read().access
                if(access!=connectedAccess || intent?.getBooleanExtra("manual_access",false)==true) {
                    traces.stop(helper.remote); NativeCore.request("gap","access_changed")
                    helper.connect(access,intent?.getBooleanExtra("manual_access",false)==true); connectedAccess=access
                }
            }
        }
        return START_STICKY
    }
    private suspend fun runRecorder(prompt: Boolean) {
        try {
            NativeCore.open(this)
            helper.connect(PreferenceStore(this).read().access,prompt)
            connectedAccess=PreferenceStore(this).read().access
            while(currentCoroutineContext().isActive) {
                val prefs=PreferenceStore(this).read()
                if(!prefs.desired) break
                helper.maintain(prefs.access)
                val interactive=collector.interactive
                if(interactive) screenOffAt=0L else if(screenOffAt==0L) screenOffAt=SystemClock.elapsedRealtime()
                val ecoSleep=prefs.energy==Energy.Eco && screenOffAt>0 && SystemClock.elapsedRealtime()-screenOffAt>=30_000
                if(ecoSleep!=sleeping) {
                    sleeping=ecoSleep
                    NativeCore.request("gap",if(sleeping) "screen_off" else "screen_on")
                    if(sleeping) {traces.stop(helper.remote);runCatching {helper.remote?.sample(prefs.target,false,false)}}
                }
                if(!prefs.paused) {
                    val extra=if(!ecoSleep) runCatching { helper.remote?.sample(prefs.target,prefs.frames,prefs.gpu)?.let(::JSONObject) }
                        .onFailure { e -> helper.accessLost(e) }.getOrNull() else null
                    NativeCore.request("ingest",collector.sample(prefs,extra).toString())
                    if(!ecoSleep && prefs.detailed) traces.tick(helper.remote,prefs.energy==Energy.Full)
                    else traces.stop(helper.remote)
                }
                MonitorRuntime.refresh()
                MonitorRuntime.update { it.copy(running=true,status=if(prefs.paused) "paused" else if(ecoSleep) "eco_idle" else "recording") }
                withContext(Dispatchers.Main) {
                    val key="${prefs.language}:${prefs.paused}:${MonitorRuntime.view.value.status}"
                    if(key!=notificationState) {
                        notificationState=key
                        getSystemService(NotificationManager::class.java).notify(1,notification())
                    }
                    overlay.update(prefs,MonitorRuntime.view.value)
                }
                withTimeoutOrNull(if(ecoSleep) 60_000L else 1000L) {wake.receive()}
            }
        } catch(e: CancellationException) { throw e }
        catch(e: Exception) { MonitorRuntime.update { it.copy(detail=e.message.orEmpty(),status="recorder_error") } }
        finally {
            runCatching { NativeCore.request("flush") }
        }
    }
    private fun notification(): Notification {
        val prefs=PreferenceStore(this).read()
        val tr=Translations(this,prefs.language)
        fun action(name: String)=PendingIntent.getService(this,name.hashCode(),Intent(this,RecordingService::class.java).setAction(name),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val open=PendingIntent.getActivity(this,0,Intent(this,MainActivity::class.java),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return NotificationCompat.Builder(this,CHANNEL).setSmallIcon(R.drawable.ic_monitor).setContentTitle("Stutter Watch")
            .setContentText(tr[if(prefs.paused) "paused" else MonitorRuntime.view.value.status]).setContentIntent(open)
            .setOngoing(true).setOnlyAlertOnce(true).setSilent(true)
            .addAction(0,tr["mark"],action(MARK)).addAction(0,tr[if(prefs.paused) "resume" else "pause"],action(PAUSE))
            .addAction(0,tr["stop"],action(STOP)).build()
    }
    override fun onDestroy() {
        loop?.cancel(); collector.close(); overlay.close(); runCatching {unregisterReceiver(screenReceiver)}
        scope.launch {
            runCatching { traces.stop(helper.remote); NativeCore.request("gap","service_stopped"); NativeCore.request("flush") }
            helper.close()
            scope.cancel()
        }
        MonitorRuntime.update { it.copy(running=false,status=if(closing) "stopped" else "interrupted") }
        super.onDestroy()
    }
}
