use application::ports::FunctionCatalog;
use application::{Accounts, PublishFunction};
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiState {
    pub publish: Arc<PublishFunction>,
    pub catalog: Arc<dyn FunctionCatalog>,
    pub accounts: Arc<Accounts>,
    pub min_deploy_credits: u64,
}

#[derive(Clone)]
pub struct CatalogState {
    pub catalog: Arc<dyn FunctionCatalog>,
}

#[derive(Clone)]
pub struct PublishState {
    pub publish: Arc<PublishFunction>,
    pub accounts: Arc<Accounts>,
    pub min_deploy_credits: u64,
}
