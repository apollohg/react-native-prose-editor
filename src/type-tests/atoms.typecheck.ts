import { defineAtomNode, type AtomComponentProps } from '../index';

const CounterCard = (props: AtomComponentProps) => {
    const update: Promise<void> = props.updateAttrs({ title: 'Sample item' });
    void update;
    void props.attrs;
    void props.nodeType;
    void props.selected;
    return null;
};

defineAtomNode({
    name: 'counterCard',
    attrs: { title: { default: '' } },
    html: {
        tag: 'div',
        staticAttrs: { 'data-type': 'counter-card' },
        attrMap: { title: 'data-title' },
    },
    component: CounterCard,
});

defineAtomNode({
    name: 'invalidAtom',
    html: { tag: 'div', staticAttrs: { 'data-type': 'invalid-atom' } },
    component: CounterCard,
    // @ts-expect-error atom nodes cannot admit undeclared attributes
    allowUndeclaredAttrs: true,
});
