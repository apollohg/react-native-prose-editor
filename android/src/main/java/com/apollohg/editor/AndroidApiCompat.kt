package com.apollohg.editor

import android.os.Build
import android.text.Layout
import android.view.accessibility.AccessibilityNodeInfo
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat

internal object AndroidApiCompat {
    fun lineBottomWithoutSpacing(layout: Layout, line: Int): Int =
        if (Build.VERSION.SDK_INT >= 34) {
            layout.getLineBottom(line, false)
        } else {
            layout.getLineBottom(line)
        }

    fun setScreenReaderFocusable(info: AccessibilityNodeInfo, value: Boolean) {
        AccessibilityNodeInfoCompat.wrap(info).isScreenReaderFocusable = value
    }
}
