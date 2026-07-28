package com.apollohg.editor.viewer

import org.json.JSONObject
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedProseInstrumentationContractTest {
    @Test
    fun classifierAndSchemaContract() {
        val fixture = fixture().getJSONObject("frameClassification")
        val period = fixture.getLong("nominalFramePeriodNanos")
        val tolerance = fixture.getLong("singleTickToleranceNanos")
        val oneTick = fixture.getJSONArray("oneTickDeltasNanos")
        for (index in 0 until oneTick.length()) {
            assertEquals(
                PreparedProseInstrumentation.FrameClassification(1, false),
                PreparedProseInstrumentation.classifyFrame(oneTick.getLong(index), period, tolerance),
            )
        }
        val delayed = fixture.getLong("delayedDeltaNanos")
        assertEquals(
            PreparedProseInstrumentation.FrameClassification(3, true),
            PreparedProseInstrumentation.classifyFrame(delayed, period, tolerance),
        )

        assertFalse(
            PreparedProseInstrumentation.viewerCaused(
                0L,
                delayed,
                listOf(
                    PreparedProseInstrumentation.ViewerWorkSpan(10_000_000L, 13_900_000L, PreparedProseInstrumentation.ViewerWorkKind.DRAW),
                    PreparedProseInstrumentation.ViewerWorkSpan(10_000_000L, 13_900_000L, PreparedProseInstrumentation.ViewerWorkKind.LAYOUT),
                ),
                period,
            ),
        )
        assertTrue(
            PreparedProseInstrumentation.viewerCaused(
                0L,
                delayed,
                listOf(
                    PreparedProseInstrumentation.ViewerWorkSpan(0L, 12_000_000L, PreparedProseInstrumentation.ViewerWorkKind.LAYOUT),
                    PreparedProseInstrumentation.ViewerWorkSpan(12_000_000L, 24_000_000L, PreparedProseInstrumentation.ViewerWorkKind.DRAW),
                ),
                period,
            ),
        )

        PreparedProseInstrumentation.beginBenchmark()
        listOf(
            PreparedProseInstrumentation.TraversalPhase.COLD,
            PreparedProseInstrumentation.TraversalPhase.WARM,
            PreparedProseInstrumentation.TraversalPhase.IMAGES_DISABLED,
        ).forEach {
            PreparedProseInstrumentation.beginPhase(it)
            PreparedProseInstrumentation.endPhase()
        }
        val export = JSONObject(PreparedProseInstrumentation.exportJson())
        assertEquals(2, export.getInt("schemaVersion"))
        assertEquals(period, export.getLong("nominalFramePeriodNanos"))
        assertEquals(tolerance, export.getLong("singleTickToleranceNanos"))
        listOf("preResetSnapshot", "postResetSnapshot").forEach { name ->
            val snapshot = export.getJSONObject(name)
            listOf(
                "unmountedCurrentBytes",
                "unmountedHighWaterBytes",
                "unmountedCurrentResidentCount",
                "unmountedHighWaterResidentCount",
                "compiledCurrentBytes",
                "compiledCurrentResidentCount",
            ).forEach { field -> assertEquals(0L, snapshot.getLong(field)) }
        }
        val phases = export.getJSONObject("phaseSamples")
        listOf("cold", "warm", "imagesDisabled").forEach { name ->
            val phase = phases.getJSONObject(name)
            listOf("imageRequestCount", "imageMetadataCount", "imageDecodeCount")
                .forEach { field -> assertEquals(0, phase.getInt(field)) }
        }
    }

    private fun fixture(): JSONObject {
        val stream = RuntimeEnvironment.getApplication().assets.open("prepared-prose-harness-static-fixtures.json")
        return JSONObject(stream.bufferedReader().use { it.readText() })
    }
}
