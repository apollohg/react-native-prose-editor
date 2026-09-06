import { type FakeNativeEditorV2Runtime } from './nativeEditorV2FakeTypes';
import { createFakeRuntimeState } from './createFakeRuntimeState';
import { createFakeLifecycleModule } from './createFakeLifecycleModule';
import { createFakeEditingModule } from './createFakeEditingModule';
import { createFakeCollaborationModule } from './createFakeCollaborationModule';
import { createFakeRuntimeControls } from './createFakeRuntimeControls';

export function createFakeNativeEditorV2Runtime(): FakeNativeEditorV2Runtime {
    const state = createFakeRuntimeState();
    const lifecycle = createFakeLifecycleModule(state);
    const editing = createFakeEditingModule(state);
    const collaboration = createFakeCollaborationModule(state);
    return createFakeRuntimeControls({ ...lifecycle, ...editing, ...collaboration, ...state });
}
