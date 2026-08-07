import React, { useCallback } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import {
    CONTROLLED_SOURCE_OPTIONS,
    VALUE_JSON_UPDATE_MODE_OPTIONS,
    type ControlledSourceMode,
} from '../constants';
import { MAX_NUMERIC_FONT_SCALE, SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';
import type { ControlledSettings } from '../types';
import { ActionButton } from './ActionButton';
import { ChoiceRow } from './ChoiceRow';
import { PanelSection } from './PanelSection';

const SOURCE_HINTS: Record<ControlledSourceMode, string> = {
    uncontrolled:
        'The editor owns its document. Typing flows straight through the engine and nothing is pushed back in.',
    html: 'The `value` prop drives the document. External writes are diffed against the engine and applied as HTML.',
    json: 'The `valueJSON` prop drives the document. `valueJSONRevision` lets the editor skip reserializing an unchanged doc.',
};

type ContentSettingsPanelProps = {
    settings: ControlledSettings;
    onChange: (patch: Partial<ControlledSettings>) => void;
    valueJSONRevision: string;
    documentRevision: string | null;
    onBumpValueRevision: () => void;
    onBumpDocumentRevision: () => void;
    onLoadControlledDocument: () => void;
    onLoadInitialDocument: () => void;
    chrome: ExampleAppChrome;
};

function ContentSettingsPanelInner({
    settings,
    onChange,
    valueJSONRevision,
    documentRevision,
    onBumpValueRevision,
    onBumpDocumentRevision,
    onLoadControlledDocument,
    onLoadInitialDocument,
    chrome,
}: ContentSettingsPanelProps) {
    const controlled = settings.mode !== 'uncontrolled';

    const setMode = useCallback(
        (mode: ControlledSettings['mode']) => onChange({ mode }),
        [onChange]
    );
    const setUpdateMode = useCallback(
        (updateMode: ControlledSettings['updateMode']) => onChange({ updateMode }),
        [onChange]
    );

    return (
        <View style={sharedStyles.settingsPanel}>
            <PanelSection title='Content source' hint={SOURCE_HINTS[settings.mode]} chrome={chrome}>
                <ChoiceRow
                    scrollable
                    options={CONTROLLED_SOURCE_OPTIONS}
                    value={settings.mode}
                    onChange={setMode}
                    chrome={chrome}
                    accessibilityLabel='Content source'
                />
            </PanelSection>

            {controlled ? (
                <PanelSection
                    title='Controlled document'
                    hint='Swap the document the controlled prop is holding, then confirm the editor follows.'
                    chrome={chrome}>
                    <View style={styles.buttonRow}>
                        <ActionButton
                            label='Load sample doc'
                            onPress={onLoadControlledDocument}
                            chrome={chrome}
                        />
                        <ActionButton
                            label='Load initial doc'
                            tone='secondary'
                            onPress={onLoadInitialDocument}
                            chrome={chrome}
                        />
                    </View>
                </PanelSection>
            ) : null}

            {settings.mode === 'json' ? (
                <PanelSection
                    title='Update mode'
                    hint='Replace preserves undo history across an external write. Reset drops it.'
                    chrome={chrome}>
                    <ChoiceRow
                        fill
                        options={VALUE_JSON_UPDATE_MODE_OPTIONS}
                        value={settings.updateMode}
                        onChange={setUpdateMode}
                        chrome={chrome}
                        accessibilityLabel='Value JSON update mode'
                    />
                </PanelSection>
            ) : null}

            <PanelSection
                title='Revisions'
                hint='valueJSONRevision short-circuits reserialization of an equal doc. documentRevision forces an authoritative engine re-read, the signal the collaboration controller drives.'
                chrome={chrome}>
                <View style={styles.revisionRow}>
                    <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                        valueJSONRevision
                    </Text>
                    <Text
                        maxFontSizeMultiplier={MAX_NUMERIC_FONT_SCALE}
                        style={[sharedStyles.numericValue, { color: chrome.sliderValueColor }]}>
                        {valueJSONRevision}
                    </Text>
                </View>
                <View style={styles.revisionRow}>
                    <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                        documentRevision
                    </Text>
                    <Text
                        maxFontSizeMultiplier={MAX_NUMERIC_FONT_SCALE}
                        style={[sharedStyles.numericValue, { color: chrome.sliderValueColor }]}>
                        {documentRevision ?? 'null'}
                    </Text>
                </View>
                <View style={styles.buttonRow}>
                    <ActionButton
                        label='Bump value revision'
                        tone='secondary'
                        onPress={onBumpValueRevision}
                        chrome={chrome}
                    />
                    <ActionButton
                        label='Bump doc revision'
                        tone='secondary'
                        onPress={onBumpDocumentRevision}
                        chrome={chrome}
                    />
                </View>
            </PanelSection>
        </View>
    );
}

export const ContentSettingsPanel = React.memo(ContentSettingsPanelInner);

const styles = StyleSheet.create({
    buttonRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
    revisionRow: {
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: SPACE.md,
    },
});
