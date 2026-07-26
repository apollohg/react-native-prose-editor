package com.apollohg.editor.viewer

import android.graphics.Rect
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

    @Test fun unknownMetadataAdvancesAttachmentRevisionOnce() {
        val revisions = ViewerAttachmentRevisionState()
        assertTrue(revisions.recordIntrinsicSize("i", 10, 20, null))
        assertFalse(revisions.recordIntrinsicSize("i", 20, 40, null))
        assertEquals(1, revisions.revision)
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
}
