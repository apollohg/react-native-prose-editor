#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct YrsMutationPlan {
    pub actions: Vec<YrsMutationAction>,
}

#[derive(Debug)]
pub(crate) enum YrsMutationAction {}
