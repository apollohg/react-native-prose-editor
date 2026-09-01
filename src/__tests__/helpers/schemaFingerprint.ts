import { createHash } from 'node:crypto';
import {
    resolveDocumentSchema,
    type AttrSpec,
    type MarkSpec,
    type NodeSpec,
    type SchemaDefinition,
} from '../../schemas';

type CanonicalPrimitive = null | boolean | string;
type CanonicalValue = CanonicalPrimitive | CanonicalValue[] | CanonicalObject;
type CanonicalObject = {
    readonly entries: ReadonlyArray<readonly [string, CanonicalValue]>;
};

function compareScalarStrings(left: string, right: string): number {
    const leftCodePoints = [...left].map((value) => value.codePointAt(0)!);
    const rightCodePoints = [...right].map((value) => value.codePointAt(0)!);
    const sharedLength = Math.min(leftCodePoints.length, rightCodePoints.length);
    for (let index = 0; index < sharedLength; index += 1) {
        const difference = leftCodePoints[index]! - rightCodePoints[index]!;
        if (difference !== 0) return difference;
    }
    return leftCodePoints.length - rightCodePoints.length;
}

function canonicalObject(
    entries: ReadonlyArray<readonly [string, CanonicalValue]>
): CanonicalObject {
    return { entries };
}

function canonicalMap<T extends CanonicalValue>(
    entries: Iterable<readonly [string, T]>
): CanonicalObject {
    return canonicalObject(
        [...entries].sort(([left], [right]) => compareScalarStrings(left, right))
    );
}

function writeCanonicalJson(value: CanonicalValue): string {
    if (value === null) return 'null';
    if (typeof value === 'boolean') return value ? 'true' : 'false';
    if (typeof value === 'string') return JSON.stringify(value)!;
    if (Array.isArray(value)) return `[${value.map(writeCanonicalJson).join(',')}]`;
    return `{${value.entries
        .map(([key, nested]) => `${JSON.stringify(key)!}:${writeCanonicalJson(nested)}`)
        .join(',')}}`;
}

