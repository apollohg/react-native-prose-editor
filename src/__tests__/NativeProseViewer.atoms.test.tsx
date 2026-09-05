jest.mock('../specs/PreparedProseViewerNativeComponent', () => {
    const React = require('react');
    const { View } = require('react-native');
    return React.forwardRef((props: Record<string, unknown>, _ref: React.Ref<unknown>) => (
        <View testID='prepared-prose-viewer' {...props} />
    ));
});

import React from 'react';
import { View } from 'react-native';
import { act, fireEvent, render } from '@testing-library/react-native';
import { NativeProseViewer } from '../NativeProseViewer';
import { defineAtomNode, type AtomComponentProps } from '../atoms';

const Counter = (props: AtomComponentProps) => <View testID='counter' atomProps={props} />;
const atom = defineAtomNode({
    name: 'counterCard',
    attrs: { count: { default: 0 } },
    html: { tag: 'div', staticAttrs: { 'data-counter': '' } },
    component: Counter,
    estimatedHeight: 80,
});
const atoms = [atom];
const content = { type: 'doc', content: [{ type: 'counterCard', attrs: { count: 2 } }] };
const position = {
    nodeType: atom.name,
    docPos: 0,
    attrsJson: '{"count":2}',
    x: 12,
    y: 24,
    width: 276,
    height: 80,
};

function publish(view: ReturnType<typeof render>, positions = [position]) {
    const native = view.getByTestId('prepared-prose-viewer');
    const configuration = JSON.parse(native.props.themeJson).viewerAtoms;
    fireEvent(native, 'atomLayout', {
        nativeEvent: {
            generation: configuration.generation,
            revision: configuration.revision,
            layoutWidth: 300,
            atomsJson: JSON.stringify(positions),
        },
    });
}

