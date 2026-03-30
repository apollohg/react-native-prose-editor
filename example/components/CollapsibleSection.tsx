import React, { useRef, useState } from 'react';
import { MaterialIcons } from '@expo/vector-icons';
import { SymbolView } from 'expo-symbols';
import {
  Animated,
  Easing,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
  type StyleProp,
  type ViewStyle,
} from 'react-native';
import { sharedStyles } from '../sharedStyles';
import type { ExampleThemePreset } from '../themePresets';

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
  appChrome: ExampleThemePreset['appChrome'];
  initiallyExpanded?: boolean;
  style?: StyleProp<ViewStyle>;
  children: React.ReactNode;
};

export function CollapsibleSection({
  title,
  appChrome,
  initiallyExpanded = false,
  style,
  children,
}: CollapsibleSectionProps) {
  const [expanded, setExpanded] = useState(initiallyExpanded);
  const [contentHeight, setContentHeight] = useState(0);
  const chevronRotation = useRef(new Animated.Value(initiallyExpanded ? 1 : 0)).current;
  const bodyAnimation = useRef(new Animated.Value(initiallyExpanded ? 1 : 0)).current;

  const toggleExpanded = () => {
    const nextExpanded = !expanded;
    Animated.timing(chevronRotation, {
      toValue: nextExpanded ? 1 : 0,
      duration: 220,
      easing: Easing.out(Easing.cubic),
      useNativeDriver: true,
    }).start();
    Animated.timing(bodyAnimation, {
      toValue: nextExpanded ? 1 : 0,
      duration: 220,
      easing: Easing.out(Easing.cubic),
      useNativeDriver: false,
    }).start();
    setExpanded(nextExpanded);
  };

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
  const animatedBodyStyle = {
    height: bodyAnimation.interpolate({
      inputRange: [0, 1],
      outputRange: [0, Math.max(contentHeight, 1)],
    }),
    marginTop: bodyAnimation.interpolate({
      inputRange: [0, 1],
      outputRange: [0, 16],
    }),
    opacity: bodyAnimation.interpolate({
      inputRange: [0, 1],
      outputRange: [0, 1],
    }),
  };

  return (
    <View style={style}>
      <Pressable
        style={styles.header}
        onPress={toggleExpanded}
      >
        <Text style={[sharedStyles.sectionLabel, { color: appChrome.sectionLabelColor }]}>
          {title}
        </Text>
        <Animated.View
          style={[
            styles.iconWrapper,
            {
              backgroundColor: withAlpha(appChrome.sectionLabelColor, '14'),
            },
            chevronTransform,
          ]}
        >
          {Platform.OS === 'ios' ? (
            <SymbolView
              name="chevron.down"
              size={14}
              tintColor={appChrome.sectionLabelColor}
              weight="semibold"
            />
          ) : (
            <MaterialIcons
              name="keyboard-arrow-down"
              size={18}
              color={appChrome.sectionLabelColor}
            />
          )}
        </Animated.View>
      </Pressable>
      <Animated.View style={[styles.body, animatedBodyStyle]}>
        <View
          style={styles.bodyContent}
          onLayout={(event) => {
            const nextHeight = event.nativeEvent.layout.height;
            if (nextHeight > 0 && nextHeight !== contentHeight) {
              setContentHeight(nextHeight);
            }
          }}
        >
          {children}
        </View>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  iconWrapper: {
    width: 28,
    height: 28,
    borderRadius: 14,
    alignItems: 'center',
    justifyContent: 'center',
  },
  body: {
    overflow: 'hidden',
  },
  bodyContent: {
    gap: 16,
  },
});
