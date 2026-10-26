// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.UiAutomation
import android.graphics.Rect
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import java.util.ArrayDeque
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

/** Actual production Compose controls -> exact finite Answer values. Not provider conformance. */
class ProviderFormsTest {
    @get:Rule val ui=createComposeRule()
    private val memory=FormMemory()
    private val requestKey=Uuid.random()
    private var received: Answer?=null
    private var presentation by mutableStateOf<Presentation>(PresentationDesktopOnly(c("Desktop"),"unsupported native request"))
    private var enabled by mutableStateOf(true)
    private var requestOpen by mutableStateOf(true)
    private var draft by mutableStateOf("Retained selected draft")
    private var sends=0
    private var stops=0
    private fun c(value: String)=ContentText(InlineText(value))
    private fun render(fontScale: Float=1f,narrow: Boolean=false,pinned:Boolean=false) {
        ui.setContent {MaterialTheme {val density=LocalDensity.current;CompositionLocalProvider(LocalDensity provides Density(density.density,fontScale)) {
            val actualForm: @Composable () -> Unit = {
                val context=LocalContext.current
                val provider=if(presentation is PresentationOmpSelect || presentation is PresentationOmpEditor) Provider.Omp else Provider.Codex
                val kind=when(presentation) {
                    is PresentationOmpSelect -> PresentationKind.OmpSelect
                    is PresentationOmpEditor -> PresentationKind.OmpEditor
                    is PresentationCodexApproval -> PresentationKind.CodexApproval
                    is PresentationCodexQuestions -> PresentationKind.CodexQuestions
                    is PresentationCodexForm -> PresentationKind.CodexForm
                    is PresentationCodexUrl -> PresentationKind.CodexUrl
                    is PresentationDesktopOnly -> PresentationKind.DesktopOnly
                }
                ProviderForm(PendingRequest(requestKey,provider,kind,presentation),memory,enabled,"Stale source: writes disabled",{content,loaded->ContentView(content,null,null,null,onLoaded=loaded)},{url->openHttpUrl(context,url)}) {received=it}
            }
            val bounds=if(narrow)Modifier.width(300.dp).height(260.dp) else Modifier.fillMaxSize()
            if(pinned)BoxWithConstraints(bounds.semantics {testTagsAsResourceId=true}) {
                ConversationFrame(chrome={},history={
                    if(requestOpen)ConversationRequestPanel(requestKey,{requestOpen=false},memory){actualForm()}
                    else androidx.compose.material3.Text("Original conversation remains")
                },editor={ConversationControlRail(enabled,"Stale source: writes disabled",{stops++},{}) {}},actions={ConversationDraftControls(draft,{draft=it},enabled,{sends++})},requestOpen=requestOpen)
            }else Column(bounds.verticalScroll(rememberScrollState()).semantics {testTagsAsResourceId=true}) {actualForm()}
        }}}
    }
    private fun show(p: Presentation) {ui.runOnIdle {memory.clear();received=null;presentation=p};ui.waitForIdle()}
    private fun tap(tag: String) {ui.onNodeWithTag(tag).performScrollTo().performClick();ui.waitForIdle()}

