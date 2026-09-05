import { validateAttributeSpec, validateAttributes } from './attributeValidation';
import type { ComponentType } from 'react';

import type { DocumentJSON } from './NativeEditorBridge';
import { DEFAULT_ATOM_CHIP_HEIGHT } from './atomConstants';
import {
    ATOM_HTML_DENIED_ATTRS,
    ATOM_HTML_DENIED_TAGS,
    ATOM_HTML_IDENTIFIER,
    RESERVED_WIRE_NODE_TYPES,
} from './atomPolicy';
import type {
    AttrSpec,
    NodeHtmlRules,
    NodeSpec,
    ResolvedDocumentSchema,
    SchemaDefinition,
} from './schemas';

export type AtomAttrsUpdate<A = Record<string, unknown>> =
    | Partial<A>
    | ((current: Readonly<A>) => Partial<A>)
    | readonly (Partial<A> | ((current: Readonly<A>) => Partial<A>))[];

export interface AtomEditorActions {
    select(): Promise<void>;
    delete(): Promise<void>;
    focusBefore(): Promise<void>;
    focusAfter(): Promise<void>;
}

export interface AtomComponentProps<A = Record<string, unknown>> {
    attrs: Readonly<A>;
    selected: boolean;
    readOnly: boolean;
    isViewer: boolean;
    nodeType: string;
    updateAttrs: (update: AtomAttrsUpdate<A>) => Promise<void>;
    interactive: boolean;
    updatePending: boolean;
    updateError: Error | null;
    editor?: AtomEditorActions;
    setActive: (active: boolean) => void;
}

export type AtomComponent<A = Record<string, unknown>> = ComponentType<AtomComponentProps<A>>;

type JsonAttributeKind<T> = T extends string
    ? 'string'
    : T extends number
      ? 'number'
      : T extends boolean
        ? 'boolean'
        : T extends readonly unknown[]
          ? 'array'
          : T extends object
            ? 'object'
            : never;
type AttributeKind<T> = unknown extends T ? AttrSpec['type'] : JsonAttributeKind<NonNullable<T>>;
type AtomAttributeSpecs<A> = {
    [K in keyof A]-?: Omit<AttrSpec, 'default' | 'type' | 'enum'> & {
        default?: A[K];
        type?: AttributeKind<A[K]>;
        enum?: readonly A[K][];
    };
};
type WidenDefault<T> = T extends string
    ? string
    : T extends number
      ? number
      : T extends boolean
        ? boolean
        : T extends readonly (infer Item)[]
          ? [Item] extends [never]
              ? unknown[]
              : WidenDefault<Item>[]
          : T extends object
            ? { -readonly [K in keyof T]: WidenDefault<T[K]> }
            : T;
type HasDefault<S> = S extends { default: infer D } ? (undefined extends D ? false : true) : false;

export interface AtomNodeConfig<A = Record<string, unknown>> {
    name: string;
    attrs?: AtomAttributeSpecs<A>;
    html: NodeHtmlRules;
    component: AtomComponent<A>;
    estimatedHeight?: number;
    idAttribute?: string;
}

type RequiredKeys<A> = { [K in keyof A]-?: {} extends Pick<A, K> ? never : K }[keyof A];
export interface AtomNodeDefinition<A = Record<string, unknown>, Input = A> extends Readonly<
    AtomNodeConfig<A>
> {
    readonly nodeSpec: NodeSpec;
    buildFragmentJson(
        ...args: RequiredKeys<Input> extends never
            ? [attrs?: Input, descriptor?: Pick<ResolvedDocumentSchema, 'documentNodeName'>]
            : [attrs: Input, descriptor?: Pick<ResolvedDocumentSchema, 'documentNodeName'>]
    ): DocumentJSON;
}

type AttrValue<S> = S extends { enum: readonly (infer E)[] }
    ? E
    : S extends { type: 'string' }
      ? string
      : S extends { type: 'number' }
        ? number
        : S extends { type: 'boolean' }
          ? boolean
          : S extends { type: 'object' }
            ? Record<string, unknown>
            : S extends { type: 'array' }
              ? unknown[]
              : S extends { default: infer D }
                ? WidenDefault<D>
                : unknown;
export type InferAtomAttrs<S extends Record<string, AttrSpec>> = {
    -readonly [K in keyof S]: AttrValue<S[K]>;
};
type AtomInput<S extends Record<string, AttrSpec>> = {
    -readonly [K in keyof S as HasDefault<S[K]> extends true ? never : K]: AttrValue<S[K]>;
} & { -readonly [K in keyof S as HasDefault<S[K]> extends true ? K : never]?: AttrValue<S[K]> };

export interface SerializedEditorAtoms {
    nodeTypes: string[];
    estimatedHeights: Record<string, number>;
}

function isAtomComponent(value: unknown): value is AtomComponent {
    if (typeof value === 'function') return true;
    if (value == null || typeof value !== 'object' || !('$$typeof' in value)) return false;
    return (
        value.$$typeof === Symbol.for('react.memo') ||
        value.$$typeof === Symbol.for('react.forward_ref') ||
        value.$$typeof === Symbol.for('react.lazy')
    );
}

