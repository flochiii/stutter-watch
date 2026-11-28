package io.github.flochiii.stutterwatch

import android.content.Context
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.view.Gravity
import android.view.WindowManager
import android.widget.TextView
import java.util.Locale

class Overlay(private val context: Context) : AutoCloseable {
    private val manager=context.getSystemService(WindowManager::class.java)
    private var view: TextView?=null
    fun update(prefs: Preferences, state: ViewState) {
        if(!prefs.overlay || !state.running || !android.provider.Settings.canDrawOverlays(context)) { close(); return }
        val tr=Translations(context,prefs.language)
        val latest=state.snapshot.optJSONObject("latest")
        val metrics=latest?.optJSONObject("metrics")
        fun number(value: Double?)=value?.takeIf { it.isFinite() }?.let { String.format(Locale.ROOT,"%.0f",it) } ?: "—"
        val rows=mutableListOf("Stutter Watch")
        if(prefs.overlayFps) rows += "${tr["fps"]}: ${number(metrics?.optDouble("fps"))}"
        if(prefs.overlayCpu) rows += "CPU: ${number(latest?.optDouble("cpu"))}%"
        if(prefs.overlayRam) rows += "RAM: ${number(latest?.optDouble("ram"))}%"
        if(view==null) {
            view=TextView(context).apply {
                setTextColor(0xffe1e5eb.toInt()); setBackgroundColor(0xe6191c21.toInt()); textSize=12f
                setPadding(16,10,16,10); typeface=Typeface.createFromAsset(context.assets,"fonts/NotoSans-Regular.ttf")
            }
            val params=WindowManager.LayoutParams(WindowManager.LayoutParams.WRAP_CONTENT,WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE,PixelFormat.TRANSLUCENT)
            params.gravity=Gravity.TOP or Gravity.END; params.x=12; params.y=100
            runCatching { manager.addView(view,params) }.onFailure { view=null }
        }
        view?.text=rows.joinToString("\n")
    }
    override fun close() { view?.let { runCatching { manager.removeView(it) } }; view=null }
}
