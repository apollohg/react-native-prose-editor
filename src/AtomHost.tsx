import React, { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { AtomUpdateAttrsError } from './atomInstances';
import type { AtomAttrsUpdate, AtomComponent, AtomComponentProps } from './atoms';

export interface AtomViewport {
    /** Visible vertical range in the atom layout's coordinate space, in points. */
    y: number;
    height: number;
    overscan?: number;
}

export function atomIsVisible(y: number, height: number, viewport?: AtomViewport): boolean {
    if (!viewport) return true;
    if (
        ![viewport.y, viewport.height, viewport.overscan ?? 200].every(Number.isFinite) ||
        viewport.height < 0 ||
        (viewport.overscan ?? 200) < 0
    ) {
        throw new Error(
            'Atom viewport must have finite coordinates and non-negative height and overscan.'
        );
    }
    const overscan = viewport.overscan ?? 200;
    return y + height >= viewport.y - overscan && y <= viewport.y + viewport.height + overscan;
}

class AtomErrorBoundary extends React.Component<
    {
        component: AtomComponent;
        nodeType: string;
        children: React.ReactNode;
    },
    { error: boolean; component: AtomComponent }
> {
    state = { error: false, component: this.props.component };

    static getDerivedStateFromError() {
        return { error: true };
    }

    static getDerivedStateFromProps(
        props: { component: AtomComponent },
        state: { component: AtomComponent }
    ) {
        return props.component === state.component
            ? null
            : { component: props.component, error: false };
    }

    render() {
        if (!this.state.error) return this.props.children;
        return (
            <View accessibilityRole='alert' style={{ minHeight: 32, padding: 8 }}>
                <Text>Unable to display {this.props.nodeType}</Text>
                <Pressable
                    accessibilityRole='button'
                    onPress={() => this.setState({ error: false })}>
                    <Text>Retry</Text>
                </Pressable>
            </View>
        );
    }
}

export function AtomHost({
    component: Component,
    atomProps,
    width,
    estimatedHeight,
    visible = true,
    onMeasure,
}: {
    component: AtomComponent;
    atomProps: Omit<AtomComponentProps, 'updatePending' | 'updateError' | 'setActive'>;
    width: number;
    estimatedHeight: number;
    visible?: boolean;
    onMeasure?: (event: import('react-native').LayoutChangeEvent) => void;
}) {
    const [measurement, setMeasurement] = useState({ width, height: estimatedHeight });
    const [focused, setFocused] = useState(false);
    const [active, setActive] = useState(false);
    const [pending, setPending] = useState(0);
    const [updateError, setUpdateError] = useState<Error | null>(null);
    const latestRequest = useRef(0);
    const applyUpdate = atomProps.updateAttrs;
    const mounted = useRef(true);
    useEffect(() => {
        mounted.current = true;
        return () => {
            mounted.current = false;
        };
    }, []);
    const updateAttrs = useCallback(
        async (update: AtomAttrsUpdate) => {
            if (!mounted.current)
                throw new AtomUpdateAttrsError('not-ready', 'The atom is unmounted.');
            const request = ++latestRequest.current;
            setPending((count) => count + 1);
            setUpdateError(null);
            try {
                await applyUpdate(update);
            } catch (error) {
                if (mounted.current && latestRequest.current === request)
                    setUpdateError(error instanceof Error ? error : new Error(String(error)));
                throw error;
            } finally {
                if (mounted.current) setPending((count) => count - 1);
            }
        },
        [applyUpdate]
    );
    const editor = useMemo(() => {
        if (!atomProps.editor) return undefined;
        const guard = (action: () => Promise<void>) => async () => {
            if (!mounted.current)
                throw new AtomUpdateAttrsError('not-ready', 'The atom is unmounted.');
            return action();
        };
        return {
            select: guard(atomProps.editor.select),
            delete: guard(atomProps.editor.delete),
            focusBefore: guard(atomProps.editor.focusBefore),
            focusAfter: guard(atomProps.editor.focusAfter),
        };
    }, [atomProps.editor]);
    const retainedHeight = measurement.width === width ? measurement.height : estimatedHeight;
    const show = visible || focused || active || pending > 0 || atomProps.selected;
    return (
        <View
            testID='atom-host'
            pointerEvents={atomProps.interactive === false ? 'none' : 'box-none'}
            accessibilityElementsHidden={!show}
            importantForAccessibility={!show ? 'no-hide-descendants' : 'auto'}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            onLayout={(event) => {
                onMeasure?.(event);
                const size = event.nativeEvent.layout;
                if (
                    show &&
                    Number.isFinite(size.height) &&
                    size.height >= 0 &&
                    Math.abs(size.width - width) <= 1
                ) {
                    setMeasurement((previous) =>
                        previous.width === width && previous.height === size.height
                            ? previous
                            : { width, height: size.height }
                    );
                }
            }}
            style={show ? undefined : { height: retainedHeight }}>
            {show && (
                <AtomErrorBoundary component={Component} nodeType={atomProps.nodeType}>
                    <Suspense
                        fallback={
                            <View style={{ height: retainedHeight }}>
                                <Text>Loading {atomProps.nodeType}…</Text>
                            </View>
                        }>
                        <Component
                            {...atomProps}
                            editor={editor}
                            updateAttrs={updateAttrs}
                            updatePending={pending > 0}
                            updateError={updateError}
                            setActive={setActive}
                        />
                    </Suspense>
                </AtomErrorBoundary>
            )}
        </View>
    );
}
