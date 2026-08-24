use crate::application::{App, app::StateTopic};
impl App {
    pub(crate) fn sample_traffic(&self) {
        let Ok(counters) = self.backend.read_traffic_counters() else {
            return;
        };
        let hw_offload_enabled = self
            .backend
            .read_switch_state()
            .map(|state| state.hw_offload.enabled)
            .unwrap_or(false);
        let mut inner = self.inner.lock().expect("app state poisoned");
        let changed = self
            .traffic_sampler
            .lock()
            .expect("traffic sampler poisoned")
            .sample(counters, hw_offload_enabled, &mut inner.traffic);
        let traffic = inner.traffic.clone();
        drop(inner);
        if changed {
            crate::application::traffic_sampler::save_history(&traffic);
            self.queue_state_update(StateTopic::Traffic, self.state(), false);
        }
    }
}
