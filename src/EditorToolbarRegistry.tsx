import React, { createContext, useEffect, useSyncExternalStore, useState } from 'react';
import type { MentionSuggestion } from './addons';
import type { EditorMentionTheme } from './EditorTheme';

export interface EditorToolbarFrame {
    x: number;
    y: number;
    width: number;
    height: number;
}

export type EditorToolbarFrameListener = () => void;

export interface EditorToolbarFrameRegistration {
    ownerId: number | null;
    frame: EditorToolbarFrame;
}

export const editorToolbarFrames = new Map<number, EditorToolbarFrameRegistration>();

export const editorToolbarFrameListeners = new Set<EditorToolbarFrameListener>();

export const editorToolbarMentionStateListeners = new Set<EditorToolbarFrameListener>();

export let nextEditorToolbarRegistrationId = 1;

export let activeEditorToolbarInteractions = 0;

export let editorToolbarFocusPreserveUntil = 0;

export let activeEditorToolbarFrameOwnerId: number | null = null;

export function allocateEditorToolbarRegistrationId(): number {
    return nextEditorToolbarRegistrationId++;
}

export const EditorToolbarFrameOwnerContext = createContext<number | null>(null);

export function EditorToolbarFrameOwnerProvider({
    ownerId,
    children,
}: {
    ownerId: number;
    children: React.ReactNode;
}) {
    return (
        <EditorToolbarFrameOwnerContext.Provider value={ownerId}>
            {children}
        </EditorToolbarFrameOwnerContext.Provider>
    );
}

export interface EditorToolbarMentionState {
    ownerId: number;
    trigger: string;
    suggestions: readonly MentionSuggestion[];
    theme?: EditorMentionTheme;
    suggestionThemes?: Readonly<Record<string, EditorMentionTheme | undefined>>;
    onSelectSuggestion: (suggestion: MentionSuggestion) => void;
}

export let editorToolbarMentionState: EditorToolbarMentionState | null = null;

export const EDITOR_TOOLBAR_FOCUS_PRESERVE_MS = 750;

export function areToolbarFramesEqual(
    left: EditorToolbarFrame | undefined,
    right: EditorToolbarFrame | undefined
): boolean {
    return (
        left?.x === right?.x &&
        left?.y === right?.y &&
        left?.width === right?.width &&
        left?.height === right?.height
    );
}

export function areToolbarFrameListsEqual(
    left: readonly EditorToolbarFrame[],
    right: readonly EditorToolbarFrame[]
): boolean {
    if (left.length !== right.length) {
        return false;
    }
    return left.every((frame, index) => areToolbarFramesEqual(frame, right[index]));
}

export function notifyEditorToolbarFrameListeners() {
    editorToolbarFrameListeners.forEach((listener) => listener());
}

export function notifyEditorToolbarMentionStateListeners() {
    editorToolbarMentionStateListeners.forEach((listener) => listener());
}

export function getEditorToolbarFramesSnapshot(ownerId: number): EditorToolbarFrame[] {
    return Array.from(editorToolbarFrames.values())
        .filter(
            (registration) =>
                registration.ownerId === ownerId ||
                (registration.ownerId == null && activeEditorToolbarFrameOwnerId === ownerId)
        )
        .map((registration) => registration.frame);
}

export function subscribeEditorToolbarMentionState(listener: EditorToolbarFrameListener) {
    editorToolbarMentionStateListeners.add(listener);
    return () => {
        editorToolbarMentionStateListeners.delete(listener);
    };
}

export function getEditorToolbarMentionStateSnapshot(): EditorToolbarMentionState | null {
    return editorToolbarMentionState;
}

export function useEditorToolbarMentionState(): EditorToolbarMentionState | null {
    return useSyncExternalStore(
        subscribeEditorToolbarMentionState,
        getEditorToolbarMentionStateSnapshot,
        getEditorToolbarMentionStateSnapshot
    );
}

