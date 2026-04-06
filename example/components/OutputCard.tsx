import React from 'react';
import { ScrollView, StyleSheet, Text, View } from 'react-native';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

const BASE64_IMAGE_DATA_URI_PATTERN = /data:image\/([a-zA-Z0-9.+-]+);base64,([A-Za-z0-9+/=_-]+)/g;

function formatBase64PayloadSummary(mediaSubtype: string, payloadLength: number): string {
    const estimatedBytes = Math.floor((payloadLength * 3) / 4);
    const estimatedKilobytes = estimatedBytes / 1024;

    if (estimatedKilobytes >= 1024) {
        return `[base64 image omitted: image/${mediaSubtype}, ${(
            estimatedKilobytes / 1024
        ).toFixed(1)} MB]`;
    }

    return `[base64 image omitted: image/${mediaSubtype}, ${estimatedKilobytes.toFixed(0)} KB]`;
}

function summarizeEmbeddedImagePayloads(value: string): string {
    return value.replace(BASE64_IMAGE_DATA_URI_PATTERN, (_, mediaSubtype, payload) =>
        formatBase64PayloadSummary(mediaSubtype, payload.length)
    );
}

type OutputCardProps = {
    html: string;
    jsonSnapshot: string;
    mentionQuerySummary: string;
    mentionSelectionSummary: string;
    appChrome: ExampleThemePreset['appChrome'];
};

type OutputSectionProps = {
    title: string;
    value: string;
    textColor: string;
};

function OutputSection({ title, value, textColor }: OutputSectionProps) {
    return (
        <View style={styles.outputSection}>
            <Text style={[sharedStyles.sectionLabel, { color: textColor }]}>{title}</Text>
            <ScrollView
                nestedScrollEnabled
                style={styles.outputScroller}
                contentContainerStyle={styles.outputScrollerContent}>
                <Text selectable style={[styles.outputText, { color: textColor }]}>
                    {value}
                </Text>
            </ScrollView>
        </View>
    );
}

export function OutputCard({
    html,
    jsonSnapshot,
    mentionQuerySummary,
    mentionSelectionSummary,
    appChrome,
}: OutputCardProps) {
    const summarizedHtml = React.useMemo(() => summarizeEmbeddedImagePayloads(html), [html]);
    const summarizedJsonSnapshot = React.useMemo(
        () => summarizeEmbeddedImagePayloads(jsonSnapshot),
        [jsonSnapshot]
    );

    return (
        <View style={[styles.outputCard, { backgroundColor: appChrome.outputCardBackgroundColor }]}>
            <OutputSection
                title='HTML Snapshot'
                value={summarizedHtml}
                textColor={appChrome.outputTextColor}
            />
            <OutputSection
                title='JSON Snapshot'
                value={summarizedJsonSnapshot}
                textColor={appChrome.outputTextColor}
            />
            <OutputSection
                title='Mention Query Event'
                value={mentionQuerySummary}
                textColor={appChrome.outputTextColor}
            />
            <OutputSection
                title='Mention Select Event'
                value={mentionSelectionSummary}
                textColor={appChrome.outputTextColor}
            />
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
    },
    outputScroller: {
        maxHeight: 176,
    },
    outputScrollerContent: {
        paddingBottom: 2,
    },
    outputText: {
        fontSize: 12,
        lineHeight: 18,
    },
});
