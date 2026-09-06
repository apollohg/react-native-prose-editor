import type { MentionSuggestion } from './addons';
import { StyleSheet, Text, View } from 'react-native';
import { MaterialIcons } from '@expo/vector-icons';
import { type EditorToolbarDefaultIconId, type EditorToolbarIcon } from './EditorToolbarTypes';
import { BUTTON_HIT } from './EditorToolbarRegistry';

export const BUTTON_VISIBLE = 32;

export const TOOLBAR_PADDING_H = 12;

export const TOOLBAR_PADDING_V = 4;

export const MAX_BUTTON_SIZE = 40;

export const BUTTON_HEIGHT_INSET = 4;

export const MENU_MARGIN = 8;

export const MENU_WIDTH = 192;

export const KEYBOARD_FRAME_REMEASURE_DELAYS_MS = [50, 150, 300] as const;

export const ACTIVE_BG = 'rgba(0, 122, 255, 0.12)';

export const ACTIVE_COLOR = '#007AFF';

export const DEFAULT_COLOR = '#666666';

export const DISABLED_COLOR = '#C7C7CC';

export const SEPARATOR_COLOR = '#E5E5EA';

export const TOOLBAR_BG = '#FFFFFF';

export const TOOLBAR_BORDER = '#E5E5EA';

export const TOOLBAR_RADIUS = 0;

export const BUTTON_RADIUS = 6;

export const MENU_BORDER = '#D1D1D6';

export const MENU_SHADOW = '#000000';

export const DEFAULT_GLYPH_ICONS: Record<EditorToolbarDefaultIconId, string> = {
    bold: 'B',
    italic: 'I',
    underline: 'U',
    strike: 'S',
    link: '🔗',
    image: '🖼',
    blockquote: '❝',
    h1: 'H1',
    h2: 'H2',
    h3: 'H3',
    h4: 'H4',
    h5: 'H5',
    h6: 'H6',
    bulletList: '•≡',
    orderedList: '1.',
    indentList: '→',
    outdentList: '←',
    lineBreak: '↵',
    horizontalRule: '—',
    undo: '↩',
    redo: '↪',
};

export const DEFAULT_MATERIAL_ICONS: Partial<Record<EditorToolbarDefaultIconId, string>> = {
    bold: 'format-bold',
    italic: 'format-italic',
    underline: 'format-underlined',
    strike: 'strikethrough-s',
    link: 'link',
    image: 'image',
    h1: 'title',
    h2: 'title',
    h3: 'title',
    h4: 'title',
    h5: 'title',
    h6: 'title',
    blockquote: 'format-quote',
    bulletList: 'format-list-bulleted',
    orderedList: 'format-list-numbered',
    indentList: 'format-indent-increase',
    outdentList: 'format-indent-decrease',
    lineBreak: 'keyboard-return',
    horizontalRule: 'horizontal-rule',
    undo: 'undo',
    redo: 'redo',
};

export function resolveMentionSuggestionDisplayLabel(
    suggestion: MentionSuggestion,
    trigger: string
): string {
    const label = suggestion.label?.trim() || suggestion.title;
    return trigger.length > 0 && !label.startsWith(trigger) ? `${trigger}${label}` : label;
}

export function ToolbarIcon({
    icon,
    color,
    size,
}: {
    icon: EditorToolbarIcon;
    color: string;
    size?: number;
}) {
    const materialIconName = resolveMaterialIconName(icon);
    if (materialIconName) {
        return (
            <View style={styles.iconContainer}>
                <MaterialIcons name={materialIconName as never} size={size ?? 20} color={color} />
            </View>
        );
    }

    const glyph = resolveGlyphText(icon) ?? '?';
    return (
        <View style={styles.iconContainer}>
            <Text style={[styles.iconText, size == null ? null : { fontSize: size }, { color }]}>
                {glyph}
            </Text>
        </View>
    );
}

export function resolveMaterialIconName(icon: EditorToolbarIcon): string | undefined {
    switch (icon.type) {
        case 'default':
            return DEFAULT_MATERIAL_ICONS[icon.id];
        case 'platform':
            return icon.android?.type === 'material' ? icon.android.name : undefined;
        case 'glyph':
            return undefined;
    }
}

export function resolveGlyphText(icon: EditorToolbarIcon): string | undefined {
    switch (icon.type) {
        case 'default':
            return DEFAULT_GLYPH_ICONS[icon.id];
        case 'glyph':
            return icon.text;
        case 'platform':
            return icon.fallbackText;
    }
}

export const styles = StyleSheet.create({
    container: {
        backgroundColor: TOOLBAR_BG,
        borderTopWidth: StyleSheet.hairlineWidth,
        borderTopColor: TOOLBAR_BORDER,
        paddingVertical: TOOLBAR_PADDING_V,
        overflow: 'hidden',
    },
    containerWithoutTopBorder: {
        borderTopWidth: 0,
    },
    scrollContent: {
        flexDirection: 'row',
        alignItems: 'center',
        paddingHorizontal: TOOLBAR_PADDING_H,
        minWidth: '100%',
    },
    mentionSuggestionsContent: {
        paddingHorizontal: 12,
        paddingVertical: 4,
        alignItems: 'center',
        minWidth: '100%',
    },
    mentionSuggestionsScroll: {
        overflow: 'hidden',
    },
    toolbarRow: {
        flexDirection: 'row',
        alignItems: 'center',
    },
    fixedSection: {
        flexDirection: 'row',
        alignItems: 'center',
        flexShrink: 0,
    },
    startFixedSection: {
        paddingStart: TOOLBAR_PADDING_H,
    },
    endFixedSection: {
        paddingEnd: TOOLBAR_PADDING_H,
    },
    scrollSection: {
        flex: 1,
        minWidth: 0,
    },
    mentionSuggestion: {
        minWidth: 88,
        minHeight: 40,
        marginRight: 8,
        paddingHorizontal: 12,
        paddingVertical: 8,
        justifyContent: 'center',
    },
    mentionSuggestionTitle: {
        fontSize: 14,
        fontWeight: '600',
    },
    mentionSuggestionSubtitle: {
        marginTop: 1,
        fontSize: 12,
    },
    buttonAnchor: {
        position: 'relative',
    },
    button: {
        width: BUTTON_HIT,
        height: BUTTON_VISIBLE,
        justifyContent: 'center',
        alignItems: 'center',
        borderRadius: BUTTON_RADIUS,
    },
    groupDisclosure: {
        position: 'absolute',
        right: 5,
        bottom: 2,
        fontSize: 9,
        fontWeight: '700',
    },
    separator: {
        width: StyleSheet.hairlineWidth,
        height: 20,
        marginHorizontal: 4,
        backgroundColor: SEPARATOR_COLOR,
    },
    iconContainer: {
        justifyContent: 'center',
        alignItems: 'center',
    },
    iconText: {
        fontSize: 16,
        fontWeight: '600',
    },
    menuBackdrop: {
        flex: 1,
    },
    menuCard: {
        position: 'absolute',
        width: MENU_WIDTH,
        borderRadius: 14,
        borderWidth: StyleSheet.hairlineWidth,
        paddingVertical: 8,
        shadowColor: MENU_SHADOW,
        shadowOpacity: 0.16,
        shadowRadius: 18,
        shadowOffset: { width: 0, height: 8 },
        elevation: 10,
    },
    menuItem: {
        minHeight: 40,
        paddingHorizontal: 12,
        flexDirection: 'row',
        alignItems: 'center',
    },
    menuLabel: {
        flex: 1,
        marginLeft: 10,
        fontSize: 14,
        fontWeight: '500',
    },
});
