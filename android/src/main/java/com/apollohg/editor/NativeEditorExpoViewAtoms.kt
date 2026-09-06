package com.apollohg.editor

import android.graphics.Rect
import android.view.View
import android.view.ViewGroup

internal fun NativeEditorExpoView.atomChildAtImpl(index: Int): View? = reactChildren.getOrNull(index)

internal fun NativeEditorExpoView.addAtomChildImpl(child: View, index: Int) {
    reactChildren.remove(child)
    reactChildren.add(index.coerceIn(0, reactChildren.size), child)
    val atomKey = atomKey(child)
    if (atomKey == null) {
        (child.parent as? ViewGroup)?.removeView(child)
        addNativeAtomView(child, childCount)
        return
    }
    (child.parent as? ViewGroup)?.removeView(child)
    richTextView.mountAtomChild(child, atomKey)
    richTextView.orderAtomChildren(reactChildren)
}

internal fun NativeEditorExpoView.removeAtomChildAtImpl(index: Int) {
    reactChildren.getOrNull(index)?.let(::removeAtomChild)
}

internal fun NativeEditorExpoView.removeAtomChildImpl(child: View) {
    reactChildren.remove(child)
    if (!richTextView.unmountAtomChild(child)) {
        (child.parent as? ViewGroup)?.removeView(child)
    }
}

internal fun NativeEditorExpoView.emitAtomLayout(widthPx: Float, positions: List<AtomLayoutPosition>) {
    val density = resources.displayMetrics.density.takeIf { it > 0f } ?: 1f
    val contentOrigin = Rect()
    offsetDescendantRectToMyCoords(richTextView.editorContentFrame, contentOrigin)
    val event = mapOf<String, Any>(
        "width" to widthPx / density,
        "positions" to positions.map { position ->
            mapOf(
                "key" to position.key,
                "x" to position.xPx / density,
                "y" to position.yPx / density,
                "hostX" to (contentOrigin.left + position.xPx) / density,
                "hostY" to (contentOrigin.top + position.yPx) / density,
                "height" to position.heightPx / density,
                "width" to position.widthPx / density,
            )
        },
        "viewport" to mapOf(
            "y" to richTextView.editorScrollView.scrollY / density,
            "height" to richTextView.editorScrollView.height / density,
        ),
        "editorId" to eventEditorId(richTextView.editorId)
    )
    onAtomLayoutForTesting?.invoke(event) ?: onAtomLayout(event)
}

internal fun NativeEditorExpoView.atomKey(child: View): String? {
    val nativeId = child.getTag(com.facebook.react.R.id.view_tag_native_id) as? String
    if (nativeId == null || !nativeId.startsWith(ATOM_NATIVE_ID_PREFIX)) return null
    return nativeId.removePrefix(ATOM_NATIVE_ID_PREFIX).takeIf(String::isNotEmpty)
}
