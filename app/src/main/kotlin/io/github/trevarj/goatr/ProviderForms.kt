// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.material3.Text as UiText
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp

/** Rotation memory only. Secret answer arrays are wiped on every lifetime boundary. */
class FormMemory(val source: Uuid? = null) {
    val text = mutableStateMapOf<String,String>()
    val answers = mutableStateMapOf<String,List<String>>()
    val fields = mutableStateMapOf<Id,FieldValue>()
    val revealed = mutableStateMapOf<Content,Boolean>()
    private val secrets = mutableMapOf<String,MutableList<CharArray>>()
    private val secretDrafts = mutableMapOf<String,CharArray>()
    var revision by mutableIntStateOf(0)
        private set
    var secretLifetime by mutableIntStateOf(0)
        private set
    var validation by mutableStateOf<String?>(null)
    fun secret(id: String): String { revision; return secretDrafts[id]?.concatToString().orEmpty() }
    fun secret(id: String,value: String) { secretDrafts.remove(id)?.fill('\u0000');secretDrafts[id]=value.toCharArray();revision++ }
    fun values(id: String,secret: Boolean): List<String> { revision; return if(secret) secrets[id]?.map {it.concatToString()}.orEmpty() else answers[id].orEmpty() }
    fun add(id: String,value: String,secret: Boolean) { if(secret) { secrets.getOrPut(id) {mutableListOf()}.add(value.toCharArray()); revision++ } else answers[id] = answers[id].orEmpty()+value }
    fun remove(id: String,index: Int,secret: Boolean) { if(secret) {secrets[id]?.removeAt(index)?.fill('\u0000');revision++} else answers[id] = answers[id].orEmpty().filterIndexed {i,_->i!=index} }
    fun clearSecrets() {
        secretDrafts.values.forEach {it.fill('\u0000')};secretDrafts.clear()
        secrets.values.forEach {values->values.forEach {it.fill('\u0000')}};secrets.clear();revision++;secretLifetime++
    }
    fun clear() { clearSecrets(); text.clear(); answers.clear(); fields.clear(); revealed.clear(); validation = null }
}

@Composable
internal fun Action(label: String,tag: String,modifier: Modifier = Modifier,enabled: Boolean = true,onClick: () -> Unit) {
    val nativeFormAction=tag.startsWith("omp_option_") || tag.startsWith("form_") || tag.startsWith("approval_") || tag.startsWith("question_") || tag.startsWith("field_") || tag.startsWith("url_")
    val glyph=when {
        tag=="form_cancel" || tag=="form_decline" || tag.contains("_remove_") || tag.endsWith("_omit") -> MaterialGlyph.Close
        tag=="form_submit" -> MaterialGlyph.Send
        tag.startsWith("question_") && (tag.endsWith("_add") || tag.contains("_option_")) -> MaterialGlyph.Add
        tag=="pair_begin" -> MaterialGlyph.Scan
        tag.startsWith("new_session") || tag=="session_create" || tag=="create_new_action" || tag=="prompt_new_action" -> MaterialGlyph.Add
        tag.startsWith("host_reconnect") || tag=="inventory_refresh" || tag=="conversation_refresh" || tag=="receipt_inspect" || tag=="content_refresh" -> MaterialGlyph.Refresh
        tag=="recovery_replace" -> MaterialGlyph.Send
        tag.startsWith("history_") || tag.startsWith("actions_") || tag=="content_expand" -> MaterialGlyph.ExpandMore
        tag=="switcher_close" -> MaterialGlyph.Close
        tag=="switcher" || tag=="pane_open" -> MaterialGlyph.Chat
        else -> null
    }
    Button(onClick=onClick,enabled=enabled,modifier=(if(nativeFormAction) modifier.fillMaxWidth() else modifier).heightIn(min=48.dp).testTag(tag)) {
        if(nativeFormAction) {
            // Native labels and scopes must remain readable at narrow widths and large fonts.
            Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                glyph?.let {GoatrIcon(it,null)}
                UiText(label,modifier=Modifier.weight(1f))
            }
        } else if(glyph==null) UiText(label) else IconLabel(glyph,label)
    }
}
@Composable
internal fun Heading(label: String) { UiText(label,style=MaterialTheme.typography.titleMedium,modifier=Modifier.semantics {heading()}) }

