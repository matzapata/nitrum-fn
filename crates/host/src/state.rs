use std::sync::Arc;

use application::InvokeFunction;

#[derive(Clone)]
pub struct AppState {
    pub invoke: Arc<InvokeFunction>,
}
