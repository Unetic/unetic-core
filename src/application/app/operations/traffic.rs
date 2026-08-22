use crate::application::App;
use crate::domain::traffic::TrafficState;

impl App {
    pub(crate) fn update_traffic(&self, traffic: TrafficState) {
        let mut inner = self.inner.lock().unwrap();
        if inner.traffic != traffic {
            inner.traffic = traffic;
            drop(inner);
            self.publish();
        }
    }
}
