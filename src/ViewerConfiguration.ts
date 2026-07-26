import type { SchemaDefinition } from './schemas';
import type { ResolvedEditorResourceLimits } from './ResourceLimits';

export interface PreparedProseViewerMentionsConfiguration {
    trigger?: string;
    prefix?: string;
}

export interface PreparedProseViewerConfiguration {
    initialization: { type: 'localEmpty' };
    schema: SchemaDefinition;
    policy?: { allowBase64Images: true };
    limits?: { resource: ResolvedEditorResourceLimits };
    mentions?: PreparedProseViewerMentionsConfiguration;
}

export function serializePreparedProseViewerConfiguration(
    configuration: PreparedProseViewerConfiguration
): string {
    return JSON.stringify(configuration);
}
