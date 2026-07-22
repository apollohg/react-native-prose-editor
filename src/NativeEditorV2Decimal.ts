const CANONICAL_V2_U64 = /^(0|[1-9]\d*)$/;
const V2_U64_MAX = '18446744073709551615';

/**
 * Canonical v2 u64 wire values are decimal strings only. The range check is
 * lexical after syntax validation, so JavaScript never coerces the value.
 */
export function normalizeNativeEditorV2U64(value: unknown): string | null {
    if (typeof value !== 'string' || !CANONICAL_V2_U64.test(value)) return null;
    if (value.length < V2_U64_MAX.length) return value;
    if (value.length > V2_U64_MAX.length || value > V2_U64_MAX) return null;
    return value;
}
