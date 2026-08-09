import { Keyboard } from 'react-native';
import { act, renderHook } from '@testing-library/react-native';

import {
    useFocusPreservingFrames,
    type NativeRichTextEditorFocusPreservingElement,
    type NativeRichTextEditorFocusPreservingRef,
    type NativeRichTextEditorFocusPreservingRefs,
} from '../useFocusPreservingFrames';

function element(
    x: number,
    y: number,
    width: number,
    height: number
): NativeRichTextEditorFocusPreservingElement {
    return {
        measureInWindow: (callback) => callback(x, y, width, height),
    };
}

describe('useFocusPreservingFrames', () => {
    afterEach(() => {
        jest.restoreAllMocks();
    });

    it('measures arrays in caller order', () => {
        const first: NativeRichTextEditorFocusPreservingRef = {
            current: element(10, 20, 30, 40),
        };
        const second: NativeRichTextEditorFocusPreservingRef = {
            current: element(50, 60, 70, 80),
        };
        const { result } = renderHook(() => useFocusPreservingFrames([first, second], true));

        expect(result.current.frames).toEqual([
            { x: 10, y: 20, width: 30, height: 40 },
            { x: 50, y: 60, width: 70, height: 80 },
        ]);
    });

    it('ignores detached, non-measurable, invalid, and zero-sized targets', () => {
        const detached: NativeRichTextEditorFocusPreservingRef = { current: null };
        const nonMeasurable = { current: {} } as unknown as NativeRichTextEditorFocusPreservingRef;
        const invalid: NativeRichTextEditorFocusPreservingRef = {
            current: element(Number.NaN, 20, 30, 40),
        };
        const empty: NativeRichTextEditorFocusPreservingRef = {
            current: element(10, 20, 0, 40),
        };
        const { result } = renderHook(() =>
            useFocusPreservingFrames([detached, nonMeasurable, invalid, empty], true)
        );

        expect(result.current.frames).toEqual([]);
    });

    it('remeasures when the same ref points to a replacement element', () => {
        const ref: NativeRichTextEditorFocusPreservingRef = {
            current: element(10, 20, 30, 40),
        };
        const { result, rerender } = renderHook(() => useFocusPreservingFrames(ref, true));

        expect(result.current.frames).toEqual([{ x: 10, y: 20, width: 30, height: 40 }]);

        ref.current = element(50, 60, 70, 80);
        rerender({});

        expect(result.current.frames).toEqual([{ x: 50, y: 60, width: 70, height: 80 }]);
    });

    it('removes the old frame while a replacement element is being measured', () => {
        let finishReplacement:
            | ((x: number, y: number, width: number, height: number) => void)
            | undefined;
        const ref: NativeRichTextEditorFocusPreservingRef = {
            current: element(10, 20, 30, 40),
        };
        const { result, rerender } = renderHook(() => useFocusPreservingFrames(ref, true));

        ref.current = {
            measureInWindow: (callback) => {
                finishReplacement = callback;
            },
        };
        rerender({});

        expect(result.current.frames).toEqual([]);

        act(() => finishReplacement?.(50, 60, 70, 80));
        expect(result.current.frames).toEqual([{ x: 50, y: 60, width: 70, height: 80 }]);
    });

    it('rejects a callback from a replaced ref generation', () => {
        let finishFirst:
            | ((x: number, y: number, width: number, height: number) => void)
            | undefined;
        const first: NativeRichTextEditorFocusPreservingRef = {
            current: {
                measureInWindow: (callback) => {
                    finishFirst = callback;
                },
            },
        };
        const second: NativeRichTextEditorFocusPreservingRef = {
            current: element(50, 60, 70, 80),
        };
        const { result, rerender } = renderHook(
            ({ refs }: { refs: NativeRichTextEditorFocusPreservingRefs }) =>
                useFocusPreservingFrames(refs, true),
            { initialProps: { refs: first as NativeRichTextEditorFocusPreservingRefs } }
        );

        rerender({ refs: second });
        if (finishFirst == null) throw new Error('first measurement was not requested');
        act(() => finishFirst(10, 20, 30, 40));

        expect(result.current.frames).toEqual([{ x: 50, y: 60, width: 70, height: 80 }]);
    });

    it('clears frames when disabled and refreshes after keyboard movement', () => {
        let x = 10;
        const target: NativeRichTextEditorFocusPreservingRef = {
            current: {
                measureInWindow: (callback) => callback(x, 20, 30, 40),
            },
        };
        const listeners = new Map<string, () => void>();
        jest.spyOn(Keyboard, 'addListener').mockImplementation((eventName, listener) => {
            listeners.set(eventName, () => listener({} as never));
            return { remove: jest.fn() } as never;
        });
        const { result, rerender } = renderHook(
            ({ enabled }: { enabled: boolean }) => useFocusPreservingFrames(target, enabled),
            { initialProps: { enabled: true } }
        );

        x = 90;
        act(() => listeners.get('keyboardDidChangeFrame')?.());
        expect(result.current.frames).toEqual([{ x: 90, y: 20, width: 30, height: 40 }]);

        rerender({ enabled: false });
        expect(result.current.frames).toEqual([]);
    });
});
