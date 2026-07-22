package com.apollohg.editor

/**
 * v2 integer boundary rules. u64 values are always carried over JS/native
 * boundaries as canonical decimal strings; numeric fields are exact u32s.
 */
internal fun canonicalV2U64(value: String?): String? {
    if (value.isNullOrEmpty()) return null
    if (value != "0" && value.first() == '0') return null
    if (!value.all { it in '0'..'9' }) return null
    return value.takeIf { it.toULongOrNull() != null }
}

internal fun exactV2U32(value: Number?): UInt? {
    value ?: return null
    return when (value) {
        is Byte -> value.toLong().takeIf { it >= 0 }?.toUInt()
        is Short -> value.toLong().takeIf { it >= 0 }?.toUInt()
        is Int -> value.toLong().takeIf { it >= 0 }?.toUInt()
        is Long -> value.takeIf { it >= 0L && it <= UInt.MAX_VALUE.toLong() }?.toUInt()
        is Float -> exactV2U32FromDouble(value.toDouble())
        is Double -> exactV2U32FromDouble(value)
        // Expo's numeric bridge supplies only the primitive boxed forms
        // above. Reject arbitrary Number implementations rather than first
        // rounding them to Double and accidentally accepting a lossy value.
        else -> null
    }
}

private fun exactV2U32FromDouble(value: Double): UInt? {
    if (!value.isFinite() || value < 0.0 || value > UInt.MAX_VALUE.toDouble()) return null
    val integral = value.toLong()
    if (integral.toDouble() != value) return null
    return integral.toUInt()
}

/** The view uses signed [Int] scalar offsets, so reject values it cannot represent. */
internal fun exactV2ScalarInt(value: Number?): Int? =
    exactV2U32(value)?.takeIf { it <= Int.MAX_VALUE.toUInt() }?.toInt()
