// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.Activity
import android.app.Application
import android.app.Instrumentation
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.os.Build
import android.os.Bundle
import android.os.Process
import android.system.Os
import android.system.OsConstants
import org.json.JSONArray
import org.json.JSONObject
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.File
import java.lang.ref.WeakReference
import java.net.InetAddress
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.security.KeyStore
import java.security.MessageDigest
import java.util.UUID
import org.xmlpull.v1.XmlPullParser

/** Platform-only fresh-install boundary; baseline never initializes normal Goatr runtime. */
internal object PhysicalMvpPlatform {
    const val PRODUCT="io.github.trevarj.goatr.debug"
    const val TEST="io.github.trevarj.goatr.debug.test"
    const val VERIFY="io.github.trevarj.goatr.debug.verify"
    const val MODE="physicalMvpMode"
    private val hex=Regex("[0-9a-f]{64}")
    private val host=Regex("[0-9a-f]{64}")
    private val allowed=setOf(MODE,"physicalMvpFreshInstall","physicalMvpRunId","physicalMvpExpectedSignerSha256","physicalMvpExpectedProductSha256","physicalMvpExpectedProductUid",
        "physicalMvpExpectedTestSha256","physicalMvpExpectedVerifySha256","physicalMvpExpectedVerifySignerSha256","physicalMvpDependencyProof","physicalMvpHostLabel","physicalMvpHost","stockControlPort")
    fun requested(arguments:Bundle)=arguments.keySet().any {it.startsWith("physicalMvp")}
    fun arguments(arguments:Bundle):JSONObject {
        check(arguments.keySet().all {it in allowed}) {"physical_arguments_closed"}
        val mode=arguments.getString(MODE);check(mode=="baseline"||mode=="observe") {"physical_mode"}
        check(arguments.getString("physicalMvpFreshInstall")=="true") {"physical_fresh_install_required"}
        val run=arguments.getString("physicalMvpRunId") ?: error("physical_run")
        check(UUID.fromString(run).toString()==run) {"physical_run"}
        val result=JSONObject().put("mode",mode).put("runId",run).put("freshInstall",true)
        for(key in listOf("physicalMvpExpectedSignerSha256","physicalMvpExpectedProductSha256","physicalMvpExpectedTestSha256","physicalMvpDependencyProof")) {
            val value=arguments.getString(key) ?: error("physical_pin")
            check(hex.matches(value)) {"physical_pin"};result.put(key,value)
        }
        val uid=arguments.getString("physicalMvpExpectedProductUid") ?: error("physical_product_uid")
        val productUid=uid.toIntOrNull();check(productUid!=null&&productUid>=10000&&productUid.toString()==uid) {"physical_product_uid"};result.put("productUid",productUid)
        val verify=arguments.getString("physicalMvpExpectedVerifySha256");val verifySigner=arguments.getString("physicalMvpExpectedVerifySignerSha256")
        check((verify==null)==(verifySigner==null)) {"physical_verify_pair"}
        if(verify!=null){check(hex.matches(verify)&&hex.matches(checkNotNull(verifySigner))) {"physical_pin"};result.put("physicalMvpExpectedVerifySha256",verify).put("physicalMvpExpectedVerifySignerSha256",verifySigner)}
        val label=arguments.getString("physicalMvpHostLabel") ?: error("physical_label")
        check(label=="Goatr MVP $run") {"physical_label"};result.put("hostLabel",label)
        val port=arguments.getString("stockControlPort")?.toIntOrNull() ?: error("physical_control_port")
        check(port in 1..65535) {"physical_control_port"};result.put("port",port)
        val selected=arguments.getString("physicalMvpHost")
        if(mode=="observe")check(selected!=null&&host.matches(selected)) {"physical_host"}
        else check(selected==null) {"physical_host_unexpected"}
        result.put("host",selected ?: JSONObject.NULL)
        return result
    }
    fun digest(bytes:ByteArray):String=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") {"%02x".format(it.toInt() and 255)}
    fun digest(text:String)=digest(text.toByteArray(StandardCharsets.UTF_8))
    private fun fileDigest(file:File):String {
        check(file.isFile&&file.length() in 1..(1024L*1024*1024)) {"physical_apk_file"}
        val hash=MessageDigest.getInstance("SHA-256");val buffer=ByteArray(65536)
        file.inputStream().use {input->while(true) {val n=input.read(buffer);if(n<0)break;hash.update(buffer,0,n)}}
        return hash.digest().joinToString("") {"%02x".format(it.toInt() and 255)}
    }
    @Suppress("DEPRECATION")
    fun qualify(test:Context,target:Context,args:JSONObject) {
        check(test.packageName==TEST&&target.packageName==PRODUCT&&target.applicationInfo.uid==Process.myUid()
            &&target.applicationInfo.uid==args.getInt("productUid")&&test.applicationInfo.uid!=target.applicationInfo.uid&&android.app.Application.getProcessName()==PRODUCT) {"physical_package"}
        check(Build.VERSION.SDK_INT==30||Build.VERSION.SDK_INT==34) {"physical_api"}
        val manager=target.packageManager
        val expected=args.getString("physicalMvpExpectedSignerSha256")
        for((name,pin) in listOf(PRODUCT to "physicalMvpExpectedProductSha256",TEST to "physicalMvpExpectedTestSha256")) {
            val info=manager.getPackageInfo(name,PackageManager.GET_SIGNATURES or PackageManager.GET_PROVIDERS or PackageManager.GET_META_DATA)
            val app=checkNotNull(info.applicationInfo)
            check(app.flags and ApplicationInfo.FLAG_DEBUGGABLE!=0) {"physical_debug_package"}
            check(info.sharedUserId==null&&app.processName==name
                &&app.uid==if(name==PRODUCT)target.applicationInfo.uid else test.applicationInfo.uid) {"physical_process_boundary"}
            check(info.signatures?.map {digest(it.toByteArray())}?.toSet()==setOf(expected)) {"physical_signer"}
            check(fileDigest(File(app.sourceDir))==args.getString(pin)) {"physical_apk_changed"}
        }
        val verify=try {manager.getPackageInfo(VERIFY,PackageManager.GET_SIGNATURES)}catch(_:PackageManager.NameNotFoundException){null}
        if(args.has("physicalMvpExpectedVerifySha256")) {
            checkNotNull(verify) {"physical_verify_missing"}
            check(verify.signatures?.map {digest(it.toByteArray())}?.toSet()==setOf(args.getString("physicalMvpExpectedVerifySignerSha256"))
                &&fileDigest(File(checkNotNull(verify.applicationInfo).sourceDir))==args.getString("physicalMvpExpectedVerifySha256")) {"physical_verify_changed"}
        }else check(verify==null) {"physical_verify_unexpected"}
        val instruments=manager.getPackageInfo(TEST,PackageManager.GET_INSTRUMENTATION).instrumentation.orEmpty()
        check(instruments.size==2&&instruments.associate {it.name to it.targetPackage}==mapOf(
            "io.github.trevarj.goatr.OwnedInstrumentationRunner" to PRODUCT,
            "io.github.trevarj.goatr.OwnedUiAutomationRunner" to TEST)) {"physical_packaged_instrumentations"}
        check(instruments.all {it.targetProcesses==null}) {"physical_target_process_override"}
        val providers=manager.getPackageInfo(PRODUCT,PackageManager.GET_PROVIDERS or PackageManager.GET_META_DATA).providers.orEmpty()
        check(providers.size==1) {"physical_provider_set"}
        val provider=providers.single()
        check(provider.name=="androidx.startup.InitializationProvider"&&provider.authority=="$PRODUCT.androidx-startup"&&!provider.exported&&provider.enabled) {"physical_provider"}
        val initializers=mapOf("androidx.emoji2.text.EmojiCompatInitializer" to "androidx.startup",
            "androidx.lifecycle.ProcessLifecycleInitializer" to "androidx.startup","androidx.profileinstaller.ProfileInstallerInitializer" to "androidx.startup")
        val metadata=checkNotNull(provider.metaData)
        check(metadata.keySet()==initializers.keys&&initializers.all {(key,value)->metadata.getString(key)==value}) {"physical_provider_initializers"}
    }
    fun process():JSONObject {
        val pid=Process.myPid();val stat=File("/proc/self/stat").readText()
        check(stat.substringBefore(' ').toInt()==pid) {"physical_pid"}
        val start=stat.substring(stat.lastIndexOf(')')+2).trim().split(Regex("\\s+"))[19]
        check(start.toLong()>0) {"physical_start"}
        val fields=File("/proc/self/status").readLines().mapNotNull {line->line.indexOf(':').takeIf {it>0}?.let {line.substring(0,it) to line.substring(it+1).trim()}}.toMap()
        val caps=JSONObject()
        for(name in listOf("CapInh","CapPrm","CapEff","CapAmb")) {
            val value=checkNotNull(fields[name]);check(value.matches(Regex("[0-9a-fA-F]{16}"))) {"physical_caps"}
            check(value.toULong(16)==0uL) {"physical_app_caps"};caps.put(name,value.lowercase())
        }
        return JSONObject().put("pid",pid).put("start",start).put("uid",Process.myUid()).put("name",android.app.Application.getProcessName()).put("caps",caps)
    }
    fun update(hash:MessageDigest,value:String) {val bytes=value.toByteArray(StandardCharsets.UTF_8);val size=bytes.size
        hash.update((size ushr 24).toByte());hash.update((size ushr 16).toByte());hash.update((size ushr 8).toByte());hash.update(size.toByte());hash.update(bytes)}
    fun finished(hash:MessageDigest)=hash.digest().joinToString("") {"%02x".format(it.toInt() and 255)}
    private fun dataRoot(target:Context):File=File(target.applicationInfo.dataDir).canonicalFile.also {
        check(it.isDirectory&&Os.stat(it.path).st_uid==Process.myUid()) {"physical_data_root"}
    }
    private class HostRow(var metadata:JSONObject?=null,val hash:MessageDigest=MessageDigest.getInstance("SHA-256"),var rows:Int=0,var actions:Int=0,var registration:String?=null,var ciphertext:String?=null)
    private fun preferences(target:Context):Pair<JSONArray,Map<String,Map<String,Pair<String,String>>>> {
        val directory=File(dataRoot(target),"shared_prefs")
        if(!directory.exists())return JSONArray() to emptyMap()
        check(directory.isDirectory&&directory.canonicalFile==directory) {"physical_preferences_directory"}
        val files=directory.listFiles()?.sortedBy {it.name} ?: error("physical_preferences_list")
        check(files.size<=64&&files.none {it.name.endsWith(".bak")}) {"physical_preferences_unsettled"}
        val rows=JSONArray();val all=linkedMapOf<String,Map<String,Pair<String,String>>>()
        var count=0
        for(file in files) {
            check(file.name.matches(Regex("[A-Za-z0-9_.-]+\\.xml"))&&file.canonicalFile==file&&file.isFile&&file.length()<=1048576) {"physical_preferences_file"}
            val values=sortedMapOf<String,Pair<String,String>>()
            file.inputStream().use {input->
                val parser=android.util.Xml.newPullParser();parser.setInput(input,"UTF-8")
                check(parser.nextTag()==XmlPullParser.START_TAG&&parser.name=="map") {"physical_preferences_xml"}
                while(parser.nextTag()==XmlPullParser.START_TAG) {
                    val type=parser.name;val key=parser.getAttributeValue(null,"name") ?: error("physical_preferences_key")
                    check(key.length<=4096&&++count<=8192&&!values.containsKey(key)) {"physical_preferences_bound"}
                    val value=when(type) {
                        "string"->parser.nextText()
                        "boolean","int","long","float"->{val scalar=parser.getAttributeValue(null,"value") ?: error("physical_preferences_value");check(parser.nextTag()==XmlPullParser.END_TAG);scalar}
                        "set"->{val entries=mutableListOf<String>();while(parser.nextTag()==XmlPullParser.START_TAG) {check(parser.name=="string"&&entries.size<1024);entries+=parser.nextText()};entries.sorted().joinToString("") {"${it.length}:$it"}}
                        else->error("physical_preferences_type")
                    }
                    check(value.length<=1048576) {"physical_preferences_value_bound"};values[key]=type to value
                }
            }
            val name=file.name.removeSuffix(".xml");all[name]=values
            values.forEach {(key,value)->rows.put(JSONObject().put("file",name).put("keySha256",digest(key)).put("type",value.first).put("valueSha256",digest(value.second)))}
        }
        return rows to all
    }
    private val connectorSchema=sortedMapOf(
        "distributors" to "CREATE TABLE distributors (distributor TEXT PRIMARY KEY,fallback_from TEXT,ack INTEGER,date_insertion INTEGER,FOREIGN KEY (fallback_from) REFERENCES distributors(distributor) ON DELETE CASCADE)",
        "registrations" to "CREATE TABLE registrations (instance TEXT PRIMARY KEY,message TEXT,vapid TEXT)",
        "tokens" to "CREATE TABLE tokens (connectorToken TEXT PRIMARY KEY,instance TEXT,distributor TEXT,FOREIGN KEY (instance) REFERENCES registrations(instance) ON DELETE CASCADE,FOREIGN KEY (distributor) REFERENCES distributors(distributor) ON DELETE CASCADE)",
        "keys" to "CREATE TABLE keys (instance TEXT PRIMARY KEY,auth TEXT,pubkey TEXT,privkey TEXT,iv TEXT,FOREIGN KEY (instance) REFERENCES registrations(instance) ON DELETE CASCADE)",
    )
    private fun sql(value:String)=value.trim().trimEnd(';').filterNot(Char::isWhitespace).lowercase()
    /** Connector3.3.5 has its own DB. Never instantiate DBStore: that migrates/deletes legacy prefs. */
    private fun connector(target:Context,legacyFilePresent:Boolean,legacyKeys:Int):JSONObject {
        check(legacyKeys==0) {"physical_connector_legacy_material"}
        val file=File(dataRoot(target),"databases/unifiedpush-connector")
        val distributors=JSONArray();val registrations=JSONArray();val tokens=JSONArray();val keys=JSONArray()
        var schemaHash:String?=null;var platformHash:String?=null
        var localeChanges=0L;val platform=MessageDigest.getInstance("SHA-256")
        if(file.exists()) {
            check(file.isFile&&file.canonicalFile==file) {"physical_connector_database_path"}
            SQLiteDatabase.openDatabase(file.path,null,SQLiteDatabase.OPEN_READONLY or SQLiteDatabase.NO_LOCALIZED_COLLATORS).use {db->
                check(db.version==1) {"physical_connector_database_version"}
                val actual=sortedMapOf<String,String>()
                var platformTable=false
                db.rawQuery("SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name",null).use {rows->
                    while(rows.moveToNext()) {
                        check(actual.size<5&&rows.getString(0)=="table") {"physical_connector_schema"}
                        val name=rows.getString(1);val definition=sql(rows.getString(2))
                        if(name=="android_metadata") {
                            check(definition==sql("CREATE TABLE android_metadata (locale TEXT)")) {"physical_connector_platform_schema"}
                            platformTable=true
                        }else check(actual.put(name,definition)==null) {"physical_connector_schema"}
                    }
                }
                check(actual==connectorSchema.mapValues {sql(it.value)}&&platformTable) {"physical_connector_schema"}
                schemaHash=digest(actual.entries.joinToString("\n") {"${it.key}:${it.value}"})
                val locale=android.database.DatabaseUtils.sqlEscapeString(java.util.Locale.getDefault().toString())
                val query="""
                    SELECT 'distributor' AS kind,rowid AS rid,distributor AS owner,distributor AS a,fallback_from AS b,ack AS c,date_insertion AS d,NULL AS e FROM distributors
                    UNION ALL SELECT 'registration',rowid,instance,instance,message,vapid,NULL,NULL FROM registrations
                    UNION ALL SELECT 'token',rowid,instance,connectorToken,instance,distributor,NULL,NULL FROM tokens
                    UNION ALL SELECT 'key',rowid,instance,instance,auth,pubkey,privkey,iv FROM keys
                    UNION ALL SELECT 'metadata',rowid,'android_metadata',locale,NULL,NULL,NULL,NULL FROM android_metadata
                    UNION ALL SELECT 'locale',1,'android_metadata',abs((SELECT count(*) FROM android_metadata)-1)+(SELECT count(*) FROM android_metadata WHERE locale IS NULL OR locale<>$locale),NULL,NULL,NULL,NULL
                    ORDER BY kind,rid
                """.trimIndent()
                var count=0;var total=0L
                residentRows(db,query,16L*1024*1024,4098).use {rows->while(rows.moveToNext()) {
                    check(++count<=4098&&rows.getType(1)==Cursor.FIELD_TYPE_INTEGER&&rows.getLong(1)>0) {"physical_connector_row_bound"}
                    val kind=rows.getString(0);val rowId=rows.getLong(1);val owner=rows.getString(2)
                    check(owner.length in 1..512) {"physical_connector_owner"}
                    val hash=MessageDigest.getInstance("SHA-256");update(hash,kind)
                    for(index in 3..7) {
                        val type=rows.getType(index);check(type!=Cursor.FIELD_TYPE_BLOB&&type!=Cursor.FIELD_TYPE_FLOAT) {"physical_connector_field_type"}
                        val value=if(type==Cursor.FIELD_TYPE_NULL)"" else rows.getString(index)
                        total+=value.length;check(value.length<=65536&&total<=4194304) {"physical_connector_field_bound"}
                        update(hash,type.toString());update(hash,value)
                    }
                    val item=JSONObject().put("rowId",rowId).put("sha256",finished(hash))
                    when(kind) {
                        "distributor"->{
                            check(owner.matches(Regex("[A-Za-z0-9_.]{1,256}"))&&rows.getType(5)==Cursor.FIELD_TYPE_INTEGER&&rows.getLong(5) in 0L..1L
                                &&rows.getType(6)==Cursor.FIELD_TYPE_INTEGER&&rows.getLong(6)>=0) {"physical_connector_distributor"}
                            distributors.put(item.put("distributor",owner).put("fallbackFrom",if(rows.isNull(4))JSONObject.NULL else rows.getString(4))
                                .put("ack",rows.getLong(5)==1L).put("dateInsertion",rows.getLong(6)))
                        }
                        "registration"->registrations.put(item.put("instanceSha256",digest(owner))
                            .put("messageMatchesPrivate",!rows.isNull(4)&&rows.getString(4)=="Goatr paired desktop").put("vapidNull",rows.isNull(5)))
                        "token"->tokens.put(item.put("instanceSha256",digest(owner)).put("distributor",rows.getString(5)))
                        "key"->keys.put(item.put("instanceSha256",digest(owner)).put("sealed",(4..7).all {!rows.isNull(it)&&rows.getString(it).isNotEmpty()}))
                        "metadata"->{update(platform,rowId.toString());update(platform,item.getString("sha256"))}
                        "locale"->{check(rows.getType(3)==Cursor.FIELD_TYPE_INTEGER&&rows.getLong(3)>=0);localeChanges=rows.getLong(3)}
                        else->error("physical_connector_row_kind")
                    }
                }}
                platformHash=finished(platform)
            }
        }
        return JSONObject().put("databaseExists",file.exists()).put("version",if(file.exists())1 else 0)
            .put("schemaSha256",schemaHash ?: JSONObject.NULL).put("platformMetadataSha256",platformHash ?: JSONObject.NULL)
            .put("legacyFilePresent",legacyFilePresent).put("legacyKeyCount",legacyKeys).put("localeChanges",localeChanges)
            .put("distributors",distributors).put("registrations",registrations).put("tokens",tokens).put("keys",keys)
    }
    /** A single fill owns the whole read snapshot; never refill/requery a moving database. */
    internal fun residentRows(db:SQLiteDatabase,sql:String,windowBytes:Long,maxRows:Int):Cursor {
        val cursor=db.rawQuery(sql,null)
        try {
            val windowed=cursor as? android.database.sqlite.SQLiteCursor ?: error("physical_retained_snapshot_unavailable")
            val window=android.database.CursorWindow("goatr-physical-retained",windowBytes)
            try {windowed.window=window}catch(error:Throwable) {window.close();throw error}
            val count=cursor.count
            check(windowed.window===window&&window.startPosition==0&&window.numRows==count&&count in 0..maxRows) {"physical_retained_snapshot_unavailable"}
            return cursor
        }catch(error:Throwable) {cursor.close();throw error}
    }
    fun requireFreshInstall(facts:JSONObject) {
        check(facts.getInt("schema")==1) {"physical_fresh_schema"}
        check(!facts.getBoolean("databaseExists")) {"physical_fresh_database_present"}
        val database=facts.getJSONObject("database")
        check(database.isNull("schemaSha256")&&database.isNull("globalSha256")&&!database.getJSONObject("quiescence").keys().hasNext()) {"physical_fresh_database_facts"}
        for(key in listOf("hosts","orphans","preferences","aliases"))check(facts.getJSONArray(key).length()==0) {"physical_fresh_${key}_present"}
        check(facts.getString("aliasSha256")==digest("")) {"physical_fresh_alias_facts"}
        check(!facts.getBoolean("serviceRequested")&&!facts.getBoolean("serviceRequestedPresent")&&!facts.getBoolean("runLabelCollision")) {"physical_fresh_request_or_label"}
        val files=facts.getJSONObject("keyFiles")
        check(files.isNull("directory")&&files.getJSONArray("files").length()==0) {"physical_fresh_key_material_present"}
        val connector=facts.getJSONObject("connector")
        check(!connector.getBoolean("databaseExists")&&connector.getInt("version")==0&&connector.isNull("schemaSha256")&&connector.isNull("platformMetadataSha256")
            &&connector.getLong("localeChanges")==0L&&!connector.getBoolean("legacyFilePresent")&&connector.getInt("legacyKeyCount")==0) {"physical_fresh_connector_present"}
        for(table in listOf("distributors","registrations","tokens","keys"))check(connector.getJSONArray(table).length()==0) {"physical_fresh_connector_rows"}
    }
    fun preRuntime(target:Context,args:JSONObject):JSONObject {
        check(args.getBoolean("freshInstall")) {"physical_fresh_install_required"}
        val facts=retained(target,args.getString("hostLabel"));requireFreshInstall(facts);return facts
    }
    private val db8Columns=sortedMapOf(
        "android_metadata" to mapOf("locale" to "TEXT:0:<null>:0"),
        "hosts" to mapOf("host" to "TEXT:0:<null>:1","metadata" to "TEXT:1:<null>:0"),
        "sessions" to mapOf("host" to "TEXT:1:<null>:1","session" to "TEXT:1:<null>:2","generation" to "TEXT:1:<null>:0","summary" to "TEXT:1:<null>:0",
            "partial" to "INTEGER:1:1:0","viewed" to "INTEGER:1:0:0","used" to "INTEGER:1:<null>:0","source" to "TEXT:0:<null>:0"),
        "records" to mapOf("host" to "TEXT:1:<null>:1","session" to "TEXT:1:<null>:2","id" to "TEXT:1:<null>:3","ord" to "INTEGER:1:<null>:0",
            "data" to "TEXT:1:<null>:0","bytes" to "INTEGER:1:<null>:0","at" to "INTEGER:1:<null>:0","origins" to "TEXT:1:<null>:0"),
        "content" to mapOf("host" to "TEXT:1:<null>:1","session" to "TEXT:1:<null>:0","generation" to "TEXT:1:<null>:0","source" to "TEXT:1:<null>:0","handle" to "TEXT:1:<null>:2",
            "data" to "BLOB:1:<null>:0","bytes" to "INTEGER:1:<null>:0","at" to "INTEGER:1:<null>:0","expires" to "INTEGER:1:<null>:0","used" to "INTEGER:1:<null>:0"),
        "actions" to mapOf("host" to "TEXT:1:<null>:1","action" to "TEXT:1:<null>:2","metadata" to "TEXT:1:<null>:0","fingerprint" to "BLOB:1:<null>:0",
            "receipt" to "TEXT:0:<null>:0","state" to "TEXT:0:<null>:0","authoritative" to "INTEGER:1:0:0","shown" to "INTEGER:1:0:0","updated" to "INTEGER:1:<null>:0"),
        "recovery" to mapOf("host" to "TEXT:1:<null>:1","action" to "TEXT:1:<null>:2","text" to "TEXT:1:<null>:0","bytes" to "INTEGER:1:<null>:0",
            "at" to "INTEGER:1:<null>:0","expires" to "INTEGER:1:<null>:0","consent" to "TEXT:1:<null>:0","options" to "TEXT:1:'{}':0"),
        "notifications" to mapOf("host" to "TEXT:1:<null>:1","sequence" to "INTEGER:1:<null>:2","id" to "TEXT:1:<null>:0","event" to "TEXT:1:<null>:0","session" to "TEXT:1:<null>:0","at" to "INTEGER:1:<null>:0"),
        "notification_state" to mapOf("host" to "TEXT:0:<null>:1","highwater" to "INTEGER:1:0:0","retained" to "INTEGER:1:1:0","presentation_floor" to "INTEGER:1:0:0"),
        "readwater" to mapOf("host" to "TEXT:1:<null>:1","session" to "TEXT:1:<null>:2","through" to "INTEGER:1:<null>:0","last_event" to "INTEGER:1:<null>:0"),
        "private_registration" to mapOf("host" to "TEXT:0:<null>:1","generation" to "TEXT:1:<null>:0","ciphertext" to "BLOB:1:<null>:0"),
        "retired_push" to mapOf("host" to "TEXT:1:<null>:1","generation" to "TEXT:1:<null>:2","at" to "INTEGER:1:<null>:0"),
        "key_deletion" to mapOf("host" to "TEXT:0:<null>:1","aliases" to "TEXT:1:<null>:0"),
        "inventory" to mapOf("host" to "TEXT:0:<null>:1","payload" to "TEXT:1:<null>:0","bytes" to "INTEGER:1:<null>:0","at" to "INTEGER:1:<null>:0"),
        "snippets" to mapOf("id" to "TEXT:0:<null>:1","host" to "TEXT:1:<null>:0","title" to "TEXT:1:<null>:0","body" to "TEXT:1:<null>:0","title_bytes" to "INTEGER:1:<null>:0","body_bytes" to "INTEGER:1:<null>:0"),
        "notification_presentations" to mapOf("host" to "TEXT:1:<null>:1","event" to "TEXT:1:<null>:2","sequence" to "INTEGER:1:<null>:0","session" to "TEXT:1:<null>:0","disposition" to "TEXT:1:<null>:0","active" to "INTEGER:1:0:0","attempted_at" to "INTEGER:1:<null>:0"),
        "notification_cancel_pending" to mapOf("host" to "TEXT:1:<null>:1","event" to "TEXT:1:<null>:2","attempted_at" to "INTEGER:1:<null>:0","attempts" to "INTEGER:1:0:0"),
        "host_routes" to mapOf("host" to "TEXT:0:<null>:1","revision" to "INTEGER:1:<null>:0","direct" to "TEXT:1:<null>:0","relay" to "TEXT:0:<null>:0"),
        "attachment_local" to mapOf("host" to "TEXT:1:<null>:1","upload" to "TEXT:1:<null>:2","record" to "TEXT:1:<null>:0"),
        "push_hints" to mapOf("host" to "TEXT:1:<null>:1","event" to "TEXT:1:<null>:2","sequence" to "INTEGER:1:<null>:0","session" to "TEXT:1:<null>:0","category" to "TEXT:1:<null>:0","at" to "INTEGER:1:<null>:0"),
    )
    private fun database(target:Context,hosts:java.util.SortedMap<String,HostRow>,now:Long):JSONObject {
        val file=File(dataRoot(target),"databases/goatr-private.db");val checks=JSONObject()
        val report=JSONObject().put("schemaSha256",JSONObject.NULL).put("globalSha256",JSONObject.NULL).put("measuredAt",now).put("quiescence",checks)
        if(!file.exists())return report
        check(file.isFile&&file.canonicalFile==file) {"physical_database_path"}
        SQLiteDatabase.openDatabase(file.path,null,SQLiteDatabase.OPEN_READONLY or SQLiteDatabase.NO_LOCALIZED_COLLATORS).use {db->
            check(db.version==8) {"physical_database_version"}
            val tables=sortedSetOf<String>()
            db.rawQuery("SELECT name,type FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name",null).use {rows->
                while(rows.moveToNext()){check(tables.size<db8Columns.size&&rows.getString(1)=="table"&&tables.add(rows.getString(0))) {"physical_db8_schema"}}
            }
            check(tables.toSet()==db8Columns.keys) {"physical_db8_schema"}
            val schema=MessageDigest.getInstance("SHA-256");val columns=sortedMapOf<String,List<String>>();val primary=mutableMapOf<String,List<String>>()
            for(table in tables) {
                val actual=sortedMapOf<String,String>();val keys=sortedMapOf<Int,String>()
                db.rawQuery("PRAGMA table_info(\"$table\")",null).use {rows->while(rows.moveToNext()) {
                    val name=rows.getString(1);check(actual.put(name,listOf(rows.getString(2),rows.getInt(3).toString(),if(rows.isNull(4))"<null>" else rows.getString(4),rows.getInt(5).toString()).joinToString(":"))==null) {"physical_db8_column"}
                    if(rows.getInt(5)>0)keys[rows.getInt(5)]=name
                }}
                check(actual==db8Columns.getValue(table)) {"physical_db8_columns"}
                update(schema,table);actual.forEach {(name,facts)->update(schema,name);update(schema,facts)}
                columns[table]=actual.keys.toList();primary[table]=keys.values.toList().ifEmpty {actual.keys.toList()}
                val parent=when(table) {"records","content"->"sessions";"recovery"->"actions";"android_metadata","hosts","key_deletion","notification_cancel_pending","attachment_local"->null;else->"hosts"}
                val references=when(table) {"records","content"->listOf("host","session");"recovery"->listOf("host","action");else->listOf("host")}
                val expectedFk=if(parent==null)emptyList()else references.mapIndexed {index,column->"$index:$parent:$column:$column:NO ACTION:CASCADE:NONE"}
                val foreign=mutableListOf<String>()
                db.rawQuery("PRAGMA foreign_key_list(\"$table\")",null).use {rows->while(rows.moveToNext()) {
                    check(foreign.size<2) {"physical_db8_foreign_keys"}
                    foreign+=listOf(rows.getInt(1).toString(),rows.getString(2),rows.getString(3),rows.getString(4),rows.getString(5),rows.getString(6),rows.getString(7)).joinToString(":")
                }}
                foreign.sort();check(foreign==expectedFk) {"physical_db8_foreign_keys"}
                foreign.forEach {update(schema,it)}
                val indexes=mutableListOf<String>()
                db.rawQuery("PRAGMA index_list(\"$table\")",null).use {rows->while(rows.moveToNext()) {
                    val name=rows.getString(1);val origin=rows.getString(3)
                    check(indexes.size<2&&rows.getInt(2)==1&&rows.getInt(4)==0&&(origin=="pk"||origin=="u")
                        &&name.matches(Regex("sqlite_autoindex_${table}_[1-9][0-9]*"))) {"physical_db8_indexes"}
                    val indexed=sortedMapOf<Int,String>();var auxiliary=0
                    db.rawQuery("PRAGMA index_xinfo(\"$name\")",null).use {fields->while(fields.moveToNext()) {
                        check(fields.getInt(3)==0&&fields.getString(4)=="BINARY") {"physical_db8_index_order"}
                        if(fields.getInt(5)==1) {
                            check(fields.getInt(1)>=0&&!fields.isNull(2)&&indexed.put(fields.getInt(0),fields.getString(2))==null) {"physical_db8_index_columns"}
                        }else {
                            check(fields.getInt(5)==0&&fields.getInt(1)==-1&&fields.isNull(2)&&++auxiliary==1) {"physical_db8_index_auxiliary"}
                        }
                    }}
                    var position=0;check(auxiliary==1&&indexed.keys.all {it==position++}) {"physical_db8_index_columns"}
                    indexes+="$origin:${indexed.values.joinToString(",")}"
                }}
                val expectedIndexes=mutableListOf<String>();if(keys.isNotEmpty())expectedIndexes+="pk:${keys.values.joinToString(",")}"
                when(table) {"notifications"->expectedIndexes+="u:host,id";"notification_presentations","push_hints"->expectedIndexes+="u:host,sequence"}
                indexes.sort();expectedIndexes.sort();check(indexes==expectedIndexes) {"physical_db8_indexes"}
                indexes.forEach {update(schema,it)}
            }
            val cutoff=now-30L*24*60*60*1000
            val locale=android.database.DatabaseUtils.sqlEscapeString(java.util.Locale.getDefault().toString())
            val predicates=sortedMapOf(
                "locale" to "SELECT abs((SELECT count(*) FROM android_metadata)-1)+(SELECT count(*) FROM android_metadata WHERE locale IS NULL OR locale<>$locale)",
                "viewed" to "SELECT count(*) FROM sessions WHERE viewed!=0",
                "pendingCancellation" to "SELECT count(*) FROM notification_cancel_pending",
                "pendingKeyDeletion" to "SELECT count(*) FROM key_deletion",
                "expiredRecovery" to "SELECT count(*) FROM recovery WHERE expires<=$now",
                "staleInventory" to "SELECT count(*) FROM inventory WHERE at<=$cutoff",
                "staleRecords" to "SELECT count(*) FROM records WHERE at<=$cutoff",
                "expiredContent" to "SELECT count(*) FROM content WHERE expires<=$now OR at<=$cutoff",
                "staleHints" to "SELECT count(*) FROM push_hints WHERE at<=$cutoff",
                "staleNotifications" to "SELECT count(*) FROM notifications WHERE at<=$cutoff",
                "notificationCapacity" to "SELECT count(*) FROM (SELECT host FROM notifications GROUP BY host HAVING count(*)>10000)",
                "stalePresentation" to "SELECT count(*) FROM notification_presentations WHERE active=0 AND attempted_at<=$cutoff",
                "presentationCapacity" to "SELECT count(*) FROM (SELECT host FROM notification_presentations WHERE active=0 GROUP BY host HAVING count(*)>10000)",
                "staleReadWater" to "SELECT count(*) FROM readwater r WHERE last_event<=$cutoff AND NOT EXISTS(SELECT 1 FROM notifications n WHERE n.host=r.host AND n.session=r.session)",
                "oldTerminalAction" to "SELECT count(*) FROM actions a WHERE updated<=$cutoff AND receipt IS NOT NULL AND state IN ('Dispatched','Succeeded','Failed','Uncertain') AND NOT EXISTS(SELECT 1 FROM recovery r WHERE r.host=a.host AND r.action=a.action)",
                "retainedFloor" to "SELECT count(*) FROM notification_state s WHERE retained<max(retained,coalesce((SELECT max(sequence)+1 FROM notifications n WHERE n.host=s.host AND at<=$cutoff),1),coalesce((SELECT sequence+1 FROM notifications n WHERE n.host=s.host ORDER BY sequence DESC LIMIT 1 OFFSET 10000),1))",
                "presentationFloor" to "SELECT count(*) FROM notification_state s WHERE presentation_floor<max(presentation_floor,coalesce((SELECT max(sequence) FROM notification_presentations p WHERE p.host=s.host AND active=0 AND attempted_at<=$cutoff),0),coalesce((SELECT sequence FROM notification_presentations p WHERE p.host=s.host AND active=0 ORDER BY sequence DESC LIMIT 1 OFFSET 10000),0))",
                "cacheBudget" to "SELECT CASE WHEN coalesce((SELECT sum(bytes) FROM records),0)+coalesce((SELECT sum(bytes) FROM content),0)+coalesce((SELECT sum(bytes) FROM inventory),0)>52428800 THEN 1 ELSE 0 END",
            )
            val width=columns.values.maxOf {it.size};val parts=mutableListOf<String>()
            for((table,names) in columns) {
                val owner=if(table=="android_metadata")"''" else "\"host\""
                val key=primary.getValue(table).joinToString("||") {"length(quote(\"$it\"))||':'||quote(\"$it\")"}
                val values=(names.map {"\"$it\""}+List(width-names.size){"NULL"}).joinToString(",")
                parts+="SELECT '$table' AS kind,$owner AS owner,$key AS item,$values FROM \"$table\""
            }
            for((name,query) in predicates)parts+="SELECT '<admission>','$name','',($query),${List(width-1){"NULL"}.joinToString(",")}"
            val global=MessageDigest.getInstance("SHA-256");var count=0;var bytes=0L
            // One fully resident compound SELECT owns all rows and first-open facts, without a write transaction.
            residentRows(db,parts.joinToString(" UNION ALL ")+" ORDER BY kind,owner,item",128L*1024*1024,200000).use {rows->while(rows.moveToNext()) {
                check(++count<=200000) {"physical_database_bound"}
                val table=rows.getString(0);val owner=rows.getString(1)
                if(table=="<admission>") {
                    check(rows.getType(3)==Cursor.FIELD_TYPE_INTEGER&&rows.getLong(3)>=0) {"physical_db8_quiescence"}
                    checks.put(owner,rows.getLong(3));continue
                }
                val names=columns.getValue(table)
                val record=if(table=="android_metadata")null else {
                    check(host.matches(owner)) {"physical_host_id"}
                    hosts.getOrPut(owner){check(hosts.size<256) {"physical_host_bound"};HostRow()}
                }
                val hash=record?.hash ?: global;update(hash,table)
                for((offset,column) in names.withIndex()) {
                    val index=offset+3;val type=rows.getType(index)
                    val value=when(type) {
                        Cursor.FIELD_TYPE_NULL->null
                        Cursor.FIELD_TYPE_BLOB->{val value=rows.getBlob(index);check(value.size<=8388608) {"physical_database_blob_bound"};bytes+=value.size;digest(value)}
                        else->rows.getString(index).also {check(it.length<=8388608) {"physical_database_value_bound"};bytes+=it.length}
                    }
                    check(bytes<=100663296) {"physical_database_total_bound"}
                    update(hash,column);update(hash,type.toString());update(hash,value ?: "")
                    if(table=="hosts"&&column=="metadata") {
                        check(record?.metadata==null) {"physical_duplicate_host"}
                        record?.metadata=JSONObject(checkNotNull(value)).also {check(it.getString("hostId")==owner) {"physical_host_identity"}}
                    }
                    if(table=="private_registration"&&column=="generation")record?.registration=value
                    if(table=="private_registration"&&column=="ciphertext")record?.ciphertext=value
                }
                record?.let {it.rows++;if(table=="actions")it.actions++}
            }}
            check(checks.getLong("pendingKeyDeletion")==0L) {"physical_pending_key_deletion"}
            report.put("schemaSha256",finished(schema)).put("globalSha256",finished(global))
        }
        return report
    }
    private fun keyFiles(target:Context,aliases:List<String>):JSONObject {
        val parent=File(dataRoot(target),"no_backup");val directory=File(parent,"goatr-keys");val output=JSONArray()
        if(!directory.exists())return JSONObject().put("directory",JSONObject.NULL).put("files",output)
        check(parent.canonicalFile==parent&&directory.canonicalFile==directory) {"physical_key_directory_alias"}
        val dir=Os.lstat(directory.path);check(OsConstants.S_ISDIR(dir.st_mode)&&dir.st_uid==Process.myUid()&&dir.st_mode and 0xfff==0x1c0) {"physical_key_directory"}
        val children=directory.listFiles()?.sortedBy {it.name} ?: error("physical_key_directory_list");check(children.size<=1026) {"physical_key_file_bound"}
        val seen=sortedSetOf<String>()
        fun stamp(stat:android.system.StructStat)=listOf(stat.st_dev,stat.st_ino,stat.st_size,stat.st_mode.toLong(),stat.st_uid.toLong(),stat.st_gid.toLong(),stat.st_nlink,stat.st_mtime,stat.st_ctime)
        for(file in children) {
            check(file.canonicalFile==file&&file.name.matches(Regex("goatr\\.(installation\\.endpoint|host\\.[0-9a-f]{64}\\.(receipt|push))\\.gcm(\\.initialized)?"))) {"physical_key_file_unknown_or_backup"}
            val before=Os.lstat(file.path);check(OsConstants.S_ISREG(before.st_mode)&&before.st_uid==Process.myUid()&&before.st_mode and 0xfff==0x180&&before.st_nlink==1L) {"physical_key_file_metadata"}
            val marker=file.name.endsWith(".initialized");check(before.st_size==if(marker)4L else 61L) {"physical_key_file_size"}
            val alias=file.name.removeSuffix(".initialized").removeSuffix(".gcm");check(alias in aliases) {"physical_key_file_missing_alias"}
            val fd=Os.open(file.path,OsConstants.O_RDONLY or OsConstants.O_NOFOLLOW or OsConstants.O_CLOEXEC,0)
            val hash=MessageDigest.getInstance("SHA-256")
            java.io.FileInputStream(fd).use {input->
                check(stamp(Os.fstat(fd))==stamp(before)) {"physical_key_file_changed"}
                val value=ByteArray(if(marker)4 else 61);var offset=0
                while(offset<value.size){val n=input.read(value,offset,value.size-offset);check(n>0) {"physical_key_file_partial"};offset+=n}
                check(input.read()==-1&&(if(marker)value.contentEquals(byteArrayOf(71,65,73,49))else value[0]==1.toByte())) {"physical_key_file_format"}
                hash.update(value);value.fill(0);check(stamp(Os.fstat(fd))==stamp(before)) {"physical_key_file_changed"}
            }
            check(stamp(Os.lstat(file.path))==stamp(before)) {"physical_key_file_changed"}
            output.put(JSONObject().put("name",file.name).put("sha256",finished(hash)).put("device",before.st_dev).put("inode",before.st_ino).put("size",before.st_size)
                .put("mode",before.st_mode and 0xfff).put("uid",before.st_uid).put("gid",before.st_gid).put("links",before.st_nlink).put("mtime",before.st_mtime).put("ctime",before.st_ctime))
            seen+=file.name
        }
        for(name in seen){val base=name.removeSuffix(".initialized");check(base in seen&&"$base.initialized" in seen) {"physical_key_file_partial"}}
        return JSONObject().put("directory",JSONObject().put("device",dir.st_dev).put("inode",dir.st_ino).put("mode",dir.st_mode and 0xfff).put("uid",dir.st_uid)).put("files",output)
    }
    /** One read-only SQL statement pins its complete cursor snapshot; no SessionStore or writable open. */
    fun retained(target:Context,label:String):JSONObject {
        val before=preferences(target)
        val hosts=sortedMapOf<String,HostRow>()
        val databaseFile=File(dataRoot(target),"databases/goatr-private.db")
        val databaseFacts=database(target,hosts,System.currentTimeMillis())
        val aliases=KeyStore.getInstance("AndroidKeyStore").apply {load(null)}.aliases().toList().sorted()
        check(aliases.size<=256&&aliases.all {it.length<=512&&!it.any(Char::isISOControl)}) {"physical_alias_bound"}
        val after=preferences(target);check(before.first.toString()==after.first.toString()) {"physical_preferences_changed_during_read"}
        val legacy=after.second["unifiedpush.connector"].orEmpty()
        val connector=connector(target,after.second.containsKey("unifiedpush.connector"),legacy.size)
        val connectorMaterial=listOf("registrations","tokens","keys").sumOf {connector.getJSONArray(it).length()}
        check(connectorMaterial==0||"UnifiedPush" in aliases) {"physical_connector_missing_wrapping_alias"}
        val delivery=after.second["delivery_preferences"].orEmpty()
        val output=JSONArray();val orphans=JSONArray();var collision=false
        hosts.forEach {(id,record)->
            val metadata=record.metadata
            if(metadata==null){orphans.put(JSONObject().put("host",id).put("rowsSha256",finished(record.hash)).put("rowCount",record.rows));return@forEach}
            val name=metadata.getString("name");collision=collision||name==label
            val device=if(metadata.has("deviceId")&&!metadata.isNull("deviceId"))metadata.getString("deviceId").also {check(UUID.fromString(it).toString()==it) {"physical_device_id"}}else null
            val expected=metadata.optJSONArray("keyAliases") ?: JSONArray();check(expected.length()<=8)
            val expectedNames=(0 until expected.length()).map {expected.getString(it)}.sorted();check(aliases.containsAll(expectedNames)) {"physical_alias_missing"}
            val suffix=digest(id)
            output.put(JSONObject().put("host",id).put("device",device ?: JSONObject.NULL).put("metadataSha256",digest(metadata.toString()))
                .put("rowsSha256",finished(record.hash)).put("actionCount",record.actions).put("keyAliases",JSONArray(expectedNames))
                .put("registrationGeneration",record.registration ?: JSONObject.NULL).put("registrationSha256",record.ciphertext ?: JSONObject.NULL)
                .put("mode",if(delivery["mode.$suffix"]?.second=="ntfy")"Ntfy" else "Persistent")
                .put("enabled",delivery["enabled.$suffix"]?.second=="true").put("completed",delivery["completed.$suffix"]?.second=="true"))
        }
        return JSONObject().put("schema",1).put("databaseExists",databaseFile.exists()).put("database",databaseFacts).put("orphans",orphans).put("keyFiles",keyFiles(target,aliases))
            .put("hosts",output).put("preferences",after.first).put("connector",connector)
            .put("aliases",JSONArray(aliases)).put("aliasSha256",digest(aliases.joinToString("\u0000")))
            .put("serviceRequested",delivery["service_requested"]?.second=="true").put("serviceRequestedPresent",delivery.containsKey("service_requested")).put("runLabelCollision",collision)
            .also {check(it.toString().toByteArray(StandardCharsets.UTF_8).size<=60000) {"physical_snapshot_bound"}}
    }
    fun registration(target:Context,selected:String):Pair<String,String>? {
        check(host.matches(selected)) {"physical_registration_host"}
        val file=File(dataRoot(target),"databases/goatr-private.db")
        if(!file.exists())return null
        check(file.isFile&&file.canonicalFile==file) {"physical_database_path"}
        return SQLiteDatabase.openDatabase(file.path,null,SQLiteDatabase.OPEN_READONLY or SQLiteDatabase.NO_LOCALIZED_COLLATORS).use {db->
            check(db.version==8) {"physical_database_version"}
            db.rawQuery("SELECT generation,ciphertext FROM private_registration WHERE host=?",arrayOf(selected)).use {rows->
                if(!rows.moveToFirst())null else {
                    val generation=rows.getString(0);check(UUID.fromString(generation).toString()==generation) {"physical_registration_generation"}
                    val ciphertext=rows.getBlob(1);check(ciphertext.size in 1..65536&&!rows.moveToNext()) {"physical_registration_bound"}
                    generation to digest(ciphertext)
                }
            }
        }
    }
    fun notificationLedger(target:Context,selected:String,event:String):JSONObject {
        check(host.matches(selected)&&UUID.fromString(event).toString()==event) {"physical_notification_identity"}
        val result=JSONObject().put("present",false).put("sequence",JSONObject.NULL).put("session",JSONObject.NULL)
            .put("dispositionSha256",JSONObject.NULL).put("active",JSONObject.NULL).put("attemptedAt",JSONObject.NULL).put("readThrough",0)
        val file=File(dataRoot(target),"databases/goatr-private.db")
        if(!file.exists())return result
        check(file.isFile&&file.canonicalFile==file) {"physical_database_path"}
        SQLiteDatabase.openDatabase(file.path,null,SQLiteDatabase.OPEN_READONLY or SQLiteDatabase.NO_LOCALIZED_COLLATORS).use {db->
            check(db.version==8) {"physical_database_version"}
            db.rawQuery("SELECT p.sequence,p.session,p.disposition,p.active,p.attempted_at,r.through FROM notification_presentations p LEFT JOIN readwater r ON r.host=p.host AND r.session=p.session WHERE p.host=? AND p.event=?",arrayOf(selected,event)).use {rows->
                if(rows.moveToFirst()) {
                    val session=rows.getString(1);check(UUID.fromString(session).toString()==session) {"physical_notification_session"}
                    val disposition=rows.getString(2);check(disposition.length<=64) {"physical_notification_disposition"}
                    result.put("present",true).put("sequence",rows.getLong(0)).put("session",session).put("dispositionSha256",digest(disposition))
                        .put("active",rows.getLong(3)!=0L).put("attemptedAt",if(rows.isNull(4))JSONObject.NULL else rows.getLong(4))
                        .put("readThrough",if(rows.isNull(5))0 else rows.getLong(5))
                    check(!rows.moveToNext()) {"physical_notification_ledger_duplicate"}
                }
            }
        }
        return result
    }
    fun actionIds(target:Context,selected:String):JSONArray {
        check(host.matches(selected)) {"physical_action_host"}
        val output=JSONArray();val file=File(dataRoot(target),"databases/goatr-private.db")
        if(!file.exists())return output
        check(file.isFile&&file.canonicalFile==file) {"physical_database_path"}
        SQLiteDatabase.openDatabase(file.path,null,SQLiteDatabase.OPEN_READONLY or SQLiteDatabase.NO_LOCALIZED_COLLATORS).use {db->
            check(db.version==8) {"physical_database_version"}
            db.rawQuery("SELECT action FROM actions WHERE host=? ORDER BY action",arrayOf(selected)).use {rows->
                while(rows.moveToNext()) {val action=rows.getString(0);check(output.length()<4096&&UUID.fromString(action).toString()==action) {"physical_action_bound"};output.put(action)}
            }
        }
        return output
    }
    fun exchange(runner:OwnedInstrumentationRunner,port:Int,request:JSONObject):JSONObject {
        val bytes=request.toString().toByteArray(StandardCharsets.UTF_8);check(bytes.size in 1..65536)
        try {
            Socket(InetAddress.getLoopbackAddress(),port).use {socket->
                socket.soTimeout=15000
                val input=DataInputStream(socket.getInputStream());val output=DataOutputStream(socket.getOutputStream())
                val challenge=ByteArray(32);input.readFully(challenge)
                val identity=runner.controlIdentity();val signature=runner.controlSignature(challenge,bytes)
                output.write(identity);output.writeByte(signature.size);output.write(signature);output.writeInt(bytes.size);output.write(bytes);output.flush()
                val length=input.readInt();check(length in 2..65536) {"physical_reply_bound"}
                val reply=ByteArray(length);input.readFully(reply)
                return JSONObject(String(reply,StandardCharsets.UTF_8))
            }
        }finally{bytes.fill(0)}
    }
}

