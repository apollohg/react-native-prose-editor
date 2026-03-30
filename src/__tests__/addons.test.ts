import {
    MENTION_NODE_NAME,
    buildMentionFragmentJson,
    normalizeEditorAddons,
    serializeEditorAddons,
    withMentionsSchema,
} from '../addons';
import { tiptapSchema } from '../schemas';

describe('mentions addon helpers', () => {
    it('extends the schema with a mention node exactly once', () => {
        const once = withMentionsSchema(tiptapSchema);
        const twice = withMentionsSchema(once);

        expect(once.nodes.filter((node) => node.name === MENTION_NODE_NAME)).toHaveLength(1);
        expect(twice.nodes.filter((node) => node.name === MENTION_NODE_NAME)).toHaveLength(1);
        expect(once.nodes.find((node) => node.name === MENTION_NODE_NAME)).toEqual({
            name: 'mention',
            content: '',
            group: 'inline',
            role: 'inline',
            isVoid: true,
            attrs: {
                label: { default: null },
            },
        });
    });

    it('normalizes mention suggestions with default trigger, label, and attrs', () => {
        const normalized = normalizeEditorAddons({
            mentions: {
                suggestions: [
                    {
                        key: 'u1',
                        title: 'Alice',
                        subtitle: 'Design',
                        attrs: { id: 'u1', kind: 'user' },
                    },
                ],
            },
        });

        expect(normalized).toEqual({
            mentions: {
                trigger: '@',
                suggestions: [
                    {
                        key: 'u1',
                        title: 'Alice',
                        subtitle: 'Design',
                        label: '@Alice',
                        attrs: {
                            label: '@Alice',
                            id: 'u1',
                            kind: 'user',
                        },
                    },
                ],
            },
        });
    });

    it('serializes mention addon config for native consumption', () => {
        const serialized = serializeEditorAddons({
            mentions: {
                trigger: '@',
                theme: { textColor: '#112233', popoverBackgroundColor: '#ffffff' },
                suggestions: [
                    {
                        key: 'u1',
                        title: 'Alice',
                        label: '@Alice',
                        attrs: { id: 'u1' },
                    },
                ],
            },
        });

        expect(serialized).toBe(
            JSON.stringify({
                mentions: {
                    trigger: '@',
                    theme: { textColor: '#112233', popoverBackgroundColor: '#ffffff' },
                    suggestions: [
                        {
                            key: 'u1',
                            title: 'Alice',
                            label: '@Alice',
                            attrs: { label: '@Alice', id: 'u1' },
                        },
                    ],
                },
            })
        );
    });

    it('builds a mention fragment JSON payload that preserves custom attrs', () => {
        expect(
            buildMentionFragmentJson({
                id: 'u1',
                kind: 'user',
                label: '@Alice',
            })
        ).toEqual({
            type: 'doc',
            content: [
                {
                    type: 'mention',
                    attrs: {
                        id: 'u1',
                        kind: 'user',
                        label: '@Alice',
                    },
                },
            ],
        });
    });
});
