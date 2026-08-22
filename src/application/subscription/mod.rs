use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[repr(u32)]
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum SubscribeError {
    NotFound = 1,
    InvalidTtl = 2,
}

pub struct SubscriptionManager {
    subscribers: Mutex<HashMap<String, Instant>>,
}

impl SubscriptionManager {
    pub fn new() -> Self {
        Self {
            subscribers: Mutex::new(HashMap::new()),
        }
    }

    pub fn create(&self, ttl_mins: u32) -> Result<String, SubscribeError> {
        let ttl_mins = validate_ttl(ttl_mins)?;
        let id = super::state::generate_id("sub");
        let expiry = Instant::now() + Duration::from_secs(ttl_mins as u64 * 60);
        let mut subs = self.subscribers.lock().unwrap();
        subs.insert(id.clone(), expiry);
        Ok(id)
    }

    pub fn continue_sub(&self, id: &str, ttl_mins: u32) -> Result<(), SubscribeError> {
        let ttl_mins = validate_ttl(ttl_mins)?;
        let mut subs = self.subscribers.lock().unwrap();
        if !subs.contains_key(id) {
            return Err(SubscribeError::NotFound);
        }
        let expiry = Instant::now() + Duration::from_secs(ttl_mins as u64 * 60);
        subs.insert(id.to_string(), expiry);
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> Result<(), SubscribeError> {
        let mut subs = self.subscribers.lock().unwrap();
        if subs.remove(id).is_some() {
            Ok(())
        } else {
            Err(SubscribeError::NotFound)
        }
    }

    pub fn has_active_subscribers(&self) -> bool {
        let mut subs = self.subscribers.lock().unwrap();
        let now = Instant::now();
        subs.retain(|_, expiry| *expiry > now);
        !subs.is_empty()
    }
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_ttl(ttl_mins: u32) -> Result<u32, SubscribeError> {
    if (1..=99).contains(&ttl_mins) {
        Ok(ttl_mins)
    } else {
        Err(SubscribeError::InvalidTtl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_excessive_ttl() {
        let subscriptions = SubscriptionManager::new();

        assert!(matches!(
            subscriptions.create(0),
            Err(SubscribeError::InvalidTtl)
        ));
        assert!(matches!(
            subscriptions.create(100),
            Err(SubscribeError::InvalidTtl)
        ));
    }
}
