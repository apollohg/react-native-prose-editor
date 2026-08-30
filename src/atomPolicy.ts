export const RESERVED_WIRE_NODE_TYPES: ReadonlySet<string> = new Set([
    '__opaque',
    '__opaque_json',
    '__skip',
]);

export const ATOM_HTML_IDENTIFIER = /^[a-z][a-z0-9-]*$/;

export const ATOM_HTML_DENIED_TAGS: ReadonlySet<string> = new Set([
    'script',
    'style',
    'iframe',
    'object',
    'embed',
    'link',
    'meta',
    'base',
    'title',
    'head',
    'html',
    'body',
    'form',
    'textarea',
    'select',
    'option',
    'button',
    'area',
    'br',
    'col',
    'hr',
    'img',
    'input',
    'param',
    'source',
    'track',
    'wbr',
]);

export const ATOM_HTML_DENIED_ATTRS: ReadonlySet<string> = new Set([
    'style',
    'srcdoc',
    'href',
    'src',
    'srcset',
    'action',
    'formaction',
]);
