import {
    type DocumentJSON,
    type HistoryState,
    type NativeEditorDocumentHandle,
    type ReadonlyActiveState,
    type Selection,
} from './NativeEditorBridge';
import { type ImageNodeAttributes } from './schemas';
import { type EditorToolbarHeadingLevel, type EditorToolbarItem } from './EditorToolbar';
import { type EditorImageLoadingPolicy } from './ImageLoadingPolicy';
import { type RichTextEditorFocusPreservingRefs } from './useFocusPreservingFrames';
import { type StyleProp, type ViewStyle } from 'react-native';
import { type EditorTheme } from './EditorTheme';
import { type EditorAddons } from './addons';
import { type AtomNodeDefinition } from './atoms';
import { type AtomViewport } from './AtomHost';
import {
    type ExternalTextCompositionOptions,
    type ExternalTextCompositionSession,
} from './ExternalTextComposition';

/**
 * How the editor handles content taller than its frame: `'fixed'` scrolls
 * internally, `'autoGrow'` grows the view to fit.
 */
export type RichTextEditorHeightBehavior = 'fixed' | 'autoGrow';

/**
 * Where the toolbar lives: `'keyboard'` attaches the native toolbar to the
 * keyboard, `'inline'` renders the JavaScript `EditorToolbar` below the editor.
 */
export type RichTextEditorToolbarPlacement = 'keyboard' | 'inline';

/**
 * What an external `valueJSON` change does to undo history: `'replace'`
 * records one undoable step, `'reset'` clears history entirely.
 */
export type RichTextEditorValueJSONUpdateMode = 'replace' | 'reset';

/** Native keyboard auto-capitalization behavior. */
export type RichTextEditorAutoCapitalize = 'none' | 'sentences' | 'words' | 'characters';

/** Native keyboard layout. Values not supported by a platform fall back to its default. */
export type RichTextEditorKeyboardType =
    | 'default'
    | 'email-address'
    | 'numeric'
    | 'phone-pad'
    | 'ascii-capable'
    | 'numbers-and-punctuation'
    | 'url'
    | 'number-pad'
    | 'name-phone-pad'
    | 'decimal-pad'
    | 'twitter'
    | 'web-search'
    | 'visible-password'
    | 'ascii-capable-number-pad';

/** Android-specific options passed to the active input method. */
export interface RichTextEditorAndroidInputOptions {
    /** Private options interpreted by the selected Android IME. */
    privateImeOptions?: string;
}

/**
 * One remote collaborator's caret or selection, drawn as a native overlay.
 * `useYjsCollaboration` builds these from awareness and passes them through
 * `editorBindings`.
 */
export interface RemoteSelectionDecoration {
    /** Peer identity. Also the overlay's React-style key. */
    clientId: string;
    /** Fixed end of the remote selection, in engine doc positions. */
    anchor: number;
    /** Moving end. Equals `anchor` for a collapsed remote caret. */
    head: number;
    /** Caret and label color, as a color string. */
    color: string;
    /** Name shown on the caret label. */
    name?: string;
    avatarUrl?: string;
    /** Whether that peer's editor holds focus. The caret bar is drawn only when
     *  true; the highlighted range is drawn either way. Absent counts as false. */
    isFocused?: boolean;
}

export interface LinkRequestContext {
    /** Current link href at the selection, when one is active. */
    href?: string;
    /** Whether a link mark is active at the selection. */
    isActive: boolean;
    /** The selection the request was issued for (engine doc positions). */
    selection: Selection;
    /** Apply or update the link on the engine selection. */
    setLink: (href: string) => void;
    /** Remove the link from the engine selection. */
    unsetLink: () => void;
}

export interface ImageRequestContext {
    /** The selection the request was issued for (engine doc positions). */
    selection: Selection;
    /** Insert a block image node at the engine selection. */
    insertImage: (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) => void;
}

/**
 * The v2 prop contract. Initialization and input policy are NOT props:
 * - `initialContent` / `initialJSON` are gone: initial document state
 *   belongs to the handle's creation config (`NativeEditorInitialization`:
 *   `localEmpty` / `localJson` / `localHtml` / `room`).
 * - `schema` and `fragmentName` belong to `NativeEditorCreateConfig`.
 * - `maxLength`, engine-enforced `allowBase64Images`, `readOnly`, and
 *   `inputFilter` belong to `NativeEditorCreateConfig.policy`.
 * - `resourceLimits` belongs to `NativeEditorCreateConfig.limits.resource`.
 *   `editable` is only a per-view interaction gate; it never changes the
 *   handle's engine read-only policy.
 * - `autoDetectLinks`, `preserveSelectionOnValueJSONReset`, and
 *   `selectionOnValueJSONReset` were removed with the legacy bridge and have
 *   no v2 equivalent.
 */
