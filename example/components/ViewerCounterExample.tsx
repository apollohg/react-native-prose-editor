import { useCallback, useMemo, useState } from 'react';
import {
    NativeProseViewer,
    type NativeProseViewerAtomAttrsUpdateEvent,
} from 'react-native-rich-text-editor';

import { editorTheme } from '../theme';
import { counterCardAtom } from './CounterCard';

const atoms = [counterCardAtom];

export function ViewerCounterExample({ readOnly = false }: { readOnly?: boolean }) {
    const [attrs, setAttrs] = useState<Record<string, unknown>>({
        title: 'Viewer counter',
        count: 2,
    });
    const contentJSON = useMemo(() => counterCardAtom.buildFragmentJson(attrs), [attrs]);
    const updateAttrs = useCallback(({ partial }: NativeProseViewerAtomAttrsUpdateEvent) => {
        setAttrs((current) => ({ ...current, ...partial }));
    }, []);

    return (
        <NativeProseViewer
            contentJSON={contentJSON}
            atoms={atoms}
            theme={editorTheme}
            readOnly={readOnly}
            onUpdateAtomAttrs={updateAttrs}
        />
    );
}
