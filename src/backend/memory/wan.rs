use super::MemoryBackend;
use crate::{
    errors::{DomainError, ErrorCode, ErrorStage},
    model::{DiscoveredWan, WanDesired, WanPublicState},
};

impl MemoryBackend {
    pub(crate) fn mem_discover_primary_wan(&self) -> Result<DiscoveredWan, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        Ok(DiscoveredWan {
            present: state.wan_committed.present,
            device: state.wan_committed.device.clone(),
            proto: state.wan_committed.proto,
            custom_mac: state.wan_committed.custom_mac.clone(),
            custom_mtu: state.wan_committed.custom_mtu,
            custom_dns: state.wan_committed.custom_dns.clone(),
            static_config: state.wan_committed.static_config.clone(),
            pppoe_config: state.wan_committed.pppoe_config.clone(),
        })
    }

    pub(crate) fn mem_read_wan_config(
        &self,
        session: Option<&str>,
    ) -> Result<WanDesired, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        if let Some(session) = session
            && let Some(staged) = state.wan_sessions.get(session)
        {
            return Ok(staged.clone());
        }
        Ok(state.wan_committed.clone())
    }

    pub(crate) fn mem_stage_wan_config(
        &self,
        session: &str,
        config: &WanDesired,
    ) -> Result<(), DomainError> {
        let mut state = self.state.lock().expect("memory backend poisoned");
        if state.failure.fail_stage {
            return Err(DomainError::new(
                ErrorCode::UciStageFailed,
                ErrorStage::Stage,
                "injected stage failure",
            ));
        }
        state
            .wan_sessions
            .insert(session.to_owned(), config.clone());
        Ok(())
    }

    pub(crate) fn mem_read_wan_runtime_status(&self) -> Result<WanPublicState, DomainError> {
        let state = self.state.lock().expect("memory backend poisoned");
        Ok(state.wan_runtime.clone())
    }
}