function binary64Bits(value: number): string {
    if (!Number.isFinite(value)) throw new Error('canonical JSON numbers must be finite');
    const normalized = value === 0 ? 0 : value;
    const bytes = new ArrayBuffer(8);
    const view = new DataView(bytes);
    view.setFloat64(0, normalized, false);
    return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

function canonicalJson(value: unknown): CanonicalObject {
    if (value === null) return canonicalObject([['type', 'null']]);
    if (typeof value === 'boolean') {
        return canonicalObject([
            ['type', 'bool'],
            ['value', value],
        ]);
    }
    if (typeof value === 'number') {
        return canonicalObject([
            ['type', 'number'],
            ['value', binary64Bits(value)],
        ]);
    }
    if (typeof value === 'string') {
        return canonicalObject([
            ['type', 'string'],
            ['value', value],
        ]);
    }
    if (Array.isArray(value)) {
        return canonicalObject([
            ['type', 'array'],
            ['value', value.map(canonicalJson)],
        ]);
    }
    return canonicalObject([
        ['type', 'object'],
        [
            'value',
            canonicalMap(
                Object.entries(value as Record<string, unknown>).map(
                    ([name, nested]) => [name, canonicalJson(nested)] as const
                )
            ),
        ],
    ]);
}

function canonicalAttrs(attrs: Record<string, AttrSpec> | undefined): CanonicalObject {
    return canonicalMap(
        Object.entries(attrs ?? {}).map(([name, attr]) => {
            const hasDefault = Object.prototype.hasOwnProperty.call(attr, 'default');
            return [
                name,
                canonicalObject([
                    ['hasDefault', hasDefault],
                    ['default', canonicalJson(hasDefault ? attr.default : null)],
                ]),
            ] as const;
        })
    );
}

function canonicalRole(node: NodeSpec): string {
    if (node.role !== 'list') return node.role;
    return node.name.includes('ordered') || node.name.includes('Ordered')
        ? 'listOrdered'
        : 'listUnordered';
}

function canonicalNode(node: NodeSpec): CanonicalObject {
    const group = [...new Set((node.group ?? '').split(/\s+/u).filter(Boolean))].sort(
        compareScalarStrings
    );
    const jsonProjection: ReadonlyArray<readonly [string, CanonicalValue]> =
        node.json == null
            ? []
            : [
                  [
                      'jsonProjection',
                      canonicalObject([
                          ['nodeType', node.json.type],
                          [
                              'attrs',
                              canonicalMap(
                                  Object.entries(node.json.attrs ?? {}).map(
                                      ([name, value]) => [name, canonicalJson(value)] as const
                                  )
                              ),
                          ],
                      ]),
                  ],
              ];
    return canonicalObject([
        ['content', node.content.trim()],
        ['group', group],
        ['attrs', canonicalAttrs(node.attrs)],
        ['role', canonicalRole(node)],
        ['htmlTag', node.htmlTag ?? null],
        ...jsonProjection,
        ['isVoid', node.isVoid ?? false],
        ...(node.deletableOnBackspace == null
            ? []
            : ([['deletableOnBackspace', node.deletableOnBackspace]] as const)),
        ['allowUndeclaredAttrs', node.allowUndeclaredAttrs ?? false],
    ]);
}

function canonicalMark(mark: MarkSpec): CanonicalObject {
    return canonicalObject([
        ['htmlTag', mark.htmlTag ?? null],
        ['attrs', canonicalAttrs(mark.attrs)],
        ['excludes', mark.excludes ?? null],
        ['allowUndeclaredAttrs', mark.allowUndeclaredAttrs ?? false],
    ]);
}

function firstHtmlTags(specs: ReadonlyArray<NodeSpec | MarkSpec>): CanonicalObject {
    const tags = new Map<string, string>();
    for (const spec of specs) {
        if (spec.htmlTag != null && !tags.has(spec.htmlTag)) tags.set(spec.htmlTag, spec.name);
    }
    return canonicalMap(tags);
}

function preferredTextBlockName(nodes: NodeSpec[]): string | null {
    const candidates = nodes.filter(
        (node) =>
            node.role === 'textBlock' &&
            Object.values(node.attrs ?? {}).every((attr) =>
                Object.prototype.hasOwnProperty.call(attr, 'default')
            )
    );
    candidates.sort((left, right) => {
        const leftPriority = left.htmlTag === 'p' || left.name === 'paragraph' ? 0 : 1;
        const rightPriority = right.htmlTag === 'p' || right.name === 'paragraph' ? 0 : 1;
        return leftPriority - rightPriority || compareScalarStrings(left.name, right.name);
    });
    return candidates[0]?.name ?? null;
}

function canonicalResolvedSchema(schema: SchemaDefinition): CanonicalObject {
    const resolved = resolveDocumentSchema(schema);
    const documentNodeName = resolved.nodes.find((node) => node.role === 'doc')!.name;
    const textNodeName = resolved.nodes.find((node) => node.role === 'text')!.name;
    const fallbackListItemName =
        resolved.nodes
            .filter((node) => node.role === 'listItem')
            .map((node) => node.name)
            .sort(compareScalarStrings)[0] ?? null;

    return canonicalObject([
        [
            'nodes',
            canonicalMap(resolved.nodes.map((node) => [node.name, canonicalNode(node)] as const)),
        ],
        [
            'marks',
            canonicalMap(resolved.marks.map((mark) => [mark.name, canonicalMark(mark)] as const)),
        ],
        ['markOrder', resolved.marks.map((mark) => mark.name)],
        ['nodeHtmlTags', firstHtmlTags(resolved.nodes)],
        ['markHtmlTags', firstHtmlTags(resolved.marks)],
        ['preferredTextBlockName', preferredTextBlockName(resolved.nodes)],
        ['fallbackListItemName', fallbackListItemName],
        ['documentNodeName', documentNodeName],
        ['textNodeName', textNodeName],
    ]);
}

export function testSchemaFingerprint(schema: SchemaDefinition): string {
    const canonicalBytes = writeCanonicalJson(canonicalResolvedSchema(schema));
    return createHash('sha256').update(canonicalBytes, 'utf8').digest('hex');
}
