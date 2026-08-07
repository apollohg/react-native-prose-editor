import React from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';

type PanelSectionProps = {
    title: string;
    hint?: string;
    chrome: ExampleAppChrome;
    children: React.ReactNode;
};

function PanelSectionInner({ title, hint, chrome, children }: PanelSectionProps) {
    return (
        <View style={styles.section}>
            <Text style={[sharedStyles.controlLabel, { color: chrome.controlLabelColor }]}>
                {title}
            </Text>
            {hint != null ? (
                <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                    {hint}
                </Text>
            ) : null}
            <View style={styles.body}>{children}</View>
        </View>
    );
}

export const PanelSection = React.memo(PanelSectionInner);

const styles = StyleSheet.create({
    section: {
        gap: SPACE.xs,
    },
    body: {
        gap: SPACE.md,
        paddingTop: SPACE.xs,
    },
});
