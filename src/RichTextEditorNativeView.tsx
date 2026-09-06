import { requireNativeViewManager } from 'expo-modules-core';
import React from 'react';
import { StyleSheet } from 'react-native';
import {
    type NativeEditorViewProps,
    type NativeEditorViewHandle,
} from './RichTextEditorNativeTypes';

export const NativeEditorView = requireNativeViewManager('NativeEditor') as React.ComponentType<
    NativeEditorViewProps & React.RefAttributes<NativeEditorViewHandle>
>;

export const styles = StyleSheet.create({
    container: {
        position: 'relative',
    },
    inlineToolbar: {
        flexDirection: 'row',
        alignItems: 'center',
    },
});
