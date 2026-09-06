package com.apollohg.editor.viewer
import android.graphics.Paint
import android.graphics.Rect
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import android.text.TextDirectionHeuristics
import android.view.View
import android.view.ViewGroup
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import uniffi.editor_core.FfiViewerMark
import java.text.Bidi
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class PreparedProseAccessibilityTest : PreparedProseAccessibilityTestFixture() {
    @Test
    @Config(sdk = [24])
    fun `virtual accessibility nodes are screen reader focusable on API 24`() {
        val view = PreparedProseDrawingView(RuntimeEnvironment.getApplication())
        view.install(preparedArtifact("api-24"))

        val node = requireNotNull(view.accessibilityNodeProvider.createAccessibilityNodeInfo(1))

        assertTrue(AccessibilityNodeInfoCompat.wrap(node).isScreenReaderFocusable)
    }

    @Test
    fun `inert annotations neither consume touch nor publish virtual children`() {
        val view = PreparedProseDrawingView(RuntimeEnvironment.getApplication())
        val activated = mutableListOf<PreparedProseInteraction.Kind>()
        view.onInteractionActivated = { interaction ->
            activated += interaction.kind
            true
        }
        view.install(interactiveArtifact())
        view.linkInteractionsEnabled = false
        view.mentionInteractionsEnabled = false

        assertFalse(tap(view, 10f, 10f))
        assertFalse(tap(view, 40f, 50f))
        assertEquals(
            0,
            requireNotNull(
                view.accessibilityNodeProvider.createAccessibilityNodeInfo(View.NO_ID)
            ).childCount,
        )

        view.mentionInteractionsEnabled = true
        assertFalse(tap(view, 10f, 10f))
        assertTrue(tap(view, 40f, 50f))
        assertEquals(listOf(PreparedProseInteraction.Kind.MENTION), activated)
        assertEquals(
            1,
            requireNotNull(
                view.accessibilityNodeProvider.createAccessibilityNodeInfo(View.NO_ID)
            ).childCount,
        )
        assertEquals(
            "mention",
            AccessibilityNodeInfoCompat.wrap(
                requireNotNull(view.accessibilityNodeProvider.createAccessibilityNodeInfo(1))
            ).roleDescription,
        )
    }

    @Test
    fun `hidden Fabric annotations reject accessibility focus and activation`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, view)
        var activations = 0
        view.onInteractionActivated = {
            activations += 1
            true
        }
        view.install(preparedArtifact("hidden-actions"))
        view.accessibilityVisibilityForTesting = { false }
        view.visibility = View.INVISIBLE

        assertFalse(
            view.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null,
            )
        )
        assertFalse(
            view.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )
        assertEquals(0, activations)
    }

    @Test
    fun `Fabric capability changes clear focus before virtual nodes are renumbered`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context).apply {
            mentionInteractionsEnabled = true
        }
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, view)
        view.install(interactiveArtifact())
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                2,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )
        var clearedNodeLabel: CharSequence? = null
        parent.onEvent = { event ->
            if (event.eventType == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED) {
                clearedNodeLabel = view.accessibilityNodeProvider
                    .createAccessibilityNodeInfo(2)
                    ?.contentDescription
            }
        }

        view.linkInteractionsEnabled = false

        assertEquals("@Ada", clearedNodeLabel)
    }

    @Test
    fun `virtual node visibility follows clipping and hidden ancestors`() {
        val globalVisibleBounds = Rect(0, 0, 100, 30)

        assertTrue(
            accessibilityBoundsVisible(
                Rect(0, 0, 20, 20),
                globalVisibleBounds,
                shown = true,
                windowVisible = true,
                alphaVisible = true,
            )
        )
        assertFalse(
            accessibilityBoundsVisible(
                Rect(30, 40, 50, 60),
                globalVisibleBounds,
                shown = true,
                windowVisible = true,
                alphaVisible = true,
            )
        )
        assertFalse(
            accessibilityBoundsVisible(
                Rect(0, 0, 20, 20),
                globalVisibleBounds,
                shown = false,
                windowVisible = true,
                alphaVisible = true,
            )
        )
    }

    @Test
    fun `focused virtual node clears when its host becomes hidden`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, view)
        view.install(preparedArtifact("focus-visibility"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        view.accessibilityVisibilityForTesting = { false }
        view.visibility = View.INVISIBLE
        val hiddenNode = requireNotNull(
            view.accessibilityNodeProvider.createAccessibilityNodeInfo(1)
        )

        assertFalse(hiddenNode.isAccessibilityFocused)
        assertEquals(
            listOf(AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED),
            parent.eventTypes,
        )
    }

    @Test
    fun `Fabric-equivalent prepared replacement announces one subtree change and preserves focus clear`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, view)
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        var clearedNodeLabel: CharSequence? = null
        parent.onEvent = { event ->
            if (event.eventType == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED) {
                clearedNodeLabel = view.accessibilityNodeProvider
                    .createAccessibilityNodeInfo(1)
                    ?.contentDescription
            }
        }
        view.install(null, announceAccessibilitySubtree = false)
        view.install(preparedArtifact("replacement"))

        assertEquals("first", clearedNodeLabel)
        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )

        parent.clearEvents()
        val installedArtifact = view.preparedLayout
        view.linkInteractionsEnabled = false
        assertEquals(1, parent.subtreeChangeCount())
        assertTrue(view.preparedLayout === installedArtifact)

        parent.clearEvents()
        view.install(null)
        assertEquals(1, parent.subtreeChangeCount())
    }

    @Test
    fun `Fabric mount success lets final installation own one replacement subtree notification`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, view)
        val transaction = FabricReplacementAccessibilityTransaction()
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        transaction.clearReplacing(view)
        transaction.installMountedReplacement(view, preparedArtifact("replacement"))

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )
    }

    @Test
    fun `Fabric mount miss announces a removed subtree once and suppresses a later deferred install`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, view)
        val transaction = FabricReplacementAccessibilityTransaction()
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        transaction.clearReplacing(view)
        transaction.finishWithoutMountedReplacement(view)

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )

        parent.clearEvents()
        transaction.installMountedReplacement(view, preparedArtifact("deferred replacement"))
        assertEquals(0, parent.subtreeChangeCount())
        assertEquals(emptyList<Int>(), parent.eventTypes)
    }

    @Test
    fun `host geometry freezes wrapped links and long-safe mentions in reading order`() {
        val document = ViewerDocument(
            semanticKey = "interaction-fixture",
            blocks = listOf(
                ViewerBlock(
                    nodeType = "paragraph",
                    depth = 0,
                    inBlockquote = false,
                    listContext = null,
                    listItemBoundary = null,
                    inlines = listOf(
                        ViewerInline.Text(
                            "linked ".repeat(12),
                            listOf(FfiViewerMark("link", "{\"href\":\"https://example.test/wrapped\"}")),
                        ),
                        ViewerInline.Atom("mention", UInt.MAX_VALUE.toLong(), "{}", "@Ada"),
                    ),
                ),
            ),
            isEmpty = false,
            retainedBytes = 64,
        )
        val layout = StaticLayoutAndroidProseLayoutEngine().prepare(
            document,
            key(document),
            PreparedProseTheme.resolve(null, 1f),
            90,
            1f,
            false,
        )

        assertEquals(listOf(PreparedProseInteraction.Kind.LINK, PreparedProseInteraction.Kind.MENTION), layout.interactions.map { it.kind })
        assertEquals("https://example.test/wrapped", layout.interactions.first().href)
        assertTrue(layout.interactions.first().rects.isNotEmpty())
        assertEquals(UInt.MAX_VALUE.toLong(), layout.interactions.last().docPos)
        assertEquals(listOf(PreparedProseAccessibilityNode.Role.LINK, PreparedProseAccessibilityNode.Role.MENTION), layout.accessibilityNodes.map { it.role })
        assertTrue(layout.retainedBytes > document.retainedBytes)
    }
}
