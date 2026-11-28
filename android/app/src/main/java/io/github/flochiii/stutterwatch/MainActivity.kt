package io.github.flochiii.stutterwatch

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.json.JSONArray
import org.json.JSONObject
import rikka.shizuku.Shizuku
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val Background=Color(0xff191c21)
private val Panel=Color(0xff242931)
private val Line=Color(0xff3d4652)
private val Blue=Color(0xff80b4ff)
private val Orange=Color(0xffefb65c)
private val Ink=Color(0xffe1e5eb)
private val Red=Color(0xffff6870)

class MainActivity : ComponentActivity() {
    private val notifications=registerForActivityResult(ActivityResultContracts.RequestPermission()) { startRecording() }
    private val permission=Shizuku.OnRequestPermissionResultListener { _, grant ->
        if(grant==PackageManager.PERMISSION_GRANTED) MonitorRuntime.action(this,RecordingService.CONFIGURE,true)
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Shizuku.addRequestPermissionResultListener(permission)
        MonitorRuntime.load(this)
        setContent {
            val prefs by MonitorRuntime.preferences.collectAsStateWithLifecycle()
            val state by MonitorRuntime.view.collectAsStateWithLifecycle()
            val font=remember { FontFamily(Typeface.createFromAsset(assets,"fonts/NotoSans-Regular.ttf")) }
            val typography=Typography().let { it.copy(bodyLarge=it.bodyLarge.copy(fontFamily=font),bodyMedium=it.bodyMedium.copy(fontFamily=font),labelLarge=it.labelLarge.copy(fontFamily=font),titleMedium=it.titleMedium.copy(fontFamily=font)) }
            MaterialTheme(colorScheme=darkColorScheme(primary=Blue,secondary=Orange,background=Background,surface=Background,surfaceVariant=Panel,onSurface=Ink,onBackground=Ink,primaryContainer=Panel,onPrimaryContainer=Ink,secondaryContainer=Panel,onSecondaryContainer=Ink,surfaceContainer=Panel,surfaceContainerHigh=Panel,surfaceContainerHighest=Panel,surfaceContainerLow=Background,surfaceContainerLowest=Background,outline=Line),typography=typography) {
                MonitorApp(prefs,state,::startWithPermission,::chooseShizuku)
            }
        }
    }
    private fun startRecording()=MonitorRuntime.action(this,RecordingService.START,true)
    private fun startWithPermission() {
        if(Build.VERSION.SDK_INT>=33 && ContextCompat.checkSelfPermission(this,Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED) notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        else startRecording()
    }
    private fun chooseShizuku() {
        if(runCatching { Shizuku.pingBinder() }.getOrDefault(false)) {
            if(Shizuku.checkSelfPermission()!=PackageManager.PERMISSION_GRANTED) Shizuku.requestPermission(1)
            else MonitorRuntime.action(this,RecordingService.CONFIGURE,true)
        } else startActivity(Intent(Intent.ACTION_VIEW,Uri.parse("https://shizuku.rikka.app/guide/setup/")))
    }
    override fun onDestroy() { Shizuku.removeRequestPermissionResultListener(permission); super.onDestroy() }
}

@Composable private fun MonitorApp(prefs: Preferences,state: ViewState,start:()->Unit,shizuku:()->Unit) {
    val context=LocalContext.current
    val tr=remember(prefs.language) { Translations(context,prefs.language) }
    var tab by remember { mutableIntStateOf(0) }
    var information by remember { mutableStateOf<String?>(null) }
    fun save(next: Preferences,manual: Boolean=false) {
        PreferenceStore(context).save(next)
        if(state.running) MonitorRuntime.action(context,RecordingService.CONFIGURE,manual)
    }
    Surface(color=Background,modifier=Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).padding(horizontal=14.dp)) {
            Row(Modifier.fillMaxWidth().padding(top=8.dp,bottom=4.dp),verticalAlignment=Alignment.CenterVertically) {
                Icon(painterResource(R.drawable.ic_monitor),contentDescription=null,tint=Blue,modifier=Modifier.size(24.dp))
                Text("Stutter Watch",modifier=Modifier.padding(start=10.dp).weight(1f),style=MaterialTheme.typography.titleMedium)
                Text(tr[state.status],color=if(state.status=="recorder_error") Red else Ink.copy(alpha=.7f),fontSize=12.sp)
            }
            Row(Modifier.fillMaxWidth().padding(vertical=8.dp),horizontalArrangement=Arrangement.spacedBy(6.dp)) {
                listOf("live","data","incidents","settings").forEachIndexed { index,key ->
                    TextButton(onClick={tab=index},shape=RoundedCornerShape(2.dp),modifier=Modifier.weight(1f).height(42.dp).background(if(tab==index) Color(0xff10151b) else Panel,RoundedCornerShape(2.dp))
                        .border(1.dp,if(tab==index) Blue else Line,RoundedCornerShape(2.dp)),contentPadding=PaddingValues(2.dp)) { ButtonLabel(tr[key]) }
                }
            }
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                if(!state.running) Control(tr["start"],Modifier.fillMaxWidth(),start)
                else {
                    Control(tr["mark"],Modifier.weight(1f)) { MonitorRuntime.action(context,RecordingService.MARK) }
                    Control(tr[if(prefs.paused) "resume" else "pause"],Modifier.weight(1f)) { MonitorRuntime.action(context,RecordingService.PAUSE) }
                    Control(tr["stop"],Modifier.weight(1f)) { MonitorRuntime.action(context,RecordingService.STOP) }
                }
            }
            HorizontalDivider(Modifier.padding(vertical=10.dp),color=Line)
            when(tab) {
                0 -> MonitorCharts(state.snapshot.optJSONArray("points") ?: JSONArray(),tr,prefs,state,Modifier.weight(1f))
                1 -> LazyColumn(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                    item { Section(tr["access"]) }
                    item { Row(horizontalArrangement=Arrangement.spacedBy(6.dp)) { Access.entries.forEach { access ->
                        FilterChip(selected=prefs.access==access,onClick={save(prefs.copy(access=access),access==Access.Root)},label={Text(tr[access.name.lowercase()],maxLines=1)},shape=RoundedCornerShape(3.dp),modifier=Modifier.weight(1f).height(42.dp))
                    } } }
                    item { Text(tr[state.helper],color=Ink.copy(alpha=.7f),fontSize=12.sp) }
                    if(prefs.access==Access.Shizuku) item { Control(tr["setup_shizuku"],onClick=shizuku); Text(tr["shizuku_help"],fontSize=12.sp,color=Ink.copy(alpha=.7f)) }
                    if(prefs.access==Access.Root) item { Text(tr["root_help"],fontSize=12.sp,color=Ink.copy(alpha=.7f)) }
                    item { Section(tr["collection"]) }
                    item { SourceRow(tr["cpu_total"],prefs.cpu,{save(prefs.copy(cpu=it))}) { information="cpu_help" } }
                    item { SourceRow(tr["memory"],prefs.memory,{save(prefs.copy(memory=it))}) { information="memory_help" } }
                    item { SourceRow(tr["fps"],prefs.frames,{save(prefs.copy(frames=it))}) { information="frames_help" } }
                    item { SourceRow(tr["battery"],prefs.battery,{save(prefs.copy(battery=it))}) { information="battery_help" } }
                    item { SourceRow(tr["thermal"],prefs.thermal,{save(prefs.copy(thermal=it))}) { information="thermal_help" } }
                    item { SourceRow(tr["gpu_sensors"],prefs.gpu,{save(prefs.copy(gpu=it))}) { information="gpu_help" } }
                    item { SourceRow(tr["detailed"],prefs.detailed,{save(prefs.copy(detailed=it))}) { information="trace_help" } }
                    item { TargetEditor(prefs,tr) { save(it) } }
                    item { Section(tr["source_status"]) }
                    val statuses=state.snapshot.optJSONObject("status") ?: JSONObject()
                    items(statuses.keys().asSequence().toList().sorted(),key={it}) { name ->
                        val source=statuses.optJSONObject(name)
                        val available=source?.optBoolean("available")==true
                        val label=when(name) {"cpu"->"cpu_total";"frames"->"fps";"gpu"->"gpu_sensors";else->name}
                        Column(Modifier.padding(vertical=5.dp)) {
                            Text(tr[label],color=if(available) Blue else Ink)
                            Text(tr[if(available) "active" else source?.optString("reason").orEmpty().ifBlank { "unknown" }],fontSize=12.sp,color=Ink.copy(alpha=.65f))
                        }
                    }
                    item { Text("${Build.MANUFACTURER} ${Build.MODEL} · Android ${Build.VERSION.RELEASE} · ${Build.SUPPORTED_ABIS.joinToString()}",fontSize=11.sp,color=Ink.copy(alpha=.5f),modifier=Modifier.padding(vertical=12.dp)) }
                }
                2 -> {
                    val detail=state.incident
                    if(detail!=null) {
                        Control(tr["close"]) { MonitorRuntime.update { it.copy(incident=null) } }
                        MonitorCharts(detail.optJSONArray("points") ?: JSONArray(),tr,prefs,state,Modifier.weight(1f))
                    } else LazyColumn(Modifier.weight(1f)) {
                        if(state.incidents.length()==0) item { Text(tr["empty"],modifier=Modifier.padding(vertical=16.dp),color=Ink.copy(alpha=.65f)) }
                        items((0 until state.incidents.length()).map { state.incidents.getJSONObject(it) },key={it.getString("id")}) { incident ->
                            Column(Modifier.fillMaxWidth().clickable { MonitorRuntime.selectIncident(incident.getString("id")) }.padding(vertical=12.dp)) {
                                Text(time(incident.optLong("start_ms")),color=Blue)
                                val reason=incident.optJSONArray("triggers")?.optJSONObject(0)?.optString("reason").orEmpty()
                                Text(tr[if(reason=="manual_mark") "event_manual" else if(reason=="memory_pressure") "event_memory" else reason])
                                Text(tr[if(incident.optBoolean("complete")) "complete" else "provisional"],fontSize=12.sp,color=Ink.copy(alpha=.65f))
                            }
                            HorizontalDivider(color=Line)
                        }
                    }
                }
                3 -> LazyColumn(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(6.dp)) {
                    item { Section(tr["language"]) }
                    item { LanguagePicker(prefs,tr) { save(prefs.copy(language=it)) } }
                    item { Section(tr["energy"]) }
                    item { Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) { Energy.entries.forEach { mode -> FilterChip(selected=prefs.energy==mode,onClick={save(prefs.copy(energy=mode))},label={Text(tr[mode.name.lowercase()])},shape=RoundedCornerShape(3.dp),modifier=Modifier.weight(1f).height(42.dp)) } } }
                    item { Text(tr[if(prefs.energy==Energy.Eco) "eco_help" else "full_help"],fontSize=12.sp,color=Ink.copy(alpha=.7f)) }
                    item { SourceRow(tr["autostart"],prefs.autostart,{save(prefs.copy(autostart=it))}) { information="bg_settings" } }
                    item { Control(tr["unrestricted"]) { context.startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,Uri.parse("package:${context.packageName}"))) } }
                    item { Section(tr["overlay"]) }
                    item { SourceRow(tr["overlay"],prefs.overlay,{ enabled ->
                        if(enabled && !android.provider.Settings.canDrawOverlays(context)) context.startActivity(Intent(android.provider.Settings.ACTION_MANAGE_OVERLAY_PERMISSION,Uri.parse("package:${context.packageName}")))
                        save(prefs.copy(overlay=enabled))
                    }) { information="overlay_permission" } }
                    item { SourceRow(tr["fps"],prefs.overlayFps,{save(prefs.copy(overlayFps=it))}) { information="frames_help" } }
                    item { SourceRow("CPU",prefs.overlayCpu,{save(prefs.copy(overlayCpu=it))}) { information="cpu_help" } }
                    item { SourceRow("RAM",prefs.overlayRam,{save(prefs.copy(overlayRam=it))}) { information="memory_help" } }
                    item { HistoryEditor(prefs,tr) { save(it) } }
                }
            }
            if(state.detail.isNotBlank()) TextButton(onClick={information="technical"}) { Text(tr["details"],fontSize=12.sp) }
            val usage=state.snapshot.optJSONObject("writer")?.optJSONObject("usage")
            Text("${tr["ordinary"]}: ${format(usage?.optDouble("ordinary_bytes")?.div(1_000_000))} / ${prefs.budgetMb} MB · ${tr["protected"]}: ${format(usage?.optDouble("protected_bytes")?.div(1_000_000))} MB",fontSize=11.sp,maxLines=1,modifier=Modifier.padding(vertical=8.dp),color=Ink.copy(alpha=.7f))
        }
        information?.let { key -> AlertDialog(onDismissRequest={information=null},title={Text(tr["info"])},text={Text(if(key=="technical") state.detail else tr[key])},confirmButton={TextButton(onClick={information=null}) {Text(tr["close"])} }) }
    }
}

