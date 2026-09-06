import { useCallback, useEffect, useMemo } from 'react';
import {
    buildMentionFragmentJson,
    normalizeEditorAddons,
    type EditorAddonEvent,
    type MentionQueryChangeEvent,
    type MentionSelectionAttrsEvent,
    type MentionSuggestion,
} from './addons';
import { type EditorMentionTheme } from './EditorTheme';
import { validEditorMentionTheme, type NativeEditorV2PositionAffinity } from './NativeEditorBridge';
import { type NativeSyntheticEvent } from 'react-native';
import { setEditorToolbarMentionState } from './EditorToolbar';
import { useRichTextEditorState } from './useRichTextEditorState';
import { useRichTextEditorCommands } from './useRichTextEditorCommands';
import { useRichTextEditorUpdates } from './useRichTextEditorUpdates';
import { useRichTextEditorEvents } from './useRichTextEditorEvents';
import {
    isRecord,
    isPositionInvalidError,
    isRevisionMismatchError,
    mergeMentionSuggestionTheme,
} from './RichTextEditorSerialization';
import { type NativeAddonEvent } from './RichTextEditorNativeTypes';

export function useRichTextEditorMentions(
    context: Pick<
        ReturnType<typeof useRichTextEditorState>,
        | 'addonsRef'
        | 'bridge'
        | 'documentHandle'
        | 'currentPushedUpdateEditorIdRef'
        | 'documentDescriptor'
        | 'latestRevisionRef'
        | 'document'
        | 'setMentionQuery'
        | 'mentionQuery'
        | 'addons'
        | 'editable'
        | 'isFocused'
        | 'activeStateRef'
        | 'toolbarFrameOwnerId'
    > &
        Pick<ReturnType<typeof useRichTextEditorCommands>, 'editableRef'> &
        Pick<ReturnType<typeof useRichTextEditorUpdates>, 'afterLocalEngineMutation'> &
        Pick<ReturnType<typeof useRichTextEditorEvents>, 'isForThisEditor'>
) {
    const {
        addonsRef,
        editableRef,
        bridge,
        documentHandle,
        currentPushedUpdateEditorIdRef,
        documentDescriptor,
        latestRevisionRef,
        document,
        afterLocalEngineMutation,
        isForThisEditor,
        setMentionQuery,
        mentionQuery,
        addons,
        editable,
        isFocused,
        activeStateRef,
        toolbarFrameOwnerId,
    } = context;

    const resolveMentionSelectionAttrs = useCallback(
        (selectionEvent: MentionSelectionAttrsEvent): Record<string, unknown> => {
            let resolvedAttrs: Record<string, unknown> | null | undefined;
            try {
                resolvedAttrs =
                    addonsRef.current?.mentions?.resolveSelectionAttrs?.(selectionEvent);
            } catch (error) {
                if (__DEV__) {
                    console.error(
                        'NativeRichTextEditor: mentions.resolveSelectionAttrs threw',
                        error
                    );
                }
            }
            return isRecord(resolvedAttrs)
                ? { ...selectionEvent.attrs, ...resolvedAttrs }
                : selectionEvent.attrs;
        },
        []
    );

    const resolveMentionTheme = useCallback(
        (selectionEvent: MentionSelectionAttrsEvent): EditorMentionTheme | undefined => {
            let resolvedTheme: unknown;
            try {
                resolvedTheme = addonsRef.current?.mentions?.resolveTheme?.(selectionEvent);
            } catch (error) {
                if (__DEV__) {
                    console.error('NativeRichTextEditor: mentions.resolveTheme threw', error);
                }
            }
            if (resolvedTheme === undefined || resolvedTheme === null) return undefined;
            // A rejected theme is dropped rather than written into the
            // document: every later renderUpdate revalidates it, so one bad
            // value would make the content permanently unrenderable.
            if (!validEditorMentionTheme(resolvedTheme)) {
                if (__DEV__) {
                    console.error(
                        'NativeRichTextEditor: mentions.resolveTheme did not return an EditorMentionTheme; ignoring it',
                        resolvedTheme
                    );
                }
                return undefined;
            }
            return resolvedTheme;
        },
        []
    );

    const resolveMentionInsertionAttrs = useCallback(
        (selectionEvent: MentionSelectionAttrsEvent): Record<string, unknown> => {
            const attrs = resolveMentionSelectionAttrs(selectionEvent);
            const resolvedTheme = resolveMentionTheme({ ...selectionEvent, attrs });
            return resolvedTheme != null ? { ...attrs, mentionTheme: resolvedTheme } : attrs;
        },
        [resolveMentionSelectionAttrs, resolveMentionTheme]
    );

    const insertMentionSuggestion = useCallback(
        (request: {
            trigger: string;
            suggestion: MentionSuggestion;
            attrs: Record<string, unknown>;
            range: { anchor: number; head: number };
            documentVersion?: string;
        }) => {
            const mentions = addonsRef.current?.mentions;
            if (!mentions || !editableRef.current) return;

            const snapshot = bridge.renderUpdate({
                anchor: request.range.anchor,
                head: request.range.head,
            });
            if (
                snapshot.selection.type !== 'text' ||
                (request.documentVersion != null &&
                    request.documentVersion !== snapshot.documentVersion)
            ) {
                return;
            }
            const markAttrs = Object.fromEntries(
                Object.entries(snapshot.activeState.markAttrs).map(([mark, attrs]) => [
                    mark,
                    { ...attrs },
                ])
            );
            const callbackEvent: MentionSelectionAttrsEvent = {
                trigger: request.trigger,
                suggestion: request.suggestion,
                attrs: request.attrs,
                markAttrs,
                range: request.range,
                documentVersion: snapshot.documentVersion,
            };
            const attrs = resolveMentionInsertionAttrs(callbackEvent);
            // Selection envelopes address scalars, not document positions.
            const anchorScalar = snapshot.selection.anchorScalar;
            const headScalar = snapshot.selection.headScalar;
            if (
                documentHandle.isDestroyed ||
                currentPushedUpdateEditorIdRef.current !== documentHandle.editorId ||
                anchorScalar == null ||
                headScalar == null
            ) {
                return;
            }

            // Affinity policy mirrors the native adapters and the engine's
            // own cursor resolution: a collapsed caret prefers After with a
            // deterministic Before fallback at text-boundary positions; a
            // range uses Before. The fallback changes only the stickiness
            // of the SAME position — it is not a guessed-position retry.
            const collapsed = anchorScalar === headScalar;
            const syncSelection = (affinity: NativeEditorV2PositionAffinity) =>
                bridge.setSelection({
                    baseDocumentRevision: snapshot.documentVersion,
                    selection: {
                        type: 'text',
                        anchor: { offset: anchorScalar, kind: 'scalar', affinity },
                        head: { offset: headScalar, kind: 'scalar', affinity },
                    },
                });

            try {
                try {
                    syncSelection(collapsed ? 'after' : 'before');
                } catch (error) {
                    if (!collapsed || !isPositionInvalidError(error)) throw error;
                    syncSelection('before');
                }
                const outcome = bridge.applyCommand({
                    baseDocumentRevision: snapshot.documentVersion,
                    command: {
                        type: 'insertContentJson',
                        json: buildMentionFragmentJson(attrs, documentDescriptor, {
                            trailingSpace: true,
                        }),
                    },
                });
                if (outcome.type !== 'transaction' || !outcome.changed) return;
                latestRevisionRef.current = outcome.documentRevision;
            } catch (error) {
                if (isRevisionMismatchError(error)) {
                    document.refresh();
                    return;
                }
                throw error;
            }

            afterLocalEngineMutation();
            mentions.onSelect?.({
                trigger: request.trigger,
                suggestion: request.suggestion,
                attrs,
                documentVersion: snapshot.documentVersion,
            });
        },
        [
            afterLocalEngineMutation,
            bridge,
            document,
            documentDescriptor,
            documentHandle,
            resolveMentionInsertionAttrs,
        ]
    );

    const handleAddonEvent = useCallback(
        (event: NativeSyntheticEvent<NativeAddonEvent>) => {
            if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
            let parsed: EditorAddonEvent;
            try {
                const value = JSON.parse(event.nativeEvent.eventJson) as unknown;
                if (!isRecord(value) || typeof value.type !== 'string') return;
                parsed = value as unknown as EditorAddonEvent;
            } catch {
                return;
            }

            const mentions = addonsRef.current?.mentions;
            if (!mentions) return;
            const documentVersion =
                typeof parsed.documentVersion === 'string' ? parsed.documentVersion : undefined;

            if (parsed.type === 'mentionsQueryChange') {
                if (
                    typeof parsed.query !== 'string' ||
                    typeof parsed.trigger !== 'string' ||
                    typeof parsed.isActive !== 'boolean' ||
                    !isRecord(parsed.range) ||
                    typeof parsed.range.anchor !== 'number' ||
                    typeof parsed.range.head !== 'number'
                ) {
                    return;
                }
                const queryEvent: MentionQueryChangeEvent = {
                    query: parsed.query,
                    trigger: parsed.trigger,
                    range: parsed.range,
                    isActive: parsed.isActive,
                    ...(documentVersion ? { documentVersion } : {}),
                };
                mentions.onQueryChange?.(queryEvent);
                setMentionQuery(parsed.isActive ? queryEvent : null);
                return;
            }

            if (parsed.type === 'mentionsSelect') {
                if (
                    typeof parsed.trigger !== 'string' ||
                    typeof parsed.suggestionKey !== 'string' ||
                    !isRecord(parsed.attrs)
                ) {
                    return;
                }
                const suggestion = mentions.suggestions?.find(
                    (candidate) => candidate.key === parsed.suggestionKey
                );
                if (!suggestion) return;
                mentions.onSelect?.({
                    trigger: parsed.trigger,
                    suggestion,
                    attrs: parsed.attrs,
                    ...(documentVersion ? { documentVersion } : {}),
                });
                return;
            }

            if (
                parsed.type !== 'mentionsSelectRequest' ||
                typeof parsed.trigger !== 'string' ||
                typeof parsed.suggestionKey !== 'string' ||
                !isRecord(parsed.attrs) ||
                !isRecord(parsed.range) ||
                !Number.isInteger(parsed.range.anchor) ||
                !Number.isInteger(parsed.range.head) ||
                parsed.range.anchor < 0 ||
                parsed.range.head < 0 ||
                parsed.range.anchor > 0xffff_ffff ||
                parsed.range.head > 0xffff_ffff
            ) {
                return;
            }
            const suggestion = mentions.suggestions?.find(
                (candidate) => candidate.key === parsed.suggestionKey
            );
            if (!suggestion) return;

            insertMentionSuggestion({
                trigger: parsed.trigger,
                suggestion,
                attrs: parsed.attrs,
                range: parsed.range,
                documentVersion,
            });
        },
        [documentHandle, insertMentionSuggestion, isForThisEditor]
    );

    const handleMentionSuggestionPress = useCallback(
        (suggestion: MentionSuggestion) => {
            if (mentionQuery == null) return;
            const normalized = normalizeEditorAddons(addonsRef.current)?.mentions?.suggestions.find(
                (candidate) => candidate.key === suggestion.key
            );
            if (normalized == null) return;

            setMentionQuery(null);
            insertMentionSuggestion({
                trigger: mentionQuery.trigger,
                suggestion,
                attrs: normalized.attrs,
                range: mentionQuery.range,
                documentVersion: mentionQuery.documentVersion,
            });
        },
        [insertMentionSuggestion, mentionQuery]
    );

    const mentionSuggestions = addons?.mentions?.suggestions;

    const mentionSuggestionTheme = addons?.mentions?.theme;

    const shouldPublishMentionSuggestions =
        editable && isFocused && mentionQuery != null && (mentionSuggestions?.length ?? 0) > 0;

    const mentionSuggestionThemes = useMemo(() => {
        if (
            mentionQuery == null ||
            mentionSuggestions == null ||
            typeof addons?.mentions?.resolveTheme !== 'function'
        ) {
            return undefined;
        }

        const normalized = normalizeEditorAddons(addons)?.mentions?.suggestions;
        if (normalized == null) return undefined;

        const themes: Record<string, EditorMentionTheme> = {};
        for (const suggestion of mentionSuggestions) {
            const normalizedSuggestion = normalized.find(
                (candidate) => candidate.key === suggestion.key
            );
            if (normalizedSuggestion == null) continue;

            const selectionEvent: MentionSelectionAttrsEvent = {
                trigger: mentionQuery.trigger,
                suggestion,
                attrs: normalizedSuggestion.attrs,
                markAttrs: activeStateRef.current.markAttrs,
                range: mentionQuery.range,
                ...(mentionQuery.documentVersion
                    ? { documentVersion: mentionQuery.documentVersion }
                    : {}),
            };
            const attrs = resolveMentionSelectionAttrs(selectionEvent);
            const merged = mergeMentionSuggestionTheme(
                mentionSuggestionTheme,
                resolveMentionTheme({ ...selectionEvent, attrs })
            );
            if (merged != null) {
                themes[suggestion.key] = merged;
            }
        }

        return Object.keys(themes).length > 0 ? themes : undefined;
    }, [
        addons,
        mentionQuery,
        mentionSuggestionTheme,
        mentionSuggestions,
        resolveMentionSelectionAttrs,
        resolveMentionTheme,
    ]);

    useEffect(() => {
        if (
            !shouldPublishMentionSuggestions ||
            mentionQuery == null ||
            mentionSuggestions == null
        ) {
            setEditorToolbarMentionState(toolbarFrameOwnerId, null);
            return;
        }

        setEditorToolbarMentionState(toolbarFrameOwnerId, {
            trigger: mentionQuery.trigger,
            suggestions: mentionSuggestions,
            theme: mentionSuggestionTheme,
            suggestionThemes: mentionSuggestionThemes,
            onSelectSuggestion: handleMentionSuggestionPress,
        });
    }, [
        handleMentionSuggestionPress,
        mentionQuery,
        mentionSuggestionTheme,
        mentionSuggestionThemes,
        mentionSuggestions,
        shouldPublishMentionSuggestions,
        toolbarFrameOwnerId,
    ]);
    return { mentionSuggestionTheme, handleAddonEvent };
}
