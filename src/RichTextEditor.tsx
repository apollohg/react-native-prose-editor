import { forwardRef } from 'react';
import { type RichTextEditorRef, type RichTextEditorProps } from './RichTextEditorTypes';
import { useRichTextEditorState } from './useRichTextEditorState';
import { useRichTextEditorUpdates } from './useRichTextEditorUpdates';
import { useRichTextEditorCommands } from './useRichTextEditorCommands';
import { useRichTextEditorEvents } from './useRichTextEditorEvents';
import { useRichTextEditorMentions } from './useRichTextEditorMentions';
import { useRichTextEditorPresentation } from './useRichTextEditorPresentation';

/**
 * Renders a shared v2 document session as a genuinely interactive editor.
 * The native view binds to the handle's session id, so the Task 15 native
 * v2 adapters own typing/IME (one commit per transaction; transient
 * composing text never reaches the engine), selection mirroring, and the
 * native toolbar. This component owns the JS side: controlled
 * `value`/`valueJSON`, the typing/command ref methods (routed through the
 * v2 bridge with refresh-never-retry mismatch semantics), the link/image
 * request flows, and pushing the engine's render update back to the view
 * after every JS-driven change. A room document awaiting the server renders
 * nothing (loading), never an unshared fallback paragraph.
 */
export const RichTextEditor = forwardRef<RichTextEditorRef, RichTextEditorProps>(
    function RichTextEditor(props, ref) {
        const state = useRichTextEditorState(props, ref);
        const updates = useRichTextEditorUpdates(state);
        const commands = useRichTextEditorCommands({ ...state, ...updates });
        const events = useRichTextEditorEvents({ ...state, ...updates, ...commands });
        const mentions = useRichTextEditorMentions({
            ...state,
            ...commands,
            ...updates,
            ...events,
        });
        return useRichTextEditorPresentation({ ...state, ...mentions, ...commands, ...events });
    }
);

/** @deprecated Use RichTextEditor instead. */
export const NativeRichTextEditor = RichTextEditor;