@Composable private fun ButtonLabel(text:String) { BasicText(text,maxLines=1,style=MaterialTheme.typography.labelLarge.copy(color=Ink),autoSize=TextAutoSize.StepBased(minFontSize=10.sp,maxFontSize=12.sp,stepSize=0.5.sp)) }
@Composable private fun Control(text: String,modifier:Modifier=Modifier,onClick:()->Unit) { FilledTonalButton(onClick=onClick,modifier=modifier.heightIn(min=42.dp),shape=RoundedCornerShape(3.dp),contentPadding=PaddingValues(horizontal=8.dp,vertical=6.dp)) {ButtonLabel(text)} }
@Composable private fun Section(text:String) { Text(text,style=MaterialTheme.typography.titleMedium,modifier=Modifier.padding(top=14.dp,bottom=6.dp)) }
@Composable private fun SourceRow(label:String,checked:Boolean,change:(Boolean)->Unit,info:()->Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min=48.dp),verticalAlignment=Alignment.CenterVertically) {
        Row(Modifier.weight(1f).heightIn(min=48.dp).clip(RoundedCornerShape(3.dp)).toggleable(value=checked,role=Role.Checkbox,onValueChange=change),verticalAlignment=Alignment.CenterVertically) {
            Checkbox(checked=checked,onCheckedChange=null,modifier=Modifier.padding(horizontal=12.dp))
            Text(label,modifier=Modifier.weight(1f),fontSize=14.sp)
        }
        TextButton(onClick=info,contentPadding=PaddingValues(4.dp)) {Text("ⓘ",fontSize=16.sp)}
    }
}
@Composable private fun LanguagePicker(prefs:Preferences,tr:Translations,change:(String)->Unit) {
    var expanded by remember { mutableStateOf(false) }
    Box { Control(Translations.languages.firstOrNull {it.first==prefs.language}?.second ?: "English") {expanded=true}
        DropdownMenu(expanded=expanded,onDismissRequest={expanded=false},modifier=Modifier.heightIn(max=260.dp)) { Translations.languages.forEach { (code,name) -> DropdownMenuItem(text={Text(name)},onClick={change(code);expanded=false}) } }
    }
}
@Composable private fun TargetEditor(prefs:Preferences,tr:Translations,save:(Preferences)->Unit) {
    var target by remember(prefs.target) {mutableStateOf(prefs.target)}
    Section(tr["target"])
    OutlinedTextField(value=target,onValueChange={target=it.take(200)},label={Text(tr["target_package"])},placeholder={Text(tr["target_auto"])},singleLine=true,modifier=Modifier.fillMaxWidth())
    TextButton(onClick={ if(target.isBlank() || target.matches(Regex("[A-Za-z][A-Za-z0-9_.]{1,199}"))) save(prefs.copy(target=target.trim())) }) {Text(tr["apply"])}
}
@Composable private fun HistoryEditor(prefs:Preferences,tr:Translations,save:(Preferences)->Unit) {
    var budget by remember(prefs.budgetMb) {mutableStateOf(prefs.budgetMb.toString())}
    var hours by remember(prefs.retentionHours) {mutableStateOf(prefs.retentionHours.toString())}
    Section(tr["history_settings"])
    OutlinedTextField(value=budget,onValueChange={budget=it.filter(Char::isDigit).take(4)},label={Text(tr["budget"])},singleLine=true,modifier=Modifier.fillMaxWidth())
    OutlinedTextField(value=hours,onValueChange={hours=it.filter(Char::isDigit).take(3)},label={Text(tr["retention"])},singleLine=true,modifier=Modifier.fillMaxWidth().padding(top=8.dp))
    TextButton(onClick={val b=budget.toIntOrNull();val h=hours.toIntOrNull();if(b!=null && b in 10..4096 && h!=null && h in 1..168) save(prefs.copy(budgetMb=b,retentionHours=h))}) {Text(tr["apply"])}
}