    @Test fun allSevenFinitePresentationsUseActualTypedControlsWithoutOptimisticClosure() {
        render()
        ui.onNodeWithText("Desktop only").assertExists();ui.onNodeWithTag("form_submit").assertDoesNotExist()
        show(PresentationOmpSelect(c("Sequential checkbox"),listOf(SelectOption("same",Label("A")),SelectOption("same",Label("B")),SelectOption("done",Label("Done"))),SelectionMarker.Checkbox,c("Native help"),Uint(2),Uint(0),listOf(Uint(0),Uint(0))))
        ui.onNodeWithText("Checked · A").assertExists();ui.onNodeWithText("Done").assertExists()
        tap("omp_option_1");assertEquals(AnswerOmpSelect(SubmitAction.Submit,"same"),received)
        ui.onNodeWithText("Sequential checkbox").assertExists()
        tap("form_cancel");assertEquals(AnswerOmpSelect(SubmitAction.Cancel),received)
        show(PresentationOmpEditor(c("Native editor"),c("  exact\n\tbacking  ")))
        ui.onNodeWithTag("editor_input").assertTextContains("  exact\n\tbacking  ")
        tap("form_submit");assertEquals(AnswerOmpEditor(SubmitAction.Submit,PromptText("  exact\n\tbacking  ")),received)
        show(PresentationCodexApproval(ApprovalType.Permissions,c("Grant request"),c("Read /safe; deny /private; network exact.example"),listOf(ApprovalChoice(Id("exact-session"),Label("Grant exactly"),ApprovalScope.Session,c("Exact session profile")),ApprovalChoice(Id("exact-policy"),Label("Policy change"),ApprovalScope.Policy,c("Persistent policy scope")))))
        ui.onNodeWithText("Persistent policy scope").assertExists();tap("approval_1")
        assertEquals(AnswerCodexApproval(Id("exact-policy")),received);ui.onNodeWithText("Grant request").assertExists()
        ui.runOnIdle {enabled=false;received=null};ui.onNodeWithTag("approval_0").assertIsNotEnabled();ui.onNodeWithText("Stale source: writes disabled").assertExists()
        ui.runOnIdle {enabled=true}
        show(PresentationCodexQuestions(true,listOf(Question(Id("native-q"),"Native question",c("Choose ordered values or Other"),true,false,listOf(QuestionOption(Label("Offered"),c("Exact description")))))))
        tap("question_0_option_0");tap("question_0_option_0")
        ui.onNodeWithTag("question_0_input").performScrollTo().performTextInput("Other exact")
        tap("question_0_add");tap("form_submit")
        assertEquals(AnswerCodexQuestions(listOf(QuestionAnswer(Id("native-q"),listOf("Offered","Offered","Other exact")))),received)
        ui.onNodeWithTag("form_cancel").assertDoesNotExist()
        show(PresentationCodexForm(c("MCP exact form"),listOf(FormField(Id("number"),"Exact number",required=true,type=FieldType.Number,minimum=Decimal.parse("-999999999999999999999.5"),maximum=Decimal.parse("999999999999999999999.5")),FormField(Id("scalar"),"Scalar length",required=true,type=FieldType.String,minLength=Uint(2)),FormField(Id("multi"),"Repeated enum",required=true,type=FieldType.Multi,`default`=FieldValue.Multi(listOf("x","x")),minItems=Uint(2),maxItems=Uint(4),options=listOf(FormOption("x",Label("X")),FormOption("x",Label("Duplicate X")))))))
        ui.onNodeWithTag("field_0").performScrollTo().performTextInput("999999999999999999999.500")
        ui.onNodeWithTag("field_1").performScrollTo().performTextInput("\uD801\uDC00")
        tap("form_submit");ui.onNodeWithTag("form_validation").assertExists();assertNull(received)
        ui.onNodeWithTag("field_1").performScrollTo().performTextReplacement("\uD801\uDC00x")
        tap("field_2_option_1");tap("form_submit")
        val form=received as AnswerCodexForm
        assertEquals(FieldValue.Text("999999999999999999999.5"),form.values!![0].value)
        assertEquals(FieldValue.Text("\uD801\uDC00x"),form.values!![1].value)
        assertEquals(FieldValue.Multi(listOf("x","x","x")),form.values!![2].value)
        val urlCard=PresentationCodexUrl(c("Native URL request"),"https://trusted.example@evil.example/")
        urlCard.validate() // Valid native card; external navigation deliberately has a stricter policy.
        show(urlCard)
        ui.onNodeWithTag("url_open").assertIsNotEnabled()
        tap("url_Decline");assertEquals(AnswerCodexUrl(UrlAction.Decline),received)
        ui.onNodeWithTag("url_Decline").assertExists()
    }

