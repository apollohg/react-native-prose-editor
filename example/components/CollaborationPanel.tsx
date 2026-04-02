import React from 'react';
import { Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import type { CollaborationPeer } from '@apollohg/react-native-prose-editor';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';
import { CollapsibleSection } from './CollapsibleSection';

type CollaborationPanelProps = {
    collaborationEnabled: boolean;
    onCollaborationEnabledChange: (value: boolean) => void;
    collaborationEndpoint: string;
    onCollaborationEndpointChange: (value: string) => void;
    collaborationRoomId: string;
    onCollaborationRoomIdChange: (value: string) => void;
    collaborationDisplayName: string;
    onCollaborationDisplayNameChange: (value: string) => void;
    collaborationStatusText: string;
    collaborationLastErrorMessage?: string | null;
    collaborationIsConnected: boolean;
    remotePeers: readonly CollaborationPeer[];
    onConnect: () => void;
    onDisconnect: () => void;
    appChrome: ExampleThemePreset['appChrome'];
};

export function CollaborationPanel({
    collaborationEnabled,
    onCollaborationEnabledChange,
    collaborationEndpoint,
    onCollaborationEndpointChange,
    collaborationRoomId,
    onCollaborationRoomIdChange,
    collaborationDisplayName,
    onCollaborationDisplayNameChange,
    collaborationStatusText,
    collaborationLastErrorMessage,
    collaborationIsConnected,
    remotePeers,
    onConnect,
    onDisconnect,
    appChrome,
}: CollaborationPanelProps) {
    return (
        <CollapsibleSection
            title='Collaboration'
            appChrome={appChrome}
            style={[styles.card, { backgroundColor: appChrome.cardBackgroundColor }]}>
            <View style={sharedStyles.settingsPanel}>
                <View style={styles.switchRow}>
                    <Text
                        style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                        Enable Collaboration
                    </Text>
                    <Pressable
                        style={[
                            styles.toggle,
                            {
                                backgroundColor: collaborationEnabled
                                    ? appChrome.tabActiveBackgroundColor
                                    : appChrome.tabBackgroundColor,
                                borderColor: collaborationEnabled
                                    ? appChrome.tabActiveBorderColor
                                    : appChrome.tabBorderColor,
                            },
                        ]}
                        onPress={() => onCollaborationEnabledChange(!collaborationEnabled)}>
                        <Text
                            style={[
                                styles.toggleText,
                                {
                                    color: collaborationEnabled
                                        ? appChrome.tabActiveTextColor
                                        : appChrome.tabTextColor,
                                },
                            ]}>
                            {collaborationEnabled ? 'On' : 'Off'}
                        </Text>
                    </Pressable>
                </View>

                <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
                    The example expects a backend that speaks standard Yjs sync and awareness over
                    WebSocket.
                </Text>

                <View style={sharedStyles.inputGroup}>
                    <Text
                        style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                        WebSocket Endpoint
                    </Text>
                    <TextInput
                        value={collaborationEndpoint}
                        onChangeText={onCollaborationEndpointChange}
                        autoCapitalize='none'
                        autoCorrect={false}
                        editable={!collaborationIsConnected}
                        placeholder='ws://localhost:1234/collaboration'
                        placeholderTextColor={appChrome.controlHintColor}
                        style={[
                            styles.input,
                            {
                                color: appChrome.titleColor,
                                backgroundColor: appChrome.cardSecondaryBackgroundColor,
                                borderColor: appChrome.tabBorderColor,
                            },
                        ]}
                    />
                </View>

                <View style={styles.inputRow}>
                    <View style={sharedStyles.inputGroup}>
                        <Text
                            style={[
                                sharedStyles.controlLabel,
                                { color: appChrome.controlLabelColor },
                            ]}>
                            Room ID
                        </Text>
                        <TextInput
                            value={collaborationRoomId}
                            onChangeText={onCollaborationRoomIdChange}
                            autoCapitalize='none'
                            autoCorrect={false}
                            editable={!collaborationIsConnected}
                            placeholder='example-room'
                            placeholderTextColor={appChrome.controlHintColor}
                            style={[
                                styles.input,
                                {
                                    color: appChrome.titleColor,
                                    backgroundColor: appChrome.cardSecondaryBackgroundColor,
                                    borderColor: appChrome.tabBorderColor,
                                },
                            ]}
                        />
                    </View>

                    <View style={sharedStyles.inputGroup}>
                        <Text
                            style={[
                                sharedStyles.controlLabel,
                                { color: appChrome.controlLabelColor },
                            ]}>
                            Local Name
                        </Text>
                        <TextInput
                            value={collaborationDisplayName}
                            onChangeText={onCollaborationDisplayNameChange}
                            autoCorrect={false}
                            placeholder='Your name'
                            placeholderTextColor={appChrome.controlHintColor}
                            style={[
                                styles.input,
                                {
                                    color: appChrome.titleColor,
                                    backgroundColor: appChrome.cardSecondaryBackgroundColor,
                                    borderColor: appChrome.tabBorderColor,
                                },
                            ]}
                        />
                    </View>
                </View>

                <View
                    style={[
                        styles.statusCard,
                        {
                            backgroundColor: appChrome.cardSecondaryBackgroundColor,
                            borderColor: appChrome.tabBorderColor,
                        },
                    ]}>
                    <Text
                        style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                        Status
                    </Text>
                    <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
                        {collaborationStatusText}
                    </Text>
                    {collaborationLastErrorMessage ? (
                        <Text style={[sharedStyles.controlHint, { color: '#c4432b' }]}>
                            {collaborationLastErrorMessage}
                        </Text>
                    ) : null}
                </View>

                <View style={styles.buttonRow}>
                    <Pressable
                        style={[
                            styles.actionButton,
                            { backgroundColor: appChrome.actionButtonBackgroundColor },
                            !collaborationEnabled && styles.actionButtonDisabled,
                        ]}
                        disabled={!collaborationEnabled}
                        onPress={onConnect}>
                        <Text
                            style={[
                                styles.actionButtonText,
                                { color: appChrome.actionButtonTextColor },
                            ]}>
                            Connect
                        </Text>
                    </Pressable>

                    <Pressable
                        style={[
                            styles.actionButton,
                            { backgroundColor: appChrome.actionButtonBackgroundColor },
                            !collaborationEnabled && styles.actionButtonDisabled,
                        ]}
                        disabled={!collaborationEnabled}
                        onPress={onDisconnect}>
                        <Text
                            style={[
                                styles.actionButtonText,
                                { color: appChrome.actionButtonTextColor },
                            ]}>
                            Disconnect
                        </Text>
                    </Pressable>
                </View>

                <View style={styles.peerList}>
                    {remotePeers.length === 0 ? (
                        <Text
                            style={[
                                sharedStyles.controlHint,
                                { color: appChrome.controlHintColor },
                            ]}>
                            No remote peers yet.
                        </Text>
                    ) : (
                        remotePeers.map((peer) => (
                            <CollaborationPeerRow
                                key={peer.clientId}
                                peer={peer}
                                appChrome={appChrome}
                            />
                        ))
                    )}
                </View>
            </View>
        </CollapsibleSection>
    );
}

function CollaborationPeerRow({
    peer,
    appChrome,
}: {
    peer: CollaborationPeer;
    appChrome: ExampleThemePreset['appChrome'];
}) {
    const state =
        peer.state && typeof peer.state === 'object'
            ? (peer.state as Record<string, unknown>)
            : null;
    const user =
        state?.user && typeof state.user === 'object'
            ? (state.user as Record<string, unknown>)
            : null;
    const label =
        typeof user?.name === 'string' && user.name.length > 0
            ? user.name
            : `Peer ${peer.clientId}`;

    return (
        <View
            style={[
                styles.peerRow,
                {
                    backgroundColor: appChrome.cardSecondaryBackgroundColor,
                    borderColor: appChrome.tabBorderColor,
                },
            ]}>
            <View
                style={[
                    styles.peerSwatch,
                    {
                        backgroundColor: typeof user?.color === 'string' ? user.color : '#007AFF',
                    },
                ]}
            />
            <View style={styles.peerText}>
                <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                    {label}
                </Text>
                <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
                    client {peer.clientId}
                </Text>
            </View>
        </View>
    );
}

const styles = StyleSheet.create({
    card: {
        padding: 16,
        borderRadius: 18,
    },
    switchRow: {
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
    },
    toggle: {
        paddingHorizontal: 14,
        paddingVertical: 8,
        borderRadius: 999,
        borderWidth: 1,
        minWidth: 60,
        alignItems: 'center',
    },
    toggleText: {
        fontSize: 13,
        fontWeight: '700',
    },
    inputRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        justifyContent: 'space-between',
        gap: 12,
    },
    input: {
        borderWidth: 1,
        borderRadius: 12,
        paddingHorizontal: 12,
        paddingVertical: 10,
        fontSize: 14,
    },
    statusCard: {
        gap: 4,
        borderWidth: 1,
        borderRadius: 14,
        padding: 12,
    },
    buttonRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: 10,
    },
    actionButton: {
        paddingHorizontal: 14,
        paddingVertical: 10,
        borderRadius: 999,
    },
    actionButtonDisabled: {
        opacity: 0.45,
    },
    actionButtonText: {
        fontWeight: '700',
    },
    peerList: {
        gap: 10,
    },
    peerRow: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: 12,
        borderWidth: 1,
        borderRadius: 14,
        paddingHorizontal: 12,
        paddingVertical: 10,
    },
    peerSwatch: {
        width: 12,
        height: 12,
        borderRadius: 999,
    },
    peerText: {
        gap: 2,
    },
});
