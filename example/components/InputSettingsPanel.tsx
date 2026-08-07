import React, { useCallback } from 'react';
import { View } from 'react-native';

import {
    AUTO_CAPITALIZE_OPTIONS,
    HEIGHT_BEHAVIOR_OPTIONS,
    KEYBOARD_TYPE_OPTIONS,
    TOOLBAR_PLACEMENT_OPTIONS,
} from '../constants';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';
import type { EditorBehaviorSettings } from '../types';
import { ChoiceRow } from './ChoiceRow';
import { PanelSection } from './PanelSection';
import { ToggleRow } from './ToggleRow';

type InputSettingsPanelProps = {
    settings: EditorBehaviorSettings;
    onChange: (patch: Partial<EditorBehaviorSettings>) => void;
    chrome: ExampleAppChrome;
};

function InputSettingsPanelInner({ settings, onChange, chrome }: InputSettingsPanelProps) {
    const setEditable = useCallback((editable: boolean) => onChange({ editable }), [onChange]);
    const setAutoFocus = useCallback((autoFocus: boolean) => onChange({ autoFocus }), [onChange]);
    const setAutoCorrect = useCallback(
        (autoCorrect: boolean) => onChange({ autoCorrect }),
        [onChange]
    );
    const setShowToolbar = useCallback(
        (showToolbar: boolean) => onChange({ showToolbar }),
        [onChange]
    );
    const setAutoCapitalize = useCallback(
        (autoCapitalize: EditorBehaviorSettings['autoCapitalize']) => onChange({ autoCapitalize }),
        [onChange]
    );
    const setKeyboardType = useCallback(
        (keyboardType: EditorBehaviorSettings['keyboardType']) => onChange({ keyboardType }),
        [onChange]
    );
    const setHeightBehavior = useCallback(
        (heightBehavior: EditorBehaviorSettings['heightBehavior']) => onChange({ heightBehavior }),
        [onChange]
    );
    const setToolbarPlacement = useCallback(
        (toolbarPlacement: EditorBehaviorSettings['toolbarPlacement']) =>
            onChange({ toolbarPlacement }),
        [onChange]
    );

    return (
        <View style={sharedStyles.settingsPanel}>
            <PanelSection
                title='Editing'
                hint='With editing off, every mutating ref method rejects with MUTATION_REJECTED while selection and controlled content keep flowing.'
                chrome={chrome}>
                <ToggleRow
                    label='Editable'
                    value={settings.editable}
                    onChange={setEditable}
                    chrome={chrome}
                />
                <ToggleRow
                    label='Auto focus'
                    hint='Applies on next mount. Toggle a preset to remount.'
                    value={settings.autoFocus}
                    onChange={setAutoFocus}
                    chrome={chrome}
                />
                <ToggleRow
                    label='Auto correct'
                    value={settings.autoCorrect}
                    onChange={setAutoCorrect}
                    chrome={chrome}
                />
            </PanelSection>

            <PanelSection
                title='Auto capitalize'
                hint='Native keyboard capitalization mode.'
                chrome={chrome}>
                <ChoiceRow
                    options={AUTO_CAPITALIZE_OPTIONS}
                    value={settings.autoCapitalize}
                    onChange={setAutoCapitalize}
                    chrome={chrome}
                    accessibilityLabel='Auto capitalize'
                />
            </PanelSection>

            <PanelSection
                title='Keyboard type'
                hint='Changes the native keyboard layout on focus.'
                chrome={chrome}>
                <ChoiceRow
                    scrollable
                    options={KEYBOARD_TYPE_OPTIONS}
                    value={settings.keyboardType}
                    onChange={setKeyboardType}
                    chrome={chrome}
                    accessibilityLabel='Keyboard type'
                />
            </PanelSection>

            <PanelSection
                title='Height behaviour'
                hint='Fixed scrolls inside the editor. Auto grow expands the view with content.'
                chrome={chrome}>
                <ChoiceRow
                    fill
                    options={HEIGHT_BEHAVIOR_OPTIONS}
                    value={settings.heightBehavior}
                    onChange={setHeightBehavior}
                    chrome={chrome}
                    accessibilityLabel='Height behaviour'
                />
            </PanelSection>

            <PanelSection
                title='Toolbar'
                hint='Keyboard attaches the toolbar natively above the keyboard. Inline renders it in React below the editor.'
                chrome={chrome}>
                <ToggleRow
                    label='Show toolbar'
                    value={settings.showToolbar}
                    onChange={setShowToolbar}
                    chrome={chrome}
                />
                <ChoiceRow
                    fill
                    options={TOOLBAR_PLACEMENT_OPTIONS}
                    value={settings.toolbarPlacement}
                    onChange={setToolbarPlacement}
                    chrome={chrome}
                    accessibilityLabel='Toolbar placement'
                />
            </PanelSection>
        </View>
    );
}

export const InputSettingsPanel = React.memo(InputSettingsPanelInner);
