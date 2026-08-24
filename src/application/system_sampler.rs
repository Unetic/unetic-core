use tracing::warn;

use super::app::App;

impl App {
    pub(crate) fn refresh_system_runtime(&self) {
        let runtime = match self.backend.read_system_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                warn!(%error, "failed to read system runtime metrics");
                return;
            }
        };

        self.inner
            .lock()
            .expect("app state poisoned")
            .system_runtime = runtime;
        self.publish_system_runtime();
    }
}
