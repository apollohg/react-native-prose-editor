import { useCallback, useMemo, useState } from 'react';
import {
    RichTextViewer,
    defineAtomNode,
    type RichTextViewerAtomAttrsUpdateEvent,
} from 'react-native-rich-text-editor';

import { editorTheme } from '../theme';
import { counterCardAtom } from './CounterCard';

const viewerCounterAtom = defineAtomNode({
    ...counterCardAtom,
    component: (props) => {
        const Counter = counterCardAtom.component;
        return <Counter {...props} />;
    },
    attrs: { ...counterCardAtom.attrs, id: { type: 'string' } },
    idAttribute: 'id',
});
const atoms = [viewerCounterAtom];

export function ViewerCounterExample({ readOnly = false }: { readOnly?: boolean }) {
    const [attrs, setAttrs] = useState({
        id: 'viewer-counter',
        title: 'Viewer counter',
        count: 2,
    });
    const contentJSON = useMemo(() => viewerCounterAtom.buildFragmentJson(attrs), [attrs]);
    const updateAttrs = useCallback(({ partial }: RichTextViewerAtomAttrsUpdateEvent) => {
        setAttrs((current) => ({ ...current, ...partial }));
    }, []);

    return (
        <RichTextViewer
            contentJSON={contentJSON}
            atoms={atoms}
            theme={editorTheme}
            readOnly={readOnly}
            onUpdateAtomAttrs={updateAttrs}
        />
    );
}
