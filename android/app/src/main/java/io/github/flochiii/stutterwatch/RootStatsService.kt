package io.github.flochiii.stutterwatch

import android.content.Intent
import android.os.IBinder
import com.topjohnwu.superuser.ipc.RootService

class RootStatsService : RootService() {
    private val binder = StatsBinder()
    override fun onBind(intent: Intent): IBinder = binder
    override fun onDestroy() { binder.destroy(); super.onDestroy() }
}
