package io.github.flochiii.stutterwatch

import android.content.*
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val store=PreferenceStore(context); store.respectUserStop(); val prefs=store.read()
        if(prefs.autostart && prefs.desired && intent.action in setOf(Intent.ACTION_BOOT_COMPLETED,Intent.ACTION_MY_PACKAGE_REPLACED)) {
            runCatching { MonitorRuntime.action(context,RecordingService.START) }
        }
    }
}
class BookmarkTile : TileService() {
    override fun onStartListening() {
        qsTile?.apply { state=if(MonitorRuntime.view.value.running) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE; updateTile() }
    }
    @android.annotation.SuppressLint("StartActivityAndCollapseDeprecated") // The Intent overload is required on API 29–33.
    override fun onClick() {
        if(MonitorRuntime.view.value.running) MonitorRuntime.action(this,RecordingService.MARK)
        else {
            val intent=Intent(this,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if(android.os.Build.VERSION.SDK_INT>=34) startActivityAndCollapse(android.app.PendingIntent.getActivity(this,0,intent,android.app.PendingIntent.FLAG_IMMUTABLE))
            else @Suppress("DEPRECATION") startActivityAndCollapse(intent)
        }
    }
}
