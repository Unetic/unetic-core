use crate::application::app::App;
use crate::domain::extender::KnownExtender;

impl App {
    pub fn mesh_pair_accept(&self, mac: String) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let idx = inner.pending_extenders.iter().position(|e| e.mac == mac);
        if let Some(i) = idx {
            let pending = inner.pending_extenders.remove(i);
            let token = uuid::Uuid::new_v4().to_string();
            let known = KnownExtender {
                mac: pending.mac,
                model: pending.model,
                ip: "".to_string(),
                auth_token: token,
            };
            inner.config.extenders.push(known);
            if let Err(e) = self.store.persist_config(&inner.config) {
                return Err(e.to_string());
            }
        } else {
            return Err("Extender not found in pending list".to_string());
        }
        drop(inner);
        self.publish();
        Ok(())
    }

    pub fn mesh_pair_reject(&self, mac: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending_extenders.retain(|e| e.mac != mac);
        drop(inner);
        self.publish();
    }
}
