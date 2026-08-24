use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    domain::errors::{ErrorCode, ErrorStage, LegacyAppError},
    domain::{DesiredConfig, TransactionJournal},
};

#[derive(Debug, Clone)]
pub struct StateStore {
    root: PathBuf,
}

impl StateStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn ensure(&self) -> Result<(), LegacyAppError> {
        fs::create_dir_all(&self.root).map_err(store_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
                .map_err(store_error)?;
            for name in ["config.json", "transaction.json", "devices.json"] {
                let path = self.root.join(name);
                if path.exists() {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                        .map_err(store_error)?;
                }
            }
        }
        Ok(())
    }

    pub fn load_config(&self) -> Result<Option<DesiredConfig>, LegacyAppError> {
        self.read_json("config.json")
    }

    pub fn persist_config(&self, value: &DesiredConfig) -> Result<(), LegacyAppError> {
        self.write_json("config.json", value)
    }

    pub fn load_transaction(&self) -> Result<Option<TransactionJournal>, LegacyAppError> {
        self.read_json("transaction.json")
    }

    pub fn persist_transaction(&self, value: &TransactionJournal) -> Result<(), LegacyAppError> {
        self.write_json("transaction.json", value)
    }

    pub fn clear_transaction(&self) -> Result<(), LegacyAppError> {
        let path = self.root.join("transaction.json");
        match fs::remove_file(&path) {
            Ok(()) => self.sync_dir(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(store_error(error)),
        }
    }

    pub fn load_device_inventory(
        &self,
    ) -> Result<Option<crate::domain::device_inventory::DeviceInventory>, LegacyAppError> {
        self.read_json("devices.json")
    }

    pub fn persist_device_inventory(
        &self,
        value: &crate::domain::device_inventory::DeviceInventory,
    ) -> Result<(), LegacyAppError> {
        self.write_json("devices.json", value)
    }

    fn read_json<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, LegacyAppError> {
        let path = self.root.join(name);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(store_error(error)),
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            LegacyAppError::new(
                ErrorCode::StateCorrupt,
                ErrorStage::Persist,
                format!("failed to parse {}: {error}", path.display()),
            )
        })
    }

    fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<(), LegacyAppError> {
        self.ensure()?;
        let path = self.root.join(name);
        let tmp = self.root.join(format!("{name}.tmp"));
        let data = serde_json::to_vec_pretty(value).map_err(|error| {
            LegacyAppError::new(
                ErrorCode::StateStoreFailed,
                ErrorStage::Persist,
                format!("failed to serialize state: {error}"),
            )
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .map_err(store_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(store_error)?;
        }
        file.write_all(&data).map_err(store_error)?;
        file.write_all(b"\n").map_err(store_error)?;
        file.sync_all().map_err(store_error)?;
        drop(file);

        fs::rename(&tmp, &path).map_err(store_error)?;
        self.sync_dir()
    }

    fn sync_dir(&self) -> Result<(), LegacyAppError> {
        File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(store_error)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn store_error(error: io::Error) -> LegacyAppError {
    LegacyAppError::new(
        ErrorCode::StateStoreFailed,
        ErrorStage::Persist,
        error.to_string(),
    )
}
