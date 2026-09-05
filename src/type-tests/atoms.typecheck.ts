import {
    defineAtomNode,
    type AtomComponentProps,
    type NativeProseViewerProps,
    type NativeProseViewerAtomAttrsUpdateEvent,
} from '../index';

const CounterCard = (props: AtomComponentProps) => {
    const update: Promise<void> = props.updateAttrs({ title: 'Sample item' });
    void update;
    void props.attrs;
    void props.nodeType;
    void props.selected;
    const isViewer: boolean = props.isViewer;
    const readOnly: boolean = props.readOnly;
    void isViewer;
    void readOnly;
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

const viewerProps: NativeProseViewerProps = {
    contentJSON: { type: 'doc', content: [] },
    atoms: [],
    readOnly: false,
    onUpdateAtomAttrs: async (event: NativeProseViewerAtomAttrsUpdateEvent) => {
        const position: number = event.docPos;
        void position;
        void event.attrs;
        void event.partial;
    },
};
void viewerProps;

const typedAtom = defineAtomNode({
    name: 'typedCard',
    attrs: { id: { type: 'string' }, count: { type: 'number', default: 0 } },
    idAttribute: 'id',
    html: { tag: 'div', staticAttrs: { 'data-card': 'typed' } },
    component: (props) => {
        const count: number = props.attrs.count;
        void props.updateAttrs((current) => ({ count: current.count + 1 }));
        void props.updateAttrs([{ count }, (current) => ({ count: current.count + 1 })]);
        // @ts-expect-error constrained attribute rejects a string update
        void props.updateAttrs({ count: 'bad' });
        return null;
    },
});
typedAtom.buildFragmentJson({ id: 'card-1' });
// @ts-expect-error required identifier cannot be omitted
typedAtom.buildFragmentJson();
// @ts-expect-error required identifier cannot be omitted
typedAtom.buildFragmentJson({ count: 1 });
// @ts-expect-error wrong attribute value type
typedAtom.buildFragmentJson({ id: 1 });
const explicitTyped = defineAtomNode<{ id: string; count: number }>({
    name: 'explicit',
    attrs: { id: { type: 'string' }, count: { type: 'number' } },
    html: { tag: 'div', staticAttrs: { 'data-card': 'explicit' } },
    component: () => null,
});
explicitTyped.buildFragmentJson({ id: 'x', count: 1 });

const structured = defineAtomNode({
    name: 'structured',
    attrs: { data: { default: { count: 0 } }, list: { default: [1, 2] } },
    html: { tag: 'div', staticAttrs: { 'data-card': 'structured' } },
    component: (props) => {
        void props.updateAttrs({ data: { count: 1 }, list: [3] });
        return null;
    },
});
structured.buildFragmentJson({ data: { count: 1 }, list: [3] });
const undefinedDefault = defineAtomNode({
    name: 'required',
    attrs: { id: { type: 'string', default: undefined } },
    html: { tag: 'div', staticAttrs: { 'data-card': 'required' } },
    component: () => null,
});
// @ts-expect-error undefined is not a default
undefinedDefault.buildFragmentJson();
defineAtomNode<{ count: number }>({
    name: 'mismatched',
    // @ts-expect-error runtime declaration must agree with the explicit attribute type
    attrs: { count: { type: 'string', default: 'wrong' } },
    html: { tag: 'div', staticAttrs: { 'data-card': 'mismatched' } },
    component: () => null,
});
const emptyList = defineAtomNode({
    name: 'list',
    attrs: { items: { default: [] } },
    html: { tag: 'div', staticAttrs: { 'data-card': 'list' } },
    component: (props) => {
        void props.updateAttrs({ items: [1] });
        return null;
    },
});
emptyList.buildFragmentJson({ items: [1] });
defineAtomNode<{ label?: string }>({
    name: 'optional',
    // @ts-expect-error optional strings cannot declare object values
    attrs: { label: { type: 'object' } },
    html: { tag: 'div', staticAttrs: { 'data-card': 'optional' } },
    component: () => null,
});
const extended = defineAtomNode({
    ...structured,
    component: () => null,
    attrs: { ...structured.attrs, id: { type: 'string' } },
    idAttribute: 'id',
});
extended.buildFragmentJson({ id: 'one' });
