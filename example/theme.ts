import { Platform } from 'react-native';
import { EditorStyleSheet } from 'react-native-rich-text-editor';
import type {
    EditorMentionTheme,
    EditorTextStyle,
    EditorToolbarTheme,
} from 'react-native-rich-text-editor';

/**
 * The one palette the app uses. Spruce is the ground the paper sheet sits on
 * and the single accent inside the document: links, mentions, list markers,
 * and active toolbar buttons all share it.
 */
export const PALETTE = {
    spruceDeep: '#123e3b',
    spruce: '#1f5f5b',
    spruceTint: '#dcebe8',
    mint: '#9fc7c0',
    paper: '#ffffff',
    wash: '#f2f4f3',
    hairline: '#e1e6e4',
    ink: '#1b1f2a',
    inkMuted: '#5a6170',
    inkFaint: '#9aa1ad',
    inkDisabled: '#c3c8d0',
    rose: '#b3261e',
} as const;

/** Generic family names resolve natively on both platforms. */
export const SERIF_FAMILY = 'serif';

/** React Native text needs a concrete family per platform. */
export const APP_SERIF_FAMILY = Platform.select({ ios: 'Georgia', default: 'serif' });

export const FONT_SIZE = {
    caption: 13,
    body: 17,
    stat: 20,
    title: 28,
} as const;

export const LINE_HEIGHT = {
    caption: 18,
    body: 27,
    stat: 24,
    title: 34,
} as const;

export const SPACE = {
    xs: 4,
    sm: 8,
    md: 12,
    lg: 16,
    xl: 20,
    xxl: 28,
    xxxl: 36,
} as const;

export const RADIUS = {
    control: 8,
    card: 12,
    sheet: 24,
} as const;

/** WCAG 2.5.5 minimum touch target. */
export const MIN_TOUCH_TARGET = 44;

const HEADING_SIZES = [32, 25, 21, 19, 17, 16] as const;
const HEADING_LINE_HEIGHT_RATIO = 1.2;
const HEADING_SPACING_RATIO = 0.35;

/** iOS keyboard accessory height; the package sizes buttons from it. */
const TOOLBAR_HEIGHT = 44;
const TOOLBAR_ICON_SIZE = 17;

/** The toolbar floats above the keyboard as a rounded bar, clear of its corners. */
export const toolbarTheme: EditorToolbarTheme = {
    appearance: 'custom',
    height: TOOLBAR_HEIGHT,
    backgroundColor: PALETTE.wash,
    borderColor: PALETTE.hairline,
    borderWidth: 1,
    borderRadius: RADIUS.card,
    showTopBorder: false,
    keyboardOffset: SPACE.sm,
    horizontalInset: SPACE.md,
    separatorColor: PALETTE.hairline,
    buttonIconSize: TOOLBAR_ICON_SIZE,
    buttonColor: PALETTE.inkMuted,
    buttonBackgroundColor: PALETTE.wash,
    buttonActiveColor: PALETTE.spruceDeep,
    buttonActiveBackgroundColor: PALETTE.spruceTint,
    buttonDisabledColor: PALETTE.inkDisabled,
    buttonDisabledBackgroundColor: PALETTE.wash,
    buttonBorderRadius: RADIUS.control,
};

export const editorTheme = EditorStyleSheet.create({
    content: {
        backgroundColor: PALETTE.paper,
        borderRadius: 0,
        paddingTop: SPACE.xxl,
        paddingHorizontal: SPACE.xl,
        paddingBottom: SPACE.xxxl,
    },
    placeholder: { color: PALETTE.inkFaint },
    text: { color: PALETTE.ink, fontSize: FONT_SIZE.body },
    paragraph: { lineHeight: LINE_HEIGHT.body, marginBottom: SPACE.md },
    h1: headingStyle(0),
    h2: headingStyle(1),
    h3: headingStyle(2),
    h4: headingStyle(3),
    h5: headingStyle(4),
    h6: headingStyle(5),
    blockquote: {
        fontFamily: SERIF_FAMILY,
        fontStyle: 'italic',
        color: PALETTE.inkMuted,
        borderLeftColor: PALETTE.spruce,
        borderLeftWidth: 2,
        paddingLeft: SPACE.md,
    },
    bulletList: { indent: SPACE.xl, baseIndentMultiplier: 1, marginBottom: SPACE.md },
    orderedList: { indent: SPACE.xl, baseIndentMultiplier: 1, marginBottom: SPACE.md },
    taskList: { indent: SPACE.xl, baseIndentMultiplier: 1, marginBottom: SPACE.md },
    listItem: { marginBottom: SPACE.xs },
    taskItem: { marginBottom: SPACE.xs },
    listMarker: { color: PALETTE.spruce, scale: 1, gap: SPACE.sm },
    horizontalRule: { backgroundColor: PALETTE.hairline, height: 1, marginVertical: SPACE.xl },
    link: { color: PALETTE.spruce, textDecorationLine: 'underline', fontWeight: '500' },
    codeBlock: {
        backgroundColor: PALETTE.wash,
        padding: SPACE.md,
        borderRadius: RADIUS.control,
        marginVertical: SPACE.md,
    },
    image: { backgroundColor: PALETTE.wash, borderRadius: RADIUS.card, marginVertical: SPACE.md },
    toolbar: toolbarTheme,
});

export const mentionTheme: EditorMentionTheme = {
    node: {
        color: PALETTE.spruceDeep,
        backgroundColor: PALETTE.spruceTint,
        fontWeight: '600',
        borderRadius: SPACE.xs,
    },
    suggestions: {
        backgroundColor: PALETTE.paper,
        borderColor: PALETTE.hairline,
        borderWidth: 1,
        borderRadius: RADIUS.card,
        shadowColor: PALETTE.ink,
        option: {
            textColor: PALETTE.ink,
            secondaryTextColor: PALETTE.inkMuted,
            backgroundColor: PALETTE.paper,
            borderRadius: RADIUS.control,
            fontWeight: '600',
            highlightedBackgroundColor: PALETTE.spruceTint,
            highlightedTextColor: PALETTE.spruceDeep,
        },
    },
};

function headingStyle(index: number): EditorTextStyle {
    const fontSize = HEADING_SIZES[index];
    return {
        fontFamily: SERIF_FAMILY,
        color: PALETTE.ink,
        fontSize,
        fontWeight: '700',
        lineHeight: Math.round(fontSize * HEADING_LINE_HEIGHT_RATIO),
        marginBottom: Math.round(fontSize * HEADING_SPACING_RATIO),
    };
}