    @Test fun secretQuestionsClearActualFormMemoryAndExposeMaskedSemantics() {
        render();show(PresentationCodexQuestions(true,listOf(Question(Id("secret-id"),"Secret input",c("Synthetic secret only"),false,true,emptyList()))))
        ui.onNodeWithTag("question_0_input").performScrollTo().performTextInput("synthetic-secret-canary")
        ui.onNodeWithTag("question_0_input").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Password))
        tap("question_0_add");tap("form_submit")
        assertEquals(listOf("synthetic-secret-canary"),(received as AnswerCodexQuestions).answers.single().values)
        ui.runOnIdle {received=null;assertTrue(memory.values("question_secret-id",true).isEmpty());assertEquals("",memory.secret("question_secret-id-input"))}
        ui.onNodeWithTag("question_0_input").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
    }

    @Test fun nativeSecretRequestKeepsActualDraftSendAndSeparateStopReachableAndClearsOnPanelTeardown() {
        render(2f,true,true)
        show(PresentationCodexQuestions(true,listOf(Question(Id("secret-id"),"Secret native answer",c("Synthetic secret request"),false,true,emptyList()))))
        ui.onNodeWithTag("prompt_send").assertIsDisplayed().assertHeightIsAtLeast(48.dp)
        ui.onNodeWithTag("turn_stop").assertIsDisplayed().assertHeightIsAtLeast(48.dp)
        ui.onNodeWithTag("composer").assertIsDisplayed().performTextReplacement("Draft remains separate from answer")
        ui.onNodeWithTag("question_0_input").performScrollTo().performTextInput("synthetic-panel-secret")
        ui.runOnIdle {assertEquals("synthetic-panel-secret",memory.secret("question_secret-id-input"));assertNull(received)}
        ui.onNodeWithTag("composer").assertIsDisplayed().assertTextContains("Draft remains separate from answer")
        ui.onNodeWithTag("prompt_send").performClick()
        ui.onNodeWithTag("turn_stop").performClick()
        ui.runOnIdle {assertEquals(1,sends);assertEquals(1,stops);assertNull(received);assertTrue(requestOpen)}
        ui.runOnIdle {enabled=false}
        ui.onNodeWithTag("form_submit").performScrollTo().assertIsNotEnabled()
        ui.onNodeWithText("Stale source: writes disabled").assertExists()
        ui.onNodeWithTag("prompt_send").assertIsNotEnabled()
        ui.onNodeWithTag("turn_stop").assertIsNotEnabled()
        val lifetime=memory.secretLifetime
        ui.onNodeWithTag("request_close").assertIsDisplayed().performClick()
        ui.runOnIdle {
            assertFalse(requestOpen);assertNull(received)
            assertEquals("",memory.secret("question_secret-id-input"))
            assertTrue(memory.secretLifetime>lifetime)
            assertEquals("Draft remains separate from answer",draft)
        }
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }

    @Test fun opaqueSecretQuestionIdsCannotOverwriteAnotherQuestionsAnswers() {
        render()
        show(PresentationCodexQuestions(true,listOf(
            Question(Id("a"),"First secret",c("First synthetic secret"),false,true,emptyList()),
            Question(Id("a-input"),"Second secret",c("Second synthetic secret"),false,true,emptyList()),
        )))
        ui.onNodeWithTag("question_1_input").performScrollTo().performTextInput("second-canary")
        tap("question_1_add")
        ui.onNodeWithTag("question_0_input").performScrollTo().performTextInput("first-canary")
        tap("question_0_add");tap("form_submit")
        val actual=(received as AnswerCodexQuestions).answers
        assertEquals(listOf(QuestionAnswer(Id("a"),listOf("first-canary")),QuestionAnswer(Id("a-input"),listOf("second-canary"))),actual)
    }

    @Test fun multiFieldDefaultsFormatsBooleanAndLargeFontActionsRemainReachable() {
        render(2f,true);show(PresentationCodexForm(c("Typed defaults"),listOf(FormField(Id("flag"),"Boolean",required=true,type=FieldType.Boolean,`default`=FieldValue.Flag(false)),FormField(Id("single"),"Single enum",required=true,type=FieldType.Single,`default`=FieldValue.Text("v"),options=listOf(FormOption("v",Label("V")))),FormField(Id("uri"),"Native URI",required=true,type=FieldType.String,format=FieldFormat.Uri,`default`=FieldValue.Text("mailto:user@example.test")))))
        ui.onNodeWithTag("field_2").performScrollTo().performClick().assertIsFocused()
        tap("form_submit");val form=received as AnswerCodexForm
        assertEquals(FieldValue.Flag(false),form.values!![0].value);assertEquals(FieldValue.Text("v"),form.values!![1].value)
        assertEquals(FieldValue.Text("mailto:user@example.test"),form.values!![2].value)
        ui.onNodeWithTag("form_submit").assertHeightIsAtLeast(48.dp)
        ui.onNodeWithText("Boolean · required").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
    }
    @Test fun actualComposeSavedStateExcludesSecretAndRestorationDoesNotAnswer() {
        val restoration=androidx.compose.ui.test.junit4.StateRestorationTester(ui)
        var registry: androidx.compose.runtime.saveable.SaveableStateRegistry?=null
        val request=PendingRequest(Uuid.random(),Provider.Codex,PresentationKind.CodexQuestions,PresentationCodexQuestions(true,listOf(Question(Id("private-state"),"Secret",c("Synthetic privacy check"),false,true,emptyList()))))
        restoration.setContent {
            registry=androidx.compose.runtime.saveable.LocalSaveableStateRegistry.current
            val state=remember {FormMemory()}
            DisposableEffect(state) {onDispose {state.clear()}}
            MaterialTheme {Column(Modifier.verticalScroll(rememberScrollState())) {
                ProviderForm(request,state,true,null,{content,loaded->ContentView(content,null,null,null,onLoaded=loaded)},{fail("Restoration must not navigate")}) {fail("Restoration must not answer")}
            }}
        }
        val canary="synthetic-saved-secret-canary"
        ui.onNodeWithTag("question_0_input").performTextInput(canary)
        var saved=ByteArray(0)
        ui.runOnIdle {val parcel=android.os.Parcel.obtain();try {parcel.writeValue(registry!!.performSave());saved=parcel.marshall()} finally {parcel.recycle()}}
        fun contains(bytes: ByteArray): Boolean = (0..saved.size-bytes.size).any {offset->bytes.indices.all {saved[offset+it]==bytes[it]}}
        assertFalse(contains(canary.toByteArray(Charsets.UTF_8)))
        assertFalse(contains(canary.toByteArray(Charsets.UTF_16LE)))
        restoration.emulateSavedInstanceStateRestore()
        ui.onNodeWithTag("question_0_input").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("")))
    }

    /** Read only actual visible AOSP IME virtual keys, never an app editor/input action. */
    private fun visibleImeKey(automation:UiAutomation,label:String):AccessibilityNodeInfo? {
        val windows=automation.windows
        try {
            for(currentWindow in windows) {
                if(currentWindow.type!=AccessibilityWindowInfo.TYPE_INPUT_METHOD) continue
                val root=currentWindow.root
                if(root==null) continue
                if(root.packageName?.toString()!="com.android.inputmethod.latin") {root.recycle();continue}
                val viewport=Rect();currentWindow.getBoundsInScreen(viewport)
                val bounds=Rect()
                val queued=ArrayDeque<AccessibilityNodeInfo>();queued.add(root)
                try {
                    var visited=0
                    while(queued.isNotEmpty() && visited++<512) {
                        val node=queued.removeFirst()
                        var retained=false
                        try {
                            node.getBoundsInScreen(bounds)
                            if(node.isVisibleToUser && node.isEnabled && !bounds.isEmpty &&
                                viewport.contains(bounds.centerX(),bounds.centerY()) &&
                                (node.text?.toString()==label || node.contentDescription?.toString()==label) &&
                                node.actionList.any {it.id==AccessibilityNodeInfo.ACTION_CLICK}) {
                                retained=true;return node
                            }
                            for(index in 0 until node.childCount) node.getChild(index)?.let(queued::add)
                        } finally {if(!retained) node.recycle()}
                    }
                } finally {queued.forEach(AccessibilityNodeInfo::recycle)}
            }
        } finally {windows.forEach(AccessibilityWindowInfo::recycle)}
        return null
    }
    private fun pressVisibleImeKey(automation:UiAutomation,label:String) {
        var key:AccessibilityNodeInfo?=null
        ui.waitUntil(10000) {key=visibleImeKey(automation,label);key!=null}
        try {assertTrue("The actual visible AOSP IME key must perform its native click",key!!.performAction(AccessibilityNodeInfo.ACTION_CLICK))}
        finally {key?.recycle()}
    }
    @Test fun actualAospImeVisibleKeysEnterBoundedNegativeDecimalAndInteger() {
        render()
        show(PresentationCodexForm(c("Nonsecret signed numeric keyboard acceptance"),listOf(
            FormField(Id("decimal"),"Negative decimal",required=true,type=FieldType.Number,minimum=Decimal.parse("-2"),maximum=Decimal.parse("-1")),
            FormField(Id("integer"),"Negative integer",required=true,type=FieldType.Integer,minimum=Decimal.parse("-2"),maximum=Decimal.parse("-1")),
        )))
        val automation=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation
        val service=automation.serviceInfo
        val originalFlags=service.flags
        service.flags=originalFlags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        automation.serviceInfo=service
        try {
            ui.onNodeWithTag("field_0").performScrollTo().performClick().assertIsFocused()
            for(key in listOf("-","1",".","5")) pressVisibleImeKey(automation,key)
            ui.onNodeWithTag("field_0").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("-1.5")))
            assertTrue(automation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK))
            ui.onNodeWithTag("field_1").performScrollTo().performClick().assertIsFocused()
            for(key in listOf("-","1")) pressVisibleImeKey(automation,key)
            ui.onNodeWithTag("field_1").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,androidx.compose.ui.text.AnnotatedString("-1")))
            assertTrue(automation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK))
            tap("form_submit")
            val answer=received as AnswerCodexForm
            assertEquals(FormAction.Accept,answer.action)
            assertEquals(listOf(FormAnswerValue(Id("decimal"),FieldValue.Text("-1.5")),FormAnswerValue(Id("integer"),FieldValue.Text("-1"))),answer.values)
        } finally {
            val restore=automation.serviceInfo
            restore.flags=originalFlags
            automation.serviceInfo=restore
        }
    }

    @Test fun nativeUrlBrowserVisitDoesNotAnswerOrCloseTheRequest() {
        render()
        val card=PresentationCodexUrl(c("Synthetic native URL"),"https://example.test/native-url-regression")
        card.validate();show(card)
        val instrumentation=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
        lateinit var hostActivity:android.app.Activity
        instrumentation.runOnMainSync {
            hostActivity=androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).single()
        }
        val automation=instrumentation.uiAutomation
        val service=automation.serviceInfo
        val originalFlags=service.flags
        service.flags=originalFlags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        automation.serviceInfo=service
        try {
            ui.onNodeWithTag("url_open").performScrollTo().performClick()
            // Android package visibility does not constrain startActivity; observe the actual launch.
            // Missing real HTTP handling is a coordinator prerequisite, never inferred from a query.
            ui.waitUntil(10000) {
                val root=automation.rootInActiveWindow
                val packageName=root?.packageName?.toString()
                !hostActivity.hasWindowFocus() && packageName!=null &&
                    packageName!=instrumentation.targetContext.packageName &&
                    packageName!="com.android.systemui" &&
                    root?.window?.type==android.view.accessibility.AccessibilityWindowInfo.TYPE_APPLICATION
            }
            assertNull("A real browser visit must not submit a native response",received)
            assertTrue(automation.performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK))
            ui.waitUntil(10000) {hostActivity.hasWindowFocus()}
            ui.onNodeWithTag("url_Decline").assertExists()
            assertNull(received)
            tap("url_Decline");assertEquals(AnswerCodexUrl(UrlAction.Decline),received)
        } finally {
            val restore=automation.serviceInfo
            restore.flags=originalFlags
            automation.serviceInfo=restore
        }
    }
}
