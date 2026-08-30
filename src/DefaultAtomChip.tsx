import React from 'react';
import { StyleSheet, Text, View } from 'react-native';

import type { AtomComponentProps } from './atoms';

export function DefaultAtomChip({ nodeType }: AtomComponentProps) {
    return (
        <View style={styles.container}>
            <Text style={styles.label}>{nodeType}</Text>
        </View>
    );
}

const styles = StyleSheet.create({
    container: {
        minHeight: 32,
        justifyContent: 'center',
        paddingHorizontal: 10,
        borderRadius: 6,
        backgroundColor: '#E5E7EB',
    },
    label: {
        color: '#374151',
        fontSize: 13,
    },
});
