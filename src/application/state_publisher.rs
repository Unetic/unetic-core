use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::Instant,
};

use super::{app::App, state::STATE_PUBLISH_INTERVAL};

impl App {
    pub fn start_state_publisher(self: &Arc<Self>) {
        let app = Arc::clone(self);
        thread::Builder::new()
            .name("unetic-state-publisher".into())
            .spawn(move || app.state_publisher_loop())
            .expect("failed to spawn state publisher thread");
    }

    fn state_publisher_loop(&self) {
        let mut next_tick = Instant::now();

        while !self.shutdown.load(Ordering::Relaxed) {
            self.sample_traffic();
            self.refresh_system_runtime();
            self.refresh_devices(true);
            self.flush_state_update();
            next_tick += STATE_PUBLISH_INTERVAL;
            next_tick = next_future_tick(next_tick);
            thread::sleep(next_tick.saturating_duration_since(Instant::now()));
        }
    }
}

fn next_future_tick(mut next_tick: Instant) -> Instant {
    let now = Instant::now();
    while next_tick <= now {
        next_tick += STATE_PUBLISH_INTERVAL;
    }
    next_tick
}
