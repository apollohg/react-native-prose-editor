[Back to docs index](../README.md)

# Schema Reference

The built-in schemas now include a block `image` node by default. The package also exports helpers for image support:

```ts
interface ImageNodeAttributes {
  src: string;
  alt?: string | null;
  title?: string | null;
  width?: number | null;
  height?: number | null;
}

const IMAGE_NODE_NAME = 'image';

function imageNodeSpec(name?: string): NodeSpec;
function withImagesSchema(schema: SchemaDefinition): SchemaDefinition;
function buildImageFragmentJson(attrs: ImageNodeAttributes): DocumentJSON;
```

## `SchemaDefinition`

```ts
interface SchemaDefinition {
  nodes: NodeSpec[];
  marks: MarkSpec[];
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `nodes` | `NodeSpec[]` | Available node definitions. |
| `marks` | `MarkSpec[]` | Available mark definitions. |

## `NodeSpec`

```ts
interface NodeSpec {
  name: string;
  content: string;
  group?: string;
  attrs?: Record<string, AttrSpec>;
  role: string;
  htmlTag?: string;
  isVoid?: boolean;
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | `string` | Schema node name used in commands and JSON. |
| `content` | `string` | Content expression string. |
| `group` | `string \| undefined` | Optional node group. |
| `attrs` | `Record<string, AttrSpec> \| undefined` | Optional attribute spec map. |
| `role` | `string` | Semantic role understood by the Rust core. |
| `htmlTag` | `string \| undefined` | HTML tag used during HTML parsing and serialization. |
| `isVoid` | `boolean \| undefined` | Whether the node behaves as a void node. |

## `MarkSpec`

```ts
interface MarkSpec {
  name: string;
  attrs?: Record<string, AttrSpec>;
  excludes?: string;
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | `string` | Schema mark name used by commands and JSON. |
| `attrs` | `Record<string, AttrSpec> \| undefined` | Optional attribute definitions for the mark. |
| `excludes` | `string \| undefined` | Optional exclusion string. |

## `AttrSpec`

```ts
interface AttrSpec {
  default?: unknown;
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `default` | `unknown` | Default value for the attribute when none is provided. |

## Built-In Presets

## `tiptapSchema`

Uses camelCase names.

### Default Node Names

| Kind | Names |
| --- | --- |
| Structural | `doc`, `paragraph`, `h1`, `h2`, `h3`, `h4`, `h5`, `h6`, `blockquote`, `text` |
| Lists | `bulletList`, `orderedList`, `listItem` |
| Void nodes | `hardBreak`, `horizontalRule`, `image` |

### Default Mark Names

| Marks |
| --- |
| `bold`, `italic`, `underline`, `strike`, `link` |

## `prosemirrorSchema`

Uses snake_case names.

### Default Node Names

| Kind | Names |
| --- | --- |
| Structural | `doc`, `paragraph`, `h1`, `h2`, `h3`, `h4`, `h5`, `h6`, `blockquote`, `text` |
| Lists | `bullet_list`, `ordered_list`, `list_item` |
| Void nodes | `hard_break`, `horizontal_rule`, `image` |

### Default Mark Names

| Marks |
| --- |
| `bold`, `italic`, `underline`, `strike`, `link` |

## Related Docs

- [NativeRichTextEditor Reference](./native-rich-text-editor.md)
- [Getting Started](../guides/getting-started.md)
