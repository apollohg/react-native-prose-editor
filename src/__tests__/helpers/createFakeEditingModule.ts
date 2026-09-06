import type { DocumentJSON } from '../../NativeEditorBridge';
import { createFakeRuntimeState } from './createFakeRuntimeState';
import {
    okRecord,
    parseV2RequestEnvelope,
    requestEnvelopeError,
    operationError,
    errRecord,
    exactV2U32,
    boundaryError,
    fakeDocumentIsEmpty,
    fakePositionEnvelopeScalar,
} from './nativeEditorV2FakeRecords';
import {
    fakeHtmlForDoc,
    fakeDocForHtml,
    cloneDoc,
    appendText,
    fakeScalarDocumentMap,
} from './nativeEditorV2FakeDocument';
import { installFakeDocument, moveFakeCursorAcrossEdit } from './nativeEditorV2FakeAwareness';

export function createFakeEditingModule(
    context: Pick<
        ReturnType<typeof createFakeRuntimeState>,
        | 'withSession'
        | 'stateJson'
        | 'admitReplacement'
        | 'applyReplacement'
        | 'revisionMismatchError'
        | 'queueDocumentUpdate'
        | 'pendingFor'
    >
) {
    const {
        withSession,
        stateJson,
        admitReplacement,
        applyReplacement,
        revisionMismatchError,
        queueDocumentUpdate,
        pendingFor,
    } = context;

    const module1: Record<string, jest.Mock> = {
        editorV2GetState: jest.fn((editorId: string) =>
            withSession(editorId, (session) => okRecord(stateJson(session)))
        ),

        editorV2GetDocumentJson: jest.fn((editorId: string) =>
            withSession(editorId, (session) => okRecord(JSON.stringify(session.doc)))
        ),

        editorV2GetDocumentHtml: jest.fn((editorId: string) =>
            withSession(editorId, (session) =>
                okRecord(JSON.stringify({ html: fakeHtmlForDoc(session.doc) }))
            )
        ),

        editorV2GetContentSnapshot: jest.fn((editorId: string) =>
            withSession(editorId, (session) =>
                okRecord(JSON.stringify({ html: fakeHtmlForDoc(session.doc), json: session.doc }))
            )
        ),

        editorV2ReplaceDocument: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, false);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                const rejected = admitReplacement(session);
                if (rejected) return rejected;
                const nextDoc =
                    request.setJson != null
                        ? (request.setJson as DocumentJSON)
                        : fakeDocForHtml(String(request.setHtml ?? ''));
                applyReplacement(session, nextDoc, String(request.history));
                return okRecord(
                    JSON.stringify({
                        changed: true,
                        documentRevision: String(session.documentRevision),
                    })
                );
            })
        ),

        editorV2ApplyInput: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (session.documentState === 'AwaitRemote') {
                    return operationError(
                        'ENGINE_NOT_READY',
                        'room document is awaiting the remote initial state'
                    );
                }
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return revisionMismatchError(session, request.baseDocumentRevision);
                }
                session.undoStack.push(cloneDoc(session.doc));
                session.redoStack = [];
                installFakeDocument(session, appendText(session.doc, String(request.text ?? '')));
                session.documentRevision += 1;
                session.documentOrigin = 'jsApi';
                session.stateRevision += 1;
                queueDocumentUpdate(session);
                return okRecord(
                    JSON.stringify({
                        type: 'transaction',
                        changed: true,
                        documentRevision: String(session.documentRevision),
                        stateRevision: String(session.stateRevision),
                        canUndo: session.undoStack.length > 0,
                        canRedo: session.redoStack.length > 0,
                    })
                );
            })
        ),

        editorV2ApplyCommand: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const injected = pendingFor(session.editorId).applyCommandErrors.shift();
                if (injected) return errRecord(injected);
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (session.documentState === 'AwaitRemote') {
                    return operationError(
                        'ENGINE_NOT_READY',
                        'room document is awaiting the remote initial state'
                    );
                }
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return revisionMismatchError(session, request.baseDocumentRevision);
                }
                const command = (request.command ?? {}) as Record<string, unknown>;
                const type = String(command.type ?? '');
                const stateOnlyOutcome = () => {
                    session.stateRevision += 1;
                    return okRecord(
                        JSON.stringify({
                            type: 'transaction',
                            changed: false,
                            documentRevision: String(session.documentRevision),
                            stateRevision: String(session.stateRevision),
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        })
                    );
                };
                const docChangeOutcome = (apply: () => void) => {
                    session.undoStack.push(cloneDoc(session.doc));
                    session.redoStack = [];
                    const before = cloneDoc(session.doc);
                    apply();
                    moveFakeCursorAcrossEdit(session, before, session.doc);
                    session.documentRevision += 1;
                    session.documentOrigin = 'jsApi';
                    session.stateRevision += 1;
                    queueDocumentUpdate(session);
                    return okRecord(
                        JSON.stringify({
                            type: 'transaction',
                            changed: true,
                            documentRevision: String(session.documentRevision),
                            stateRevision: String(session.stateRevision),
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        })
                    );
                };
                const appendBlocks = (blocks: unknown[]) => {
                    const next = cloneDoc(session.doc);
                    const content = Array.isArray(next.content) ? next.content : [];
                    content.push(...(blocks as Record<string, unknown>[]));
                    next.content = content;
                    session.doc = next;
                };
                const documentActiveState = () =>
                    fakeScalarDocumentMap(session.doc).activeStateAt(session.selection.head);
                const storedMarks = () => {
                    const documentState = documentActiveState();
                    return {
                        marks: {
                            ...(session.hasStoredMarks ? session.activeMarks : documentState.marks),
                        },
                        markAttrs: {
                            ...(session.hasStoredMarks
                                ? session.activeMarkAttrs
                                : documentState.markAttrs),
                        },
                    };
                };
                const storedNodes = () => ({
                    ...(session.hasStoredNodes ? session.activeNodes : documentActiveState().nodes),
                });
                switch (type) {
                    case 'toggleMark': {
                        const markType = String(command.markType ?? '');
                        const next = storedMarks();
                        if (next.marks[markType]) {
                            next.marks[markType] = false;
                            session.activeMarks = next.marks;
                            session.activeMarkAttrs = next.markAttrs;
                            delete session.activeMarkAttrs[markType];
                        } else {
                            next.marks[markType] = true;
                            session.activeMarks = next.marks;
                            session.activeMarkAttrs = next.markAttrs;
                        }
                        session.hasStoredMarks = true;
                        return stateOnlyOutcome();
                    }
                    case 'setMark': {
                        const markType = String(command.markType ?? '');
                        const next = storedMarks();
                        next.marks[markType] = true;
                        next.markAttrs[markType] = (command.attrs as Record<string, unknown>) ?? {};
                        session.activeMarks = next.marks;
                        session.activeMarkAttrs = next.markAttrs;
                        session.hasStoredMarks = true;
                        return stateOnlyOutcome();
                    }
                    case 'unsetMark': {
                        const markType = String(command.markType ?? '');
                        const next = storedMarks();
                        next.marks[markType] = false;
                        session.activeMarks = next.marks;
                        session.activeMarkAttrs = next.markAttrs;
                        delete session.activeMarkAttrs[markType];
                        session.hasStoredMarks = true;
                        return stateOnlyOutcome();
                    }
                    case 'toggleHeading': {
                        const level = String(command.level ?? '');
                        const key = `heading:${level}`;
                        const next = storedNodes();
                        next[key] = !next[key];
                        session.activeNodes = next;
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'toggleBlockquote': {
                        const next = storedNodes();
                        next.blockquote = !next.blockquote;
                        session.activeNodes = next;
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'wrapInList': {
                        const next = storedNodes();
                        next[String(command.listType ?? '')] = true;
                        session.activeNodes = next;
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'unwrapFromList': {
                        session.activeNodes = Object.fromEntries(
                            Object.keys(storedNodes()).map((key) => [key, false])
                        );
                        session.hasStoredNodes = true;
                        return stateOnlyOutcome();
                    }
                    case 'indentListItem':
                    case 'outdentListItem':
                        return stateOnlyOutcome();
                    case 'insertNode':
                        return docChangeOutcome(() =>
                            appendBlocks([
                                {
                                    type: 'paragraph',
                                    content: [
                                        {
                                            type: 'text',
                                            text: `[${String(command.nodeType ?? '')}]`,
                                        },
                                    ],
                                },
                            ])
                        );
                    case 'insertContentHtml':
                        return docChangeOutcome(() => {
                            const fragment = fakeDocForHtml(String(command.html ?? ''));
                            appendBlocks(Array.isArray(fragment.content) ? fragment.content : []);
                        });
                    case 'insertContentJson': {
                        const fragment = (command.json ?? {}) as DocumentJSON;
                        return docChangeOutcome(() =>
                            appendBlocks(Array.isArray(fragment.content) ? fragment.content : [])
                        );
                    }
                    case 'replaceSelectionText':
                        return docChangeOutcome(() => {
                            session.doc = appendText(session.doc, String(command.text ?? ''));
                        });
                    default:
                        return okRecord(JSON.stringify({ type: 'notApplicable' }));
                }
            })
        ),

        editorV2RenderUpdate: jest.fn(
            (editorId: string, mirrorAnchor: unknown, mirrorHead: unknown) =>
                withSession(editorId, (session) => {
                    if (
                        (mirrorAnchor == null) !== (mirrorHead == null) ||
                        (mirrorAnchor != null && exactV2U32(mirrorAnchor) == null) ||
                        (mirrorHead != null && exactV2U32(mirrorHead) == null)
                    ) {
                        return boundaryError(
                            'CONFIG_INVALID',
                            'invalid render mirror scalar offsets'
                        );
                    }
                    if (session.documentState === 'AwaitRemote') {
                        return operationError(
                            'ENGINE_NOT_READY',
                            'room document is awaiting the remote initial state'
                        );
                    }
                    const blocks = (
                        Array.isArray(session.doc.content) ? session.doc.content : []
                    ).map((block) => {
                        const inline = Array.isArray(block?.content) ? block.content : [];
                        const text = inline
                            .map((node) => (typeof node?.text === 'string' ? node.text : ''))
                            .join('');
                        const nodeType = String(block?.type ?? 'paragraph');
                        return [
                            { type: 'blockStart', nodeType, depth: 0 },
                            ...(text.length > 0
                                ? [{ type: 'textRun', text, marks: [] as string[] }]
                                : []),
                            { type: 'blockEnd' },
                        ];
                    });
                    const scalarMap = fakeScalarDocumentMap(session.doc);
                    const selection =
                        mirrorAnchor != null && mirrorHead != null
                            ? {
                                  anchor: scalarMap.scalarToDocument(exactV2U32(mirrorAnchor)!),
                                  head: scalarMap.scalarToDocument(exactV2U32(mirrorHead)!),
                              }
                            : session.selection;
                    const documentActiveState = scalarMap.activeStateAt(selection.head);
                    const usesStoredState =
                        mirrorAnchor == null &&
                        mirrorHead == null &&
                        selection.anchor === selection.head;
                    const marks = { ...documentActiveState.marks };
                    const markAttrs = { ...documentActiveState.markAttrs };
                    const nodes = { ...documentActiveState.nodes };
                    if (usesStoredState && session.hasStoredMarks) {
                        for (const [markType, active] of Object.entries(session.activeMarks)) {
                            marks[markType] = active;
                            if (!active) delete markAttrs[markType];
                        }
                        for (const [markType, attrs] of Object.entries(session.activeMarkAttrs)) {
                            if (marks[markType] && Object.keys(attrs).length > 0) {
                                markAttrs[markType] = { ...attrs };
                            }
                        }
                    }
                    if (usesStoredState && session.hasStoredNodes) {
                        Object.assign(nodes, session.activeNodes);
                    }
                    const update: Record<string, unknown> = {
                        renderBlocks: blocks,
                        renderPatch: null,
                        activeState: {
                            marks,
                            markAttrs,
                            nodes,
                            commands: {},
                            allowedMarks: ['bold', 'italic', 'underline', 'strike', 'link'],
                            insertableNodes: ['image', 'horizontalRule', 'hardBreak'],
                        },
                        historyState: {
                            canUndo: session.undoStack.length > 0,
                            canRedo: session.redoStack.length > 0,
                        },
                        documentVersion: String(session.documentRevision),
                        stateRevision: String(session.stateRevision),
                        scalarLength: scalarMap.scalarLength,
                        documentIsEmpty: fakeDocumentIsEmpty(session.doc),
                    };
                    if (mirrorAnchor != null && mirrorHead != null) {
                        update.selection = {
                            type: 'text',
                            anchor: selection.anchor,
                            head: selection.head,
                            anchorScalar: scalarMap.documentToScalar(selection.anchor),
                            headScalar: scalarMap.documentToScalar(selection.head),
                        };
                    } else {
                        update.selection = {
                            type: 'text',
                            anchor: selection.anchor,
                            head: selection.head,
                            anchorScalar: scalarMap.documentToScalar(selection.anchor),
                            headScalar: scalarMap.documentToScalar(selection.head),
                        };
                    }
                    return okRecord(JSON.stringify(update));
                })
        ),

        editorV2ApplyLocalApi: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const injected = pendingFor(session.editorId).applyLocalApiErrors.shift();
                if (injected) return errRecord(injected);
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return revisionMismatchError(session, request.baseDocumentRevision);
                }
                const rejected = admitReplacement(session);
                if (rejected) return rejected;
                const nextDoc =
                    request.setJson != null
                        ? (request.setJson as DocumentJSON)
                        : fakeDocForHtml(String(request.setHtml ?? ''));
                applyReplacement(session, nextDoc, String(request.history));
                return okRecord(
                    JSON.stringify({
                        type: 'replacement',
                        changed: true,
                        documentRevision: String(session.documentRevision),
                    })
                );
            })
        ),

        editorV2SetSelection: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, true);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                if (request.baseDocumentRevision !== String(session.documentRevision)) {
                    return revisionMismatchError(session, request.baseDocumentRevision);
                }
                const selection = request.selection as Record<string, unknown> | undefined;
                if (selection?.type !== 'text') {
                    return boundaryError('CONFIG_INVALID', 'unsupported v2 selection envelope');
                }
                const anchor = fakePositionEnvelopeScalar(selection.anchor);
                const head = fakePositionEnvelopeScalar(selection.head);
                if (anchor == null || head == null) {
                    return boundaryError(
                        'CONFIG_INVALID',
                        'selection anchor/head must be scalar position envelopes'
                    );
                }
                const scalarMap = fakeScalarDocumentMap(session.doc);
                session.selection = {
                    anchor: scalarMap.clampDocumentOffset(scalarMap.scalarToDocument(anchor)),
                    head: scalarMap.clampDocumentOffset(scalarMap.scalarToDocument(head)),
                };
                session.stateRevision += 1;
                return okRecord(JSON.stringify({ type: 'notApplicable' }));
            })
        ),

        editorV2Undo: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, false);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                const previous = session.undoStack.pop();
                if (!previous) return okRecord(JSON.stringify({ changed: false }));
                session.redoStack.push(cloneDoc(session.doc));
                installFakeDocument(session, previous);
                session.documentRevision += 1;
                session.documentOrigin = 'history';
                queueDocumentUpdate(session);
                return okRecord(JSON.stringify({ changed: true }));
            })
        ),

        editorV2Redo: jest.fn((editorId: string, requestJson: string) =>
            withSession(editorId, (session) => {
                const request = parseV2RequestEnvelope(requestJson, false);
                const envelopeError = requestEnvelopeError(request);
                if (envelopeError) return envelopeError;
                const next = session.redoStack.pop();
                if (!next) return okRecord(JSON.stringify({ changed: false }));
                session.undoStack.push(cloneDoc(session.doc));
                installFakeDocument(session, next);
                session.documentRevision += 1;
                session.documentOrigin = 'history';
                queueDocumentUpdate(session);
                return okRecord(JSON.stringify({ changed: true }));
            })
        ),
    };
    return { module1 };
}