/** Keep each native question, field or grant visibly associated with its own choices. */
@Composable
private fun NativeFormSection(content: @Composable ColumnScope.()->Unit) {
    Surface(Modifier.fillMaxWidth(),shape=MaterialTheme.shapes.medium,color=MaterialTheme.colorScheme.surface,border=BorderStroke(1.dp,MaterialTheme.colorScheme.outlineVariant)) {
        Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(12.dp),content=content)
    }
}

@Composable
fun ProviderForm(
    request: PendingRequest,memory: FormMemory,enabled: Boolean,disabledReason: String?,
    content: @Composable (Content,((String)->Unit)?) -> Unit,
    openUrl: (String)->Unit,onAnswer: (Answer)->Unit,
) {
    val p = request.presentation
    fun complete(c: Content): Boolean = c is ContentText || memory.revealed[c] == true
    fun submit(answer: () -> Answer) {
        try { val value=answer(); validateAnswer(p,value); memory.validation=null; onAnswer(value); memory.clearSecrets() }
        catch (_: Exception) { memory.validation="Check required values, offered choices, exact numeric bounds and field formats." }
    }
    GroupedSurface(Modifier.semantics {isTraversalGroup=true}.testTag("form_${request.kind}")) {
    Column(Modifier.fillMaxWidth().testTag("form_request_${request.requestKey.value}"),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        GoatrSection(when(request.kind) {PresentationKind.OmpSelect->"Native choice";PresentationKind.OmpEditor->"Native editor";PresentationKind.CodexApproval->"Native approval";PresentationKind.CodexQuestions->"Native questions";PresentationKind.CodexForm->"Native form";PresentationKind.CodexUrl->"Native link request";PresentationKind.DesktopOnly->"Desktop-only request"}, "Use only the choices and scope supplied by the desktop provider.")
        if (!enabled) UiText(disabledReason ?: "Writes disabled until authoritative snapshot and receipt recovery.",style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
        memory.validation?.let { UiText(it,color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodyMedium,modifier=Modifier.fillMaxWidth().semantics {liveRegion=LiveRegionMode.Assertive}.testTag("form_validation")) }
        when (p) {
            is PresentationOmpSelect -> {
                content(p.title,null); p.helpText?.let {content(it,null)}
                UiText(if(p.selectionMarker==SelectionMarker.Checkbox) "Each selection sends one sequential native toggle; no multi-answer." else "Select one offered native value.")
                p.options.forEachIndexed {index,option->
                    val marked=index.toLong() < (p.markableCount?.value ?: p.options.size.toLong())
                    val marker=if(!marked) "" else if(p.selectionMarker==SelectionMarker.Checkbox) {if(p.checked.any {it.value==index.toLong()}) "Checked · " else "Unchecked · "} else if(p.selected?.value==index.toLong()) "Selected · " else ""
                    NativeFormSection {
                    option.description?.let {content(it,null)}
                    Action(marker+option.label.value,"omp_option_$index",modifier=Modifier.semantics {
                        if(marked) {
                            role=if(p.selectionMarker==SelectionMarker.Checkbox) androidx.compose.ui.semantics.Role.Checkbox else androidx.compose.ui.semantics.Role.RadioButton
                            stateDescription=if(marker.isEmpty()) "Not selected" else marker.trim().removeSuffix("·").trim()
                        }
                    },enabled=enabled) {submit {AnswerOmpSelect(SubmitAction.Submit,option.value)}}
                    }
                }
                Action("Cancel native selection","form_cancel",enabled=enabled) {submit {AnswerOmpSelect(SubmitAction.Cancel)}}
            }
            is PresentationOmpEditor -> {
                content(p.title,null)
                content(p.prefill) {memory.revealed[p.prefill]=true;if(!memory.text.containsKey("editor")) memory.text["editor"]=it}
                OutlinedTextField(value=memory.text["editor"].orEmpty(),onValueChange={memory.text["editor"]=it},label={UiText("Native editor answer")},minLines=3,maxLines=8,enabled=enabled,modifier=Modifier.fillMaxWidth().testTag("editor_input"))
                Action("Submit editor","form_submit",enabled=enabled && complete(p.prefill)) {submit {AnswerOmpEditor(SubmitAction.Submit,PromptText(memory.text["editor"].orEmpty()))}}
                Action("Cancel native editor","form_cancel",enabled=enabled) {submit {AnswerOmpEditor(SubmitAction.Cancel)}}
            }
            is PresentationCodexApproval -> {
                content(p.title,null); Heading("${p.approvalType} approval · requested scope")
                content(p.details) {memory.revealed[p.details]=true}
                p.choices.forEachIndexed {index,choice->
                    NativeFormSection {
                    Heading("${choice.label.value} · scope ${choice.scope}")
                    choice.details?.let {details->content(details) {memory.revealed[details]=true}}
                    Action("${choice.label.value} (${choice.scope})","approval_$index",enabled=enabled && complete(p.details) && p.choices.all {it.details?.let(::complete)!=false}) {submit {AnswerCodexApproval(choice.choiceId)}}
                    }
                }
                if(!complete(p.details) || p.choices.any {it.details?.let(::complete)==false}) UiText("Expand all permission details before answering. Unavailable scope requires desktop inspection.")
            }
            is PresentationCodexQuestions -> {
                Heading(if(p.isBlocking) "Blocking native questions" else "Native questions")
                p.questions.forEachIndexed {index,q->
                    val id="question_${q.id.value}"
                    NativeFormSection {
                    Heading(q.header); content(q.prompt,null)
                    UiText("Question ID: ${q.id.value}${if(q.isSecret) " · secret, memory only" else ""}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                    q.options.forEachIndexed {optionIndex,option->
                        content(option.description,null)
                        Action("Add ${option.label.value}","question_${index}_option_$optionIndex",enabled=enabled) {memory.add(id,option.label.value,q.isSecret)}
                    }
                    if(q.options.isEmpty() || q.isOther) {
                        val input="$id-input"
                        androidx.compose.runtime.key(if(q.isSecret) memory.secretLifetime else 0) {
                            OutlinedTextField(value=if(q.isSecret) memory.secret(input) else memory.text[input].orEmpty(),onValueChange={if(q.isSecret) memory.secret(input,it) else memory.text[input]=it},label={UiText(if(q.isOther && q.options.isNotEmpty()) "Other answer" else "Answer")},visualTransformation=if(q.isSecret) PasswordVisualTransformation() else VisualTransformation.None,keyboardOptions=KeyboardOptions(keyboardType=if(q.isSecret) KeyboardType.Password else KeyboardType.Text),enabled=enabled,modifier=Modifier.fillMaxWidth().testTag("question_${index}_input"))
                        }
                        Action("Add answer","question_${index}_add",enabled=enabled) {val value=if(q.isSecret) memory.secret(input) else memory.text[input].orEmpty();memory.add(id,value,q.isSecret);if(q.isSecret) memory.secret(input,"") else memory.text[input]=""}
                    }
                    memory.values(id,q.isSecret).forEachIndexed {answerIndex,value->
                        Action("Remove answer ${answerIndex+1}: ${if(q.isSecret) "masked" else value}","question_${index}_remove_$answerIndex",enabled=enabled) {memory.remove(id,answerIndex,q.isSecret)}
                    }
                    }
                }
                Action("Submit answers","form_submit",enabled=enabled) {submit {AnswerCodexQuestions(p.questions.map {q->QuestionAnswer(q.id,memory.values("question_${q.id.value}",q.isSecret))})}}
                UiText("No native Cancel is offered for these questions. Back clears secret input without answering.")
            }
            is PresentationCodexForm -> {
                content(p.title,null)
                p.fields.forEachIndexed {index,f->
                    NativeFormSection {
                    LaunchedEffect(f.id) {
                        val initialized="initialized:${f.id.value}"
                        if(memory.text[initialized]!= "yes") {f.`default`?.let {memory.fields[f.id]=it};memory.text[initialized]="yes"}
                    }
                    Heading("${f.title}${if(f.required) " · required" else " · optional"}")
                    f.description?.let {content(it,null)}
                    UiText(listOfNotNull("Type ${f.type}",f.format?.let {"Format $it"},f.minLength?.let {"Min scalars ${it.value}"},f.maxLength?.let {"Max scalars ${it.value}"},f.minimum?.let {"Minimum ${it.value}"},f.maximum?.let {"Maximum ${it.value}"},f.minItems?.let {"Min items ${it.value}"},f.maxItems?.let {"Max items ${it.value}"}).joinToString(" · "),style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                    when(f.type) {
                        FieldType.Boolean -> {
                            Action("${f.title}: ${(memory.fields[f.id] as? FieldValue.Flag)?.value ?: "not supplied"}","field_$index",enabled=enabled) {memory.fields[f.id]=FieldValue.Flag((memory.fields[f.id] as? FieldValue.Flag)?.value!=true)}
                        }
                        FieldType.Single -> f.options.orEmpty().forEachIndexed {n,o->Action("${o.label.value} (${o.value})${if((memory.fields[f.id] as? FieldValue.Text)?.value==o.value) " · selected" else ""}","field_${index}_option_$n",enabled=enabled) {memory.fields[f.id]=FieldValue.Text(o.value)}}
                        FieldType.Multi -> {
                            f.options.orEmpty().forEachIndexed {n,o->Action("Add ${o.label.value} (${o.value})","field_${index}_option_$n",enabled=enabled) {memory.fields[f.id]=FieldValue.Multi((memory.fields[f.id] as? FieldValue.Multi)?.values.orEmpty()+o.value)}}
                            (memory.fields[f.id] as? FieldValue.Multi)?.values.orEmpty().forEachIndexed {n,value->Action("Remove item ${n+1}: $value","field_${index}_remove_$n",enabled=enabled) {memory.fields[f.id]=FieldValue.Multi((memory.fields[f.id] as FieldValue.Multi).values.filterIndexed {i,_->i!=n})}}
                            if(f.required && f.id !in memory.fields) Action("Supply empty list","field_${index}_empty",enabled=enabled) {memory.fields[f.id]=FieldValue.Multi(emptyList())}
                        }
                        FieldType.String,FieldType.Number,FieldType.Integer -> OutlinedTextField(value=(memory.fields[f.id] as? FieldValue.Text)?.value.orEmpty(),onValueChange={memory.fields[f.id]=FieldValue.Text(it)},label={UiText(f.title)},keyboardOptions=KeyboardOptions(keyboardType=when(f.type) {FieldType.Number->KeyboardType.DecimalSigned;FieldType.Integer->KeyboardType.NumberSigned;else->KeyboardType.Text}),enabled=enabled,modifier=Modifier.fillMaxWidth().testTag("field_$index"))
                    }
                    if(!f.required) Action("Omit optional ${f.title}","field_${index}_omit",enabled=enabled) {memory.fields.remove(f.id)}
                    }
                }
                Action("Accept form","form_submit",enabled=enabled) {submit {AnswerCodexForm(FormAction.Accept,p.fields.mapNotNull {f->memory.fields[f.id]?.let {value->val normalized=if((f.type==FieldType.Number || f.type==FieldType.Integer) && value is FieldValue.Text) FieldValue.Text(Decimal.parse(value.value).value) else value;FormAnswerValue(f.id,normalized)}})}}
                Action("Decline form","form_decline",enabled=enabled) {submit {AnswerCodexForm(FormAction.Decline)}}
                Action("Cancel form","form_cancel",enabled=enabled) {submit {AnswerCodexForm(FormAction.Cancel)}}
            }
            is PresentationCodexUrl -> {
                content(p.title,null); UiText(p.url,style=MaterialTheme.typography.bodyLarge,modifier=Modifier.fillMaxWidth())
                Action("Open HTTP(S) link separately","url_open",enabled=safeHttpUrl(p.url)!=null) {openUrl(p.url)}
                UiText("Opening a browser never answers or resolves this request.")
                UrlAction.entries.forEach {a->Action(a.name,"url_${a.name}",enabled=enabled) {submit {AnswerCodexUrl(a)}}}
            }
            is PresentationDesktopOnly -> {content(p.title,null);Heading("Desktop only");UiText(p.reason)}
        }
    }
    }
}
