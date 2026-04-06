//! Thread-safe global registry for multiple editor instances.
//!
//! Each editor is identified by a unique `EditorId` (u64). The registry uses
//! `Arc<Mutex<Editor>>` for thread-safe access from native platform threads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::editor::Editor;
use crate::intercept::InterceptorPipeline;
use crate::schema::Schema;

/// Unique identifier for an editor instance.
pub type EditorId = u64;

/// Sentinel value indicating no valid editor.
pub const INVALID_EDITOR_ID: EditorId = 0;

/// Global atomic counter for generating unique editor IDs.
/// Starts at 1 so that 0 can serve as INVALID_EDITOR_ID.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The global registry singleton.
static REGISTRY: OnceLock<Mutex<HashMap<EditorId, Arc<Mutex<Editor>>>>> = OnceLock::new();

fn global_registry() -> &'static Mutex<HashMap<EditorId, Arc<Mutex<Editor>>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Thread-safe global registry for editor instances.
pub struct EditorRegistry;

impl EditorRegistry {
    /// Create a new editor and return its ID.
    pub fn create(
        schema: Schema,
        interceptors: InterceptorPipeline,
        allow_base64_images: bool,
    ) -> EditorId {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let editor = Editor::new(schema, interceptors, allow_base64_images);
        let arc = Arc::new(Mutex::new(editor));

        let mut map = global_registry().lock().expect("registry lock poisoned");
        map.insert(id, arc);
        id
    }

    /// Get a handle to an editor by ID.
    pub fn get(id: EditorId) -> Option<Arc<Mutex<Editor>>> {
        let map = global_registry().lock().expect("registry lock poisoned");
        map.get(&id).cloned()
    }

    /// Destroy an editor, removing it from the registry.
    pub fn destroy(id: EditorId) {
        let mut map = global_registry().lock().expect("registry lock poisoned");
        map.remove(&id);
    }

    /// Number of active editors in the registry.
    pub fn count() -> usize {
        let map = global_registry().lock().expect("registry lock poisoned");
        map.len()
    }
}
