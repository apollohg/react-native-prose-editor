import React, { useCallback, useRef, useState } from 'react';
import { MaterialIcons } from '@expo/vector-icons';
import { SymbolView } from 'expo-symbols';
import {
    Animated,
    Easing,
    LayoutAnimation,
    Platform,
    Pressable,
    StyleSheet,
    Text,
    View,
    type StyleProp,
    type ViewStyle,
} from 'react-native';

import { FONT_SIZE, MIN_TOUCH_TARGET, RADIUS, SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import { useReducedMotion } from '../useReducedMotion';
import type { ExampleAppChrome } from '../themePresets';

const DISCLOSURE_DURATION_MS = 220;
const CHEVRON_ICON_SIZE_IOS = 14;
const CHEVRON_ICON_SIZE_ANDROID = 18;
const ICON_WELL_SIZE = 28;
/** Alpha suffix for the chevron well, as 8-digit hex. */
const ICON_WELL_ALPHA = '14';

function withAlpha(hexColor: string, alpha: string): string {
    if (!hexColor.startsWith('#')) {
        return hexColor;
    }

    const hex = hexColor.slice(1);
    if (hex.length === 6) {
        return `#${hex}${alpha}`;
    }
    if (hex.length === 8) {
        return `#${hex.slice(0, 6)}${alpha}`;
    }
    return hexColor;
}

type CollapsibleSectionProps = {
    title: string;
    chrome: ExampleAppChrome;
    initiallyExpanded?: boolean;
    /** Short status beside the title, e.g. "12 active". */
    badge?: string;
    style?: StyleProp<ViewStyle>;
    children: React.ReactNode;
};

function CollapsibleSectionInner({
    title,
    chrome,
    initiallyExpanded = false,
    badge,
    style,
    children,
}: CollapsibleSectionProps) {
    const [expanded, setExpanded] = useState(initiallyExpanded);
    const reduceMotion = useReducedMotion();
    const chevronRotation = useRef(new Animated.Value(initiallyExpanded ? 1 : 0)).current;

    const toggleExpanded = useCallback(() => {
        const nextExpanded = !expanded;

        if (reduceMotion) {
            chevronRotation.setValue(nextExpanded ? 1 : 0);
        } else {
            // Height is a layout property: native layout animates it, not the JS thread.
            LayoutAnimation.configureNext({
                duration: DISCLOSURE_DURATION_MS,
                create: {
                    type: LayoutAnimation.Types.easeOut,
                    property: LayoutAnimation.Properties.opacity,
                },
                update: { type: LayoutAnimation.Types.easeOut },
                delete: {
                    type: LayoutAnimation.Types.easeOut,
                    property: LayoutAnimation.Properties.opacity,
                },
            });
            Animated.timing(chevronRotation, {
                toValue: nextExpanded ? 1 : 0,
                duration: DISCLOSURE_DURATION_MS,
                easing: Easing.out(Easing.poly(4)),
                useNativeDriver: true,
            }).start();
        }

        setExpanded(nextExpanded);
    }, [chevronRotation, expanded, reduceMotion]);

    const chevronTransform = {
        transform: [
            {
                rotate: chevronRotation.interpolate({
                    inputRange: [0, 1],
                    outputRange: ['0deg', '180deg'],
                }),
            },
        ],
    };

    return (
        <View style={style}>
            <Pressable
                accessibilityRole='button'
                accessibilityLabel={badge == null ? title : `${title}, ${badge}`}
                accessibilityState={{ expanded }}
                accessibilityHint={expanded ? 'Collapses this section.' : 'Expands this section.'}
                style={styles.header}
                onPress={toggleExpanded}>
                <View style={styles.headerText}>
                    <Text style={[sharedStyles.sectionLabel, { color: chrome.sectionLabelColor }]}>
                        {title}
                    </Text>
                    {badge != null ? (
                        <Text style={[styles.badge, { color: chrome.controlHintColor }]}>
                            {badge}
                        </Text>
                    ) : null}
                </View>
                <Animated.View
                    style={[
                        styles.iconWell,
                        { backgroundColor: withAlpha(chrome.sectionLabelColor, ICON_WELL_ALPHA) },
                        chevronTransform,
                    ]}>
                    {Platform.OS === 'ios' ? (
                        <SymbolView
                            name='chevron.down'
                            size={CHEVRON_ICON_SIZE_IOS}
                            tintColor={chrome.sectionLabelColor}
                            weight='semibold'
                        />
                    ) : (
                        <MaterialIcons
                            name='keyboard-arrow-down'
                            size={CHEVRON_ICON_SIZE_ANDROID}
                            color={chrome.sectionLabelColor}
                        />
                    )}
                </Animated.View>
            </Pressable>

            {/* Unmounted, not clipped: a collapsed panel holds ~40 controls. */}
            {expanded ? <View style={styles.body}>{children}</View> : null}
        </View>
    );
}

export const CollapsibleSection = React.memo(CollapsibleSectionInner);

const styles = StyleSheet.create({
    header: {
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: SPACE.md,
        minHeight: MIN_TOUCH_TARGET,
    },
    headerText: {
        flexShrink: 1,
        gap: SPACE.xs,
    },
    badge: {
        fontSize: FONT_SIZE.hint,
        fontWeight: '600',
    },
    iconWell: {
        width: ICON_WELL_SIZE,
        height: ICON_WELL_SIZE,
        borderRadius: RADIUS,
        alignItems: 'center',
        justifyContent: 'center',
    },
    body: {
        gap: SPACE.lg,
        paddingTop: SPACE.lg,
    },
});
