import React from 'react';
import { StyleSheet, View } from 'react-native';

import { SETTINGS_TABS, type SettingsTab } from '../constants';
import { RADIUS, SPACE } from '../designTokens';
import type { ExampleAppChrome } from '../themePresets';
import { ChoiceRow } from './ChoiceRow';
import { CollapsibleSection } from './CollapsibleSection';

type SettingsCardProps = {
    tab: SettingsTab;
    onTabChange: (tab: SettingsTab) => void;
    chrome: ExampleAppChrome;
    /** Shown beside the section title while collapsed, e.g. "Editor". */
    badge?: string;
    children: React.ReactNode;
};

function SettingsCardInner({ tab, onTabChange, chrome, badge, children }: SettingsCardProps) {
    return (
        <CollapsibleSection
            title='Settings'
            badge={badge}
            chrome={chrome}
            initiallyExpanded
            style={[styles.card, { backgroundColor: chrome.cardBackgroundColor }]}>
            <ChoiceRow
                scrollable
                options={SETTINGS_TABS}
                value={tab}
                onChange={onTabChange}
                chrome={chrome}
                accessibilityLabel='Settings section'
            />
            <View style={styles.panel}>{children}</View>
        </CollapsibleSection>
    );
}

export const SettingsCard = React.memo(SettingsCardInner);

const styles = StyleSheet.create({
    card: {
        padding: SPACE.lg,
        borderRadius: RADIUS,
    },
    panel: {
        paddingTop: SPACE.xs,
    },
});
