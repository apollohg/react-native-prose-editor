package com.apollohg.editor

import android.graphics.Color
import android.util.Base64
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class RenderSvgImageDecoderTest {
    private val rectangle = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 200 100"><rect x="10" y="20" width="200" height="100" fill="#ff0000"/></svg>"""

    @After
    fun tearDown() {
        RenderImageDecoder.resetForTesting()
    }

    private fun decode(svg: String, policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT) =
        RenderImageDecoder.decodeSourceLease(
            "data:image/svg+xml;base64," + Base64.encodeToString(svg.toByteArray(), Base64.NO_WRAP),
            policy,
        )

    @Test
    fun `base64 SVG renders pixels and preserves offset viewBox`() {
        val lease = decode(rectangle)
        assertNotNull(lease)
        lease!!.use {
            assertEquals(200, it.bitmap.width)
            assertEquals(100, it.bitmap.height)
            assertEquals(Color.RED, it.bitmap.getPixel(100, 50))
        }
    }

    @Test
    fun `remote SVG is identified by bytes with no SVG suffix or content type`() {
        MockWebServer().use { server ->
            server.enqueue(MockResponse().setBody(rectangle).setHeader("Content-Type", "application/octet-stream"))
            server.start()
            val lease = RenderImageDecoder.decodeSourceLease(server.url("/image?id=42").toString())
            assertNotNull(lease)
            lease!!.use { assertEquals(Color.RED, it.bitmap.getPixel(100, 50)) }
            assertEquals(1, server.requestCount)
        }
    }

    @Test
    fun `SVG rasterization obeys dimension and pixel budget before allocation`() {
        val lease = decode(rectangle, ImageLoadingPolicy.DEFAULT.copy(maxDecodeDimensionPx = 80, maxDecodedBytes = 3200))
        assertNotNull(lease)
        lease!!.use {
            assertEquals(40, it.bitmap.width)
            assertEquals(20, it.bitmap.height)
            assertTrue(it.byteCount <= 3200)
            assertEquals(Color.RED, it.bitmap.getPixel(20, 10))
        }
        assertNull(decode(rectangle, ImageLoadingPolicy.DEFAULT.copy(maxDecodedBytes = 3)))
    }

    @Test
    fun `SVG fixture renders shapes and text`() {
        val svg = requireNotNull(javaClass.getResource("/shapes-and-text.svg")).readText()
        val lease = decode(svg)
        assertNotNull(lease)
        lease!!.use {
            assertEquals(320, it.bitmap.width)
            assertEquals(180, it.bitmap.height)
            var opaque = 0
            for (y in 0 until 180) for (x in 0 until 320) {
                if (Color.alpha(it.bitmap.getPixel(x, y)) != 0) opaque++
            }
            assertTrue("Expected diagram shapes and labels", opaque > 4000)
            var textPixels = 0
            for (y in 24 until 48) for (x in 104 until 200) {
                if (Color.alpha(it.bitmap.getPixel(x, y)) != 0) textPixels++
            }
            assertTrue("Expected the Sample text label", textPixels > 100)
        }
    }

    @Test
    fun `SVG complexity and cyclic local references are rejected`() {
        val crowded = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">" +
            "<rect width=\"1\" height=\"1\"/>".repeat(8192) + "</svg>"
        assertNull(decode(crowded))
        val deep = "<svg xmlns=\"http://www.w3.org/2000/svg\">" + "<g>".repeat(128) +
            "</g>".repeat(128) + "</svg>"
        assertNull(decode(deep))
        assertNull(decode("""<svg xmlns="http://www.w3.org/2000/svg"><g id="loop"><use href="#loop"/></g></svg>"""))
    }

    @Test
    fun `local use references render and keep budget until lease release`() {
        val budget = DecodedBitmapBudget.shared()
        val before = budget.retainedProcessBytesForTesting()
        val lease = decode("""<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><defs><rect id="shape" width="20" height="10" fill="red"/></defs><use xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="#shape"/></svg>""")
        assertNotNull(lease)
        lease!!.use {
            assertEquals(Color.RED, it.bitmap.getPixel(10, 5))
            assertEquals(before + it.byteCount, budget.retainedProcessBytesForTesting())
        }
        assertEquals(before, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `literal SVG metadata permits at signs backslashes and URL-like names`() {
        val svg = """<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" id="mail@host" title="C:\icons" aria-label="mail@example.com" data-source="url(unclosed" class="curl-icon"><style>.curl-icon { fill: red; }</style><rect width="20" height="10"/></svg>"""
        val lease = decode(svg)
        assertNotNull(lease)
        lease!!.use { assertEquals(Color.RED, it.bitmap.getPixel(10, 5)) }
    }

    @Test
    fun `malformed CSS URL functions are rejected in styles and presentation attributes`() {
        for (attributes in listOf("fill=\"url(\"", "style=\"fill: url (\"")) {
            assertNull(decode("""<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" $attributes/></svg>"""))
        }
        assertNull(decode("""<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><style>rect {fill: url(}</style><rect width="20" height="10"/></svg>"""))
    }

    @Test
    fun `malformed SVG and active or external content are rejected`() {
        for (svg in listOf(
            "<svg", "<html/>",
            """<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><style>.paint {fill: url(#color)}</style><defs><linearGradient id="color"><stop stop-color="red"/></linearGradient></defs><rect class="paint" width="10" height="10"/></svg>""",
            """<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///etc/passwd">]><svg xmlns="http://www.w3.org/2000/svg"><text>&x;</text></svg>""",
            """<?xml-stylesheet href="https://example.com/style.css"?><svg xmlns="http://www.w3.org/2000/svg"/>""",
            """<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>""",
            """<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/image.png"/></svg>""",
        )) assertNull(svg, decode(svg))
    }
}
