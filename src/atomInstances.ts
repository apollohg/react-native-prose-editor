import type { Selection } from './NativeEditorBridge';

export { DEFAULT_ATOM_CHIP_HEIGHT } from './atomConstants';

export interface AtomInstance {
    key: string;
    hasStableKey: boolean;
    nodeType: string;
    attrs: Readonly<Record<string, unknown>>;
    docPos: number;
}

export interface AtomInstanceCollection {
    instanceBlocks: AtomInstance[][];
    instances: AtomInstance[];
    hasOnlyStableKeys: boolean;
}

export const NATIVE_VOID_BLOCK_TYPES: ReadonlySet<string> = new Set([
    'horizontalRule',
    'horizontal_rule',
    'image',
]);

export type AtomUpdateAttrsErrorCode =
    | 'not-applicable'
    | 'stale-revision'
    | 'not-ready'
    | 'engine-error';

export class AtomUpdateAttrsError extends Error {
    readonly code: AtomUpdateAttrsErrorCode;

    constructor(code: AtomUpdateAttrsErrorCode, message: string) {
        super(message);
        this.name = 'AtomUpdateAttrsError';
        this.code = code;
    }
}

interface AtomRenderElement {
    readonly type: string;
    readonly nodeType?: string;
    readonly docPos?: number;
    readonly atomId?: string;
    readonly attrs?: Readonly<Record<string, unknown>>;
}

const EMPTY_ATOM_ATTRS: Readonly<Record<string, unknown>> = Object.freeze({});

export function collectAtomInstanceBlocks(
    renderBlocks: ReadonlyArray<ReadonlyArray<AtomRenderElement>>,
    registeredTypes: ReadonlySet<string>
): AtomInstanceCollection {
    const occurrences = new Map<string, number>();
    const instanceBlocks: AtomInstance[][] = [];
    const instances: AtomInstance[] = [];
    let hasOnlyStableKeys = true;
    for (const block of renderBlocks) {
        const blockInstances: AtomInstance[] = [];
        for (const element of block) {
            if (
                element.type !== 'voidBlock' ||
                typeof element.nodeType !== 'string' ||
                typeof element.docPos !== 'number'
            ) {
                continue;
            }
            const occurrence = occurrences.get(element.nodeType) ?? 0;
            occurrences.set(element.nodeType, occurrence + 1);
            if (
                !registeredTypes.has(element.nodeType) &&
                NATIVE_VOID_BLOCK_TYPES.has(element.nodeType)
            ) {
                continue;
            }
            const hasStableKey = typeof element.atomId === 'string';
            const instance = {
                key: hasStableKey ? element.atomId! : `${element.nodeType}:${occurrence}`,
                hasStableKey,
                nodeType: element.nodeType,
                attrs: element.attrs ?? EMPTY_ATOM_ATTRS,
                docPos: element.docPos,
            };
            hasOnlyStableKeys &&= hasStableKey;
            blockInstances.push(instance);
            instances.push(instance);
        }
        instanceBlocks.push(blockInstances);
    }
    return { instanceBlocks, instances, hasOnlyStableKeys };
}

export function collectAtomInstances(
    renderBlocks: ReadonlyArray<ReadonlyArray<AtomRenderElement>>,
    registeredTypes: ReadonlySet<string>
): AtomInstance[] {
    return collectAtomInstanceBlocks(renderBlocks, registeredTypes).instances;
}

export function applyRenderPatch<Element>(
    previousBlocks: ReadonlyArray<ReadonlyArray<Element>>,
    patch: {
        readonly startIndex: number;
        readonly deleteCount: number;
        readonly renderBlocks: ReadonlyArray<ReadonlyArray<Element>>;
    }
): Array<ReadonlyArray<Element>> {
    return [
        ...previousBlocks.slice(0, patch.startIndex),
        ...patch.renderBlocks,
        ...previousBlocks.slice(patch.startIndex + patch.deleteCount),
    ];
}

export function atomSelected(selection: Selection, docPos: number): boolean {
    if (selection.type === 'all') return true;
    if (selection.type === 'node') return selection.pos === docPos;
    const { anchor, head } = selection;
    if (anchor == null || head == null) return false;
    const [from, to] = anchor <= head ? [anchor, head] : [head, anchor];
    return from <= docPos && docPos + 1 <= to;
}
