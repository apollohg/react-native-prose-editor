import React from 'react';
import { StyleSheet, Text, View } from 'react-native';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type OutputCardProps = {
    html: string;
    jsonSnapshot: string;
    mentionQuerySummary: string;
    mentionSelectionSummary: string;
    appChrome: ExampleThemePreset['appChrome'];
};

export function OutputCard({
    html,
    jsonSnapshot,
    mentionQuerySummary,
    mentionSelectionSummary,
    appChrome,
}: OutputCardProps) {
    return (
        <View style={[styles.outputCard, { backgroundColor: appChrome.outputCardBackgroundColor }]}>
            <Text style={[sharedStyles.sectionLabel, { color: appChrome.outputTextColor }]}>
                HTML Snapshot
            </Text>
            <Text style={[styles.outputText, { color: appChrome.outputTextColor }]}>{html}</Text>

            <View style={styles.outputSection}>
                <Text style={[sharedStyles.sectionLabel, { color: appChrome.outputTextColor }]}>
                    JSON Snapshot
                </Text>
                <Text style={[styles.outputText, { color: appChrome.outputTextColor }]}>
                    {jsonSnapshot}
                </Text>
            </View>

            <View style={styles.outputSection}>
                <Text style={[sharedStyles.sectionLabel, { color: appChrome.outputTextColor }]}>
                    Mention Query Event
                </Text>
                <Text style={[styles.outputText, { color: appChrome.outputTextColor }]}>
                    {mentionQuerySummary}
                </Text>
            </View>

            <View style={styles.outputSection}>
                <Text style={[sharedStyles.sectionLabel, { color: appChrome.outputTextColor }]}>
                    Mention Select Event
                </Text>
                <Text style={[styles.outputText, { color: appChrome.outputTextColor }]}>
                    {mentionSelectionSummary}
                </Text>
            </View>
        </View>
    );
}

const styles = StyleSheet.create({
    outputCard: {
        borderRadius: 18,
        padding: 16,
        gap: 10,
    },
    outputSection: {
        gap: 8,
        paddingTop: 8,
    },
    outputText: {
        fontSize: 12,
        lineHeight: 18,
    },
});
