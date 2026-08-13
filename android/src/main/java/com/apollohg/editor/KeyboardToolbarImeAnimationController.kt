package com.apollohg.editor

import android.view.View
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat

internal class KeyboardToolbarImeAnimationController(
    private val toolbarView: View,
    private val onTargetImeBottomChanged: (Int) -> Unit,
    private val onImeAnimationSettled: (Int) -> Unit
) {
    private var activeImeAnimation: WindowInsetsAnimationCompat? = null
    private var displayedImeBottom = 0
    private var startImeBottom = 0
    private var targetImeBottom = 0

    val animationCallback = object : WindowInsetsAnimationCompat.Callback(
        DISPATCH_MODE_CONTINUE_ON_SUBTREE
    ) {
        override fun onPrepare(animation: WindowInsetsAnimationCompat) {
            if (!animation.affectsIme()) return
            activeImeAnimation = animation
            startImeBottom = displayedImeBottom
        }

        override fun onStart(
            animation: WindowInsetsAnimationCompat,
            bounds: WindowInsetsAnimationCompat.BoundsCompat
        ): WindowInsetsAnimationCompat.BoundsCompat {
            if (animation !== activeImeAnimation) return bounds
            updateToolbarForAnimatedImeBottom(startImeBottom)
            return bounds
        }

        override fun onProgress(
            insets: WindowInsetsCompat,
            runningAnimations: MutableList<WindowInsetsAnimationCompat>
        ): WindowInsetsCompat {
            if (
                activeImeAnimation != null &&
                runningAnimations.any { it.affectsIme() }
            ) {
                updateToolbarForAnimatedImeBottom(insets.imeBottom())
            }
            return insets
        }

        override fun onEnd(animation: WindowInsetsAnimationCompat) {
            if (animation !== activeImeAnimation) return
            activeImeAnimation = null
            settleAtTargetImeBottom()
        }
    }

    fun onApplyWindowInsets(insets: WindowInsetsCompat) {
        targetImeBottom = insets.imeBottom()
        onTargetImeBottomChanged(targetImeBottom)
        if (activeImeAnimation == null) {
            settleAtTargetImeBottom()
        } else if (startImeBottom > 0 || targetImeBottom > 0) {
            toolbarView.visibility = View.VISIBLE
        }
    }

    fun cancel() {
        activeImeAnimation = null
        displayedImeBottom = targetImeBottom
        startImeBottom = targetImeBottom
        toolbarView.translationY = 0f
    }

    fun reset() {
        activeImeAnimation = null
        displayedImeBottom = 0
        startImeBottom = 0
        targetImeBottom = 0
        toolbarView.translationY = 0f
    }

    private fun updateToolbarForAnimatedImeBottom(animatedImeBottom: Int) {
        displayedImeBottom = animatedImeBottom
        toolbarView.translationY = (targetImeBottom - animatedImeBottom).toFloat()
        if (displayedImeBottom > 0 || startImeBottom > 0 || targetImeBottom > 0) {
            toolbarView.visibility = View.VISIBLE
        }
    }

    private fun settleAtTargetImeBottom() {
        displayedImeBottom = targetImeBottom
        startImeBottom = targetImeBottom
        toolbarView.translationY = 0f
        toolbarView.visibility = if (targetImeBottom > 0) View.VISIBLE else View.INVISIBLE
        onImeAnimationSettled(targetImeBottom)
    }

    private fun WindowInsetsAnimationCompat.affectsIme(): Boolean =
        typeMask and WindowInsetsCompat.Type.ime() != 0

    private fun WindowInsetsCompat.imeBottom(): Int =
        getInsets(WindowInsetsCompat.Type.ime()).bottom
}