private fun format(value:Double?,digits:Int=1)=value?.takeIf {it.isFinite()}?.let {String.format(Locale.ROOT,"%.${digits}f",it)} ?: "—"
private fun time(ms:Long)=SimpleDateFormat("HH:mm:ss",Locale.ROOT).format(Date(ms))
private fun JSONArray.objects()=(0 until length()).map {getJSONObject(it)}
private fun JSONObject.metric(key:String):Double?=optJSONObject("metrics")?.optDouble(key)?.takeIf {it.isFinite()}
private fun JSONObject.number(key:String):Double?=optDouble(key).takeIf {it.isFinite()}
private data class PlotSeries(val name:String,val color:Color,val get:(JSONObject)->Double?)

@Composable private fun MonitorCharts(data:JSONArray,tr:Translations,prefs:Preferences,state:ViewState,modifier:Modifier) {
    val points=remember(data) {data.objects()}
    LazyColumn(modifier,verticalArrangement=Arrangement.spacedBy(12.dp)) {
        item("cpu") { Plot(tr["graph_cpu"],points,listOf(PlotSeries("CPU",Blue) {it.number("cpu")},PlotSeries(tr["memory"],Orange) {it.number("ram")}),100.0,tr,alertMask=1 or 4) }
        if(prefs.frames) item("fps") {
            val unavailable = if(state.incident==null) {
                val key = if(state.helper in setOf("helper_start_failed", "access_lost", "shizuku_required", "root_required")) state.helper
                    else state.snapshot.optJSONObject("status")?.optJSONObject("frames")?.optString("reason").orEmpty()
                key.takeIf { it.isNotBlank() }?.let { tr[it] }
            } else null
            Plot(tr["fps"],points,listOf(PlotSeries(tr["fps"],Blue) {it.metric("fps")}),null,tr,alertMask=8,emptyReason=unavailable)
        }
        if(points.any {it.metric("frame_duration_ms")!=null}) item("duration") {Plot(tr["frame_duration"],points,listOf(PlotSeries(tr["frame_duration"],Orange) {it.metric("frame_duration_ms")}),null,tr,alertMask=8)}
        if(points.any {it.metric("frame_ms")!=null}) item("frames") {Plot(tr["frame_time"],points,listOf(PlotSeries(tr["frame_time"],Orange) {it.metric("frame_ms")}),null,tr,alertMask=8)}
        if(prefs.battery) item("battery") { Plot(tr["battery"]+", %",points,listOf(PlotSeries(tr["battery"],Blue) {it.metric("battery_pct")}),100.0,tr) }
        if(prefs.battery) item("temperature") { Plot(tr["temperature"],points,listOf(PlotSeries(tr["battery_temperature"],Orange) {it.metric("battery_temperature_c")}),null,tr) }
        if(points.any {it.metric("cpu_frequency_mhz")!=null}) item("frequency") { Plot(tr["cpu_frequency"],points,listOf(PlotSeries("CPU",Blue) {it.metric("cpu_frequency_mhz")}),null,tr) }
        if(points.any {it.metric("cpu_headroom")!=null}) item("cpu-headroom") {Plot(tr["cpu_headroom"],points,listOf(PlotSeries("CPU",Blue) {it.metric("cpu_headroom")}),100.0,tr)}
        if(points.any {it.metric("gpu_headroom")!=null}) item("gpu-headroom") {Plot(tr["gpu_headroom"],points,listOf(PlotSeries("GPU",Orange) {it.metric("gpu_headroom")}),100.0,tr)}
        if(points.isEmpty()) item {Text(tr[if(state.running) "no_data" else "never_started"],fontSize=12.sp,color=Ink.copy(alpha=.65f))}
    }
}

