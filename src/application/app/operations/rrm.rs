use std::time::Duration;
use tracing::info;

use crate::application::app::App;

impl App {
    pub async fn optimize_channels(&self) {
        info!("Optimization triggered");
        let _ = self.rrm_tx.send(());

        tokio::time::sleep(Duration::from_secs(3)).await;

        let scans = {
            let inner = self.inner.lock().unwrap();
            inner.latest_scans.clone()
        };

        info!("Aggregated scans: {:?}", scans);
    }
}
