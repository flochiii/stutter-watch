package io.github.flochiii.stutterwatch

import android.content.Context
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import org.json.JSONObject
import java.io.File

/** Owns only this app's bounded trace segments. Raw files are never uploaded. */
class TraceController(private val context: Context) {
    private val root=context.filesDir.resolve("traces").apply { mkdirs() }
    private var file: File?=null
    private var lastBackend: IStats?=null
    private var protectUntil=0L
    private var protected=false
    private var retryAt=0L
    @Synchronized fun protect() { if(file!=null) { protectUntil=SystemClock.elapsedRealtime()+10_000; protected=true } }
    @Synchronized fun tick(backend: IStats?, full: Boolean) {
        if(backend==null) { stop(lastBackend); return }
        if(file!=null) {
            val status=runCatching { JSONObject(backend.traceStatus()) }.getOrNull()
            if(status?.optBoolean("running")==false || (protected && SystemClock.elapsedRealtime()>=protectUntil)) stop(backend)
        }
        if(file==null && SystemClock.elapsedRealtime()>=retryAt) {
            maintain()
            if(root.usableSpace<96L*1024*1024) { retryAt=SystemClock.elapsedRealtime()+60_000; return }
            val current=root.resolve("trace-${System.currentTimeMillis()}.partial")
            val fd=ParcelFileDescriptor.open(current,ParcelFileDescriptor.MODE_CREATE or ParcelFileDescriptor.MODE_TRUNCATE or ParcelFileDescriptor.MODE_WRITE_ONLY)
            runCatching { backend.startTrace(fd,full) }.onSuccess {
                file=current; lastBackend=backend
                MonitorRuntime.update { state -> state.copy(detail="",helper=state.helper) }
            }.onFailure {
                current.delete(); retryAt=SystemClock.elapsedRealtime()+30_000
                MonitorRuntime.update { state -> state.copy(detail=it.message.orEmpty()) }
            }
            fd.close()
        }
    }
    @Synchronized fun stop(backend: IStats?) {
        val current=file ?: return
        val status=runCatching { JSONObject((backend ?: lastBackend)?.stopTrace() ?: "{}") }.getOrDefault(JSONObject())
        val destination=root.resolve(current.name.removeSuffix(".partial")+if(protected) "-protected.perfetto-trace" else ".perfetto-trace")
        if(current.length()>0 && current.length()<=17L*1024*1024) {
            if(current.renameTo(destination)) destination.resolveSibling(destination.name+".meta.json").writeText(
                JSONObject().put("complete",status.optInt("exit",-1)==0).put("bytes",destination.length()).put("protected",protected)
                    .put("source",status.optString("id")).put("detail",status.optString("detail")).toString())
        } else current.delete()
        file=null; lastBackend=null; protected=false; protectUntil=0
        maintain()
    }
    private fun maintain() {
        val prefs=PreferenceStore(context).read()
        val files=root.listFiles()?.filter { it.name.endsWith(".perfetto-trace") && !it.name.contains("-protected") }?.sortedByDescending { it.lastModified() }.orEmpty()
        var bytes=0L; val cutoff=System.currentTimeMillis()-prefs.retentionHours*3_600_000L
        for(file in files) {
            bytes+=file.length()
            if(file.lastModified()<cutoff || bytes>prefs.budgetMb.toLong()*1024*1024) { file.delete(); file.resolveSibling(file.name+".meta.json").delete() }
        }
    }
    private fun File.resolveSibling(name: String)=parentFile!!.resolve(name)
}
