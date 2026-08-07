import React, { useEffect, useRef } from 'react';
import { Modal, StyleSheet, Text, TextInput, View } from 'react-native';

import { FONT_SIZE, RADIUS, SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';
import { ActionButton } from './ActionButton';

/** URL prompt driven by `onRequestLink`. */

const BACKDROP_COLOR = 'rgba(8, 6, 4, 0.45)';

type LinkEditorModalProps = {
    visible: boolean;
    isActive: boolean;
    linkDraft: string;
    onLinkDraftChange: (value: string) => void;
    onClose: () => void;
    onRemove: () => void;
    onApply: () => void;
    chrome: ExampleAppChrome;
};

function LinkEditorModalInner({
    visible,
    isActive,
    linkDraft,
    onLinkDraftChange,
    onClose,
    onRemove,
    onApply,
    chrome,
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
        <Modal
            animationType='fade'
            transparent
            visible={visible}
            onRequestClose={onClose}
            accessibilityViewIsModal>
            <View style={styles.backdrop}>
                <View style={[styles.card, { backgroundColor: chrome.cardBackgroundColor }]}>
                    <Text
                        accessibilityRole='header'
                        style={[sharedStyles.heading, { color: chrome.controlLabelColor }]}>
                        {isActive ? 'Edit link' : 'Add link'}
                    </Text>

                    <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                        Applies to the current selection. Saving an empty value removes the link.
                    </Text>

                    <TextInput
                        ref={inputRef}
                        accessibilityLabel='Link URL'
                        autoCapitalize='none'
                        autoCorrect={false}
                        keyboardType='url'
                        placeholder='https://example.com'
                        placeholderTextColor={chrome.controlHintColor}
                        style={[
                            styles.input,
                            {
                                color: chrome.controlLabelColor,
                                backgroundColor: chrome.controlSurfaceColor,
                            },
                        ]}
                        value={linkDraft}
                        onChangeText={onLinkDraftChange}
                        onSubmitEditing={onApply}
                        returnKeyType='done'
                    />

                    <View style={styles.buttonRow}>
                        <ActionButton
                            label='Cancel'
                            tone='secondary'
                            onPress={onClose}
                            chrome={chrome}
                        />
                        {isActive ? (
                            <ActionButton
                                label='Remove'
                                tone='secondary'
                                onPress={onRemove}
                                chrome={chrome}
                                accessibilityHint='Removes the link but keeps the text.'
                            />
                        ) : null}
                        <ActionButton label='Save' onPress={onApply} chrome={chrome} />
                    </View>
                </View>
            </View>
        </Modal>
    );
}

export const LinkEditorModal = React.memo(LinkEditorModalInner);

const styles = StyleSheet.create({
    backdrop: {
        flex: 1,
        backgroundColor: BACKDROP_COLOR,
        justifyContent: 'center',
        paddingHorizontal: SPACE.xl,
    },
    card: {
        borderRadius: RADIUS,
        padding: SPACE.xl,
        gap: SPACE.md,
    },
    input: {
        borderRadius: RADIUS,
        paddingHorizontal: SPACE.lg,
        paddingVertical: SPACE.md,
        fontSize: FONT_SIZE.label,
    },
    buttonRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
        justifyContent: 'flex-end',
        paddingTop: SPACE.xs,
    },
});