@Composable private fun Plot(title:String,points:List<JSONObject>,series:List<PlotSeries>,fixedMax:Double?,tr:Translations,alertMask:Int=0,emptyReason:String?=null) {
    val values=points.flatMap { p ->series.mapNotNull {it.get(p)} }
    val empty=values.isEmpty()
    val max=fixedMax ?: ((values.maxOrNull() ?: 1.0)*1.12).coerceAtLeast(1.0)
    val end=points.lastOrNull()?.optLong("ts") ?: System.currentTimeMillis()
    val start=(end-600_000).coerceAtLeast(points.firstOrNull()?.optLong("ts") ?: end)
    val font=remember {android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {color=0xffaab7c7.toInt();textSize=24f}}
    Column {
        Text(title,fontSize=14.sp,color=Ink)
        Row(Modifier.padding(vertical=4.dp),horizontalArrangement=Arrangement.spacedBy(16.dp)) {series.forEach {s ->
            Text("━ ${s.name}  ${format(points.lastOrNull()?.let(s.get))}",color=s.color,fontSize=12.sp)
        }}
        if(empty) Text(emptyReason ?: tr["no_data"],color=Ink.copy(alpha=.5f),fontSize=12.sp,modifier=Modifier.padding(vertical=12.dp))
        else Canvas(Modifier.fillMaxWidth().height(170.dp)) {
            val left=38.dp.toPx();val top=8.dp.toPx();val width=size.width-left;val height=size.height-28.dp.toPx()
            font.textSize=10.sp.toPx()
            for(i in 0..4) {
                val y=top+height*i/4f
                drawLine(Line,Offset(left,y),Offset(size.width,y),1f)
                drawContext.canvas.nativeCanvas.drawText(format(max*(4-i)/4,0),0f,y+4.dp.toPx(),font)
            }
            val duration=(end-start).coerceAtLeast(1000)
            points.filter { it.optLong("ts") >= start && it.optInt("alerts") and alertMask != 0 }.forEach { p ->
                val x=left+(p.optLong("ts")-start).toFloat()/duration*width
                val severe = p.optInt("alerts") and alertMask and (8 or 4 or 4096) != 0
                drawLine((if(severe) Red else Orange).copy(alpha=.25f),Offset(x,top),Offset(x,top+height),2.dp.toPx())
            }
            series.forEach {s ->
                var last:Offset?=null;var lastTime=0L;var session="";var target=""
                points.forEach {p ->
                    val now=p.optLong("ts");val meta=p.optJSONObject("metrics")
                    val sid=meta?.optString("session_id").orEmpty();val pkg=meta?.optString("target_package").orEmpty()
                    val value=s.get(p)
                    if(value==null || now<start) {last=null;return@forEach}
                    val point=Offset(left+(now-start).toFloat()/duration*width,top+height-(value/max).toFloat().coerceIn(0f,1f)*height)
                    if(last!=null && now-lastTime in 1..3000 && sid==session && (s.name!=tr["fps"] || pkg==target)) drawLine(s.color,last!!,point,1.5f)
                    last=point;lastTime=now;session=sid;target=pkg
                }
            }
            drawContext.canvas.nativeCanvas.drawText(time(start),left,top+height+18.dp.toPx(),font)
            val endLabel=time(end);drawContext.canvas.nativeCanvas.drawText(endLabel,size.width-font.measureText(endLabel),top+height+18.dp.toPx(),font)
        }
    }
}
