package com.apollohg.editor

enum class EditorHeightBehavior {
    FIXED,
    AUTO_GROW;

    companion object {
        fun fromRaw(rawValue: String?): EditorHeightBehavior =
            when (rawValue) {
                "autoGrow" -> AUTO_GROW
                else -> FIXED
            }
    }
}
