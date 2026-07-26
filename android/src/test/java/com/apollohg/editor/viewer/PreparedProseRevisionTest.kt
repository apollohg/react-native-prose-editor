package com.apollohg.editor.viewer

import android.graphics.Rect
import android.graphics.Typeface
import android.content.res.Configuration
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

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
        assertTrue(revisions.recordIntrinsicSize("i", 10, 20, null))
        assertFalse(revisions.recordIntrinsicSize("i", 20, 40, null))
        assertEquals(1, revisions.revision)
    }

    @Test fun intrinsicPublicationResetsAfterMetadataLRUEvictionForReuse() {
        val state = ViewerAttachmentRevisionState()
        val evictedMetadata = ViewerImageIntrinsicStore(entryLimit = 1)
        assertTrue(state.recordIntrinsicSize("7:https://example.test/a", 10, 20, null))
        evictedMetadata.store("7:https://example.test/a", 10 to 20)
        evictedMetadata.store("8:https://example.test/b", 20 to 10)
        assertEquals(null, evictedMetadata.size("7:https://example.test/a"))

        state.reset()
        assertTrue(state.recordIntrinsicSize("7:https://example.test/a", 10, 20, null))
        assertEquals(1, state.revision)
    }

    @Test fun intrinsicPublicationStateIsBoundedWithoutEvictingPublishedIds() {
        val state = ViewerAttachmentRevisionState()
        repeat(ViewerAttachmentRevisionState.PUBLICATION_LIMIT) { index ->
            assertTrue(state.recordIntrinsicSize("$index:https://example.test/image", 1, 1, null))
        }
        assertEquals(ViewerAttachmentRevisionState.PUBLICATION_LIMIT, state.retainedPublicationCountForTesting)
        assertFalse(state.recordIntrinsicSize("overflow:https://example.test/image", 1, 1, null))
        assertFalse(state.recordIntrinsicSize("0:https://example.test/image", 2, 2, null))
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

    @Test fun registeredCustomFamilyIsNotWarnedAndDemonstrablyMissingFamilyWarnsOnce() {
        ViewerFontEnvironment.resetFamilyRegistryForTesting()
        ViewerFontEnvironment.registerAvailableFamily("viewer-test-font", Typeface.DEFAULT)
        assertFalse(ViewerFontEnvironment.resolveFamily("viewer-test-font", Typeface.NORMAL, Typeface.SANS_SERIF).isDemonstrablyMissing)
        ViewerFontEnvironment.markFamilyUnavailable("viewer-missing-font")
        assertTrue(ViewerFontEnvironment.resolveFamily("viewer-missing-font", Typeface.NORMAL, Typeface.SANS_SERIF).isDemonstrablyMissing)
        assertTrue(ViewerFontEnvironment.warnOnceForMissingFamily("viewer-missing-font", "semantic", "revision"))
        assertFalse(ViewerFontEnvironment.warnOnceForMissingFamily("viewer-missing-font", "semantic", "revision"))
        assertFalse(ViewerFontEnvironment.resolveFamily("unproven-font", Typeface.NORMAL, Typeface.SANS_SERIF).isDemonstrablyMissing)
        ViewerFontEnvironment.resetFamilyRegistryForTesting()
    }

    @Test fun resourceFailureIsPublishedOncePerGenerationAndAttachment() {
        val pipeline = ViewerImagePipeline()
        var failures = 0
        pipeline.onResourceFailure = { failures += 1 }
        pipeline.begin("resource", true)
        val attachment = ViewerImageAttachment("secret", "https://user:credential@example.test/a", Rect(), null)
        pipeline.reportFailureForTesting(attachment)
        pipeline.reportFailureForTesting(attachment)
        assertEquals(1, failures)
    }
}
