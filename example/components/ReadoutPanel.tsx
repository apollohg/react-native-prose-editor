import React, { useMemo } from 'react';
import { ScrollView, StyleSheet, Text, View } from 'react-native';

import type { ChoiceOption } from '../constants';
import { FONT_SIZE, RADIUS, SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';
import type { EditorEventEntry } from '../types';
import { ChoiceRow } from './ChoiceRow';

/** HTML, JSON, mention payloads and the event log. One pane at a time. */

const READOUT_HEIGHT = 404;
const BASE64_IMAGE_DATA_URI_PATTERN = /data:image\/([a-zA-Z0-9.+-]+);base64,([A-Za-z0-9+/=_-]+)/g;
const BYTES_PER_KILOBYTE = 1024;
const BASE64_BYTES_PER_CHAR = 3 / 4;

export type ReadoutPane = 'html' | 'json' | 'mentions' | 'events';

export const READOUT_PANES: readonly ChoiceOption<ReadoutPane>[] = [
    { value: 'html', label: 'HTML' },
    { value: 'json', label: 'JSON' },
    { value: 'mentions', label: 'Mentions' },
    { value: 'events', label: 'Events' },
];

function formatBase64PayloadSummary(mediaSubtype: string, payloadLength: number): string {
    const estimatedBytes = Math.floor(payloadLength * BASE64_BYTES_PER_CHAR);
    const estimatedKilobytes = estimatedBytes / BYTES_PER_KILOBYTE;

    if (estimatedKilobytes >= BYTES_PER_KILOBYTE) {
        return `[base64 image omitted: image/${mediaSubtype}, ${(
            estimatedKilobytes / BYTES_PER_KILOBYTE
        ).toFixed(1)} MB]`;
    }

    return `[base64 image omitted: image/${mediaSubtype}, ${estimatedKilobytes.toFixed(0)} KB]`;
}

function summarizeEmbeddedImagePayloads(value: string): string {
    return value.replace(BASE64_IMAGE_DATA_URI_PATTERN, (_, mediaSubtype, payload) =>
        formatBase64PayloadSummary(mediaSubtype, payload.length)
    );
}

type ReadoutPanelProps = {
    pane: ReadoutPane;
    onPaneChange: (pane: ReadoutPane) => void;
    html: string;
    jsonSnapshot: string;
    mentionQuerySummary: string;
    mentionSelectionSummary: string;
    events: readonly EditorEventEntry[];
    chrome: ExampleAppChrome;
};

type SubsectionProps = {
    title: string;
    value: string;
    textColor: string;
    labelColor: string;
};

function Subsection({ title, value, textColor, labelColor }: SubsectionProps) {
    return (
        <View style={styles.subsection}>
            <Text style={[sharedStyles.sectionLabel, { color: labelColor }]}>{title}</Text>
            <Text selectable style={[sharedStyles.monoReadout, { color: textColor }]}>
                {value}
            </Text>
        </View>
    );
}

function ReadoutPanelInner({
    pane,
    onPaneChange,
    html,
    jsonSnapshot,
    mentionQuerySummary,
    mentionSelectionSummary,
    events,
    chrome,
}: ReadoutPanelProps) {
    const summarizedHtml = useMemo(() => summarizeEmbeddedImagePayloads(html), [html]);
    const summarizedJsonSnapshot = useMemo(
        () => summarizeEmbeddedImagePayloads(jsonSnapshot),
        [jsonSnapshot]
    );

    // The readout card is always dark, so labels use the output tokens.
    const labelColor = chrome.outputTextColor;

    return (
        <View style={[styles.card, { backgroundColor: chrome.outputCardBackgroundColor }]}>
            <ChoiceRow
                fill
                options={READOUT_PANES}
                value={pane}
                onChange={onPaneChange}
                chrome={chrome}
                accessibilityLabel='Readout'
            />

            <ScrollView
                style={styles.scroller}
                contentContainerStyle={styles.scrollerContent}
                keyboardShouldPersistTaps='always'>
                {pane === 'html' ? (
                    <Text
                        selectable
                        style={[sharedStyles.monoReadout, { color: chrome.outputTextColor }]}>
                        {summarizedHtml}
                    </Text>
                ) : null}

                {pane === 'json' ? (
                    <Text
                        selectable
                        style={[sharedStyles.monoReadout, { color: chrome.outputTextColor }]}>
                        {summarizedJsonSnapshot}
                    </Text>
                ) : null}

                {pane === 'mentions' ? (
                    <View style={styles.stack}>
                        <Subsection
                            title='Query event'
                            value={mentionQuerySummary}
                            textColor={chrome.outputTextColor}
                            labelColor={labelColor}
                        />
                        <Subsection
                            title='Select event'
                            value={mentionSelectionSummary}
                            textColor={chrome.outputTextColor}
                            labelColor={labelColor}
                        />
                    </View>
                ) : null}

                {pane === 'events' ? (
                    events.length === 0 ? (
                        <Text style={[sharedStyles.monoReadout, { color: chrome.outputTextColor }]}>
                            {
                                'No events yet.\n\nFocus the editor, move the caret, or apply a mark. Every editor callback lands here newest first.'
                            }
                        </Text>
                    ) : (
                        <View style={styles.stack}>
                            {events.map((event) => (
                                <View key={event.id} style={styles.eventRow}>
                                    <Text
                                        style={[
                                            styles.eventKind,
                                            { color: chrome.outputTextColor },
                                        ]}>
                                        {event.atSeconds.toFixed(1)}s {event.kind}
                                    </Text>
                                    <Text
                                        selectable
                                        style={[
                                            sharedStyles.monoReadout,
                                            { color: chrome.outputTextColor },
                                        ]}>
                                        {event.detail}
                                    </Text>
                                </View>
                            ))}
                        </View>
                    )
                ) : null}
            </ScrollView>
        </View>
    );
}

export const ReadoutPanel = React.memo(ReadoutPanelInner);

const styles = StyleSheet.create({
    card: {
        borderRadius: RADIUS,
        padding: SPACE.lg,
        gap: SPACE.md,
    },
    scroller: {
        height: READOUT_HEIGHT,
    },
    scrollerContent: {
        paddingBottom: SPACE.xs,
    },
    stack: {
        gap: SPACE.lg,
    },
    subsection: {
        gap: SPACE.sm,
    },
    eventRow: {
        gap: SPACE.xs,
    },
    eventKind: {
        fontSize: FONT_SIZE.hint,
        fontWeight: '700',
        fontVariant: ['tabular-nums'],
    },
});
