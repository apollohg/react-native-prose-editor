package com.apollohg.editor.viewer

import android.graphics.Bitmap
import android.graphics.Rect
import android.graphics.Typeface
import android.content.res.Configuration
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerSource
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class PreparedProseRevisionTest {
    @Test fun disabledImagesDoNotCreateRequests() {
        val pipeline = ViewerImagePipeline()
        pipeline.begin("disabled", false)
        pipeline.updateVisibleRect(Rect(0, 0, 100, 100), listOf(ViewerImageAttachment("i", "https://example.test/i.png", Rect(0, 0, 10, 10), null)))
        assertEquals(0, pipeline.requestCountForTesting)
    }

    @Test fun imagePipelineDoesNotAcquireBeforeMountedVisibility() {
        val pipeline = ViewerImagePipeline()
        pipeline.begin("mounted", true)
        assertEquals(0, pipeline.requestCountForTesting)
    }

    @Test fun zeroAndOffscreenVisibleRectsDoNotAcquireImages() {
        val pipeline = ViewerImagePipeline()
        pipeline.begin("visible", true)
        val attachment = ViewerImageAttachment("i", "data:image/png;base64,", Rect(1000, 1000, 1010, 1010), null)
        pipeline.updateVisibleRect(Rect(), listOf(attachment))
        pipeline.updateVisibleRect(Rect(0, 0, 20, 20), listOf(attachment))
        assertEquals(0, pipeline.requestCountForTesting)
    }

    @Test fun unknownMetadataAdvancesAttachmentRevisionOnce() {
        val revisions = ViewerAttachmentRevisionState()
        revisions.admit(1)
        assertTrue(revisions.recordIntrinsicSize("i", 0, 10, 20, null))
        assertFalse(revisions.recordIntrinsicSize("i", 0, 20, 40, null))
        assertEquals(1, revisions.revision)
    }

    @Test fun intrinsicMetadataDoesNotReopenAcrossFabricReinstall() {
        val state = ViewerAttachmentRevisionState()
        assertTrue(state.beginSemanticGeneration("semantic-a"))
        state.admit(1)
        assertTrue(state.recordIntrinsicSize("7:https://example.test/a", 0, 10, 20, null))
        state.admit(1)
        assertFalse(state.recordIntrinsicSize("7:https://example.test/a", 0, 10, 20, null))
        assertEquals(1, state.revision)
    }

    @Test fun fabricMeasurementResetsSemanticSidecarBeforeMountBindsOrdinals() {
        val surface = FabricSurfaceToken(71, 9)
        try {
            val first = FabricAttachmentSidecars.begin(surface, "semantic-a")
            first.admit(1)
            assertTrue(first.recordIntrinsicSize("7:https://example.test/a", 0, 10, 20, null))

            val replacement = FabricAttachmentSidecars.begin(surface, "semantic-b")
            assertTrue(first === replacement)
            assertEquals(0, replacement.revision)
            replacement.admit(1)
            assertTrue(replacement.recordIntrinsicSize("7:https://example.test/a", 0, 10, 20, null))
            assertEquals(1, replacement.revision)
        } finally {
            FabricAttachmentSidecars.remove(surface)
        }
    }

    @Test fun semanticIdentityIncludesAllPublicationInputsButExcludesStateRevisions() {
        val base = ProseViewerRequest(
            ProseViewerSource.Json("{\"type\":\"doc\"}"),
            ProseViewerConfiguration(
                configJson = "{\"mentions\":{\"prefix\":\"@\"},\"maxLines\":2,\"overflow\":\"clip\"}",
                themeJson = "{\"paragraph\":{\"fontSize\":16}}",
                imagePolicyJson = "{\"maxDecodedBytes\":1024}",
                imagesEnabled = true,
                collapsesWhenEmpty = true,
            ),
        )
        val stateRevision = base.copy(nativeFontRevision = 3, fontEnvironmentRevision = 4, attachmentRevision = 5)
        assertEquals(base.semanticGenerationIdentity, stateRevision.semanticGenerationIdentity)
        assertFalse(base.generationIdentity == stateRevision.generationIdentity)

        val variants = listOf(
            base.copy(source = ProseViewerSource.Html(base.source.value)),
            base.copy(configuration = base.configuration.copy(configJson = "{\"mentions\":{\"prefix\":\"#\"},\"maxLines\":2,\"overflow\":\"clip\"}")),
            base.copy(configuration = base.configuration.copy(themeJson = "{\"paragraph\":{\"fontSize\":18}}")),
            base.copy(configuration = base.configuration.copy(imagePolicyJson = "{\"maxDecodedBytes\":2048}")),
            base.copy(configuration = base.configuration.copy(imagesEnabled = false)),
            base.copy(configuration = base.configuration.copy(collapsesWhenEmpty = false)),
        )
        variants.forEach { assertFalse(base.semanticGenerationIdentity == it.semanticGenerationIdentity) }
    }

    @Test fun semanticReplacementResetsPublicationAndResourceErrorBitsExactlyOnce() {
        val state = ViewerAttachmentRevisionState()
        assertTrue(state.beginSemanticGeneration("semantic-a"))
        state.admit(1)
        assertTrue(state.recordIntrinsicSize("7:https://example.test/a", 0, 40, 20, null))
        assertTrue(state.recordResourceFailure(0))
        assertFalse(state.beginSemanticGeneration("semantic-a"))
        assertFalse(state.recordResourceFailure(0))
        assertTrue(state.beginSemanticGeneration("semantic-b"))
        state.admit(1)
        assertEquals(0, state.revision)
        assertTrue(state.recordResourceFailure(0))
    }

    @Test fun allAdmittedUnknownAttachmentsBeyond256PublishOnceWithCompactBitset() {
        val state = ViewerAttachmentRevisionState()
        val count = 513
        val semanticIdentity = "semantic-byte-fixture"
        assertTrue(state.beginSemanticGeneration(semanticIdentity))
        state.admit(count)
        repeat(count) { index ->
            assertTrue(state.recordIntrinsicSize("$index:https://example.test/image", index, 1, 1, null))
        }
        assertEquals(count.toLong(), state.revision)
        assertEquals(
            ViewerAttachmentRevisionState.FIXED_RETAINED_BYTES +
                ViewerAttachmentRevisionState.COLLECTION_RETAINED_BYTES * 5 +
                (count + 7) / 8 * 2 +
                count * (Int.SIZE_BYTES * 3 + Long.SIZE_BYTES) +
                semanticIdentity.length * 2 +
                    (0 until count).sumOf { "$it:https://example.test/image".length * 2 },
            state.retainedPublicationBytesForTesting,
        )
        assertEquals(1 to 1, state.intrinsicSize(count - 1))
        assertFalse(state.recordIntrinsicSize("0:https://example.test/image", 0, 2, 2, null))
    }

    @Test fun globalMetadataLRUEvictionFallsBackToOwnMeasurementSidecarWithoutRepublishing() {
        val state = ViewerAttachmentRevisionState()
        ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting(1)
        try {
            assertTrue(state.beginSemanticGeneration("semantic-a"))
            state.admit(1)
            assertTrue(state.recordIntrinsicSize("7:https://example.test/a", 0, 10, 20, null))
            ViewerImageIntrinsicStore.shared.store("8:https://example.test/b", 20 to 10)
            assertEquals(null, ViewerImageIntrinsicStore.shared.globalSize("7:https://example.test/a"))
            assertEquals(10 to 20, FabricAttachmentSidecars.withMeasurementState(state) {
                ViewerImageIntrinsicStore.shared.size("7:https://example.test/a")
            })
            assertFalse(state.recordIntrinsicSize("7:https://example.test/a", 0, 10, 20, null))
            assertEquals(1, state.revision)
        } finally {
            ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting()
        }
    }

    @Test fun concurrentFabricMeasurementScopesKeepEvictedIntrinsicMetadataSurfaceLocalAndCleanUp() {
        val first = FabricSurfaceToken(91, 1)
        val second = FabricSurfaceToken(92, 1)
        // This deliberately collides an attachment identity across semantic
        // source/revision states; only the stable Fabric token may select it.
        val id = "7:https://example.test/shared"
        val executor = Executors.newFixedThreadPool(2)
        ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting(1)
        try {
            val firstState = FabricAttachmentSidecars.begin(first, "source-a-revision-1")
            firstState.admit(1)
            assertTrue(firstState.recordIntrinsicSize(id, 0, 80, 40, null))
            val secondState = FabricAttachmentSidecars.begin(second, "source-b-revision-2")
            secondState.admit(1)
            assertTrue(secondState.recordIntrinsicSize(id, 0, 30, 60, null))
            assertEquals(1, firstState.revision)
            assertEquals(1, secondState.revision)
            ViewerImageIntrinsicStore.shared.store("8:https://example.test/evict", 1 to 1)
            assertEquals(null, ViewerImageIntrinsicStore.shared.globalSize(id))

            val ready = CountDownLatch(2)
            val proceed = CountDownLatch(1)
            val firstResult = executor.submit<Pair<Int, Int>?> {
                FabricAttachmentSidecars.withMeasurementState(firstState) {
                    ready.countDown()
                    assertTrue(proceed.await(1, TimeUnit.SECONDS))
                    ViewerImageIntrinsicStore.shared.size(id)
                }
            }
            val secondResult = executor.submit<Pair<Int, Int>?> {
                FabricAttachmentSidecars.withMeasurementState(secondState) {
                    ready.countDown()
                    assertTrue(proceed.await(1, TimeUnit.SECONDS))
                    ViewerImageIntrinsicStore.shared.size(id)
                }
            }
            assertTrue(ready.await(1, TimeUnit.SECONDS))
            proceed.countDown()
            assertEquals(80 to 40, firstResult.get(1, TimeUnit.SECONDS))
            assertEquals(30 to 60, secondResult.get(1, TimeUnit.SECONDS))

            FabricAttachmentSidecars.withMeasurementState(firstState) {
                try {
                    FabricAttachmentSidecars.withMeasurementState(secondState) {
                        throw IllegalStateException("fixture failure")
                    }
                } catch (_: IllegalStateException) {
                    // The nested exception must restore the outer scope first.
                }
                assertTrue(FabricAttachmentSidecars.currentMeasurementState === firstState)
            }
            assertEquals(null, FabricAttachmentSidecars.currentMeasurementState)

            FabricAttachmentSidecars.remove(first)
            FabricAttachmentSidecars.remove(second)
            assertEquals(null, FabricAttachmentSidecars.state(first))
            assertEquals(null, FabricAttachmentSidecars.state(second))
        } finally {
            executor.shutdownNow()
            FabricAttachmentSidecars.remove(first)
            FabricAttachmentSidecars.remove(second)
            ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting()
        }
    }

    @Test fun mountedPixelOwnershipCountsOnlySurfaceMapEntries() {
        val shared = Bitmap.createBitmap(3, 2, Bitmap.Config.ARGB_8888)
        val replacement = Bitmap.createBitmap(4, 2, Bitmap.Config.ARGB_8888)
        assertEquals(
            PreparedProseDrawingView.IMAGE_PIXEL_MAP_RETAINED_BYTES +
                PreparedProseDrawingView.IMAGE_PIXEL_ENTRY_RETAINED_BYTES * 2,
            PreparedProseDrawingView.retainedImagePixelsBytes(mapOf("first" to shared, "second" to shared)),
        )
        assertEquals(
            PreparedProseDrawingView.IMAGE_PIXEL_MAP_RETAINED_BYTES +
                PreparedProseDrawingView.IMAGE_PIXEL_ENTRY_RETAINED_BYTES,
            PreparedProseDrawingView.retainedImagePixelsBytes(mapOf("first" to replacement)),
        )
        assertEquals(0, PreparedProseDrawingView.retainedImagePixelsBytes(emptyMap()))
    }

    @Test fun boundedIntrinsicMetadataEvictsOldestEntryDeterministically() {
        val store = ViewerImageIntrinsicStore(entryLimit = 2)
        store.store("a", 10 to 10)
        store.store("b", 20 to 20)
        store.store("c", 30 to 30)
        assertEquals(null, store.size("a"))
        assertEquals(20 to 20, store.size("b"))
        assertEquals(30 to 30, store.size("c"))
    }

    @Test fun staleGenerationCompletionIsRejected() {
        val pipeline = ViewerImagePipeline()
        pipeline.begin("one", true)
        pipeline.begin("two", true)
        assertFalse(pipeline.acceptsCompletion("one"))
        assertTrue(pipeline.acceptsCompletion("two"))
    }

    @Test fun missingFamilyWarningSurvivesFontReplacementButNewSemanticGenerationWarns() {
        ViewerFontEnvironment.resetMissingWarningsForTesting()
        try {
            assertTrue(ViewerFontEnvironment.warnOnceForMissingFamily("missing", "semantic-a"))
            assertFalse(ViewerFontEnvironment.warnOnceForMissingFamily("missing", "semantic-a"))
            ViewerFontEnvironment().invalidateRegisteredFonts()
            assertFalse(ViewerFontEnvironment.warnOnceForMissingFamily("missing", "semantic-a"))
            assertTrue(ViewerFontEnvironment.warnOnceForMissingFamily("missing", "semantic-b"))
        } finally {
            ViewerFontEnvironment.resetMissingWarningsForTesting()
        }
    }

    @Test fun warningContextUsesSemanticIdentityInsteadOfLayoutReplacementIdentity() {
        val base = ProseViewerRequest(ProseViewerSource.Json("{\"type\":\"doc\"}"), ProseViewerConfiguration())
        val replacement = base.copy(nativeFontRevision = 1, fontEnvironmentRevision = 2, attachmentRevision = 3)
        val baseKey = ProseLayoutKey(
            semanticKey = "fixture",
            widthPx = 100,
            themeDigest = base.themeDigest,
            nativeFontRevision = base.nativeFontRevision,
            fontEnvironmentRevision = base.fontEnvironmentRevision,
            densityBits = 1f.toRawBits().toLong(),
            attachmentRevision = base.attachmentRevision,
            generationIdentity = base.generationIdentity,
            semanticGenerationIdentity = base.semanticGenerationIdentity,
        )
        val replacementKey = baseKey.copy(
            widthPx = 120,
            nativeFontRevision = replacement.nativeFontRevision,
            fontEnvironmentRevision = replacement.fontEnvironmentRevision,
            attachmentRevision = replacement.attachmentRevision,
            generationIdentity = replacement.generationIdentity,
            semanticGenerationIdentity = replacement.semanticGenerationIdentity,
        )
        assertFalse(baseKey.generationIdentity == replacementKey.generationIdentity)
        assertEquals(baseKey.semanticGenerationIdentity, replacementKey.semanticGenerationIdentity)
    }

    @Test fun explicitFontAvailabilityAndSystemScaleEachPublishOneReplacementRevision() {
        val environment = ViewerFontEnvironment()
        val revisions = mutableListOf<Long>()
        environment.onInvalidated = revisions::add
        environment.invalidateRegisteredFonts()
        environment.onConfigurationChanged(Configuration().apply { fontScale = 1.3f })
        environment.onConfigurationChanged(Configuration().apply { fontScale = 1.3f })
        assertEquals(listOf(1L, 2L), revisions)
    }

    @Test fun fontScaleChangesResolvedGeometryWithoutDoubleDensity() {
        val base = PreparedProseTheme.resolve(null, density = 2f, fontScale = 1f)
        val scaled = PreparedProseTheme.resolve(null, density = 2f, fontScale = 1.5f)
        assertEquals(34f, base.paragraph.sizePx)
        assertEquals(51f, scaled.paragraph.sizePx)
    }

    @Test fun registeredCustomFamilyNeverFalseWarnsAndOrdinaryPlatformFallbackWarnsOnce() {
        ViewerFontEnvironment.resetFamilyRegistryForTesting()
        ViewerFontEnvironment.registerAvailableFamily("viewer-test-font", Typeface.DEFAULT)
        assertFalse(ViewerFontEnvironment.resolveFamily("viewer-test-font", Typeface.NORMAL, Typeface.SANS_SERIF).isDemonstrablyMissing)
        ViewerFontEnvironment.setPlatformFamilyResolverForTesting { false }
        assertTrue(ViewerFontEnvironment.resolveFamily("ordinary-missing-font", Typeface.NORMAL, Typeface.SANS_SERIF).isDemonstrablyMissing)
        assertTrue(ViewerFontEnvironment.warnOnceForMissingFamily("ordinary-missing-font", "semantic"))
        assertFalse(ViewerFontEnvironment.warnOnceForMissingFamily("ordinary-missing-font", "semantic"))
        assertFalse(ViewerFontEnvironment.resolveFamily("sans-serif", Typeface.NORMAL, Typeface.SANS_SERIF).isDemonstrablyMissing)
        ViewerFontEnvironment.resetFamilyRegistryForTesting()
    }

    @Test fun familyRegistrationInvalidatesMountedDirectAndFabricObserversOnceAndTeardownRemovesObserver() {
        ViewerFontEnvironment.resetFamilyRegistryForTesting()
        val direct = ViewerFontEnvironment()
        val fabric = ViewerFontEnvironment()
        val directRevisions = mutableListOf<Long>()
        val fabricRevisions = mutableListOf<Long>()
        direct.onInvalidated = directRevisions::add
        fabric.onInvalidated = fabricRevisions::add
        direct.activate()
        fabric.activate()
        ViewerFontEnvironment.registerAvailableFamily("observer-font", Typeface.DEFAULT)
        ViewerFontEnvironment.registerAvailableFamily("observer-font", Typeface.DEFAULT)
        assertEquals(listOf(1L), directRevisions)
        assertEquals(listOf(1L), fabricRevisions)
        direct.deactivate()
        ViewerFontEnvironment.markFamilyUnavailable("observer-font")
        assertEquals(listOf(1L), directRevisions)
        assertEquals(listOf(1L, 2L), fabricRevisions)
        fabric.deactivate()
        ViewerFontEnvironment.resetFamilyRegistryForTesting()
    }

    @Test fun resourceFailureIsPublishedOncePerGenerationAndAttachment() {
        val state = ViewerAttachmentRevisionState()
        assertTrue(state.beginSemanticGeneration("resource"))
        state.admit(1)
        assertTrue(state.recordResourceFailure(0))
        assertFalse(state.recordResourceFailure(0))
        // A Fabric attachment-revision reinstall cancels/reconfigures requests,
        // but remains in the same semantic generation.
        assertFalse(state.beginSemanticGeneration("resource"))
        assertFalse(state.recordResourceFailure(0))
    }
}