describe('NativeProseViewer custom atoms', () => {
    it('allows read-only interactions while rejecting mutations', async () => {
        const view = render(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        const props = view.getByTestId('counter').props.atomProps;
        expect(props.interactive).toBe(true);
        await act(async () => {
            await expect(props.updateAttrs({ count: 3 })).rejects.toMatchObject({
                code: 'not-applicable',
            });
        });
    });

    it('preserves explicit identity when content before an atom moves', () => {
        const mounted = jest.fn();
        const Card = () => {
            React.useEffect(mounted, []);
            return <View testID='identified' />;
        };
        const definitions = [
            {
                ...atom,
                idAttribute: 'id',
                attrs: { ...atom.attrs, id: { default: '' } },
                component: Card,
            },
        ];
        const view = render(<NativeProseViewer contentJSON={content} atoms={definitions} />);
        publish(view, [{ ...position, attrsJson: '{"id":"one","count":2}' }]);
        view.rerender(<NativeProseViewer contentJSON={{ ...content }} atoms={definitions} />);
        publish(view, [{ ...position, docPos: 10, attrsJson: '{"id":"one","count":2}' }]);
        expect(mounted).toHaveBeenCalledTimes(1);
    });

    it('rejects duplicate identities and reports the invalid layout', () => {
        const definitions = [{ ...atom, idAttribute: 'id' }];
        const onError = jest.fn();
        const view = render(
            <NativeProseViewer contentJSON={content} atoms={definitions} onError={onError} />
        );
        publish(
            view,
            [0, 1].map((docPos) => ({ ...position, docPos, attrsJson: '{"id":"same","count":2}' }))
        );
        expect(onError).toHaveBeenCalledWith(
            expect.objectContaining({ code: 'INVALID_ATOM_LAYOUT' })
        );
        expect(view.queryByTestId('counter')).toBeNull();
    });

    it('composes queued functional updates from acknowledged attrs', async () => {
        const onUpdateAtomAttrs = jest.fn();
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={atoms}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        publish(view);
        const update = view.getByTestId('counter').props.atomProps.updateAttrs;
        await act(async () => {
            await Promise.all([
                update((a: any) => ({ count: a.count + 1 })),
                update((a: any) => ({ count: a.count + 1 })),
            ]);
        });
        expect(onUpdateAtomAttrs.mock.calls.map(([event]) => event.partial.count)).toEqual([3, 4]);
    });

    it('composes schemas and mounts native HTML atom snapshots as read-only viewer components', () => {
        const view = render(
            <NativeProseViewer
                contentHTML='<div data-counter data-count="2"></div>'
                atoms={atoms}
            />
        );
        const native = view.getByTestId('prepared-prose-viewer');
        expect(JSON.parse(native.props.configJson).schema.nodes).toContainEqual(atom.nodeSpec);
        publish(view);
        expect(view.getByTestId('counter').props.atomProps).toMatchObject({
            attrs: { count: 2 },
            nodeType: 'counterCard',
            selected: false,
            readOnly: true,
            isViewer: true,
        });
    });

    it('preserves component state and measured height through controlled attribute updates', async () => {
        const mounted = jest.fn();
        const unmounted = jest.fn();
        const StatefulCounter = (props: AtomComponentProps) => {
            const [draft, setDraft] = React.useState('');
            React.useEffect(() => {
                mounted();
                return unmounted;
            }, []);
            return (
                <View
                    testID='stateful-counter'
                    atomProps={props}
                    draft={draft}
                    onChange={setDraft}
                />
            );
        };
        const definitions = [
            {
                ...atom,
                idAttribute: 'id',
                attrs: { ...atom.attrs, id: { default: 'one' } },
                component: StatefulCounter,
            },
        ];
        const onUpdateAtomAttrs = jest.fn();
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={definitions}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        publish(view, [{ ...position, attrsJson: '{"count":2,"id":"one"}' }]);
        fireEvent(view.getByTestId('stateful-counter'), 'change', 'local draft');
        fireEvent(view.getByTestId('stateful-counter').parent!, 'layout', {
            nativeEvent: { layout: { width: 276, height: 123, x: 0, y: 0 } },
        });
        const oldUpdate = view.getByTestId('stateful-counter').props.atomProps.updateAttrs;
        view.rerender(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'counterCard', attrs: { count: 3 } }],
                }}
                atoms={definitions}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        expect(unmounted).not.toHaveBeenCalled();
        await act(async () => {
            await expect(oldUpdate({ count: 4 })).rejects.toMatchObject({ code: 'stale-revision' });
        });
        publish(view, [{ ...position, attrsJson: '{"count":3,"id":"one"}' }]);
        expect(view.getByTestId('stateful-counter').props.draft).toBe('local draft');
        expect(view.getByTestId('stateful-counter').props.atomProps.attrs.count).toBe(3);
        expect(mounted).toHaveBeenCalledTimes(1);
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements['0']
        ).toEqual({ width: 276, height: 123 });
        await act(async () => {
            await expect(oldUpdate({ count: 4 })).rejects.toMatchObject({ code: 'stale-revision' });
        });
        await act(async () => {
            await view.getByTestId('stateful-counter').props.atomProps.updateAttrs({ count: 4 });
        });
        expect(onUpdateAtomAttrs).toHaveBeenCalledWith(
            expect.objectContaining({ attrs: { count: 3, id: 'one' } })
        );
        publish(view, []);
        expect(unmounted).toHaveBeenCalledTimes(1);
    });

    it('retains measurements from a replacement component while native layout is pending', () => {
        const Replacement = (props: AtomComponentProps) => (
            <View testID='replacement' atomProps={props} />
        );
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={[
                    {
                        ...atom,
                        idAttribute: 'id',
                        attrs: { ...atom.attrs, id: { default: 'one' } },
                    },
                ]}
            />
        );
        publish(view, [{ ...position, attrsJson: '{"count":2,"id":"one"}' }]);
        fireEvent(view.getByTestId('counter').parent!, 'layout', {
            nativeEvent: { layout: { width: 276, height: 123, x: 0, y: 0 } },
        });
        const oldMeasure = view
            .UNSAFE_getAllByType(View)
            .find((node) => node.props.collapsable === false)!.props.onLayout;
        view.rerender(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'counterCard', attrs: { count: 3 } }],
                }}
                atoms={[
                    {
                        ...atom,
                        idAttribute: 'id',
                        attrs: { ...atom.attrs, id: { default: 'one' } },
                        component: Replacement,
                    },
                ]}
            />
        );
        fireEvent(
            view.getByTestId('replacement', { includeHiddenElements: true }).parent!,
            'layout',
            {
                nativeEvent: { layout: { width: 276, height: 200, x: 0, y: 0 } },
            }
        );
        act(() => oldMeasure({ nativeEvent: { layout: { width: 276, height: 999, x: 0, y: 0 } } }));
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements
        ).toEqual({});
        publish(view, [{ ...position, attrsJson: '{"count":3,"id":"one"}' }]);
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements['0']
        ).toEqual({ width: 276, height: 200 });
    });

    it('preserves the wrapper measurement when a replacement renderer has the same size', () => {
        const Replacement = (props: AtomComponentProps) => (
            <View testID='replacement' atomProps={props} />
        );
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={[
                    {
                        ...atom,
                        idAttribute: 'id',
                        attrs: { ...atom.attrs, id: { default: 'one' } },
                    },
                ]}
            />
        );
        publish(view, [{ ...position, attrsJson: '{"count":2,"id":"one"}' }]);
        fireEvent(view.getByTestId('counter').parent!, 'layout', {
            nativeEvent: { layout: { width: 276, height: 123, x: 0, y: 0 } },
        });
        view.rerender(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'counterCard', attrs: { count: 3 } }],
                }}
                atoms={[
                    {
                        ...atom,
                        idAttribute: 'id',
                        attrs: { ...atom.attrs, id: { default: 'one' } },
                        component: Replacement,
                    },
                ]}
            />
        );
        publish(view, [{ ...position, attrsJson: '{"count":3,"id":"one"}' }]);
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements['0']
        ).toEqual({ width: 276, height: 123 });
    });

    it('discards mounted measurements when a renderer is unregistered', () => {
        const view = render(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        fireEvent(view.getByTestId('counter').parent!, 'layout', {
            nativeEvent: { layout: { width: 276, height: 123, x: 0, y: 0 } },
        });
        view.rerender(<NativeProseViewer contentJSON={content} atoms={[]} />);
        view.rerender(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements
        ).toEqual({});
    });

    it('retains mounted components across resize without reusing measurements at a different width', () => {
        const unmounted = jest.fn();
        const StatefulCounter = (props: AtomComponentProps) => {
            React.useEffect(() => unmounted, []);
            return <View testID='stateful-counter' atomProps={props} />;
        };
        const definitions = [{ ...atom, component: StatefulCounter }];
        const view = render(<NativeProseViewer contentJSON={content} atoms={definitions} />);
        publish(view);
        const atomHost = view.getByTestId('stateful-counter').parent!;
        fireEvent(atomHost, 'layout', {
            nativeEvent: { layout: { width: 276, height: 123, x: 0, y: 0 } },
        });
        const oldMeasure = view
            .UNSAFE_getAllByType(View)
            .find((node) => node.props.collapsable === false)!.props.onLayout;
        const container = view
            .UNSAFE_getAllByType(View)
            .find(
                (node) =>
                    typeof node.props.onLayout === 'function' && node.props.collapsable !== false
            )!;
        fireEvent(container, 'layout', {
            nativeEvent: { layout: { width: 200, height: 100, x: 0, y: 0 } },
        });
        const native = view.getByTestId('prepared-prose-viewer');
        const configuration = JSON.parse(native.props.themeJson).viewerAtoms;
        fireEvent(native, 'atomLayout', {
            nativeEvent: {
                ...configuration,
                layoutWidth: 200,
                atomsJson: JSON.stringify([{ ...position, width: 176 }]),
            },
        });
        expect(unmounted).not.toHaveBeenCalled();
        act(() => oldMeasure({ nativeEvent: { layout: { width: 276, height: 999, x: 0, y: 0 } } }));
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements
        ).toEqual({});
        fireEvent(view.getByTestId('stateful-counter').parent!, 'layout', {
            nativeEvent: { layout: { width: 176, height: 150, x: 0, y: 0 } },
        });
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements['0']
        ).toEqual({ width: 176, height: 150 });
    });

    it('delegates updates to the app without changing the rendered attributes', async () => {
        const onUpdateAtomAttrs = jest.fn();
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={atoms}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        publish(view);
        await act(async () => {
            await view.getByTestId('counter').props.atomProps.updateAttrs({ count: 3 });
        });
        expect(onUpdateAtomAttrs).toHaveBeenCalledWith({
            nodeType: 'counterCard',
            docPos: 0,
            attrs: { count: 2 },
            partial: { count: 3 },
        });
        expect(view.getByTestId('counter').props.atomProps.attrs.count).toBe(2);
    });

    it('rejects updates when read-only, missing a handler, or given undeclared attrs', async () => {
        const view = render(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        await act(async () => {
            await expect(
                view.getByTestId('counter').props.atomProps.updateAttrs({ count: 3 })
            ).rejects.toMatchObject({ code: 'not-applicable' });
        });
        view.rerender(<NativeProseViewer contentJSON={content} atoms={atoms} readOnly={false} />);
        await act(async () => {
            await expect(
                view.getByTestId('counter').props.atomProps.updateAttrs({ count: 3 })
            ).rejects.toMatchObject({ code: 'not-applicable' });
        });
        const onUpdateAtomAttrs = jest.fn();
        view.rerender(
            <NativeProseViewer
                contentJSON={content}
                atoms={atoms}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        await act(async () => {
            await expect(
                view.getByTestId('counter').props.atomProps.updateAttrs({ other: 3 })
            ).rejects.toMatchObject({ code: 'not-applicable' });
        });
        expect(onUpdateAtomAttrs).not.toHaveBeenCalled();
    });

    it('discards stale native layouts and update callbacks after content replacement', async () => {
        const onUpdateAtomAttrs = jest.fn();
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={atoms}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        publish(view);
        const update = view.getByTestId('counter').props.atomProps.updateAttrs;
        const oldNative = view.getByTestId('prepared-prose-viewer').props;
        const oldConfiguration = JSON.parse(oldNative.themeJson).viewerAtoms;
        view.rerender(
            <NativeProseViewer
                contentHTML='<p>Replaced</p>'
                atoms={atoms}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        act(() =>
            oldNative.onAtomLayout({
                nativeEvent: {
                    ...oldConfiguration,
                    layoutWidth: 300,
                    atomsJson: JSON.stringify([position]),
                },
            })
        );
        expect(view.queryByTestId('counter')).toBeNull();
        await act(async () => {
            await expect(update({ count: 3 })).rejects.toMatchObject({ code: 'stale-revision' });
        });
        expect(onUpdateAtomAttrs).not.toHaveBeenCalled();
    });

    it('feeds component measurements back into prepared layout without remounting', () => {
        const view = render(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        const wrapper = view.getByTestId('counter').parent!;
        fireEvent(wrapper, 'layout', {
            nativeEvent: { layout: { width: 276, height: 123, x: 0, y: 0 } },
        });
        const config = JSON.parse(
            view.getByTestId('prepared-prose-viewer').props.themeJson
        ).viewerAtoms;
        expect(config.measurements['0']).toEqual({ width: 276, height: 123 });
        expect(view.getByTestId('counter')).toBeTruthy();
    });

    it('propagates app update failures and rejects callbacks after unmount', async () => {
        const error = new Error('Storage unavailable');
        const onUpdateAtomAttrs = jest.fn().mockRejectedValue(error);
        const view = render(
            <NativeProseViewer
                contentJSON={content}
                atoms={atoms}
                readOnly={false}
                onUpdateAtomAttrs={onUpdateAtomAttrs}
            />
        );
        publish(view);
        const update = view.getByTestId('counter').props.atomProps.updateAttrs;
        await act(async () => {
            await expect(update({ count: 3 })).rejects.toBe(error);
        });
        view.unmount();
        await act(async () => {
            await expect(update({ count: 4 })).rejects.toMatchObject({ code: 'not-ready' });
        });
        expect(onUpdateAtomAttrs).toHaveBeenCalledTimes(1);
    });
    it('clears removed atoms and rejects malformed native attributes', () => {
        const onError = jest.fn();
        const view = render(
            <NativeProseViewer contentJSON={content} atoms={atoms} onError={onError} />
        );
        publish(view);
        publish(view, []);
        expect(view.queryByTestId('counter')).toBeNull();
        publish(view, [{ ...position, attrsJson: '[]' }]);
        expect(view.queryByTestId('counter')).toBeNull();
        expect(onError).toHaveBeenCalledWith(
            expect.objectContaining({ code: 'INVALID_ATOM_LAYOUT', fatal: false })
        );
    });

    it('rejects obsolete measurements after container width changes', () => {
        const view = render(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        const container = view
            .UNSAFE_getAllByType(View)
            .find(
                (node) =>
                    typeof node.props.onLayout === 'function' && node.props.collapsable !== false
            )!;
        const atomHost = view
            .UNSAFE_getAllByType(View)
            .find((node) => node.props.collapsable === false)!;
        const oldMeasure = view
            .UNSAFE_getAllByType(View)
            .find((node) => node.props.collapsable === false)!.props.onLayout;
        fireEvent(container, 'layout', {
            nativeEvent: { layout: { width: 200, height: 100, x: 0, y: 0 } },
        });
        act(() => oldMeasure({ nativeEvent: { layout: { width: 276, height: 200, x: 0, y: 0 } } }));
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements
        ).toEqual({});
        publish(view);
        expect(view.queryByTestId('counter')).toBeNull();
    });

    it('ignores obsolete native layout revisions and normalizes Yoga width rounding', () => {
        const view = render(<NativeProseViewer contentJSON={content} atoms={atoms} />);
        publish(view);
        const oldNative = view.getByTestId('prepared-prose-viewer').props;
        const old = JSON.parse(oldNative.themeJson).viewerAtoms;
        fireEvent(view.getByTestId('counter').parent!, 'layout', {
            nativeEvent: { layout: { width: 276.333, height: 100, x: 0, y: 0 } },
        });
        act(() =>
            oldNative.onAtomLayout({ nativeEvent: { ...old, layoutWidth: 300, atomsJson: '[]' } })
        );
        expect(view.getByTestId('counter')).toBeTruthy();
        expect(
            JSON.parse(view.getByTestId('prepared-prose-viewer').props.themeJson).viewerAtoms
                .measurements['0']
        ).toEqual({ width: 276, height: 100 });
    });
});
