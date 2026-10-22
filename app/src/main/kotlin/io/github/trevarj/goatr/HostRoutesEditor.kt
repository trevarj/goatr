// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/** Local settings only. Saving never reconnects, changes identity, or resends a session action. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun HostRoutesEditor(host: String,store: StoreView,connection: ConnectionView,model: GoatrViewModel) {
    val saved=store.routes.singleOrNull {it.hostId==host} ?: SavedHostRoutes(host)
    val paired=store.hosts.singleOrNull {it.hostId==host}?.deviceId!=null
    var direct by remember(host) {mutableStateOf(saved.direct)}
    var relay by remember(host) {mutableStateOf(saved.relay.orEmpty())}
    var revision by remember(host) {mutableLongStateOf(saved.revision)}
    var dirty by remember(host) {mutableStateOf(false)}
    var saving by remember(host) {mutableStateOf(false)}
    var technical by remember(host) {mutableStateOf(false)}
    fun load(value: SavedHostRoutes) {direct=value.direct;relay=value.relay.orEmpty();revision=value.revision;dirty=false}
    LaunchedEffect(saved.revision) {if(!dirty) load(saved)}
    val validated=runCatching {HostRoutes.validate(direct,relay.takeIf {it.isNotEmpty()})}
    val attempt=connection.routeAttempts[host]
    val active=connection.activeRoutes[host]
    val selected=connection.selectedRoutes[host]
    val matches=selected!=null && (saved.direct.any {HostRoutes.matches(selected,it,false)} || saved.relay?.let {HostRoutes.matches(selected,it,true)}==true)
    GroupedSurface(Modifier.testTag("host_routes_$host")) {
        GoatrSection("Connection hints","Optional addresses for this paired desktop. Saving applies to future connection attempts.")
        if(!paired) UiText("Finish secure pairing before saving connection hints.",style=MaterialTheme.typography.bodySmall)
        direct.forEachIndexed {index,address->
            Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(address,{value->direct=direct.toMutableList().also {it[index]=value};dirty=true},label={UiText("Direct address ${index+1}")},placeholder={UiText("192.0.2.1:12345 or [2001:db8::1]:12345")},singleLine=true,enabled=paired && !saving,modifier=Modifier.weight(1f).testTag("route_direct_$index"))
                IconButton(onClick={direct=direct.filterIndexed {position,_->position!=index};dirty=true},enabled=paired && !saving,modifier=Modifier.sizeIn(minWidth=48.dp,minHeight=48.dp).testTag("route_remove_$index")) {GoatrIcon(MaterialGlyph.Delete,"Remove direct address ${index+1}")}
            }
            if(selected!=null && saved.direct.getOrNull(index)==address && HostRoutes.matches(selected,address,false)) UiText("Current path matches this saved hint",style=MaterialTheme.typography.bodySmall)
        }
        TextButton(onClick={direct=direct+"";dirty=true},enabled=paired && !saving && direct.size<8,modifier=Modifier.heightIn(min=48.dp).testTag("route_add")) {IconLabel(MaterialGlyph.Add,"Add direct address")}
        OutlinedTextField(relay,{relay=it;dirty=true},label={UiText("Optional HTTPS relay")},placeholder={UiText("https://relay.example/")},singleLine=true,enabled=paired && !saving,modifier=Modifier.fillMaxWidth().testTag("route_relay"))
        if(selected!=null && saved.relay==relay && HostRoutes.matches(selected,relay,true)) UiText("Current path matches this saved hint",style=MaterialTheme.typography.bodySmall)
        if(dirty) UiText("Unsaved hint edits",style=MaterialTheme.typography.labelLarge)
        if(dirty && validated.isFailure) UiText((validated.exceptionOrNull() as? RouteValidation)?.message ?: "Check these connection hints.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error)
        if(dirty && revision!=saved.revision) {
            UiText("Saved hints changed while editing. Reload before saving.",style=MaterialTheme.typography.bodySmall)
            TextButton(onClick={load(saved)},enabled=!saving,modifier=Modifier.heightIn(min=48.dp).testTag("route_reload")) {IconLabel(MaterialGlyph.Refresh,"Reload saved hints")}
        }
        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            IconAction(MaterialGlyph.Save,if(saving) "Saving" else "Save hints","route_save",enabled=paired && dirty && validated.isSuccess && !saving && revision==saved.revision) {
                val value=validated.getOrNull() ?: return@IconAction
                saving=true
                model.saveHostRoutes(host,revision,value.first,value.second,onSaved={load(it)},onFinished={saving=false})
            }
            TextButton(onClick={load(saved)},enabled=dirty && !saving,modifier=Modifier.heightIn(min=48.dp).testTag("route_discard")) {IconLabel(MaterialGlyph.Close,"Discard edits")}
        }
        if(attempt!=null) {
            GoatrSection(if(attempt.supplied) "Hints supplied for this attempt" else "Hints prepared for this attempt","Iroh selects the route; supplied hints are not a record of attempted addresses.")
        } else UiText("Saved hints will be supplied on the next connection attempt.",style=MaterialTheme.typography.bodySmall)
        if(active!=null) {
            if(saved.revision!=active.revision) UiText("Saved changes differ from the current connection and apply on the next connection attempt.",style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("route_pending"))
        } else if(attempt!=null && saved.revision!=attempt.revision) UiText("Saved changes are pending for the next connection attempt.",style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("route_pending"))
        if(selected==null) UiText("Current path unavailable",style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("route_current"))
        else {
            UiText(if(matches) "Current path matches a saved hint" else "Other Iroh path",style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("route_current"))
        }
        DetailDisclosure(technical,{technical=!technical},"route_technical_details_$host")
        if(technical) {
            attempt?.let {UiText("Attempt hint revision ${it.revision}.",style=MaterialTheme.typography.bodySmall)
                SelectionContainer {UiText((it.direct+listOfNotNull(it.relay)).joinToString("\n").ifEmpty {"No saved hints. N0 discovery remains enabled."},style=MaterialTheme.typography.bodySmall,modifier=Modifier.testTag("route_attempt_hints"))}
            }
            active?.let {UiText("Current connection uses hint revision ${it.revision}.",style=MaterialTheme.typography.bodySmall)}
            selected?.let {SelectionContainer {UiText("${if(it.isRelay) "Relay" else if(it.isIp) "Direct" else "Iroh"}: ${it.remoteAddr}",style=MaterialTheme.typography.bodySmall)}}
        }
        UiText("Removing hints changes future supplied addresses. Iroh may still discover and use those routes.",style=MaterialTheme.typography.bodySmall)
        IconAction(MaterialGlyph.Refresh,"Reconnect","route_reconnect",enabled=paired && model.foreground && !model.busy && model.connectingHosts[host]!=true) {model.reconnect(host)}
    }
}
