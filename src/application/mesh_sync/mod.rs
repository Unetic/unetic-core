pub mod extender;
pub mod master;

use crate::application::app::App;
use crate::domain::PublicState;
use crate::domain::wan::WanProtocol;
use std::sync::Arc;

pub fn start_mesh_sync(app: Arc<App>, event_rx: tokio::sync::broadcast::Receiver<PublicState>) {
    let is_extender = {
        let inner = app.inner.lock().unwrap();
        inner.config.wan.proto == WanProtocol::Extender
    };

    if is_extender {
        extender::start_extender_loop(app);
    } else {
        master::start_master_loop(app, event_rx);
    }
}
