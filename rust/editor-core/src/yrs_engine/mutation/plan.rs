// Responsibility-oriented implementation shards are included into this
// private facade so existing item visibility and module paths remain stable.
include!("plan/model.rs");
include!("plan/work.rs");
include!("plan/envelope.rs");
include!("plan/preflight.rs");
include!("plan/execute.rs");
include!("plan/estimate.rs");
