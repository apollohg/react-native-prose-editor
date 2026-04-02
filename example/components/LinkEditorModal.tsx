import React, { useEffect, useRef } from 'react';
import { Modal, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type LinkEditorModalProps = {
    visible: boolean;
    isActive: boolean;
    linkDraft: string;
    onLinkDraftChange: (value: string) => void;
    onClose: () => void;
    onRemove: () => void;
    onApply: () => void;
    appChrome: ExampleThemePreset['appChrome'];
};

export function LinkEditorModal({
    visible,
    isActive,
    linkDraft,
    onLinkDraftChange,
    onClose,
    onRemove,
    onApply,
    appChrome,
}: LinkEditorModalProps) {
    const inputRef = useRef<TextInput>(null);

    useEffect(() => {
        if (!visible) {
            return;
        }

        const handle = requestAnimationFrame(() => {
            inputRef.current?.focus();
        });

        return () => cancelAnimationFrame(handle);
    }, [visible]);

    return (
        <Modal animationType='fade' transparent visible={visible} onRequestClose={onClose}>
            <View style={styles.backdrop}>
                <View
                    style={[
                        styles.card,
                        {
                            backgroundColor: appChrome.cardBackgroundColor,
                            borderColor: appChrome.tabBorderColor,
                        },
                    ]}>
                    <Text
                        style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                        {isActive ? 'Edit Link' : 'Add Link'}
                    </Text>

                    <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
                        Enter a URL to apply to the current selection. Clear it to remove the link.
                    </Text>

                    <TextInput
                        ref={inputRef}
                        autoCapitalize='none'
                        autoCorrect={false}
                        keyboardType='url'
                        placeholder='https://example.com'
                        placeholderTextColor={appChrome.controlHintColor}
                        style={[
                            styles.input,
                            {
                                color: appChrome.titleColor,
                                backgroundColor: appChrome.cardSecondaryBackgroundColor,
                                borderColor: appChrome.tabBorderColor,
                            },
                        ]}
                        value={linkDraft}
                        onChangeText={onLinkDraftChange}
                        onSubmitEditing={onApply}
                    />

                    <View style={styles.buttonRow}>
                        <Pressable
                            style={[
                                styles.actionButton,
                                styles.linkButton,
                                { backgroundColor: appChrome.actionButtonBackgroundColor },
                            ]}
                            onPress={onClose}>
                            <Text
                                style={[
                                    styles.actionButtonText,
                                    { color: appChrome.actionButtonTextColor },
                                ]}>
                                Cancel
                            </Text>
                        </Pressable>

                        {isActive ? (
                            <Pressable
                                style={[
                                    styles.actionButton,
                                    styles.linkButton,
                                    { backgroundColor: appChrome.actionButtonBackgroundColor },
                                ]}
                                onPress={onRemove}>
                                <Text
                                    style={[
                                        styles.actionButtonText,
                                        { color: appChrome.actionButtonTextColor },
                                    ]}>
                                    Remove
                                </Text>
                            </Pressable>
                        ) : null}

                        <Pressable
                            style={[
                                styles.actionButton,
                                styles.linkButton,
                                { backgroundColor: appChrome.actionButtonBackgroundColor },
                            ]}
                            onPress={onApply}>
                            <Text
                                style={[
                                    styles.actionButtonText,
                                    { color: appChrome.actionButtonTextColor },
                                ]}>
                                Save
                            </Text>
                        </Pressable>
                    </View>
                </View>
            </View>
        </Modal>
    );
}

const styles = StyleSheet.create({
    backdrop: {
        flex: 1,
        backgroundColor: 'rgba(18, 14, 10, 0.3)',
        justifyContent: 'center',
        paddingHorizontal: 20,
    },
    card: {
        borderWidth: 1,
        borderRadius: 20,
        padding: 20,
        gap: 16,
    },
    input: {
        borderWidth: 1,
        borderRadius: 12,
        paddingHorizontal: 14,
        paddingVertical: 12,
        fontSize: 15,
    },
    buttonRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: 12,
        justifyContent: 'flex-end',
    },
    linkButton: {
        minWidth: 88,
        alignItems: 'center',
    },
    actionButton: {
        paddingHorizontal: 14,
        paddingVertical: 10,
        borderRadius: 999,
    },
    actionButtonText: {
        fontWeight: '700',
    },
});
