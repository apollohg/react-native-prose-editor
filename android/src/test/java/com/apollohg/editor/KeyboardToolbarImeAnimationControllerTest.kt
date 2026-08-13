package com.apollohg.editor

import android.view.View
import androidx.core.graphics.Insets
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class KeyboardToolbarImeAnimationControllerTest {

    @Test
    fun `toolbar follows animated ime inset while showing`() {
        val toolbar = View(RuntimeEnvironment.getApplication()).apply {
            visibility = View.INVISIBLE
        }
        val targetBottoms = mutableListOf<Int>()
        val settledBottoms = mutableListOf<Int>()
        val controller = KeyboardToolbarImeAnimationController(
            toolbarView = toolbar,
            onTargetImeBottomChanged = { targetBottoms.add(it) },
            onImeAnimationSettled = { settledBottoms.add(it) }
        )
        val animation = imeAnimation()

        controller.animationCallback.onPrepare(animation)
        controller.onApplyWindowInsets(imeInsets(600))
        controller.animationCallback.onStart(animation, animationBounds())

        assertEquals(View.VISIBLE, toolbar.visibility)
        assertEquals(600f, toolbar.translationY)

        controller.animationCallback.onProgress(imeInsets(240), listOf(animation))

        assertEquals(360f, toolbar.translationY)
        assertEquals(listOf(600), targetBottoms)
        assertEquals(emptyList<Int>(), settledBottoms)

        controller.animationCallback.onEnd(animation)

        assertEquals(0f, toolbar.translationY)
        assertEquals(View.VISIBLE, toolbar.visibility)
        assertEquals(listOf(600), settledBottoms)
    }

    @Test
    fun `toolbar remains visible while ime hides and settles invisible`() {
        val toolbar = View(RuntimeEnvironment.getApplication())
        val settledBottoms = mutableListOf<Int>()
        val controller = KeyboardToolbarImeAnimationController(
            toolbarView = toolbar,
            onTargetImeBottomChanged = {},
            onImeAnimationSettled = { settledBottoms.add(it) }
        )
        controller.onApplyWindowInsets(imeInsets(600))
        val animation = imeAnimation()

        controller.animationCallback.onPrepare(animation)
        controller.onApplyWindowInsets(imeInsets(0))
        controller.animationCallback.onStart(animation, animationBounds())

        assertEquals(View.VISIBLE, toolbar.visibility)
        assertEquals(-600f, toolbar.translationY)

        controller.animationCallback.onProgress(imeInsets(240), listOf(animation))

        assertEquals(-240f, toolbar.translationY)
        assertEquals(listOf(600), settledBottoms)

        controller.animationCallback.onEnd(animation)

        assertEquals(0f, toolbar.translationY)
        assertEquals(View.INVISIBLE, toolbar.visibility)
        assertEquals(listOf(600, 0), settledBottoms)
    }

    private fun imeAnimation(): WindowInsetsAnimationCompat =
        WindowInsetsAnimationCompat(WindowInsetsCompat.Type.ime(), null, 250L)

    private fun imeInsets(bottom: Int): WindowInsetsCompat =
        WindowInsetsCompat.Builder()
            .setInsets(WindowInsetsCompat.Type.ime(), Insets.of(0, 0, 0, bottom))
            .build()

    private fun animationBounds(): WindowInsetsAnimationCompat.BoundsCompat =
        WindowInsetsAnimationCompat.BoundsCompat(Insets.NONE, Insets.NONE)
}
