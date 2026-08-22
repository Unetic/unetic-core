use super::App;
use crate::domain::{
    errors::LegacyAppError,
    extender::{ExtenderClient, PendingExtender, ScannedNetwork},
    ports::PhysicalPort,
};

impl App {
    pub(crate) fn mesh_add_pending(&self, extender: PendingExtender) {
        let mut inner = self.inner.lock().expect("app state poisoned");
        if inner
            .pending_extenders
            .iter()
            .any(|current| current.mac.eq_ignore_ascii_case(&extender.mac))
        {
            return;
        }
        if inner.pending_extenders.len() >= 50 {
            inner.pending_extenders.remove(0);
        }
        inner.pending_extenders.push(extender);
        drop(inner);
        self.publish();
    }

    pub(crate) fn take_approved_pairing_token(
        &self,
        mac: &str,
        pairing_key: &str,
    ) -> Option<String> {
        let mut inner = self.inner.lock().expect("app state poisoned");
        if inner.approved_pairings.get(mac).map(String::as_str) != Some(pairing_key) {
            return None;
        }
        inner.approved_pairings.remove(mac);
        inner
            .config
            .extenders
            .iter()
            .find(|extender| extender.mac == mac)
            .map(|extender| extender.auth_token.clone())
    }

    pub(crate) fn extender_pairing_key(&self) -> String {
        self.inner
            .lock()
            .expect("app state poisoned")
            .extender_pairing_key
            .clone()
    }

    pub(crate) fn extender_set_token(&self, token: String) -> Result<(), LegacyAppError> {
        self.update_extender_token(Some(token))
    }

    pub(crate) fn extender_clear_token(&self) -> Result<(), LegacyAppError> {
        self.update_extender_token(None)
    }

    fn update_extender_token(&self, token: Option<String>) -> Result<(), LegacyAppError> {
        let mut inner = self.inner.lock().expect("app state poisoned");
        let mut config = inner.config.clone();
        config.extender_auth_token = token;
        self.store.persist_config(&config)?;
        inner.config = config;
        drop(inner);
        self.publish();
        Ok(())
    }

    pub(crate) fn update_extender_ports(&self, mac: String, ports: Vec<PhysicalPort>) {
        self.inner
            .lock()
            .expect("app state poisoned")
            .extender_ports
            .insert(mac, ports);
        self.publish();
    }

    pub(crate) fn update_extender_telemetry(&self, mac: String, clients: Vec<ExtenderClient>) {
        self.inner
            .lock()
            .expect("app state poisoned")
            .extender_clients
            .insert(mac, clients);
        self.publish();
    }

    pub(crate) fn update_scan_results(&self, mac: String, networks: Vec<ScannedNetwork>) {
        self.inner
            .lock()
            .expect("app state poisoned")
            .latest_scans
            .insert(mac, networks);
        self.publish();
    }
}
