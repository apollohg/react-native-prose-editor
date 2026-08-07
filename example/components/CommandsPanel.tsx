import React, { useCallback } from 'react';
import { StyleSheet, View } from 'react-native';

import { EDITOR_COMMAND_GROUPS, type EditorCommandId } from '../constants';
import { SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';
import { ActionButton } from './ActionButton';
import { PanelSection } from './PanelSection';

/** Every imperative ref method as a one-tap button. */

type CommandsPanelProps = {
    onCommand: (id: EditorCommandId) => void;
    /** Mark names from the active schema, minus the ones needing an argument. */
    toggleableMarks: readonly string[];
    onToggleMark: (mark: string) => void;
    editable: boolean;
    chrome: ExampleAppChrome;
};

/** Mutating commands reject when `editable` is false. */
const MUTATING_PREFIXES = ['block:', 'heading:', 'insert:', 'doc:', 'history:undo', 'history:redo'];

const READ_ONLY_HINT = 'Unavailable because the editor is read-only.';

function isMutating(id: EditorCommandId): boolean {
    return MUTATING_PREFIXES.some((prefix) => id.startsWith(prefix));
}

function toMarkLabel(mark: string): string {
    return `${mark.charAt(0).toUpperCase()}${mark.slice(1)}`;
}

/** Owns its press closure so `ActionButton`'s memo holds. */
const CommandButton = React.memo(function CommandButton({
    id,
    label,
    disabled,
    onCommand,
    chrome,
}: {
    id: EditorCommandId;
    label: string;
    disabled: boolean;
    onCommand: (id: EditorCommandId) => void;
    chrome: ExampleAppChrome;
}) {
    const handlePress = useCallback(() => onCommand(id), [id, onCommand]);
    return (
        <ActionButton
            label={label}
            tone='secondary'
            disabled={disabled}
            onPress={handlePress}
            chrome={chrome}
            accessibilityHint={disabled ? READ_ONLY_HINT : undefined}
        />
    );
});

const MarkButton = React.memo(function MarkButton({
    mark,
    disabled,
    onToggleMark,
    chrome,
}: {
    mark: string;
    disabled: boolean;
    onToggleMark: (mark: string) => void;
    chrome: ExampleAppChrome;
}) {
    const handlePress = useCallback(() => onToggleMark(mark), [mark, onToggleMark]);
    return (
        <ActionButton
            label={toMarkLabel(mark)}
            tone='secondary'
            disabled={disabled}
            onPress={handlePress}
            chrome={chrome}
            accessibilityHint={disabled ? READ_ONLY_HINT : undefined}
        />
    );
});

function CommandsPanelInner({
    onCommand,
    toggleableMarks,
    onToggleMark,
    editable,
    chrome,
}: CommandsPanelProps) {
    return (
        <View style={sharedStyles.settingsPanel}>
            <PanelSection
                title='Marks'
                hint='Read from the active schema, so this list follows the mentions toggle rather than a hardcoded copy of it.'
                chrome={chrome}>
                <View style={styles.buttonGrid}>
                    {toggleableMarks.map((mark) => (
                        <MarkButton
                            key={mark}
                            mark={mark}
                            disabled={!editable}
                            onToggleMark={onToggleMark}
                            chrome={chrome}
                        />
                    ))}
                </View>
            </PanelSection>

            {EDITOR_COMMAND_GROUPS.map((group) => (
                <PanelSection
                    key={group.title}
                    title={group.title}
                    hint={group.hint}
                    chrome={chrome}>
                    <View style={styles.buttonGrid}>
                        {group.commands.map((command) => (
                            <CommandButton
                                key={command.id}
                                id={command.id}
                                label={command.label}
                                disabled={!editable && isMutating(command.id)}
                                onCommand={onCommand}
                                chrome={chrome}
                            />
                        ))}
                    </View>
                </PanelSection>
            ))}
        </View>
    );
}

export const CommandsPanel = React.memo(CommandsPanelInner);

const styles = StyleSheet.create({
    buttonGrid: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
});
