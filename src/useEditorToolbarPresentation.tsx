import { Modal, Pressable, ScrollView, Text, TouchableOpacity, View } from 'react-native';
import { useEditorToolbarState } from './useEditorToolbarState';
import { useEditorToolbarItems } from './useEditorToolbarItems';
import { useEditorToolbarInteractions } from './useEditorToolbarInteractions';
import {
    BUTTON_VISIBLE,
    TOOLBAR_PADDING_V,
    MAX_BUTTON_SIZE,
    BUTTON_HEIGHT_INSET,
    MENU_MARGIN,
    MENU_WIDTH,
    ACTIVE_COLOR,
    DEFAULT_COLOR,
    DISABLED_COLOR,
    ACTIVE_BG,
    BUTTON_RADIUS,
    styles,
    ToolbarIcon,
    TOOLBAR_RADIUS,
    resolveMentionSuggestionDisplayLabel,
    TOOLBAR_BG,
    MENU_BORDER,
} from './EditorToolbarVisuals';
import { type ToolbarButton, type ToolbarRenderedItem } from './EditorToolbarTypes';

export function useEditorToolbarPresentation(
    context: Pick<
        ReturnType<typeof useEditorToolbarState>,
        | 'menuState'
        | 'theme'
        | 'windowHeight'
        | 'windowWidth'
        | 'groupButtonRefs'
        | 'rootRef'
        | 'shouldRenderMentionSuggestions'
        | 'mentionState'
        | 'setMenuState'
        | 'menuCardRef'
    > &
        Pick<
            ReturnType<typeof useEditorToolbarItems>,
            'groupsByKey' | 'startItems' | 'scrollItems' | 'endItems'
        > &
        Pick<
            ReturnType<typeof useEditorToolbarInteractions>,
            | 'handleToolbarPressIn'
            | 'handleToolbarPressOut'
            | 'handleGroupPress'
            | 'handleButtonPress'
            | 'handleToolbarLayout'
            | 'resolvedShowTopBorder'
            | 'handleMenuLayout'
        >
) {
    const {
        menuState,
        groupsByKey,
        theme,
        windowHeight,
        windowWidth,
        groupButtonRefs,
        handleToolbarPressIn,
        handleToolbarPressOut,
        handleGroupPress,
        handleButtonPress,
        rootRef,
        handleToolbarLayout,
        resolvedShowTopBorder,
        startItems,
        shouldRenderMentionSuggestions,
        mentionState,
        setMenuState,
        scrollItems,
        endItems,
        menuCardRef,
        handleMenuLayout,
    } = context;

    const menuGroup = menuState != null ? (groupsByKey.get(menuState.groupKey) ?? null) : null;

    const menuHeight = menuGroup ? menuGroup.children.length * 40 + 16 : 0;

    // Sizing contract shared with ios/NativeEditorExpoView.swift
    // (resolvedToolbarHeight/resolvedButtonSize) and
    // android/NativeToolbar.kt (resolvedToolbarHeightDp/resolvedButtonSizeDp):
    // an explicit theme height is honored as-is; buttons are
    // max(1, min(MAX_BUTTON_SIZE, height - BUTTON_HEIGHT_INSET)).
    const resolvedToolbarHeight = Math.max(
        theme?.height ?? BUTTON_VISIBLE + TOOLBAR_PADDING_V * 2,
        1
    );

    const resolvedButtonHeight =
        theme?.height == null
            ? BUTTON_VISIBLE
            : Math.max(1, Math.min(MAX_BUTTON_SIZE, resolvedToolbarHeight - BUTTON_HEIGHT_INSET));

    const resolvedToolbarPaddingV =
        theme?.height == null
            ? TOOLBAR_PADDING_V
            : Math.max(0, (resolvedToolbarHeight - resolvedButtonHeight) / 2);

    const resolvedSeparatorHeight = Math.max(16, resolvedButtonHeight - 12);

    const menuTop =
        menuState == null
            ? 0
            : Math.max(
                  MENU_MARGIN,
                  Math.min(
                      menuState.y + menuState.height + 8,
                      windowHeight - menuHeight - MENU_MARGIN
                  )
              );

    const menuLeft =
        menuState == null
            ? 0
            : Math.max(
                  MENU_MARGIN,
                  Math.min(
                      menuState.x + menuState.width - MENU_WIDTH,
                      windowWidth - MENU_WIDTH - MENU_MARGIN
                  )
              );

    const resolveButtonVisuals = (
        button: Pick<ToolbarButton, 'buttonStyle' | 'isActive' | 'isDisabled'>
    ) => {
        const activeColor =
            button.buttonStyle?.activeColor ?? theme?.buttonActiveColor ?? ACTIVE_COLOR;
        const defaultColor = button.buttonStyle?.color ?? theme?.buttonColor ?? DEFAULT_COLOR;
        const disabledColor =
            button.buttonStyle?.disabledColor ?? theme?.buttonDisabledColor ?? DISABLED_COLOR;
        const backgroundColor =
            button.buttonStyle?.backgroundColor ?? theme?.buttonBackgroundColor ?? 'transparent';
        const activeBackgroundColor =
            button.buttonStyle?.activeBackgroundColor ??
            theme?.buttonActiveBackgroundColor ??
            ACTIVE_BG;
        const disabledBackgroundColor =
            button.buttonStyle?.disabledBackgroundColor ??
            theme?.buttonDisabledBackgroundColor ??
            (button.isActive ? activeBackgroundColor : backgroundColor);
        const requestedIconSize = button.buttonStyle?.iconSize ?? theme?.buttonIconSize;
        return {
            color: button.isDisabled ? disabledColor : button.isActive ? activeColor : defaultColor,
            backgroundColor: button.isDisabled
                ? disabledBackgroundColor
                : button.isActive
                  ? activeBackgroundColor
                  : backgroundColor,
            iconSize:
                requestedIconSize != null &&
                Number.isFinite(requestedIconSize) &&
                requestedIconSize > 0
                    ? Math.min(requestedIconSize, resolvedButtonHeight)
                    : undefined,
            borderRadius: Math.max(
                0,
                button.buttonStyle?.borderRadius ?? theme?.buttonBorderRadius ?? BUTTON_RADIUS
            ),
        };
    };

    const renderButton = (
        button: Pick<
            ToolbarButton,
            'key' | 'label' | 'icon' | 'buttonStyle' | 'isActive' | 'isDisabled'
        >,
        onPress: () => void,
        options?: {
            anchorGroupKey?: string;
            showsDisclosure?: boolean;
            expanded?: boolean;
        }
    ) => {
        const visuals = resolveButtonVisuals(button);
        const anchorGroupKey = options?.anchorGroupKey;

        return (
            <View
                key={button.key}
                ref={
                    anchorGroupKey == null
                        ? undefined
                        : (node) => {
                              if (node) {
                                  groupButtonRefs.current.set(anchorGroupKey, node);
                              } else {
                                  groupButtonRefs.current.delete(anchorGroupKey);
                              }
                          }
                }
                collapsable={false}
                style={styles.buttonAnchor}>
                <TouchableOpacity
                    onPressIn={handleToolbarPressIn}
                    onPressOut={handleToolbarPressOut}
                    onPress={onPress}
                    disabled={button.isDisabled}
                    style={[
                        styles.button,
                        {
                            height: resolvedButtonHeight,
                            borderRadius: visuals.borderRadius,
                            backgroundColor: visuals.backgroundColor,
                        },
                    ]}
                    activeOpacity={0.5}
                    accessibilityRole='button'
                    accessibilityLabel={button.label}
                    accessibilityState={{
                        selected: button.isActive,
                        disabled: button.isDisabled,
                        expanded: options?.showsDisclosure ? options.expanded : undefined,
                    }}>
                    <View>
                        <ToolbarIcon
                            icon={button.icon}
                            color={visuals.color}
                            size={visuals.iconSize}
                        />
                    </View>
                </TouchableOpacity>
                {options?.showsDisclosure ? (
                    <Text style={[styles.groupDisclosure, { color: visuals.color }]}>
                        {'\u25BE'}
                    </Text>
                ) : null}
            </View>
        );
    };

    const renderSeparator = (key: string) => (
        <View
            key={key}
            style={[
                styles.separator,
                { height: resolvedSeparatorHeight },
                theme?.separatorColor != null ? { backgroundColor: theme.separatorColor } : null,
            ]}
        />
    );

    const renderToolbarItems = (items: ToolbarRenderedItem[]) =>
        items.map((item) => {
            if (item.type === 'separator') {
                return renderSeparator(item.key);
            }
            if (item.type === 'group') {
                return renderButton(
                    {
                        key: item.group.key,
                        label: item.group.label,
                        icon: item.group.icon,
                        buttonStyle: item.group.buttonStyle,
                        isActive: item.group.isActive,
                        isDisabled: item.group.isDisabled,
                    },
                    () => handleGroupPress(item.group),
                    {
                        anchorGroupKey: item.group.key,
                        showsDisclosure: true,
                        expanded: item.group.isOpen,
                    }
                );
            }
            return renderButton(item.button, () => handleButtonPress(item.button));
        });

    return (
        <View
            ref={rootRef}
            testID='editor-toolbar-root'
            collapsable={false}
            onLayout={handleToolbarLayout}
            style={[
                styles.container,
                !resolvedShowTopBorder && styles.containerWithoutTopBorder,
                {
                    minHeight: resolvedToolbarHeight,
                    paddingVertical: resolvedToolbarPaddingV,
                },
                theme?.backgroundColor != null ? { backgroundColor: theme.backgroundColor } : null,
                theme?.borderColor != null
                    ? resolvedShowTopBorder
                        ? { borderTopColor: theme.borderColor }
                        : null
                    : null,
                theme?.borderWidth != null
                    ? resolvedShowTopBorder
                        ? { borderTopWidth: theme.borderWidth }
                        : null
                    : null,
                {
                    borderRadius: theme?.borderRadius ?? TOOLBAR_RADIUS,
                },
            ]}>
            <View style={styles.toolbarRow}>
                {startItems.length > 0 ? (
                    <View style={[styles.fixedSection, styles.startFixedSection]}>
                        {renderToolbarItems(startItems)}
                    </View>
                ) : null}
                {shouldRenderMentionSuggestions && mentionState != null ? (
                    <ScrollView
                        testID='editor-toolbar-mention-suggestions'
                        horizontal
                        showsHorizontalScrollIndicator={false}
                        style={[
                            styles.scrollSection,
                            styles.mentionSuggestionsScroll,
                            {
                                backgroundColor:
                                    mentionState.theme?.suggestions?.backgroundColor ??
                                    'transparent',
                                borderColor:
                                    mentionState.theme?.suggestions?.borderColor ?? 'transparent',
                                borderWidth: mentionState.theme?.suggestions?.borderWidth ?? 0,
                                borderRadius: mentionState.theme?.suggestions?.borderRadius ?? 0,
                            },
                            mentionState.theme?.suggestions?.shadowColor != null
                                ? {
                                      shadowColor: mentionState.theme.suggestions.shadowColor,
                                      shadowOpacity: 0.14,
                                      shadowRadius: 12,
                                      shadowOffset: { width: 0, height: 4 },
                                      elevation: 8,
                                  }
                                : null,
                        ]}
                        contentContainerStyle={styles.mentionSuggestionsContent}
                        keyboardShouldPersistTaps='always'>
                        {mentionState.suggestions.map((suggestion) => {
                            const label = resolveMentionSuggestionDisplayLabel(
                                suggestion,
                                mentionState.trigger
                            );
                            const optionTheme = (
                                mentionState.suggestionThemes?.[suggestion.key] ??
                                mentionState.theme
                            )?.suggestions?.option;
                            return (
                                <Pressable
                                    key={suggestion.key}
                                    testID={`editor-toolbar-mention-suggestion-${suggestion.key}`}
                                    accessibilityRole='button'
                                    accessibilityLabel={label}
                                    onPressIn={handleToolbarPressIn}
                                    onPressOut={handleToolbarPressOut}
                                    onPress={() => mentionState.onSelectSuggestion(suggestion)}
                                    style={({ pressed }) => [
                                        styles.mentionSuggestion,
                                        {
                                            backgroundColor: pressed
                                                ? (optionTheme?.highlightedBackgroundColor ??
                                                  'rgba(0, 122, 255, 0.12)')
                                                : (optionTheme?.backgroundColor ?? '#F2F2F7'),
                                            borderColor: optionTheme?.borderColor ?? 'transparent',
                                            borderWidth: optionTheme?.borderWidth ?? 0,
                                            borderRadius: optionTheme?.borderRadius ?? 12,
                                        },
                                    ]}>
                                    {({ pressed }) => (
                                        <>
                                            <Text
                                                numberOfLines={1}
                                                style={[
                                                    styles.mentionSuggestionTitle,
                                                    {
                                                        fontWeight:
                                                            optionTheme?.fontWeight ?? '600',
                                                        color: pressed
                                                            ? (optionTheme?.highlightedTextColor ??
                                                              optionTheme?.textColor ??
                                                              '#000000')
                                                            : (optionTheme?.textColor ?? '#000000'),
                                                    },
                                                ]}>
                                                {label}
                                            </Text>
                                            {suggestion.subtitle ? (
                                                <Text
                                                    numberOfLines={1}
                                                    style={[
                                                        styles.mentionSuggestionSubtitle,
                                                        {
                                                            color:
                                                                optionTheme?.secondaryTextColor ??
                                                                '#8E8E93',
                                                        },
                                                    ]}>
                                                    {suggestion.subtitle}
                                                </Text>
                                            ) : null}
                                        </>
                                    )}
                                </Pressable>
                            );
                        })}
                    </ScrollView>
                ) : (
                    <ScrollView
                        horizontal
                        showsHorizontalScrollIndicator={false}
                        style={styles.scrollSection}
                        contentContainerStyle={styles.scrollContent}
                        keyboardShouldPersistTaps='always'
                        onScrollBeginDrag={() => setMenuState(null)}>
                        {renderToolbarItems(scrollItems)}
                    </ScrollView>
                )}
                {endItems.length > 0 ? (
                    <View style={[styles.fixedSection, styles.endFixedSection]}>
                        {renderToolbarItems(endItems)}
                    </View>
                ) : null}
            </View>
            {!shouldRenderMentionSuggestions && menuState != null && menuGroup != null ? (
                <Modal
                    transparent
                    visible
                    animationType='fade'
                    onRequestClose={() => setMenuState(null)}>
                    <Pressable style={styles.menuBackdrop} onPress={() => setMenuState(null)}>
                        <View
                            ref={menuCardRef}
                            testID='editor-toolbar-menu-card'
                            collapsable={false}
                            onLayout={handleMenuLayout}
                            style={[
                                styles.menuCard,
                                {
                                    top: menuTop,
                                    left: menuLeft,
                                    backgroundColor: theme?.backgroundColor ?? TOOLBAR_BG,
                                    borderColor: theme?.borderColor ?? MENU_BORDER,
                                },
                            ]}>
                            {menuGroup.children.map((button) => {
                                const visuals = resolveButtonVisuals(button);
                                return (
                                    <Pressable
                                        key={button.key}
                                        onPressIn={handleToolbarPressIn}
                                        onPressOut={handleToolbarPressOut}
                                        onPress={() => handleButtonPress(button)}
                                        disabled={button.isDisabled}
                                        style={({ pressed }) => [
                                            styles.menuItem,
                                            {
                                                backgroundColor: visuals.backgroundColor,
                                            },
                                            pressed &&
                                                !button.isDisabled && {
                                                    opacity: 0.75,
                                                },
                                        ]}
                                        accessibilityRole='button'
                                        accessibilityLabel={button.label}
                                        accessibilityState={{
                                            selected: button.isActive,
                                            disabled: button.isDisabled,
                                        }}>
                                        <ToolbarIcon
                                            icon={button.icon}
                                            color={visuals.color}
                                            size={visuals.iconSize}
                                        />
                                        <Text style={[styles.menuLabel, { color: visuals.color }]}>
                                            {button.label}
                                        </Text>
                                    </Pressable>
                                );
                            })}
                        </View>
                    </Pressable>
                </Modal>
            ) : null}
        </View>
    );
}
