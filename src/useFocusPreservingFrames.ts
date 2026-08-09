import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { Keyboard } from 'react-native';

import type { EditorToolbarFrame } from './EditorToolbar';

export interface NativeRichTextEditorFocusPreservingElement {
    measureInWindow(callback: (x: number, y: number, width: number, height: number) => void): void;
}

export type NativeRichTextEditorFocusPreservingRef =
    RefObject<NativeRichTextEditorFocusPreservingElement | null>;

export type NativeRichTextEditorFocusPreservingRefs =
    | NativeRichTextEditorFocusPreservingRef
    | readonly NativeRichTextEditorFocusPreservingRef[];

const EMPTY_FRAMES: readonly EditorToolbarFrame[] = [];

interface MeasuredFocusPreservingFrame {
    target: NativeRichTextEditorFocusPreservingElement;
    frame: EditorToolbarFrame;
}

function sameRefs(
    left: readonly NativeRichTextEditorFocusPreservingRef[],
    right: readonly NativeRichTextEditorFocusPreservingRef[]
): boolean {
    return left.length === right.length && left.every((ref, index) => ref === right[index]);
}

function sameFrames(
    left: readonly EditorToolbarFrame[],
    right: readonly EditorToolbarFrame[]
): boolean {
    return (
        left.length === right.length &&
        left.every((frame, index) => {
            const candidate = right[index];
            return (
                frame.x === candidate.x &&
                frame.y === candidate.y &&
                frame.width === candidate.width &&
                frame.height === candidate.height
            );
        })
    );
}

function validFrame(
    x: number,
    y: number,
    width: number,
    height: number
): EditorToolbarFrame | null {
    if (![x, y, width, height].every(Number.isFinite) || width <= 0 || height <= 0) {
        return null;
    }
    return { x, y, width, height };
}

export function useFocusPreservingFrames(
    refs: NativeRichTextEditorFocusPreservingRefs | undefined,
    enabled: boolean
): {
    frames: readonly EditorToolbarFrame[];
    refresh: () => void;
} {
    const normalizedRefs: readonly NativeRichTextEditorFocusPreservingRef[] =
        refs == null
            ? []
            : Array.isArray(refs)
              ? refs
              : [refs as NativeRichTextEditorFocusPreservingRef];
    const stableRefs = useRef<readonly NativeRichTextEditorFocusPreservingRef[]>(normalizedRefs);
    if (!sameRefs(stableRefs.current, normalizedRefs)) {
        stableRefs.current = normalizedRefs;
    }
    const currentRefs = stableRefs.current;

    const generation = useRef(0);
    const framesByRef = useRef(
        new Map<NativeRichTextEditorFocusPreservingRef, MeasuredFocusPreservingFrame>()
    );
    const [frames, setFrames] = useState<readonly EditorToolbarFrame[]>(EMPTY_FRAMES);

    const publish = useCallback(() => {
        const nextFrames = currentRefs.flatMap((ref) => {
            const measurement = framesByRef.current.get(ref);
            return measurement == null ? [] : [measurement.frame];
        });
        setFrames((current) => (sameFrames(current, nextFrames) ? current : nextFrames));
    }, [currentRefs]);

    const refresh = useCallback(() => {
        const callbackGeneration = ++generation.current;
        if (!enabled) {
            framesByRef.current.clear();
            setFrames((current) => (current.length === 0 ? current : EMPTY_FRAMES));
            return;
        }

        const admittedRefs = new Set(currentRefs);
        for (const ref of framesByRef.current.keys()) {
            const measurement = framesByRef.current.get(ref);
            if (
                !admittedRefs.has(ref) ||
                ref.current == null ||
                measurement?.target !== ref.current
            ) {
                framesByRef.current.delete(ref);
            }
        }
        publish();

        currentRefs.forEach((ref) => {
            const target = ref.current;
            if (target == null || typeof target.measureInWindow !== 'function') {
                framesByRef.current.delete(ref);
                publish();
                return;
            }

            try {
                target.measureInWindow((x, y, width, height) => {
                    // An older measurement must not restore a removed or replaced target.
                    if (generation.current !== callbackGeneration || ref.current !== target) return;
                    const frame = validFrame(x, y, width, height);
                    if (frame == null) {
                        framesByRef.current.delete(ref);
                    } else {
                        framesByRef.current.set(ref, { target, frame });
                    }
                    publish();
                });
            } catch {
                if (generation.current !== callbackGeneration) return;
                framesByRef.current.delete(ref);
                publish();
            }
        });
    }, [currentRefs, enabled, publish]);

    useLayoutEffect(() => {
        refresh();
    });

    useEffect(() => {
        if (!enabled) return;
        const subscriptions = [
            Keyboard.addListener('keyboardDidShow', refresh),
            Keyboard.addListener('keyboardDidHide', refresh),
            Keyboard.addListener('keyboardDidChangeFrame', refresh),
        ];
        return () => subscriptions.forEach((subscription) => subscription.remove());
    }, [enabled, refresh]);

    useEffect(
        () => () => {
            generation.current += 1;
            framesByRef.current.clear();
        },
        []
    );

    return { frames, refresh };
}
