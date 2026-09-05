import React, { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { View, type LayoutChangeEvent, type NativeSyntheticEvent } from 'react-native';

import { serializeEditorAtoms, type AtomNodeDefinition, type AtomAttrsUpdate } from './atoms';
import { AtomHost, atomIsVisible, type AtomViewport } from './AtomHost';
import { resolveAtomAttrsUpdate } from './atomUpdates';
import { validateAttributes } from './attributeValidation';
import { AtomUpdateAttrsError } from './atomInstances';
import type { RichTextViewerErrorEvent } from './NativeProseViewer';

export interface RichTextViewerAtomAttrsUpdateEvent {
    nodeType: string;
    atomId?: string;
    /** Position in the content snapshot rendered by this viewer. */
    docPos: number;
    attrs: Readonly<Record<string, unknown>>;
    partial: Readonly<Record<string, unknown>>;
}

export interface ViewerAtomLayoutEvent {
    generation: string;
    revision: string;
    layoutWidth: number;
    atomsJson: string;
}

interface AtomPosition {
    key: string;
    atomId?: string;
    nodeType: string;
    docPos: number;
    attrsJson: string;
    attrs: Readonly<Record<string, unknown>>;
    x: number;
    y: number;
    width: number;
    height: number;
}

interface Measurement {
    width: number;
    height: number;
}

type Measurements = Record<string, Measurement>;
type UpdateHandler = (event: RichTextViewerAtomAttrsUpdateEvent) => void | Promise<void>;
let nextGeneration = 0;

function isRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

function parsePositions(
    json: string,
    definitions: ReadonlyMap<string, AtomNodeDefinition<any>>,
    snapshot: string
): AtomPosition[] {
    const values: unknown = JSON.parse(json);
    if (!Array.isArray(values)) throw new Error('Atom positions must be an array.');
    const seen = new Set<number>();
    const identities = new Set<string>();
    return values.map((value: unknown) => {
        if (
            !isRecord(value) ||
            typeof value.nodeType !== 'string' ||
            !definitions.has(value.nodeType) ||
            !Number.isInteger(value.docPos) ||
            Number(value.docPos) < 0 ||
            Number(value.docPos) > 0xffffffff ||
            typeof value.attrsJson !== 'string' ||
            !['x', 'y', 'width', 'height'].every(
                (key) => typeof value[key] === 'number' && Number.isFinite(value[key])
            ) ||
            Number(value.width) <= 0 ||
            Number(value.height) < 0 ||
            seen.has(Number(value.docPos))
        ) {
            throw new Error('Invalid prepared atom position.');
        }
        const attrs: unknown = JSON.parse(value.attrsJson);
        if (!isRecord(attrs)) throw new Error('Atom attributes must be a JSON object.');
        const idAttribute = definitions.get(value.nodeType)?.idAttribute;
        const atomId = idAttribute == null ? undefined : attrs[idAttribute];
        if (idAttribute != null && (typeof atomId !== 'string' || atomId.length === 0))
            throw new Error('Atom identity must be a non-empty string.');
        const key =
            typeof atomId === 'string'
                ? JSON.stringify([value.nodeType, atomId])
                : `${snapshot}:${value.docPos}:${value.nodeType}`;
        if (identities.has(key)) throw new Error('Duplicate atom identity.');
        identities.add(key);
        seen.add(Number(value.docPos));
        return { ...value, attrs, key, atomId } as unknown as AtomPosition;
    });
}

export function useViewerAtoms({
    atoms,
    identity,
    themeJson,
    readOnly,
    interactive = true,
    viewport,
    onUpdateAtomAttrs,
    onError,
}: {
    atoms?: readonly AtomNodeDefinition<any>[];
    identity: object;
    themeJson?: string;
    readOnly: boolean;
    interactive?: boolean;
    viewport?: AtomViewport;
    onUpdateAtomAttrs?: UpdateHandler;
    onError?: (event: RichTextViewerErrorEvent) => void;
}) {
    const [width, setWidth] = useState<number | null>(null);
    const snapshot = useMemo(() => String(++nextGeneration), [identity]);
    const serializedAtoms = serializeEditorAtoms(atoms);
    const identifiers = JSON.stringify((atoms ?? []).map((atom) => [atom.name, atom.idAttribute]));
    const generation = useMemo(
        () => String(++nextGeneration),
        [identity, identifiers, serializedAtoms, width]
    );
    const definitions = useMemo(
        () => new Map((atoms ?? []).map((atom) => [atom.name, atom])),
        [atoms]
    );
    const enabled = definitions.size > 0;
    const [measurementState, setMeasurementState] = useState({
        generation,
        revision: 0,
        values: {} as Measurements,
    });
    const measurements = measurementState.generation === generation ? measurementState.values : {};
    const revision = String(
        measurementState.generation === generation ? measurementState.revision : 0
    );
    const [layout, setLayout] = useState<{
        generation: string;
        json: string;
        positions: AtomPosition[];
    } | null>(null);
    const pendingLayout = layout?.generation !== generation;
    const renderedPositions = layout?.positions ?? [];
    const positions = pendingLayout ? [] : renderedPositions;
    const mountedMeasurements = useRef(new Map<string, { nodeType: string; size: Measurement }>());
    useLayoutEffect(() => {
        for (const [key, measured] of mountedMeasurements.current) {
            if (!definitions.has(measured.nodeType)) mountedMeasurements.current.delete(key);
        }
    }, [definitions]);
    const mounted = useRef(false);
    useLayoutEffect(() => {
        mounted.current = true;
        return () => {
            mounted.current = false;
        };
    }, []);
    const current = useRef({
        generation,
        snapshot,
        revision,
        positions,
        renderedPositions,
        pendingLayout,
        definitions,
        readOnly,
        onUpdateAtomAttrs,
        onError,
        width,
    });
    current.current = {
        generation,
        snapshot,
        revision,
        positions,
        renderedPositions,
        pendingLayout,
        definitions,
        readOnly,
        onUpdateAtomAttrs,
        onError,
        width,
    };

    const configuredThemeJson = useMemo(
        () =>
            enabled
                ? JSON.stringify({
                      ...(themeJson ? JSON.parse(themeJson) : {}),
                      viewerAtoms: {
                          ...JSON.parse(serializedAtoms!),
                          generation,
                          revision,
                          measurements,
                      },
                  })
                : themeJson,
        [enabled, themeJson, serializedAtoms, generation, revision, measurements]
    );

    const onAtomLayout = useCallback((event: NativeSyntheticEvent<ViewerAtomLayoutEvent>) => {
        const value = event.nativeEvent;
        const latest = current.current;
        if (
            !mounted.current ||
            value.generation !== latest.generation ||
            value.revision !== latest.revision ||
            !Number.isFinite(value.layoutWidth) ||
            value.layoutWidth <= 0 ||
            (latest.width != null && Math.abs(latest.width - value.layoutWidth) > 1)
        )
            return;
        try {
            const next = parsePositions(value.atomsJson, latest.definitions, latest.snapshot);
            const retainedMeasurements: Measurements = {};
            const retainedKeys = new Set<string>();
            for (const atom of next) {
                const key = atom.key;
                retainedKeys.add(key);
                const measured = mountedMeasurements.current.get(key);
                if (measured?.size.width === atom.width) {
                    retainedMeasurements[String(atom.docPos)] = measured.size;
                }
            }
            for (const key of mountedMeasurements.current.keys()) {
                if (!retainedKeys.has(key)) mountedMeasurements.current.delete(key);
            }
            // Unchanged mounted sizes do not emit another Yoga onLayout event.
            setMeasurementState((previous) =>
                previous.generation === value.generation
                    ? previous
                    : {
                          generation: value.generation,
                          revision: Object.keys(retainedMeasurements).length > 0 ? 1 : 0,
                          values: retainedMeasurements,
                      }
            );
            setLayout((previous) =>
                previous?.generation === value.generation && previous.json === value.atomsJson
                    ? previous
                    : { generation: value.generation, json: value.atomsJson, positions: next }
            );
        } catch {
            mountedMeasurements.current.clear();
            setLayout(null);
            latest.onError?.({
                domain: 'viewer',
                code: 'INVALID_ATOM_LAYOUT',
                message: 'The prepared atom layout or attributes are invalid.',
                fatal: false,
            });
        }
    }, []);

    const onContainerLayout = useCallback((event: LayoutChangeEvent) => {
        const next = event.nativeEvent.layout.width;
        if (Number.isFinite(next) && next > 0)
            setWidth((previous) => (previous === next ? previous : next));
    }, []);

    const updateQueues = useRef(new Map<string, Promise<void>>());
    const acknowledged = useRef(
        new Map<string, { generation: string; attrs: Record<string, unknown> }>()
    );
    useLayoutEffect(() => {
        acknowledged.current.clear();
    }, [generation]);
    const updateAttrs = useCallback(
        (owner: string, atom: AtomPosition, update: AtomAttrsUpdate): Promise<void> => {
            const execute = async () => {
                const latest = current.current;
                if (!mounted.current)
                    throw new AtomUpdateAttrsError('not-ready', 'The viewer is unmounted.');
                if (
                    owner !== latest.generation ||
                    !latest.positions.some(
                        (position) =>
                            position.key === atom.key && position.attrsJson === atom.attrsJson
                    )
                ) {
                    throw new AtomUpdateAttrsError(
                        'stale-revision',
                        'The viewer content has changed.'
                    );
                }
                if (latest.readOnly || !latest.onUpdateAtomAttrs)
                    throw new AtomUpdateAttrsError(
                        'not-applicable',
                        'Viewer updates require readOnly={false} and onUpdateAtomAttrs.'
                    );
                const accepted = acknowledged.current.get(atom.key);
                const attrs = accepted?.generation === owner ? accepted.attrs : atom.attrs;
                const partial = resolveAtomAttrsUpdate(attrs, update);
                const definition = latest.definitions.get(atom.nodeType)!;
                try {
                    validateAttributes({ ...attrs, ...partial }, definition.attrs ?? {});
                    if (
                        definition.idAttribute &&
                        Object.prototype.hasOwnProperty.call(partial, definition.idAttribute) &&
                        partial[definition.idAttribute] !== attrs[definition.idAttribute]
                    )
                        throw new Error('An atom identity cannot be changed.');
                } catch (error) {
                    throw new AtomUpdateAttrsError(
                        'not-applicable',
                        error instanceof Error ? error.message : String(error)
                    );
                }
                await latest.onUpdateAtomAttrs({
                    nodeType: atom.nodeType,
                    atomId: atom.atomId,
                    docPos: atom.docPos,
                    attrs,
                    partial,
                });
                if (mounted.current && current.current.generation === owner)
                    acknowledged.current.set(atom.key, {
                        generation: owner,
                        attrs: { ...attrs, ...partial },
                    });
            };
            const previous = updateQueues.current.get(atom.key);
            const result = previous ? previous.catch(() => {}).then(execute) : execute();
            updateQueues.current.set(atom.key, result);
            const clean = () => {
                if (updateQueues.current.get(atom.key) === result)
                    updateQueues.current.delete(atom.key);
            };
            void result.then(clean, clean);
            return result;
        },
        []
    );

    const measure = useCallback(
        (
            owner: string,
            atom: AtomPosition,
            component: AtomNodeDefinition['component'],
            event: LayoutChangeEvent
        ) => {
            const latest = current.current;
            const measured = event.nativeEvent.layout;
            if (
                !mounted.current ||
                owner !== latest.generation ||
                component !== latest.definitions.get(atom.nodeType)?.component ||
                !Number.isFinite(measured.width) ||
                measured.width <= 0 ||
                !Number.isFinite(measured.height) ||
                measured.height < 0 ||
                Math.abs(measured.width - atom.width) > 1 ||
                !latest.renderedPositions.some(
                    (position) =>
                        position.docPos === atom.docPos &&
                        position.nodeType === atom.nodeType &&
                        position.width === atom.width &&
                        position.attrsJson === atom.attrsJson
                )
            )
                return;
            mountedMeasurements.current.set(atom.key, {
                nodeType: atom.nodeType,
                size: { width: atom.width, height: measured.height },
            });
            if (latest.pendingLayout) return;
            setMeasurementState((previous) => {
                const values = previous.generation === owner ? previous.values : {};
                const existing = values[String(atom.docPos)];
                if (existing?.width === atom.width && existing.height === measured.height)
                    return previous;
                return {
                    generation: owner,
                    revision: previous.generation === owner ? previous.revision + 1 : 1,
                    values: {
                        ...values,
                        [String(atom.docPos)]: { width: atom.width, height: measured.height },
                    },
                };
            });
        },
        []
    );

    const children = renderedPositions.map((atom) => {
        const Component = definitions.get(atom.nodeType)?.component;
        if (!Component) return null;
        return (
            <View
                key={atom.key}
                collapsable={false}
                pointerEvents={!interactive || pendingLayout ? 'none' : 'box-none'}
                accessibilityElementsHidden={pendingLayout}
                importantForAccessibility={pendingLayout ? 'no-hide-descendants' : 'auto'}
                style={{
                    position: 'absolute',
                    left: atom.x,
                    top: atom.y,
                    width: atom.width,
                    opacity: pendingLayout ? 0 : 1,
                }}
                onLayout={(event) => measure(generation, atom, Component, event)}>
                <AtomHost
                    component={Component}
                    width={atom.width}
                    estimatedHeight={atom.height}
                    visible={atomIsVisible(atom.y, atom.height, viewport)}
                    onMeasure={(event) => measure(generation, atom, Component, event)}
                    atomProps={{
                        attrs: atom.attrs,
                        nodeType: atom.nodeType,
                        selected: false,
                        readOnly,
                        interactive: interactive && !pendingLayout,
                        isViewer: true,
                        updateAttrs: (partial) => updateAttrs(layout!.generation, atom, partial),
                    }}
                />
            </View>
        );
    });
    return { enabled, themeJson: configuredThemeJson, onAtomLayout, onContainerLayout, children };
}

/** @deprecated Use RichTextViewerAtomAttrsUpdateEvent instead. */
export type NativeProseViewerAtomAttrsUpdateEvent = RichTextViewerAtomAttrsUpdateEvent;