/** One plain, instrumented product-process observer. No UiAutomation, JUnit, ActivityScenario or runtime claim. */
internal class PhysicalMvpObserver(private val runner:OwnedInstrumentationRunner,private val args:JSONObject,private val app:Application) {
    private var current=WeakReference<MainActivity>(null)
    private var sequence=0L
    private val run=args.getString("runId")
    private val host=args.getString("host")
    private val original=PhysicalMvpPlatform.process()
    private val apk=File(app.applicationInfo.sourceDir).canonicalFile
    private val heldApk=apk.inputStream()
    private val apkIdentity=Os.fstat(heldApk.fd)
    private val runtime get()=(app as GoatrApplication).runtime
    private var previousShell:Pair<ShellControlScope,TerminalInputOwner?>?=null
    private val callbacks=object:Application.ActivityLifecycleCallbacks {
        override fun onActivityResumed(activity:Activity) {if(activity is MainActivity)current=WeakReference(activity)}
        override fun onActivityPaused(activity:Activity) {if(current.get()===activity)current.clear()}
        override fun onActivityCreated(activity:Activity,state:Bundle?) {}
        override fun onActivityStarted(activity:Activity) {}
        override fun onActivityStopped(activity:Activity) {}
        override fun onActivitySaveInstanceState(activity:Activity,state:Bundle) {}
        override fun onActivityDestroyed(activity:Activity) {if(current.get()===activity)current.clear()}
    }
    private fun checkProcess() {
        val now=PhysicalMvpPlatform.process();val image=Os.stat(apk.path);val held=Os.fstat(heldApk.fd)
        check(now.getInt("pid")==original.getInt("pid")&&now.getString("start")==original.getString("start")
            &&now.getInt("uid")==original.getInt("uid")&&now.getInt("uid")==app.applicationInfo.uid
            &&now.getString("name")==PhysicalMvpPlatform.PRODUCT) {"physical_product_epoch"}
        check(File(app.applicationInfo.sourceDir).canonicalFile==apk&&image.st_dev==held.st_dev&&image.st_ino==held.st_ino
            &&image.st_size==apkIdentity.st_size&&image.st_mtime==apkIdentity.st_mtime) {"physical_product_artifact"}
    }
    private fun <T> main(block:()->T):T {
        var result:Result<T>?=null
        runner.runOnMainSync {result=runCatching(block)}
        return checkNotNull(result).getOrThrow()
    }
    private fun owner():GoatrViewModel? {
        val activity=current.get() ?: return null
        if(activity.isFinishing||activity.isDestroyed||!activity.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED))return null
        // MainActivity.onStart has already initialized this exact default-key owner before RESUMED.
        return androidx.lifecycle.ViewModelProvider(activity,androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.getInstance(app))[GoatrViewModel::class.java]
    }
    private fun receipt(receipt:Receipt):JSONObject {
        val resources=receipt.creation?.resources
        val shell=(receipt.result as? ActionResultShellCreation)?.resources
        return JSONObject().put("action",receipt.actionId.value).put("method",receipt.method.name).put("state",receipt.state.name)
            .put("target",receipt.target?.sessionId?.value ?: JSONObject.NULL).put("generation",receipt.target?.generation?.value ?: JSONObject.NULL)
            .put("shell",receipt.shellTarget?.shellId?.value ?: JSONObject.NULL).put("shellGeneration",receipt.shellTarget?.generation?.value ?: JSONObject.NULL)
            .put("createdSession",resources?.sessionId?.value ?: JSONObject.NULL).put("createdGeneration",resources?.generation?.value ?: JSONObject.NULL)
            .put("createdShell",shell?.createdTarget?.shellId?.value ?: JSONObject.NULL).put("createdShellGeneration",shell?.createdTarget?.generation?.value ?: JSONObject.NULL)
            .put("sha256",PhysicalMvpPlatform.digest(Protocol.strictJson.encodeToString(Receipt.serializer(),receipt)))
    }
    /** Hash decoded immutable display fields without copying/serializing/exporting its cell body. */
    private fun displayFacts(grid:TerminalGrid?):JSONObject {
        if(grid==null)return JSONObject().put("present",false).put("sha256",JSONObject.NULL).put("styled",false).put("revision",0)
        check(grid.cells.size<=8192) {"physical_grid_bound"}
        val hash=MessageDigest.getInstance("SHA-256")
        val sink=object:java.io.OutputStream() {
            override fun write(value:Int){hash.update(value.toByte())}
            override fun write(bytes:ByteArray,offset:Int,length:Int){hash.update(bytes,offset,length)}
        }
        val out=java.io.DataOutputStream(java.io.BufferedOutputStream(sink,8192));var styled=false
        fun color(value:GridColor?) {when(value) {
            null->out.writeByte(3)
            GridColor.Default->out.writeByte(0)
            is GridColor.Indexed->{out.writeByte(1);out.writeInt(value.index.value.toInt())}
            is GridColor.Rgb->{out.writeByte(2);out.writeInt(value.value.value.toInt())}
        }}
        out.writeInt(grid.begin.columns.value.toInt());out.writeInt(grid.begin.rows.value.toInt())
        for(cell in grid.cells) {
            out.writeInt(cell.text.length);for(character in cell.text)out.writeChar(character.code)
            out.writeByte(cell.width.value.toInt());color(cell.foreground);color(cell.background)
            val a=cell.attributes
            val flags=(if(a.bold)1 else 0) or (if(a.faint)2 else 0) or (if(a.italic)4 else 0) or (if(a.blink)8 else 0) or (if(a.strike)16 else 0) or (if(a.inverse)32 else 0) or (if(a.concealed)64 else 0)
            out.writeByte(flags);out.writeByte(a.underline.ordinal);color(a.underlineColor)
            styled=styled||flags!=0||a.underline!=GridUnderline.None||a.underlineColor!=null||cell.foreground!=GridColor.Default||cell.background!=GridColor.Default
        }
        out.flush()
        return JSONObject().put("present",true).put("sha256",PhysicalMvpPlatform.finished(hash)).put("styled",styled).put("revision",grid.begin.revision.value)
    }
    private fun snapshot(marker:String?):JSONObject=main {
        checkProcess();val model=owner();val view=runtime.connection.state.value;val inventory=view.inventory[host]
        val sessions=JSONArray();inventory?.sessions.orEmpty().forEach {session->sessions.put(JSONObject()
            .put("session",session.sessionId.value).put("generation",session.generation.value).put("provider",session.provider.name)
            .put("native",session.nativeId.value).put("server",session.serverId.value).put("workspace",session.workspaceId.value)
            .put("pane",session.paneId.value).put("terminal",session.terminalId.value).put("ready",session.connectivity==SessionConnectivity.Ready))}
        val shells=JSONArray();inventory?.shells.orEmpty().forEach {shell->shells.put(JSONObject().put("shell",shell.target.shellId.value)
            .put("generation",shell.target.generation.value).put("server",shell.serverId.value).put("workspace",shell.workspaceId.value)
            .put("tab",shell.tabId.value).put("pane",shell.paneId.value).put("terminal",shell.terminalId.value).put("ready",shell.connectivity==SessionConnectivity.Ready))}
        val selected=model?.selection?.takeIf {it.host==host}
        val reducer=selected?.let {key->view.sessions[host]?.singleOrNull {it.state?.summary?.let {summary->Target(summary.sessionId,summary.generation)==key.target}==true}}
        val controls=model?.sessionControls?.takeIf {it.key==selected}
        val catalogue=controls?.catalogue
        val currentModel=catalogue?.let {source->source.models.singleOrNull {it.reference==source.current.modelRef}?.label?.value}
        val optionHash=catalogue?.let {PhysicalMvpPlatform.digest(JSONObject().put("model",currentModel ?: JSONObject.NULL)
            .put("effort",it.current.effort ?: JSONObject.NULL).put("mode",it.current.mode?.name ?: JSONObject.NULL).toString())}
        val records=reducer?.history.orEmpty();check(records.size<=4096) {"physical_history_bound"}
        fun markerCount(role:Role)=if(marker==null)0 else records.count {it.complete&&it.kind==RecordKind.Message&&it.role==role&&(it.text as? ContentText)?.text?.value?.contains(marker)==true}
        val queueHash=controls?.takeIf {it.queueLoaded}?.let {shown->
            val hash=MessageDigest.getInstance("SHA-256")
            for(entry in shown.queueEntries) {
                PhysicalMvpPlatform.update(hash,entry.id.value)
                PhysicalMvpPlatform.update(hash,(entry.text as? ContentText)?.text?.value ?: "<nontext>")
                hash.update(if(entry.editable)1.toByte()else 0.toByte())
            };PhysicalMvpPlatform.finished(hash)
        }
        val conversation=JSONObject().put("historyCount",records.size).put("historyIds",JSONArray(records.map {it.id.value}))
            .put("browsingOlder",reducer?.browsingOlder==true).put("hasOlder",reducer?.nextCursor!=null).put("viewId",reducer?.viewId?.value ?: JSONObject.NULL)
            .put("activity",reducer?.state?.activity?.state?.name ?: "None").put("controlsRevision",reducer?.controlsRevision ?: 0)
            .put("optionsSha256",optionHash ?: JSONObject.NULL).put("optionsLoading",controls?.loading==true).put("optionsPending",controls?.pending==true)
            .put("userMarkerCount",markerCount(Role.User)).put("assistantMarkerCount",markerCount(Role.Assistant))
            .put("queueAvailable",catalogue?.queueAvailable==true).put("queueLoaded",controls?.queueLoaded==true).put("queueCount",controls?.queueEntries?.size ?: 0)
            .put("queueSha256",queueHash ?: JSONObject.NULL).put("delivery",selected?.let {model?.composerOptions(it)?.delivery?.name ?: "Native"} ?: "None")
            .put("offeredDeliveries",JSONArray(catalogue?.deliveries.orEmpty().map {it.name}))
        val pending=JSONArray();reducer?.pending.orEmpty().forEach {pending.put(JSONObject().put("request",it.requestKey.value).put("kind",it.kind.name))}
        val actions=JSONArray();val known=model?.actions?.get(host).orEmpty().mapNotNull {it.receipt}.toMutableList()
        model?.focusedAction?.receipt?.let {known+=it};view.latestReceipt[host]?.let {known+=it}
        controls?.receipt?.let {known+=it};controls?.queueReceipt?.let {known+=it}
        known.distinctBy {it.actionId}.forEach {actions.put(receipt(it))}
        val shell=model?.shell?.takeIf {it.selection.host==host};val grid=shell?.grid
        val shellScope=shell?.let {ShellControlScope(it.selection,it.source,it.ticket)}
        val draft=if(shell?.controlBegun==true)shell?.selection?.let {model?.shellDraft(it)}else null
        val input=JSONObject().put("offered",shellScope?.let {model?.shellInputAllowed(it)}==true)
            .put("draftBytes",draft?.value?.text?.let(::utf8Size) ?: JSONObject.NULL).put("composition",draft?.value?.let {it.composition!=null} ?: JSONObject.NULL)
            .put("direct",draft?.direct ?: JSONObject.NULL)
        val creationReady=model?.let {owner->owner.terminalCreation?.takeIf {it.host==host}?.let(owner::terminalCreationAllowed)}==true
        var markerOutput=false
        if(marker!=null&&grid!=null) {
            val width=grid.begin.columns.value.toInt();check(grid.cells.size<=32768)
            for(row in 0 until grid.begin.rows.value.toInt()) {
                val line=buildString {for(column in 0 until width)append(grid.cells[row*width+column].text)}.trimEnd()
                if(line==marker)markerOutput=true
            }
        }
        val old=previousShell
        val retired=old==null||model?.terminalOwner(old.first)!==old.second||old.second?.retained()!=true
        if(shellScope!=null)previousShell=shellScope to model?.terminalOwner(shellScope)
        val keeper=keeperFacts()
        val preview=model?.preview
        val pairing=JSONObject().put("screenPairing",model?.screen?.name=="Pairing").put("previewPresent",preview!=null)
            .put("hostMatches",preview?.host?.value==host).put("nameMatches",preview?.name?.value==args.getString("hostLabel"))
            .put("fingerprintMatches",preview?.fingerprint==host.chunked(8).joinToString(" "))
        JSONObject().put("kind","snapshot").put("process",original).put("host",host).put("foreground",model?.foreground==true)
            .put("screen",model?.screen?.name ?: "None").put("selectedHost",model?.selection?.host ?: model?.host ?: JSONObject.NULL)
            .put("selectedSession",selected?.target?.sessionId?.value ?: JSONObject.NULL).put("selectedGeneration",selected?.target?.generation?.value ?: JSONObject.NULL)
            .put("selectedShell",shell?.selection?.target?.shellId?.value ?: JSONObject.NULL).put("selectedShellGeneration",shell?.selection?.target?.generation?.value ?: JSONObject.NULL)
            .put("hello",host in view.hello).put("inventoryReady",view.inventoryReady[host]==true).put("mutationReady",view.mutationReady[host]==true)
            .put("device",view.hello[host]?.deviceId?.value ?: JSONObject.NULL).put("pairing",pairing)
            .put("route",view.selectedPath[host] ?: "unknown").put("sessions",sessions).put("shells",shells).put("pending",pending).put("receipts",actions)
            .put("actionIds",PhysicalMvpPlatform.actionIds(app,host))
            .put("controlBegun",shell?.controlBegun==true).put("grid",grid!=null).put("columns",grid?.begin?.columns?.value ?: 0)
            .put("shellInput",input).put("creationReady",creationReady)
            .put("rows",grid?.begin?.rows?.value ?: 0).put("revision",grid?.begin?.revision?.value ?: 0).put("alternate",grid?.begin?.alternateScreen==true)
            .put("scrollOffset",grid?.begin?.scroll?.offsetFromBottom?.value ?: -1).put("scrollMaximum",grid?.begin?.scroll?.maxOffsetFromBottom?.value ?: -1)
            .put("scrollViewport",grid?.begin?.scroll?.viewportRows?.value ?: -1)
            .put("historyMode",shell?.history?.mode?.name ?: "None").put("markerOutput",markerOutput).put("oldInputRetired",retired)
            .put("agentTerminal",model?.terminal?.let {it.key==selected&&it.grid!=null}==true).put("agentDisplay",displayFacts(model?.terminal?.takeIf {it.key==selected}?.grid))
            .put("keeper",keeper).put("conversation",conversation).put("network",networkFacts())
            .put("ui",JSONObject().put("orientation",app.resources.configuration.orientation).put("fontScale",app.resources.configuration.fontScale.toDouble())
                .put("imeVisible",current.get()?.window?.decorView?.rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime())==true)
                .put("interactive",app.getSystemService(android.os.PowerManager::class.java).isInteractive).put("measuredAt",android.os.SystemClock.elapsedRealtime()))
    }
    private fun networkFacts():JSONObject {
        val manager=app.getSystemService(android.net.ConnectivityManager::class.java)
        val active=manager.activeNetwork?.let {manager.getNetworkCapabilities(it)}
        val available=manager.allNetworks.mapNotNull {manager.getNetworkCapabilities(it)}.filter {it.hasCapability(android.net.NetworkCapabilities.NET_CAPABILITY_INTERNET)}
        fun any(transport:Int)=available.any {it.hasTransport(transport)}
        return JSONObject().put("internet",active?.hasCapability(android.net.NetworkCapabilities.NET_CAPABILITY_INTERNET)==true)
            .put("validated",active?.hasCapability(android.net.NetworkCapabilities.NET_CAPABILITY_VALIDATED)==true)
            .put("cellular",active?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR)==true)
            .put("wifi",any(android.net.NetworkCapabilities.TRANSPORT_WIFI)).put("ethernet",any(android.net.NetworkCapabilities.TRANSPORT_ETHERNET))
            .put("vpn",any(android.net.NetworkCapabilities.TRANSPORT_VPN)).put("usb",any(android.net.NetworkCapabilities.TRANSPORT_USB))
    }
    private fun keeperFacts():JSONObject {
        val manager=app.getSystemService(NotificationManager::class.java)
        val owned=manager.activeNotifications.filter {it.packageName==PhysicalMvpPlatform.PRODUCT&&it.uid==Process.myUid()&&it.id==7124&&it.notification.channelId==NotificationCoordinator.FOREGROUND_CHANNEL}
        check(owned.size<=1) {"physical_keeper_notification_ambiguous"}
        val registration=PhysicalMvpPlatform.registration(app,host)
        val retained=JSONArray();runtime.store.state.value.hosts.filter {it.hostId!=host}.sortedBy {it.hostId}.forEach {metadata->retained.put(JSONObject()
            .put("host",metadata.hostId).put("requiresKeeper",runtime.deliveryPreferences.requiresKeeper(metadata.hostId,runtime.push.verified(metadata.hostId))))}
        val health=runtime.notifications.health()
        return JSONObject().put("running",DeliveryService.status.value.running).put("reconnecting",DeliveryService.status.value.reconnecting.size)
            .put("foregroundNotification",owned.isNotEmpty()).put("endpointOwned",GoatrConnection.endpointIsOwned()).put("networkCallback",runtime.connection.ownsNetworkCallback())
            .put("serviceRequested",runtime.deliveryPreferences.serviceRequested()).put("privateEnabled",runtime.deliveryPreferences.hostEnabled(host))
            .put("privateMode",runtime.deliveryPreferences.mode(host).name).put("privateVerified",runtime.push.verified(host))
            .put("privateCompleted",runtime.deliveryPreferences.notifyCompleted(host))
            .put("privateRequiresKeeper",runtime.deliveryPreferences.requiresKeeper(host,runtime.push.verified(host))).put("retained",retained)
            .put("privateRegistrationGeneration",registration?.first ?: JSONObject.NULL).put("privateRegistrationSha256",registration?.second ?: JSONObject.NULL)
            .put("health",JSONObject().put("globalAvailable",health.globalAvailable).put("runtimePermissionRequired",health.runtimePermissionRequired)
                .put("attentionChannelEnabled",health.attentionChannelEnabled).put("deliveryChannelEnabled",health.deliveryChannelEnabled))
    }
    private fun notification(query:JSONObject):JSONObject {
        check(query.getString("host")==host) {"physical_notification_host"}
        val event=query.getString("event");check(UUID.fromString(event).toString()==event) {"physical_notification_event"}
        val category=query.getString("category");check(category=="attention"||category=="completed") {"physical_notification_category"}
        val hint=query.getBoolean("hint")
        val title=if(hint) {if(category=="completed")"Work finished on desktop" else "Desktop needs attention"}else if(category=="completed")"Agent finished" else "Agent needs attention"
        val expectedCategory=if(!hint&&category=="attention")Notification.CATEGORY_MESSAGE else Notification.CATEGORY_STATUS
        val matches=app.getSystemService(NotificationManager::class.java).activeNotifications.filter {it.tag=="goatr.attention.$host.$event"&&it.id==1&&it.packageName==PhysicalMvpPlatform.PRODUCT&&it.uid==Process.myUid()}
        check(matches.size<=1) {"physical_notification_ambiguous"}
        val selected=matches.singleOrNull()?.notification
        val valid=selected!=null&&selected.category==expectedCategory&&selected.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()==title&&selected.contentIntent!=null
        return JSONObject().put("kind","notification").put("host",host).put("event",event).put("present",selected!=null).put("qualified",valid)
            .put("ledger",PhysicalMvpPlatform.notificationLedger(app,host,event))
    }
    fun run() {
        app.registerActivityLifecycleCallbacks(callbacks)
        try {
            var report=JSONObject().put("kind","ready").put("process",original).put("retained",PhysicalMvpPlatform.retained(app,args.getString("hostLabel")))
            while(true) {
                checkProcess()
                val request=JSONObject().put("method","physical_report").put("runId",run).put("sequence",sequence).put("report",report)
                val query=PhysicalMvpPlatform.exchange(runner,args.getInt("port"),request)
                check(query.getString("runId")==run&&query.getLong("sequence")==sequence++) {"physical_query_epoch"}
                val kind=query.getString("kind")
                val expected=when(kind) {
                    "snapshot"->setOf("kind","runId","sequence","marker")
                    "epoch"->setOf("kind","runId","sequence","actionRunId")
                    "notification"->setOf("kind","runId","sequence","host","event","category","hint")
                    "retained","keeper","finish","wait"->setOf("kind","runId","sequence")
                    else->error("physical_query_kind")
                }
                check(query.keys().asSequence().toSet()==expected) {"physical_query_closed"}
                report=when(kind) {
                    "snapshot"->{val marker=query.optString("marker","").takeIf {it.isNotEmpty()};val suffix=run.replace("-","")
                        val phases=listOf("existing","created","post-terminal","network","queue","stop","persistent-attention","persistent-completed","ntfy-attention","ntfy-completed")
                        val agent=phases.any {phase->val pin=PhysicalMvpPlatform.digest("$run|$phase").take(32);marker=="GOATR_MOBILE_OMP_$pin"||marker=="GOATR_MOBILE_CODEX_$pin"}
                        check(marker==null||marker=="GOATR_MVP_$run"||marker=="GOATR_MVP_PASTE_$run"||marker=="GOATR_MVP_KEYS_$run"||marker=="GOATR_MOBILE_OMP_$suffix"||marker=="GOATR_MOBILE_CODEX_$suffix"||agent);snapshot(marker)}
                    "epoch"->{
                        val action=query.getString("actionRunId");check(UUID.fromString(action).toString()==action)
                        JSONObject().put("kind","epoch").put("actionRunId",action).put("process",original)
                            .put("observedAt",android.os.SystemClock.elapsedRealtime()).put("apkSha256",args.getString("physicalMvpExpectedProductSha256"))
                            .put("snapshot",snapshot(null))
                    }
                    "notification"->notification(query)
                    "retained"->JSONObject().put("kind","retained").put("facts",PhysicalMvpPlatform.retained(app,args.getString("hostLabel")))
                    "keeper"->main {JSONObject().put("kind","keeper").put("facts",keeperFacts())}
                    "wait"->{Thread.sleep(100);JSONObject().put("kind","idle").put("process",original)}
                    else->{
                        checkProcess();val finish=JSONObject().put("method","physical_report").put("runId",run).put("sequence",sequence).put("report",JSONObject().put("kind","finished").put("process",original))
                        val accepted=PhysicalMvpPlatform.exchange(runner,args.getInt("port"),finish)
                        check(accepted.getString("kind")=="finished"&&accepted.getString("runId")==run&&accepted.getLong("sequence")==sequence) {"physical_finish"}
                        return
                    }
                }
            }
        }finally{app.unregisterActivityLifecycleCallbacks(callbacks);heldApk.close()}
    }
}
