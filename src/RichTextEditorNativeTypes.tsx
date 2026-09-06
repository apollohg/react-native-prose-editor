import {
    ExternalTextCompositionManager,
    type NativeExternalTextCompositionHandle,
} from './ExternalTextComposition';
import { type ReactNode } from 'react';
import { type NativeSyntheticEvent, type StyleProp, type ViewStyle } from 'react-native';
import { type DocumentJSON, type NativeEditorDocumentHandle } from './NativeEditorBridge';
import {
    type RichTextEditorAutoCapitalize,
    type RichTextEditorKeyboardType,
    type RichTextEditorToolbarPlacement,
    type RichTextEditorHeightBehavior,
} from './RichTextEditorTypes';

export interface NativeExternalTextCompositionEvent {
    editorId: string;
    resultJson: string;
}

export interface NativeEditorViewHandle extends NativeExternalTextCompositionHandle {
    focus?: () => void;
    blur?: () => void;
    getCaretRect?: () => Promise<string | null> | string | null;
}

export interface NativeEditorViewProps {
    children?: ReactNode;
    style?: StyleProp<ViewStyle>;
    onLayout?: () => void;
    accessibilityLabel?: string;
    accessibilityHint?: string;
    editorId: string;
    placeholder?: string;
    editable: boolean;
    autoFocus: boolean;
    autoCapitalize?: RichTextEditorAutoCapitalize;
    autoCorrect?: boolean;
    keyboardType?: RichTextEditorKeyboardType;
    androidInputOptionsJson?: string;
    showToolbar: boolean;
    toolbarPlacement: RichTextEditorToolbarPlacement;
    heightBehavior: RichTextEditorHeightBehavior;
    allowImageResizing: boolean;
    imageLoadingPolicyJson?: string;
    themeJson?: string;
    addonsJson?: string;
    atomsJson?: string;
    toolbarItemsJson?: string;
    toolbarFrameJson?: string;
    remoteSelectionsJson?: string;
    editorUpdateJson?: string;
    editorUpdateResetJson?: string;
    editorUpdateEditorId?: string;
    editorUpdateRevision?: number;
    onEditorUpdate: (event: NativeSyntheticEvent<NativeUpdateEvent>) => void;
    onEditorError: (event: NativeSyntheticEvent<NativeErrorEvent>) => void;
    onExternalTextCompositionEnd: (
        event: NativeSyntheticEvent<NativeExternalTextCompositionEvent>
    ) => void;
    onSelectionChange: (event: NativeSyntheticEvent<NativeSelectionEvent>) => void;
    onFocusChange: (event: NativeSyntheticEvent<NativeFocusEvent>) => void;
    onContentHeightChange: (event: NativeSyntheticEvent<NativeContentHeightEvent>) => void;
    onAtomLayout: (event: NativeSyntheticEvent<NativeAtomLayoutEvent>) => void;
    onToolbarAction: (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => void;
    onAddonEvent: (event: NativeSyntheticEvent<NativeAddonEvent>) => void;
}

export interface NativeUpdateEvent {
    updateJson: string;
    editorId: string;
    documentRevision: string;
}

export interface NativeErrorEvent {
    editorId: string;
    error: unknown;
}

export interface NativeSelectionEvent {
    anchor: number;
    head: number;
    stateJson?: string;
    editorId: string;
    documentVersion?: string;
}

export interface NativeFocusEvent {
    isFocused: boolean;
    editorId: string;
}

export interface NativeContentHeightEvent {
    contentHeight: number;
    editorId: string;
}

export interface NativeAtomLayoutEvent {
    width: number;
    editorId: string;
    positions?: readonly NativeAtomPosition[];
    viewport?: { y: number; height: number };
}

export interface NativeAtomPosition {
    /** Native editor coordinates used for Fabric responder measurements. */
    hostX?: number;
    hostY?: number;
    height?: number;
    width?: number;
    key: string;
    x: number;
    y: number;
}

export interface NativeToolbarActionEvent {
    key: string;
    editorId: string;
    updateJson?: string;
    stateJson?: string;
    documentRevision?: string;
}

export interface NativeAddonEvent {
    eventJson: string;
    editorId: string;
}

export interface NativeEditorErrorBinding {
    readonly handle: NativeEditorDocumentHandle;
    readonly editorId: string;
    readonly generation: number;
    readonly mounted: boolean;
}

export interface ControlledValueDelivery {
    manager: ExternalTextCompositionManager;
    key: string | null;
    value: string | undefined;
    valueJSON: DocumentJSON | undefined;
}

export interface ExternalCompositionDisposalToken {
    cancelled: boolean;
}
