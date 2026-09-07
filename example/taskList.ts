import type { NodeSpec, SchemaDefinition } from 'react-native-rich-text-editor';

/**
 * Node names the editor's list commands, native checklist rendering, and the
 * `taskList` / `taskItem` theme keys expect. `checked` is toggled natively by
 * tapping the checkbox.
 */
export const TASK_LIST_NODE_NAME = 'taskList';
export const TASK_ITEM_NODE_NAME = 'taskItem';

const taskListNodeSpec: NodeSpec = {
    name: TASK_LIST_NODE_NAME,
    content: `${TASK_ITEM_NODE_NAME}+`,
    group: 'block',
    role: 'list',
    htmlTag: 'ul',
};

const taskItemNodeSpec: NodeSpec = {
    name: TASK_ITEM_NODE_NAME,
    content: 'paragraph block*',
    role: 'listItem',
    attrs: { checked: { type: 'boolean', default: false } },
    htmlTag: 'li',
};

/** Adds the checklist nodes to a schema, mirroring `withImagesSchema`. */
export function withTaskListSchema(schema: SchemaDefinition): SchemaDefinition {
    const hasTaskList = schema.nodes.some((node) => node.name === TASK_LIST_NODE_NAME);
    if (hasTaskList) {
        return schema;
    }

    return {
        ...schema,
        nodes: [...schema.nodes, taskListNodeSpec, taskItemNodeSpec],
    };
}
