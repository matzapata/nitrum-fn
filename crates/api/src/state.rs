use std::sync::Arc;

use application::ports::FunctionCatalog;
use application::PublishFunction;

#[derive(Clone)]
pub struct ApiState {
    pub publish: Arc<PublishFunction>,
    pub catalog: Arc<dyn FunctionCatalog>,
}
