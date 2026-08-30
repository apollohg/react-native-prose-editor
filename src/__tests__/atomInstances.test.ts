import { applyRenderPatch, atomSelected, collectAtomInstances } from '../atomInstances';
import type { RenderElement, Selection } from '../NativeEditorBridge';

const registered = new Set(['counterCard']);
const card = (docPos: number, atomId?: string): RenderElement => ({
    type: 'voidBlock',
    nodeType: 'counterCard',
    docPos,
    attrs: { title: 't' },
    ...(atomId ? { atomId } : {}),
});
const paragraph: RenderElement[] = [
    { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
    { type: 'textRun', text: 'x', marks: [] },
    { type: 'blockEnd' },
];

test('collects registered voidBlocks with atomId keys', () => {
    const instances = collectAtomInstances(
        [[card(1, 'y1-9')], paragraph, [card(5, 'y1-3')]],
        registered
    );

    expect(instances).toEqual([
        { key: 'y1-9', nodeType: 'counterCard', attrs: { title: 't' }, docPos: 1 },
        { key: 'y1-3', nodeType: 'counterCard', attrs: { title: 't' }, docPos: 5 },
    ]);
});

test('falls back to per-type occurrence keys without atomId', () => {
    const instances = collectAtomInstances([[card(1)], [card(5)]], registered);

    expect(instances.map((instance) => instance.key)).toEqual(['counterCard:0', 'counterCard:1']);
});

test('collects unregistered non-native voidBlocks as chip instances', () => {
    const mystery: RenderElement = {
        type: 'voidBlock',
        nodeType: 'callout',
        docPos: 3,
    };

    expect(collectAtomInstances([[mystery]], registered)).toEqual([
        { key: 'callout:0', nodeType: 'callout', attrs: {}, docPos: 3 },
    ]);
});

test('never collects natively-known void blocks', () => {
    const horizontalRule: RenderElement = {
        type: 'voidBlock',
        nodeType: 'horizontalRule',
        docPos: 3,
    };

    expect(collectAtomInstances([[horizontalRule], paragraph], registered)).toEqual([]);
});

test('applyRenderPatch splices like the engine contract', () => {
    const previous: RenderElement[][] = [[card(1, 'a')], paragraph, [card(6, 'b')]];
    const next = applyRenderPatch(previous, {
        startIndex: 1,
        deleteCount: 1,
        renderBlocks: [paragraph, paragraph],
    });

    expect(next).toHaveLength(4);
    expect(next[0]).toBe(previous[0]);
});

test.each([
    [{ type: 'node', pos: 4 } as Selection, 4, true],
    [{ type: 'node', pos: 3 } as Selection, 4, false],
    [{ type: 'text', anchor: 2, head: 7 } as Selection, 4, true],
    [{ type: 'text', anchor: 7, head: 2 } as Selection, 4, true],
    [{ type: 'text', anchor: 4, head: 4 } as Selection, 4, false],
    [{ type: 'all' } as Selection, 4, true],
] as const)('atomSelected(%j, %i) is %s', (selection, docPos, expected) => {
    expect(atomSelected(selection, docPos)).toBe(expected);
});