export interface RichTextEditorProps {
    /** Accessible name announced for the native editable control. */
    accessibilityLabel?: string;
    /** Additional guidance announced for the native editable control. */
    accessibilityHint?: string;
    /** Controlled HTML content. External changes are diffed and applied. */
    value?: string;
    /** Controlled ProseMirror JSON content. Ignored if value is set. */
    valueJSON?: DocumentJSON;
    /** Optional stable revision hint for `valueJSON` to avoid reserializing equal docs on rerender. */
    valueJSONRevision?: string;
    /** Controls how external `valueJSON` changes are applied. Defaults to preserving undo history. */
    valueJSONUpdateMode?: RichTextEditorValueJSONUpdateMode;
    /** Placeholder text shown when editor is empty. */
    placeholder?: string;
    /** Whether the editor is editable. Defaults to true. When false, every mutation ref method rejects with MUTATION_REJECTED; selection and controlled content still flow. */
    editable?: boolean;
    /** Whether to auto-focus on mount. */
    autoFocus?: boolean;
    /** Controls native keyboard auto-capitalization. Defaults to sentences. */
    autoCapitalize?: RichTextEditorAutoCapitalize;
    /** Controls native keyboard autocorrection. Defaults to the platform-specific editor default. */
    autoCorrect?: boolean;
    /** Controls the native keyboard layout. Defaults to the platform default keyboard. */
    keyboardType?: RichTextEditorKeyboardType;
    /** Android-specific input-method options. Ignored on other platforms. */
    androidInputOptions?: RichTextEditorAndroidInputOptions;
    /** Controls whether the editor scrolls internally or grows with content. */
    heightBehavior?: RichTextEditorHeightBehavior;
    /** Whether to show the formatting toolbar. Defaults to true. */
    showToolbar?: boolean;
    /** Whether the toolbar is attached to the keyboard natively or rendered inline in React. */
    toolbarPlacement?: RichTextEditorToolbarPlacement;
    /** Displayed toolbar buttons, in order. Supports custom marks/nodes. */
    toolbarItems?: readonly EditorToolbarItem[];
    /** Called when a custom `action` toolbar item is pressed. */
    onToolbarAction?: (key: string) => void;
    /** Called when a toolbar link item is pressed so the host can collect/edit a URL. */
    onRequestLink?: (context: LinkRequestContext) => void;
    /** Called when a toolbar image item is pressed so the host can choose an image source. */
    onRequestImage?: (context: ImageRequestContext) => void;
    /** Bounds native data-URL and remote image loading. */
    imageLoadingPolicy?: EditorImageLoadingPolicy;
    /** Whether selected images show native resize handles. */
    allowImageResizing?: boolean;
    /** Called when content changes with the current HTML. */
    onContentChange?: (html: string) => void;
    /** Called when content changes with the current ProseMirror JSON. */
    onContentChangeJSON?: (json: DocumentJSON) => void;
    /** Called when selection changes (engine doc positions). */
    onSelectionChange?: (selection: Selection) => void;
    /** Called when active formatting state changes. */
    onActiveStateChange?: (state: ReadonlyActiveState) => void;
    /** Called when undo/redo availability changes. */
    onHistoryStateChange?: (state: HistoryState) => void;
    /** Called when the editor gains focus. */
    onFocus?: () => void;
    /** Called when the editor loses focus. */
    onBlur?: () => void;
    /** External native views whose taps preserve this editor's focus, keyboard, and selection. */
    focusPreservingRefs?: RichTextEditorFocusPreservingRefs;
    /** Style applied to the native editor view. */
    style?: StyleProp<ViewStyle>;
    /** Style applied to the outer React container wrapping the editor and inline toolbar. */
    containerStyle?: StyleProp<ViewStyle>;
    /** Optional native content theme applied to rendered blocks and typing attrs. */
    theme?: EditorTheme;
    /** Optional addon configuration. */
    addons?: EditorAddons;
    /** Custom void-block node definitions mounted as React children. */
    atoms?: readonly AtomNodeDefinition<any>[];
    /** Whether atom controls receive input, independently of editable. Defaults to true. */
    atomsInteractive?: boolean;
    /** Opt-in atom virtualization using the native scroll viewport. */
    virtualizeAtoms?: boolean;
    /** Overrides the visible range in native atom layout coordinates, in points. */
    atomViewport?: AtomViewport;
    /** Remote awareness selections rendered as native overlays. */
    remoteSelections?: readonly RemoteSelectionDecoration[];
    /**
     * Shared v2 document session — the only construction path. The native
     * view binds to the same session (its editorId is passed straight to the
     * view), so typing, IME, selection, and toolbar commands flow through
     * the native v2 adapters into the shared engine while this component
     * drives the retained document API (`value`, `valueJSON`, `setContent`,
     * `setContentJson`, `getContent`, `getContentJson`) through the same
     * handle. The same handle is handed to the collaboration controller
     * (one session). Initialization belongs to the handle's creation config.
     * A room handle awaiting the server document renders nothing until Rust
     * promotes an accepted Step 2.
     */
    documentHandle: NativeEditorDocumentHandle;
    /**
     * Revision signal rendered by the collaboration controller. Advances
     * trigger an authoritative engine re-read (remote commits, promotions).
     */
    documentRevision?: string | null;
    /** Application notification after a successful local mutation. Native transport wakes independently. */
    onLocalCommit?: () => void;
}

