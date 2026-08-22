use std::fmt;

#[derive(Debug, Clone)]
pub struct BridgeError(pub String);

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BridgeError {}

pub struct Bridge;

impl Bridge {
    pub fn load() -> Result<Self, BridgeError> {
        Err(BridgeError(
            "the OpenWrt ubus server is only available on musl targets".into(),
        ))
    }

    pub fn server<F>(
        &self,
        _methods: &[&str],
        _handler: F,
    ) -> Result<Server, BridgeError>
    where
        F: Fn(&str, &str) -> String + Send + Sync + 'static,
    {
        Err(BridgeError(
            "the OpenWrt ubus server is only available on musl targets".into(),
        ))
    }
}

pub struct Server;

impl Server {
    pub fn poll(&mut self, _timeout_ms: i32) -> Result<(), BridgeError> {
        Ok(())
    }

    pub fn notify(&mut self, _event: &str, _json: &str) -> Result<(), BridgeError> {
        Ok(())
    }
}
