import {
    defineAtomNode,
    serializeEditorAtoms,
    type AtomComponent,
    type AtomNodeConfig,
    withAtomsSchema,
} from '../atoms';
import { defaultSchema, defineSchema, imageNodeSpec, type SchemaNodeSpec } from '../schemas';

const counterConfig = {
    name: 'counterCard',
    attrs: { title: { default: '' }, count: { default: 0 } },
    html: {
        tag: 'div',
        staticAttrs: { 'data-type': 'counter-card' },
        attrMap: { title: 'data-title', count: 'data-count' },
    },
    component: (() => null) as AtomComponent,
    estimatedHeight: 120,
} satisfies AtomNodeConfig;

const minimalSchemaNodes: Readonly<Record<string, SchemaNodeSpec>> = {
    doc: { content: 'block+' },
    paragraph: { content: 'inline*', group: 'block' },
    text: { group: 'inline' },
};

test('defineAtomNode compiles a void block node spec', () => {
    const definition = defineAtomNode(counterConfig);

    expect(definition.nodeSpec).toEqual({
        name: 'counterCard',
        content: '',
        group: 'block',
        role: 'block',
        isVoid: true,
        attrs: counterConfig.attrs,
        html: counterConfig.html,
    });
});

test('atom and image helpers declare non-default collapsed-backspace policy only', () => {
    expect(defineAtomNode(counterConfig).nodeSpec.deletableOnBackspace).toBeUndefined();
    expect(imageNodeSpec('photo').deletableOnBackspace).toBe(false);
});

test('attrMap auto-derives kebab-cased data attributes when omitted', () => {
    const definition = defineAtomNode({
        ...counterConfig,
        attrs: { restSeconds: { default: 0 } },
        html: { tag: 'div', staticAttrs: { 'data-type': 'counter-card' } },
    });

    expect(definition.nodeSpec.html?.attrMap).toEqual({
        restSeconds: 'data-rest-seconds',
    });
});

test.each(['__opaque', '__opaque_json', '__skip'])('rejects reserved name %s', (name) => {
    expect(() => defineAtomNode({ ...counterConfig, name })).toThrow(/reserved/i);
});

test('rejects identifiers outside the atom HTML policy', () => {
    for (const tag of ['script', 'img', 'br', 'DIV', '1x']) {
        expect(() =>
            defineAtomNode({
                ...counterConfig,
                html: { ...counterConfig.html, tag },
            })
        ).toThrow(/tag/i);
    }
    for (const attr of ['onclick', 'style', 'href', 'ONERROR']) {
        expect(() =>
            defineAtomNode({
                ...counterConfig,
                html: { ...counterConfig.html, staticAttrs: { [attr]: 'x' } },
            })
        ).toThrow(/attr/i);
    }
});

test('rejects incomplete or colliding attr maps', () => {
    expect(() =>
        defineAtomNode({
            ...counterConfig,
            html: { ...counterConfig.html, attrMap: { title: 'data-title' } },
        })
    ).toThrow(/count/);
    expect(() =>
        defineAtomNode({
            ...counterConfig,
            html: {
                ...counterConfig.html,
                attrMap: { title: 'data-x', count: 'data-x' },
            },
        })
    ).toThrow(/data-x/);
    expect(() =>
        defineAtomNode({
            ...counterConfig,
            html: {
                ...counterConfig.html,
                attrMap: { title: 'data-type', count: 'data-count' },
            },
        })
    ).toThrow(/data-type/);
});

test('rejects a missing component', () => {
    expect(() => defineAtomNode({ ...counterConfig, component: undefined as never })).toThrow(
        /component/i
    );
});

test('buildFragmentJson wraps one atom node', () => {
    const definition = defineAtomNode(counterConfig);

    expect(definition.buildFragmentJson({ title: 'Sample item', count: 5 })).toEqual({
        type: 'doc',
        content: [{ type: 'counterCard', attrs: { title: 'Sample item', count: 5 } }],
    });
});

test('withAtomsSchema rejects non-conflicting same-tag rules', () => {
    const definition = defineAtomNode(counterConfig);
    const subset = defineAtomNode({
        ...counterConfig,
        name: 'alternateCard',
        html: {
            tag: 'div',
            staticAttrs: {
                'data-type': 'counter-card',
                'data-kind': 'sample',
            },
            attrMap: counterConfig.html.attrMap,
        },
    });

    expect(() => withAtomsSchema(defaultSchema, [definition, subset])).toThrow(/ambiguous/i);
});

test('withAtomsSchema adds nodes once and rejects conflicting redefinitions', () => {
    const definition = defineAtomNode(counterConfig);
    const schema = withAtomsSchema(defaultSchema, [definition]);

    expect(withAtomsSchema(schema, [definition])).toEqual(schema);
    const conflicting = defineAtomNode({
        ...counterConfig,
        html: {
            ...counterConfig.html,
            staticAttrs: { 'data-type': 'different-card' },
        },
    });
    expect(() => withAtomsSchema(schema, [conflicting])).toThrow(/counterCard/);
});

test('defineSchema accepts an atoms key', () => {
    const definition = defineAtomNode(counterConfig);
    const schema = defineSchema({ nodes: minimalSchemaNodes, atoms: [definition] });

    expect(schema.nodes.some((node) => node.name === 'counterCard')).toBe(true);
});

test('serializeEditorAtoms emits node types and estimated heights', () => {
    const definition = defineAtomNode(counterConfig);

    expect(JSON.parse(serializeEditorAtoms([definition])!)).toEqual({
        nodeTypes: ['counterCard'],
        estimatedHeights: { counterCard: 120 },
    });
    expect(serializeEditorAtoms([])).toBeUndefined();
    expect(serializeEditorAtoms(undefined)).toBeUndefined();
});

test('atoms without an estimated height reserve the default chip height', () => {
    const definition = defineAtomNode({ ...counterConfig, estimatedHeight: undefined });

    expect(definition.estimatedHeight).toBe(32);
    expect(JSON.parse(serializeEditorAtoms([definition])!)).toEqual({
        nodeTypes: ['counterCard'],
        estimatedHeights: { counterCard: 32 },
    });
});
