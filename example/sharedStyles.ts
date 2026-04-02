import { StyleSheet } from 'react-native';

export const sharedStyles = StyleSheet.create({
    sectionLabel: {
        fontSize: 13,
        fontWeight: '700',
        textTransform: 'uppercase',
        letterSpacing: 1,
    },
    controlLabel: {
        fontSize: 14,
        fontWeight: '700',
    },
    controlHint: {
        fontSize: 13,
        lineHeight: 18,
    },
    settingsPanel: {
        gap: 16,
    },
    inputRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        justifyContent: 'space-between',
        gap: 12,
    },
    inputGroup: {
        width: '48%',
        gap: 8,
    },
    sliderHeader: {
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
    },
    sliderValue: {
        fontSize: 13,
        fontWeight: '700',
    },
    slider: {
        width: '100%',
        height: 36,
    },
});
