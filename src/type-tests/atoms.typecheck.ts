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
