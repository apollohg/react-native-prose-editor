package com.apollohg.editor

import android.app.Instrumentation
import android.content.Context
import android.os.SystemClock
import android.text.Selection
import android.text.InputType
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
@LargeTest
class NativeDeviceImeRegressionTest {
    private val instrumentation: Instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = ApplicationProvider.getApplicationContext<Context>()

    @Test
    fun commitCompletionReplacesSelectedTextThroughRust() {
        val result = runOnMainSyncWithResult {
            val editText = createEditor("hel", selectionStart = 0, selectionEnd = 3)
            var replacement: Replacement? = null
            editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
                replacement = Replacement(scalarFrom, scalarTo, text)
            }

            val inputConnection = createInputConnection(editText)
            assertTrue(inputConnection.commitCompletion(CompletionInfo(1L, 0, "hello")))

            ImeResult(
                replacement = replacement,
                trace = editText.imeTraceSnapshotForTesting()
            )
        }

        assertEquals(Replacement(0, 3, "hello"), result.replacement)
        assertTraceContains(result.trace, "commitCompletion")
    }

    @Test
    fun commitCorrectionCoversOldTextAndInferredTokenBoundaries() {
        val explicit = runCorrectionScenario("teh", offset = 0, oldText = "teh", newText = "the")
        assertEquals(Replacement(0, 3, "the"), explicit.replacement)
        assertNull(explicit.inserted)
        assertTraceContains(explicit.trace, "correctionExplicitApply")

        val trailingPeriod = runCorrectionScenario("teh.", offset = 0, oldText = null, newText = "the")
        assertEquals(Replacement(0, 3, "the"), trailingPeriod.replacement)
        assertNull(trailingPeriod.inserted)
        assertTraceContains(trailingPeriod.trace, "correctionInferredApply")

        val punctuationOffset = runCorrectionScenario("teh.", offset = 3, oldText = null, newText = "the")
        assertNull(punctuationOffset.replacement)
        assertNull(punctuationOffset.inserted)
        assertTraceContains(punctuationOffset.trace, "correctionInferredNoop")

        val whitespaceOffset = runCorrectionScenario("teh ", offset = 3, oldText = null, newText = "the")
        assertNull(whitespaceOffset.replacement)
        assertNull(whitespaceOffset.inserted)
        assertTraceContains(whitespaceOffset.trace, "correctionInferredNoop")

        val hyphenated = runCorrectionScenario(
            "dont-stop ",
            offset = 4,
            oldText = null,
            newText = "don't-stop"
        )
        assertEquals(Replacement(0, 9, "don't-stop"), hyphenated.replacement)

        val apostrophe = runCorrectionScenario("cant's ", offset = 4, oldText = null, newText = "can't")
        assertEquals(Replacement(0, 6, "can't"), apostrophe.replacement)

        val surrogate = runCorrectionScenario("te😀h ", offset = 3, oldText = null, newText = "term")
        assertEquals(Replacement(0, 4, "term"), surrogate.replacement)
    }

    @Test
    fun visibleCompositionCorrectionCommitDeleteAndPreflightAreRouted() {
        // Mid-composition corrections are applied on a deferred main-looper turn
        // (EditorInputConnection.rememberPendingCompositionCorrectionCommit), so the
        // insertion is observed after the looper idles rather than synchronously.
        val correctionInserted = AtomicReference<Inserted?>()
        val correction = runOnMainSyncWithResult {
            val editText = createEditor("", selectionStart = 0, selectionEnd = 0)
            editText.onInsertTextInRustForTesting = { text, scalar ->
                correctionInserted.set(Inserted(text, scalar))
                editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
                editText.setSelection(text.length)
            }

            val inputConnection = createInputConnection(editText)
            assertTrue(inputConnection.setComposingText("teh", 1))
            assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "teh", "the")))

            ImeResult(
                trace = editText.imeTraceSnapshotForTesting()
            )
        }
        instrumentation.waitForIdleSync()

        assertEquals(Inserted("the", 0), correctionInserted.get())
        assertTraceContains(correction.trace, "setComposingText")
        assertTraceContains(correction.trace, "commitCorrectionComposition")

        val deletion = runOnMainSyncWithResult {
            val editText = createEditor("", selectionStart = 0, selectionEnd = 0)
            var inserted: Inserted? = null
            editText.onInsertTextInRustForTesting = { text, scalar ->
                inserted = Inserted(text, scalar)
            }

            val inputConnection = createInputConnection(editText)
            assertTrue(inputConnection.setComposingText("abcd", 1))
            assertTrue(inputConnection.deleteSurroundingText(1, 0))
            assertTrue(inputConnection.finishComposingText())

            ImeResult(
                inserted = inserted,
                trace = editText.imeTraceSnapshotForTesting()
            )
        }

        assertEquals(Inserted("abc", 0), deletion.inserted)
        assertTraceContains(deletion.trace, "finishComposingText")

        val preflight = runOnMainSyncWithResult {
            val editText = createEditor("", selectionStart = 0, selectionEnd = 0)
            var inserted: Inserted? = null
            editText.onInsertTextInRustForTesting = { text, scalar ->
                inserted = Inserted(text, scalar)
                editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
            }

            val inputConnection = createInputConnection(editText)
            assertTrue(inputConnection.setComposingText("abc", 1))
            val ready = editText.prepareForExternalEditorUpdate()

            ImeResult(
                inserted = inserted,
                trace = editText.imeTraceSnapshotForTesting(),
                ready = ready
            )
        }

        assertTrue(preflight.ready)
        assertEquals(Inserted("abc", 0), preflight.inserted)
        assertTraceContains(preflight.trace, "finishComposingText")
    }

    @Test
    fun nativeEditableAutocorrectBeforeAndAfterBlurIsAdoptedAndSelectionIsPreserved() {
        val focused = runOnMainSyncWithResult {
            val editText = createEditor("teh ", selectionStart = 4, selectionEnd = 4)
            assertTrue(editText.requestFocus())
            var replacement: Replacement? = null
            editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
                replacement = Replacement(scalarFrom, scalarTo, text)
            }

            editText.text!!.replace(0, 3, "the")

            ImeResult(
                replacement = replacement,
                selection = editText.selectionStart to editText.selectionEnd,
                trace = editText.imeTraceSnapshotForTesting()
            )
        }

        assertEquals(Replacement(1, 3, "he"), focused.replacement)
        assertEquals(4 to 4, focused.selection)
        assertTraceContains(focused.trace, "nativeMutationApply")

        val afterBlur = runOnMainSyncWithResult {
            val editText = createEditor("teh ", selectionStart = 4, selectionEnd = 4)
            assertTrue(editText.requestFocus())
            var replacement: Replacement? = null
            editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
                replacement = Replacement(scalarFrom, scalarTo, text)
            }

            editText.clearFocus()
            editText.runWithTransientInputMutationGuard {
                editText.text!!.replace(0, 3, "the")
                BaseInputConnection.setComposingSpans(editText.text!!)
                true
            }
            val ready = editText.prepareForExternalEditorUpdate()

            ImeResult(
                replacement = replacement,
                selection = editText.selectionStart to editText.selectionEnd,
                trace = editText.imeTraceSnapshotForTesting(),
                ready = ready
            )
        }

        assertTrue(afterBlur.ready)
        assertEquals(Replacement(1, 3, "he"), afterBlur.replacement)
        assertEquals(4 to 4, afterBlur.selection)
        assertTraceContains(afterBlur.trace, "nativeMutationApply")
    }

    @Test
    fun staleAndInvalidCorrectionsDoNotFallBackToInsertion() {
        val stale = runCorrectionScenario("tah ", offset = 0, oldText = "teh", newText = "the")
        assertNull(stale.replacement)
        assertNull(stale.inserted)
        assertTraceContains(stale.trace, "correctionExplicitNoop")

        val invalidExplicit = runCorrectionScenario("teh ", offset = -1, oldText = "teh", newText = "the")
        assertNull(invalidExplicit.replacement)
        assertNull(invalidExplicit.inserted)
        assertTraceContains(invalidExplicit.trace, "commitCorrectionResult")

        val invalidInferred = runCorrectionScenario("teh ", offset = -1, oldText = null, newText = "the")
        assertNull(invalidInferred.replacement)
        assertNull(invalidInferred.inserted)
        assertTraceContains(invalidInferred.trace, "correctionInferredNoop")
    }

    @Test
    fun pairedCompositionReturnRefreshesImeOnceBeforeTheNextCommit() {
        val (adapter, editorId) = createPairedV2TestEditor()
        try {
            ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
                lateinit var editText: EditorEditText
                scenario.onActivity { activity ->
                    editText = EditorEditText(activity).apply {
                        this.editorId = editorId
                        v2Driver = adapter
                        adapter.setContentHtml("<p>seed</p>")?.let {
                            applyUpdateJSON(it, notifyListener = false)
                        }
                    }
                    activity.setContentView(editText)
                }

                waitUntil("paired editor should attach to the activity window") {
                    editText.isAttachedToWindow && editText.windowToken != null
                }
                runOnMainSyncWithResult { assertTrue(editText.requestFocus()) }
                waitUntil("paired editor should gain focus before split") { editText.hasFocus() }

                lateinit var refreshedInputConnection: EditorInputConnection
                val initialGeneration = runOnMainSyncWithResult {
                    editText.setSelection(4)
                    editText.clearImeTraceForTesting()
                    val inputConnection = createInputConnection(editText)
                    assertTrue(inputConnection.setComposingText("\n", 1))
                    assertTrue(inputConnection.commitText("\n", 1))
                    editText.inputConnectionGenerationForTesting()
                }
                instrumentation.waitForIdleSync()

                val afterSplit = runOnMainSyncWithResult {
                    val editorInfo = EditorInfo()
                    refreshedInputConnection = createInputConnection(editText, editorInfo)
                    ImeResult(
                        selection = editText.selectionStart to editText.selectionEnd,
                        trace = editText.imeTraceSnapshotForTesting(),
                        ready = editorInfo.initialCapsMode and InputType.TYPE_TEXT_FLAG_CAP_SENTENCES != 0,
                        surroundingText = editorInfo.getInitialTextBeforeCursor(20, 0).toString(),
                        imeSelection = editorInfo.initialSelStart to editorInfo.initialSelEnd,
                        inputConnectionGeneration = editText.inputConnectionGenerationForTesting()
                    )
                }
                // The empty trailing block owns a zero-width render placeholder. Android is
                // given the sanitized value through EditorInfo, asserted below. The rendered
                // Editable's raw caret sits after that placeholder.
                assertEquals("seed\n\u200B", editText.text.toString())
                assertEquals(6 to 6, afterSplit.selection)
                assertEquals(5 to 5, afterSplit.imeSelection)
                assertEquals(initialGeneration, afterSplit.inputConnectionGeneration)
                assertEquals("seed\n", afterSplit.surroundingText)
                assertEquals(1, afterSplit.trace.count { it.startsWith("lineBoundaryInputRefreshScheduled") })
                assertEquals(1, afterSplit.trace.count { it.startsWith("restartInput:source=lineBoundary:splitBlock") })
                assertEquals(
                    1,
                    afterSplit.trace.count {
                        it.startsWith("createInputConnection:boundEditor=$editorId boundGen=$initialGeneration")
                    }
                )
                assertTrue(afterSplit.trace.any {
                    it.startsWith("applySelectionFromJSON:doc=") && it.contains("scalar=5..5")
                })
                assertTrue(afterSplit.ready)

                runOnMainSyncWithResult {
                    assertTrue(refreshedInputConnection.commitText("x", 1))
                }
                instrumentation.waitForIdleSync()

                assertEquals("seed\nx", editText.text.toString())
                assertEquals(
                    6 to 6,
                    runOnMainSyncWithResult { editText.selectionStart to editText.selectionEnd }
                )
                val document = adapter.documentJson()?.let(::JSONObject) ?: error("missing document JSON")
                assertEquals(2, document.getJSONArray("content").length())
            }
        } finally {
            releasePairedV2TestEditor(editorId)
        }
    }

    private fun runCorrectionScenario(
        text: String,
        offset: Int,
        oldText: String?,
        newText: String
    ): ImeResult =
        runOnMainSyncWithResult {
            val editText = createEditor(text, selectionStart = text.length, selectionEnd = text.length)
            var replacement: Replacement? = null
            var inserted: Inserted? = null
            editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, replacementText ->
                replacement = Replacement(scalarFrom, scalarTo, replacementText)
            }
            editText.onInsertTextInRustForTesting = { insertedText, scalar ->
                inserted = Inserted(insertedText, scalar)
            }

            val inputConnection = createInputConnection(editText)
            assertTrue(inputConnection.commitCorrection(CorrectionInfo(offset, oldText, newText)))

            ImeResult(
                replacement = replacement,
                inserted = inserted,
                trace = editText.imeTraceSnapshotForTesting()
            )
        }

    private fun createEditor(
        text: String,
        selectionStart: Int,
        selectionEnd: Int
    ): EditorEditText =
        EditorEditText(context).apply {
            applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
            Selection.setSelection(this.text, selectionStart, selectionEnd)
            onSetSelectionScalarInRustForTesting = { _, _ -> }
            editorId = 1
            clearImeTraceForTesting()
        }

    private fun createInputConnection(
        editText: EditorEditText,
        editorInfo: EditorInfo = EditorInfo()
    ): EditorInputConnection {
        val inputConnection = editText.onCreateInputConnection(editorInfo)
        assertTrue(inputConnection is EditorInputConnection)
        return inputConnection as EditorInputConnection
    }

    private fun assertTraceContains(trace: List<String>, event: String) {
        assertTrue(
            "expected IME trace to contain $event but was $trace",
            trace.any { it.startsWith(event) }
        )
    }

    private fun waitUntil(
        description: String,
        timeoutMs: Long = 3_000L,
        predicate: () -> Boolean
    ) {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            instrumentation.waitForIdleSync()
            if (predicate()) return
            SystemClock.sleep(50)
        }
        assertTrue(description, predicate())
    }

    private fun renderUpdateJson(text: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray()
                        .put(
                            JSONObject()
                                .put("type", "blockStart")
                                .put("nodeType", "paragraph")
                                .put("depth", 0)
                        )
                        .put(
                            JSONObject()
                                .put("type", "textRun")
                                .put("text", text)
                                .put("marks", JSONArray())
                        )
                        .put(JSONObject().put("type", "blockEnd"))
                )
            )
            .toString()

    @Suppress("UNCHECKED_CAST")
    private fun <T> runOnMainSyncWithResult(block: () -> T): T {
        val result = AtomicReference<Any?>()
        val error = AtomicReference<Throwable?>()
        instrumentation.runOnMainSync {
            try {
                result.set(block())
            } catch (throwable: Throwable) {
                error.set(throwable)
            }
        }
        error.get()?.let { throw it }
        return result.get() as T
    }

    private data class Replacement(
        val scalarFrom: Int,
        val scalarTo: Int,
        val text: String
    )

    private data class Inserted(
        val text: String,
        val scalar: Int
    )

    private data class ImeResult(
        val replacement: Replacement? = null,
        val inserted: Inserted? = null,
        val selection: Pair<Int, Int> = 0 to 0,
        val trace: List<String> = emptyList(),
        val ready: Boolean = false,
        val surroundingText: String = "",
        val imeSelection: Pair<Int, Int> = 0 to 0,
        val inputConnectionGeneration: Long = 0L
    )
}
