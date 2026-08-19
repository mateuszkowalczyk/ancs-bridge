use crate::atomic_file;
use anyhow::{bail, Context, Result};
use bluer::Address;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Configuration {
    pub schema_version: u32,
    pub bluetooth: BluetoothConfiguration,
    pub desktop: DesktopConfiguration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BluetoothConfiguration {
    pub adapter: String,
    pub device_address: String,
    pub device_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesktopConfiguration {
    pub suppress_phone_audio: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedConfiguration {
    pub adapter: String,
    pub device_address: Address,
    pub device_name: String,
    pub suppress_phone_audio: bool,
}

impl Configuration {
    pub fn validate(&self) -> Result<ValidatedConfiguration> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            bail!(
                "unsupported configuration schema version {}",
                self.schema_version
            );
        }
        validate_adapter(&self.bluetooth.adapter)?;
        let device_address = self
            .bluetooth
            .device_address
            .parse()
            .context("invalid configured Bluetooth identity address")?;
        Ok(ValidatedConfiguration {
            adapter: self.bluetooth.adapter.clone(),
            device_address,
            device_name: self.bluetooth.device_name.clone(),
            suppress_phone_audio: self.desktop.suppress_phone_audio,
        })
    }
}

fn validate_adapter(adapter: &str) -> Result<()> {
    if adapter.is_empty()
        || adapter.len() > 64
        || !adapter
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-'))
    {
        bail!("invalid configured Bluetooth adapter name");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationStore {
    path: PathBuf,
}

impl ConfigurationStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn from_environment() -> Result<Self> {
        Self::from_environment_values(
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        )
    }

    pub fn from_environment_values(
        xdg_config_home: Option<OsString>,
        home: Option<OsString>,
    ) -> Result<Self> {
        let base = absolute(xdg_config_home.as_deref())
            .or_else(|| absolute(home.as_deref()).map(|value| value.join(".config")));
        let base = base.context("no absolute XDG_CONFIG_HOME or HOME is available")?;
        Ok(Self::new(base.join("ancs-bridge/config.toml")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<ValidatedConfiguration>> {
        let source = match fs::read_to_string(&self.path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading configuration {}", self.path.display()))
            }
        };
        let configuration: Configuration = toml::from_str(&source)
            .with_context(|| format!("parsing configuration {}", self.path.display()))?;
        configuration.validate().map(Some)
    }

    pub fn save(&self, configuration: &Configuration) -> Result<ValidatedConfiguration> {
        let validated = configuration.validate()?;
        let source = toml::to_string_pretty(configuration).context("serializing configuration")?;
        atomic_file::replace(&self.path, source.as_bytes(), 0o700)?;
        Ok(validated)
    }
}

fn absolute(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|value| value.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic_file::test_support::TestDirectory;
    use std::os::unix::fs::PermissionsExt;

    fn configuration(address: &str) -> Configuration {
        Configuration {
            schema_version: CONFIG_SCHEMA_VERSION,
            bluetooth: BluetoothConfiguration {
                adapter: "hci0".into(),
                device_address: address.into(),
                device_name: "iPhone".into(),
            },
            desktop: DesktopConfiguration {
                suppress_phone_audio: true,
            },
        }
    }

    #[test]
    fn resolves_xdg_and_home_fallback_paths() {
        let xdg = ConfigurationStore::from_environment_values(
            Some(OsString::from("/tmp/config")),
            Some(OsString::from("/home/example")),
        )
        .unwrap();
        assert_eq!(xdg.path(), Path::new("/tmp/config/ancs-bridge/config.toml"));

        let fallback = ConfigurationStore::from_environment_values(
            Some(OsString::from("relative")),
            Some(OsString::from("/home/example")),
        )
        .unwrap();
        assert_eq!(
            fallback.path(),
            Path::new("/home/example/.config/ancs-bridge/config.toml")
        );
        assert!(ConfigurationStore::from_environment_values(None, None).is_err());
    }

    #[test]
    fn saves_loads_validates_and_replaces_with_private_permissions() {
        let directory = TestDirectory::new("configuration");
        let path = directory.path().join("nested/config.toml");
        let store = ConfigurationStore::new(path.clone());
        let validated = store.save(&configuration("aa:bb:cc:dd:ee:ff")).unwrap();
        assert_eq!(validated.device_address.to_string(), "AA:BB:CC:DD:EE:FF");
        assert_eq!(store.load().unwrap(), Some(validated));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
        store.save(&configuration("AA:BB:CC:DD:EE:FF")).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn invalid_save_does_not_replace_previous_configuration() {
        let directory = TestDirectory::new("configuration-failure");
        let store = ConfigurationStore::new(directory.path().join("config.toml"));
        store.save(&configuration("AA:BB:CC:DD:EE:FF")).unwrap();
        let before = fs::read(store.path()).unwrap();
        assert!(store.save(&configuration("not-an-address")).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), before);
    }

    #[test]
    fn rejects_missing_malformed_and_unsupported_values_without_payload_fields() {
        let directory = TestDirectory::new("configuration-invalid");
        let store = ConfigurationStore::new(directory.path().join("config.toml"));
        assert!(store.load().unwrap().is_none());

        fs::write(store.path(), "not = [valid").unwrap();
        assert!(store.load().is_err());

        for invalid in [
            Configuration {
                schema_version: 2,
                ..configuration("AA:BB:CC:DD:EE:FF")
            },
            Configuration {
                bluetooth: BluetoothConfiguration {
                    adapter: "hci/0".into(),
                    ..configuration("AA:BB:CC:DD:EE:FF").bluetooth
                },
                ..configuration("AA:BB:CC:DD:EE:FF")
            },
            configuration("invalid"),
        ] {
            assert!(invalid.validate().is_err());
        }

        let serialized = toml::to_string(&configuration("AA:BB:CC:DD:EE:FF")).unwrap();
        for canary in ["notification_title", "notification_body", "app_payload"] {
            assert!(!serialized.contains(canary));
        }
    }
}