function isSafeTag(tag: string): boolean {
    return ATOM_HTML_IDENTIFIER.test(tag) && !ATOM_HTML_DENIED_TAGS.has(tag);
}

function isSafeAttr(name: string): boolean {
    return (
        ATOM_HTML_IDENTIFIER.test(name) &&
        !name.startsWith('on') &&
        !ATOM_HTML_DENIED_ATTRS.has(name)
    );
}

function stringRecord(value: unknown, label: string): Record<string, string> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${label} must be an object`);
    }
    const result: Record<string, string> = {};
    for (const [key, entry] of Object.entries(value)) {
        if (typeof entry !== 'string') {
            throw new Error(`${label} '${key}' must be a string`);
        }
        result[key] = entry;
    }
    return result;
}

function kebabCase(value: string): string {
    return value
        .replace(/([A-Z]+)([A-Z][a-z])/g, '$1-$2')
        .replace(/([a-z0-9])([A-Z])/g, '$1-$2')
        .replace(/_/g, '-')
        .toLowerCase();
}

function normalizedHtmlRules(config: AtomNodeConfig<any>): NodeHtmlRules {
    if (config.html == null || typeof config.html !== 'object' || Array.isArray(config.html)) {
        throw new Error(`atom '${config.name}' requires HTML rules`);
    }
    const raw = config.html as NodeHtmlRules & Record<string, unknown>;
    if (Object.keys(raw).some((key) => !['tag', 'staticAttrs', 'attrMap'].includes(key))) {
        throw new Error(`atom '${config.name}' has an unknown HTML rule field`);
    }
    if (typeof raw.tag !== 'string' || !isSafeTag(raw.tag)) {
        throw new Error(`atom '${config.name}' has an invalid HTML tag '${String(raw.tag)}'`);
    }
    const staticAttrs = stringRecord(raw.staticAttrs, `atom '${config.name}' staticAttrs`);
    if (Object.keys(staticAttrs).length === 0) {
        throw new Error(`atom '${config.name}' requires a static attribute discriminator`);
    }
    for (const name of Object.keys(staticAttrs)) {
        if (!isSafeAttr(name)) {
            throw new Error(`atom '${config.name}' has an invalid HTML attribute '${name}'`);
        }
    }

    const attrs = config.attrs ?? {};
    const attrMap =
        raw.attrMap == null
            ? Object.fromEntries(
                  Object.keys(attrs).map((name) => [name, `data-${kebabCase(name)}`])
              )
            : stringRecord(raw.attrMap, `atom '${config.name}' attrMap`);
    for (const name of Object.keys(attrMap)) {
        if (!Object.prototype.hasOwnProperty.call(attrs, name)) {
            throw new Error(`atom '${config.name}' maps undeclared attribute '${name}'`);
        }
    }
    for (const name of Object.keys(attrs)) {
        if (!Object.prototype.hasOwnProperty.call(attrMap, name)) {
            throw new Error(`atom '${config.name}' attrMap is missing '${name}'`);
        }
    }
    const targets = new Set<string>();
    for (const target of Object.values(attrMap)) {
        if (!isSafeAttr(target)) {
            throw new Error(`atom '${config.name}' has an invalid HTML attribute '${target}'`);
        }
        if (targets.has(target)) {
            throw new Error(`atom '${config.name}' repeats HTML attribute '${target}'`);
        }
        if (Object.prototype.hasOwnProperty.call(staticAttrs, target)) {
            throw new Error(`atom '${config.name}' collides on HTML attribute '${target}'`);
        }
        targets.add(target);
    }
    return { tag: raw.tag, staticAttrs, attrMap };
}

export function defineAtomNode<const S extends Record<string, AttrSpec>>(
    config: Omit<AtomNodeConfig<NoInfer<InferAtomAttrs<S>>>, 'attrs'> & { attrs: S }
): AtomNodeDefinition<InferAtomAttrs<S>, AtomInput<S>> & { readonly attrs: S };
export function defineAtomNode<A = Record<string, unknown>>(
    config: AtomNodeConfig<A> & (string extends keyof A ? {} : { attrs: AtomAttributeSpecs<A> })
): AtomNodeDefinition<A, Required<A>>;
export function defineAtomNode(config: AtomNodeConfig<any>): AtomNodeDefinition<any> {
    if (config == null || typeof config !== 'object') {
        throw new Error('atom config must be an object');
    }
    if (typeof config.name !== 'string' || config.name.length === 0) {
        throw new Error('atom name must be a non-empty string');
    }
    if (RESERVED_WIRE_NODE_TYPES.has(config.name)) {
        throw new Error(`atom name '${config.name}' is reserved`);
    }
    if (!isAtomComponent(config.component)) {
        throw new Error(`atom '${config.name}' requires a component`);
    }
    if (
        config.estimatedHeight != null &&
        (!Number.isFinite(config.estimatedHeight) || config.estimatedHeight < 0)
    ) {
        throw new Error(`atom '${config.name}' estimatedHeight must be non-negative`);
    }
    if ('allowUndeclaredAttrs' in (config as unknown as Record<string, unknown>)) {
        throw new Error(`atom '${config.name}' cannot allow undeclared attributes`);
    }
    if (config.attrs != null && (typeof config.attrs !== 'object' || Array.isArray(config.attrs))) {
        throw new Error(`atom '${config.name}' attrs must be an object`);
    }

    for (const [name, spec] of Object.entries(config.attrs ?? {}))
        validateAttributeSpec(spec, name);
    if (config.idAttribute !== undefined && config.attrs?.[config.idAttribute]?.type !== 'string')
        throw new Error(`atom '${config.name}' identifier attribute must declare type string`);
    const html = normalizedHtmlRules(config);
    const nodeSpec: NodeSpec = {
        name: config.name,
        content: '',
        group: 'block',
        role: 'block',
        isVoid: true,
        ...(config.attrs == null ? {} : { attrs: config.attrs }),
        html,
    };
    return {
        name: config.name,
        ...(config.attrs == null ? {} : { attrs: config.attrs }),
        html,
        component: config.component,
        ...(config.idAttribute === undefined ? {} : { idAttribute: config.idAttribute }),
        estimatedHeight: config.estimatedHeight ?? DEFAULT_ATOM_CHIP_HEIGHT,
        nodeSpec,
        buildFragmentJson(attrs, descriptor) {
            validateAttributes(attrs ?? {}, config.attrs ?? {});
            return {
                type: descriptor?.documentNodeName ?? 'doc',
                content: [
                    {
                        type: config.name,
                        ...(attrs == null ? {} : { attrs }),
                    },
                ],
            };
        },
    };
}

function schemaValuesEqual(left: unknown, right: unknown): boolean {
    if (Object.is(left, right)) return true;
    if (left == null || right == null || typeof left !== 'object' || typeof right !== 'object') {
        return false;
    }
    if (Array.isArray(left) || Array.isArray(right)) {
        return (
            Array.isArray(left) &&
            Array.isArray(right) &&
            left.length === right.length &&
            left.every((value, index) => schemaValuesEqual(value, right[index]))
        );
    }
    const leftRecord = left as Record<string, unknown>;
    const rightRecord = right as Record<string, unknown>;
    const leftKeys = Object.keys(leftRecord).sort();
    const rightKeys = Object.keys(rightRecord).sort();
    return (
        leftKeys.length === rightKeys.length &&
        leftKeys.every(
            (key, index) =>
                key === rightKeys[index] && schemaValuesEqual(leftRecord[key], rightRecord[key])
        )
    );
}

function assertUnambiguousHtmlRules(nodes: readonly NodeSpec[]): void {
    const ruled = nodes.filter((node) => node.html != null);
    for (let rightIndex = 1; rightIndex < ruled.length; rightIndex += 1) {
        const right = ruled[rightIndex];
        for (let leftIndex = 0; leftIndex < rightIndex; leftIndex += 1) {
            const left = ruled[leftIndex];
            if (left.html?.tag !== right.html?.tag) continue;
            const leftStatic = left.html?.staticAttrs ?? {};
            const rightStatic = right.html?.staticAttrs ?? {};
            const conflicts = Object.entries(leftStatic).some(
                ([name, value]) =>
                    Object.prototype.hasOwnProperty.call(rightStatic, name) &&
                    rightStatic[name] !== value
            );
            if (!conflicts) {
                throw new Error(`ambiguous atom HTML rules for '${left.name}' and '${right.name}'`);
            }
        }
    }
}

export function withAtomsSchema(
    schema: SchemaDefinition,
    atoms: readonly AtomNodeDefinition<any>[]
): SchemaDefinition {
    const nodes = [...schema.nodes];
    let changed = false;
    for (const atom of atoms) {
        const existing = nodes.find((node) => node.name === atom.name);
        if (existing != null) {
            if (!schemaValuesEqual(existing, atom.nodeSpec)) {
                throw new Error(`schema already declares conflicting node '${atom.name}'`);
            }
            continue;
        }
        nodes.push(atom.nodeSpec);
        changed = true;
    }
    assertUnambiguousHtmlRules(nodes);
    return changed ? { ...schema, nodes } : schema;
}

export function serializeEditorAtoms(
    atoms?: readonly AtomNodeDefinition<any>[]
): string | undefined {
    if (atoms == null || atoms.length === 0) return undefined;
    const serialized: SerializedEditorAtoms = {
        nodeTypes: [],
        estimatedHeights: {},
    };
    for (const atom of atoms) {
        if (Object.prototype.hasOwnProperty.call(serialized.estimatedHeights, atom.name)) continue;
        serialized.nodeTypes.push(atom.name);
        serialized.estimatedHeights[atom.name] = atom.estimatedHeight ?? DEFAULT_ATOM_CHIP_HEIGHT;
    }
    return JSON.stringify(serialized);
}
