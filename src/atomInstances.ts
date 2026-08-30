import type { RenderBlocksPatch, RenderElement, Selection } from './NativeEditorBridge';

export interface AtomInstance {
    key: string;
    nodeType: string;
    attrs: Record<string, unknown>;
    docPos: number;
}

export const NATIVE_VOID_BLOCK_TYPES: ReadonlySet<string> = new Set([
    'horizontalRule',
    'horizontal_rule',
    'image',
]);

export const DEFAULT_ATOM_CHIP_HEIGHT = 32;

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

export function collectAtomInstances(
    renderBlocks: ReadonlyArray<ReadonlyArray<RenderElement>>,
    registeredTypes: ReadonlySet<string>
): AtomInstance[] {
    const occurrences = new Map<string, number>();
    const instances: AtomInstance[] = [];
    for (const block of renderBlocks) {
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
            instances.push({
                key:
                    typeof element.atomId === 'string'
                        ? element.atomId
                        : `${element.nodeType}:${occurrence}`,
                nodeType: element.nodeType,
                attrs: element.attrs ?? {},
                docPos: element.docPos,
            });
        }
    }
    return instances;
}

export function applyRenderPatch(
    previousBlocks: RenderElement[][],
    patch: RenderBlocksPatch
): RenderElement[][] {
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