export function registerEditorToolbarFrame(
    id: number,
    frame: EditorToolbarFrame | null,
    ownerId: number | null
) {
    if (frame == null || frame.width <= 0 || frame.height <= 0) {
        if (editorToolbarFrames.delete(id)) {
            notifyEditorToolbarFrameListeners();
        }
        return;
    }

    const currentRegistration = editorToolbarFrames.get(id);
    if (
        currentRegistration?.ownerId === ownerId &&
        areToolbarFramesEqual(currentRegistration.frame, frame)
    ) {
        return;
    }

    editorToolbarFrames.set(id, { ownerId, frame });
    notifyEditorToolbarFrameListeners();
}

export function unregisterEditorToolbarFrame(id: number) {
    if (editorToolbarFrames.delete(id)) {
        notifyEditorToolbarFrameListeners();
    }
}

export function preserveEditorToolbarFocusForNextBlur() {
    editorToolbarFocusPreserveUntil = Date.now() + EDITOR_TOOLBAR_FOCUS_PRESERVE_MS;
}

export function beginEditorToolbarInteraction() {
    activeEditorToolbarInteractions += 1;
    preserveEditorToolbarFocusForNextBlur();
}

export function endEditorToolbarInteraction() {
    activeEditorToolbarInteractions = Math.max(0, activeEditorToolbarInteractions - 1);
    preserveEditorToolbarFocusForNextBlur();
}

export function isEditorToolbarFocusPreservationActive(): boolean {
    return activeEditorToolbarInteractions > 0 || Date.now() <= editorToolbarFocusPreserveUntil;
}

export function setActiveEditorToolbarFrameOwnerForEditor(ownerId: number, isActive: boolean) {
    const nextOwnerId = isActive
        ? ownerId
        : activeEditorToolbarFrameOwnerId === ownerId
          ? null
          : activeEditorToolbarFrameOwnerId;
    if (activeEditorToolbarFrameOwnerId === nextOwnerId) {
        return;
    }
    activeEditorToolbarFrameOwnerId = nextOwnerId;
    notifyEditorToolbarFrameListeners();
}

export function useEditorToolbarFrames(ownerId: number): readonly EditorToolbarFrame[] {
    const [frames, setFrames] = useState<EditorToolbarFrame[]>(() =>
        getEditorToolbarFramesSnapshot(ownerId)
    );

    useEffect(() => {
        const listener = () => {
            const nextFrames = getEditorToolbarFramesSnapshot(ownerId);
            setFrames((currentFrames) =>
                areToolbarFrameListsEqual(currentFrames, nextFrames) ? currentFrames : nextFrames
            );
        };
        editorToolbarFrameListeners.add(listener);
        listener();
        return () => {
            editorToolbarFrameListeners.delete(listener);
        };
    }, [ownerId]);

    return frames;
}

export function setEditorToolbarMentionState(
    ownerId: number,
    state: Omit<EditorToolbarMentionState, 'ownerId'> | null
) {
    if (state == null) {
        if (editorToolbarMentionState?.ownerId !== ownerId) {
            return;
        }
        editorToolbarMentionState = null;
        notifyEditorToolbarMentionStateListeners();
        return;
    }

    editorToolbarMentionState = {
        ownerId,
        ...state,
    };
    notifyEditorToolbarMentionStateListeners();
}

export function _setEditorToolbarFrameForTests(
    id: number,
    frame: EditorToolbarFrame | null,
    ownerId: number | null = null
) {
    registerEditorToolbarFrame(id, frame, ownerId);
}

export function _resetEditorToolbarFrameRegistryForTests() {
    editorToolbarFrames.clear();
    editorToolbarMentionState = null;
    activeEditorToolbarInteractions = 0;
    editorToolbarFocusPreserveUntil = 0;
    activeEditorToolbarFrameOwnerId = null;
    notifyEditorToolbarFrameListeners();
    notifyEditorToolbarMentionStateListeners();
}

export function _beginEditorToolbarInteractionForTests() {
    beginEditorToolbarInteraction();
}

export function _endEditorToolbarInteractionForTests() {
    endEditorToolbarInteraction();
}

export const BUTTON_HIT = 44;