export interface RichTextEditorRef {
    /** Programmatically focus the editor. */
    focus(): void;
    /** Programmatically blur the editor. */
    blur(): void;
    /** Check whether the mounted native editor supports external text composition. */
    supportsExternalTextComposition(): boolean;
    /** Begin an external text composition session. */
    beginExternalTextComposition(
        options?: ExternalTextCompositionOptions
    ): Promise<ExternalTextCompositionSession>;
    /** Toggle a formatting mark (e.g. 'bold', 'italic'). */
    toggleMark(markType: string): void;
    /** Apply or update a hyperlink on the current selection. */
    setLink(href: string): void;
    /** Remove a hyperlink from the current selection. */
    unsetLink(): void;
    /** Toggle blockquote wrapping around the current block selection. */
    toggleBlockquote(): void;
    /** Toggle a heading level on the current block selection. */
    toggleHeading(level: EditorToolbarHeadingLevel): void;
    /** Toggle a list type supported by the active schema. */
    toggleList(listType: string): void;
    /** Indent the current list item. */
    indentListItem(): void;
    /** Outdent the current list item. */
    outdentListItem(): void;
    /** Insert a void node (e.g. 'horizontalRule'). */
    insertNode(nodeType: string): void;
    /** Insert a block image node with the given source and optional metadata. */
    insertImage(src: string, attrs?: Omit<ImageNodeAttributes, 'src'>): void;
    /** Insert text at the current cursor position. */
    insertText(text: string): void;
    /** Insert HTML content at the current selection. */
    insertContentHtml(html: string): void;
    /** Insert JSON content at the current selection. */
    insertContentJson(doc: DocumentJSON): void;
    /** Replace entire document with HTML (preserves undo history). */
    setContent(html: string): void;
    /** Replace entire document with JSON (preserves undo history). */
    setContentJson(doc: DocumentJSON): void;
    /** Clear the document to the active schema's empty text block. */
    clearContent(): void;
    /** Get the current HTML content. */
    getContent(): string;
    /** Get the current content as ProseMirror JSON. */
    getContentJson(): DocumentJSON;
    /** Ask the Rust editor core whether the current document is empty. */
    getIsEmpty(): boolean;
    /** Get the plain text content (no markup). */
    getTextContent(): string;
    /** Get the current caret rectangle in editor-local layout coordinates. */
    getCaretRect(): Promise<RichTextEditorCaretRect | null>;
    /** Undo the last operation. */
    undo(): void;
    /** Redo the last undone operation. */
    redo(): void;
    /** Check if undo is available. */
    canUndo(): boolean;
    /** Check if redo is available. */
    canRedo(): boolean;
}

export interface RichTextEditorCaretRect {
    /** Left edge of the caret, relative to the editor root view. */
    x: number;
    /** Top edge of the caret, relative to the editor root view. */
    y: number;
    /** Caret width. */
    width: number;
    /** Caret height. */
    height: number;
    /** Current editor root view width. */
    editorWidth: number;
    /** Current editor root view height. */
    editorHeight: number;
}

/** @deprecated Use RichTextEditorHeightBehavior instead. */
export type NativeRichTextEditorHeightBehavior = RichTextEditorHeightBehavior;

/** @deprecated Use RichTextEditorToolbarPlacement instead. */
export type NativeRichTextEditorToolbarPlacement = RichTextEditorToolbarPlacement;

/** @deprecated Use RichTextEditorValueJSONUpdateMode instead. */
export type NativeRichTextEditorValueJSONUpdateMode = RichTextEditorValueJSONUpdateMode;

/** @deprecated Use RichTextEditorAutoCapitalize instead. */
export type NativeRichTextEditorAutoCapitalize = RichTextEditorAutoCapitalize;

/** @deprecated Use RichTextEditorKeyboardType instead. */
export type NativeRichTextEditorKeyboardType = RichTextEditorKeyboardType;

/** @deprecated Use RichTextEditorAndroidInputOptions instead. */
export type NativeRichTextEditorAndroidInputOptions = RichTextEditorAndroidInputOptions;

/** @deprecated Use RichTextEditorProps instead. */
export type NativeRichTextEditorProps = RichTextEditorProps;

/** @deprecated Use RichTextEditorRef instead. */
export type NativeRichTextEditorRef = RichTextEditorRef;

/** @deprecated Use RichTextEditorCaretRect instead. */
export type NativeRichTextEditorCaretRect = RichTextEditorCaretRect;
