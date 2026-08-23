const mockNativeModule: Record<string, jest.Mock> = {};

jest.mock('expo-modules-core', () => ({
    requireNativeModule: () => mockNativeModule,
    requireNativeViewManager: () => 'NativeEditorView',
}));

import React from 'react';
import { act, render } from '@testing-library/react-native';

import {
    _resetNativeModuleCache,
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
} from '../NativeEditorBridge';
import { useNativeEditorDocument } from '../useNativeEditor';
import { createFakeNativeEditorV2Runtime, fakeDocForText } from './helpers/nativeEditorV2Fake';

describe('useNativeEditorDocument', () => {
    beforeEach(() => {
        _resetNativeModuleCache();
        const runtime = createFakeNativeEditorV2Runtime();
        for (const key of Object.keys(mockNativeModule)) {
            delete mockNativeModule[key];
        }
        Object.assign(mockNativeModule, runtime.module);
    });

    it('never exposes A state while rebinding to B and ignores a late A refresh', () => {
        const handleA = createNativeEditorDocumentHandle({
            initialization: { type: 'localJson', json: fakeDocForText('alpha') },
        });
        const handleB = createNativeEditorDocumentHandle({
            initialization: { type: 'localJson', json: fakeDocForText('beta') },
        });
        handleA.bridge.replaceDocument({
            setJson: fakeDocForText('alpha changed'),
            history: 'undoableBoundary',
        });
        const historyChanges = jest.fn();
        const contentChanges = jest.fn();
        const renders: Array<{
            editorId: string;
            isReady: boolean;
            documentRevision: string | null;
            historyState: { canUndo: boolean; canRedo: boolean };
            canUndo: boolean;
            content: string;
        }> = [];
        let currentDocument: ReturnType<typeof useNativeEditorDocument> | null = null;
        let refreshA: (() => void) | null = null;

        function SnapshotObserver({ handle }: { handle: NativeEditorDocumentHandle }) {
            const document = useNativeEditorDocument({
                handle,
                onContentChange: contentChanges,
                onHistoryStateChange: historyChanges,
            });
            currentDocument = document;
            if (handle === handleA) {
                refreshA = document.refresh;
            }
            renders.push({
                editorId: handle.editorId,
                isReady: document.isReady,
                documentRevision: document.documentRevision,
                historyState: document.historyState,
                canUndo: document.canUndo(),
                content: document.getContent(),
            });
            return null;
        }

        const observer = render(<SnapshotObserver handle={handleA} />);

        expect(currentDocument!.isReady).toBe(true);
        expect(currentDocument!.historyState).toEqual({ canUndo: true, canRedo: false });

        observer.rerender(<SnapshotObserver handle={handleB} />);
        const firstBRender = renders.find((rendered) => rendered.editorId === handleB.editorId);
        expect(firstBRender).toEqual({
            editorId: handleB.editorId,
            isReady: true,
            documentRevision: handleB.bridge.getState().documentRevision,
            historyState: { canUndo: false, canRedo: false },
            canUndo: false,
            content: '<p>beta</p>',
        });

        handleA.bridge.replaceDocument({
            setJson: fakeDocForText('alpha late'),
            history: 'undoableBoundary',
        });
        act(() => refreshA!());

        expect(contentChanges).not.toHaveBeenCalled();
        expect(historyChanges).not.toHaveBeenLastCalledWith({ canUndo: true, canRedo: false });
        expect(currentDocument).toMatchObject({
            isReady: true,
            documentRevision: handleB.bridge.getState().documentRevision,
            historyState: { canUndo: false, canRedo: false },
        });
        expect(currentDocument!.getContent()).toBe('<p>beta</p>');
        handleA.destroy();
        handleB.destroy();
    });
});
