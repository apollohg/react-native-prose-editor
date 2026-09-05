import { useCallback, useEffect, useRef, useState } from 'react';
import {
    KeyboardAvoidingView,
    Modal,
    Platform,
    Pressable,
    StyleSheet,
    Text,
    TextInput,
    View,
} from 'react-native';
import type { LinkRequestContext } from 'react-native-rich-text-editor';

import { FONT_SIZE, LINE_HEIGHT, MIN_TOUCH_TARGET, PALETTE, RADIUS, SPACE } from '../theme';

const BACKDROP_COLOR = 'rgba(27, 31, 42, 0.4)';

type LinkEditorModalProps = {
    /** The pending toolbar link request, or null when the sheet is closed. */
    request: LinkRequestContext | null;
    onClose: () => void;
};

/** Bottom sheet driven by `onRequestLink`. Saving an empty URL removes the link. */
export function LinkEditorModal({ request, onClose }: LinkEditorModalProps) {
    const inputRef = useRef<TextInput>(null);
    const [href, setHref] = useState('');
    const visible = request != null;
    const isActive = request?.isActive ?? false;

    useEffect(() => {
        if (request == null) {
            return;
        }
        setHref(request.href ?? '');
        const handle = requestAnimationFrame(() => inputRef.current?.focus());
        return () => cancelAnimationFrame(handle);
    }, [request]);

    const apply = useCallback(() => {
        if (request == null) {
            return;
        }
        const trimmed = href.trim();
        if (trimmed.length === 0) {
            request.unsetLink();
        } else {
            request.setLink(trimmed);
        }
        onClose();
    }, [href, onClose, request]);

    const remove = useCallback(() => {
        request?.unsetLink();
        onClose();
    }, [onClose, request]);

    return (
        <Modal
            animationType='fade'
            transparent
            visible={visible}
            onRequestClose={onClose}
            accessibilityViewIsModal>
            <KeyboardAvoidingView
                behavior={Platform.OS === 'ios' ? 'padding' : undefined}
                style={styles.backdrop}>
                <Pressable
                    accessibilityLabel='Close'
                    accessibilityRole='button'
                    onPress={onClose}
                    style={styles.dismissArea}
                />
                <View style={styles.sheet}>
                    <Text accessibilityRole='header' style={styles.title}>
                        {isActive ? 'Edit link' : 'Add link'}
                    </Text>
                    <TextInput
                        ref={inputRef}
                        accessibilityLabel='Link URL'
                        autoCapitalize='none'
                        autoCorrect={false}
                        keyboardType='url'
                        placeholder='https://'
                        placeholderTextColor={PALETTE.inkFaint}
                        returnKeyType='done'
                        style={styles.input}
                        value={href}
                        onChangeText={setHref}
                        onSubmitEditing={apply}
                    />
                    <View style={styles.actions}>
                        {isActive ? (
                            <Pressable
                                accessibilityRole='button'
                                accessibilityLabel='Remove link'
                                onPress={remove}
                                style={({ pressed }) => [styles.button, pressed && styles.pressed]}>
                                <Text style={[styles.buttonLabel, styles.removeLabel]}>Remove</Text>
                            </Pressable>
                        ) : null}
                        <View style={styles.spacer} />
                        <Pressable
                            accessibilityRole='button'
                            accessibilityLabel='Cancel'
                            onPress={onClose}
                            style={({ pressed }) => [styles.button, pressed && styles.pressed]}>
                            <Text style={styles.buttonLabel}>Cancel</Text>
                        </Pressable>
                        <Pressable
                            accessibilityRole='button'
                            accessibilityLabel='Save link'
                            onPress={apply}
                            style={({ pressed }) => [
                                styles.button,
                                styles.primaryButton,
                                pressed && styles.pressed,
                            ]}>
                            <Text style={[styles.buttonLabel, styles.primaryLabel]}>Save</Text>
                        </Pressable>
                    </View>
                </View>
            </KeyboardAvoidingView>
        </Modal>
    );
}

const styles = StyleSheet.create({
    backdrop: {
        flex: 1,
        justifyContent: 'flex-end',
        backgroundColor: BACKDROP_COLOR,
    },
    dismissArea: {
        flex: 1,
    },
    sheet: {
        backgroundColor: PALETTE.paper,
        borderTopLeftRadius: RADIUS.sheet,
        borderTopRightRadius: RADIUS.sheet,
        padding: SPACE.xl,
        paddingBottom: SPACE.xxl,
        gap: SPACE.lg,
    },
    title: {
        color: PALETTE.ink,
        fontSize: FONT_SIZE.title,
        lineHeight: LINE_HEIGHT.title,
        fontWeight: '700',
    },
    input: {
        minHeight: MIN_TOUCH_TARGET + SPACE.xs,
        paddingHorizontal: SPACE.lg,
        borderRadius: RADIUS.control,
        borderWidth: 1,
        borderColor: PALETTE.hairline,
        backgroundColor: PALETTE.wash,
        color: PALETTE.ink,
        fontSize: FONT_SIZE.body,
    },
    actions: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: SPACE.sm,
    },
    spacer: {
        flex: 1,
    },
    button: {
        minHeight: MIN_TOUCH_TARGET,
        paddingHorizontal: SPACE.lg,
        borderRadius: RADIUS.control,
        alignItems: 'center',
        justifyContent: 'center',
    },
    primaryButton: {
        backgroundColor: PALETTE.spruce,
    },
    pressed: {
        opacity: 0.6,
    },
    buttonLabel: {
        color: PALETTE.inkMuted,
        fontSize: FONT_SIZE.body,
        fontWeight: '600',
    },
    primaryLabel: {
        color: PALETTE.paper,
    },
    removeLabel: {
        color: PALETTE.rose,
    },
});
