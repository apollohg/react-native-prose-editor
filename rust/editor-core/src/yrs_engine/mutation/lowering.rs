use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;
use yrs::any::Any;
use yrs::branch::{Branch, BranchID};
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef};
use yrs::types::Attrs;
use yrs::ReadTxn;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};

use super::super::codec::{prepare_xml_nodes, PreparedTextRun, PreparedXmlChild, PreparedXmlNode};
use super::super::{OperationError, OperationResult};
use super::plan::{
    attrs_work, binary_partition_work, capture_document_guard, expected_preflight_work,
    fenwick_add, fenwick_prefix, invalid_action_range, scan_overflow, work_overflow,
    CreatedTextAction, DocumentGuard, ElementSignature, ParentSignature, StructuralParentSignature,
    TargetSignature, TextSignatureRun, XmlParentRef, YrsMutationAction, YrsMutationPlan,
};

// Responsibility shards intentionally use `include!` so lowering remains one private scope.
include!("lowering/model.rs");
include!("lowering/index.rs");
include!("lowering/list.rs");
include!("lowering/node.rs");
include!("lowering/text.rs");
include!("lowering/attrs.rs");
include!("lowering/block.rs");
include!("lowering/range.rs");
include!("lowering/prepared.rs");
include!("lowering/tests.rs");
