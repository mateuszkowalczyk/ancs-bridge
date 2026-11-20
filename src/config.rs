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
pub const MAX_IGNORED_APPS: usize = 100;
pub const MAX_IGNORED_APP_BYTES: usize = 255;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Configuration {
    pub schema_version: u32,
    pub bluetooth: BluetoothConfiguration,
    #[serde(default, skip_serializing_if = "notifications_are_empty")]
    pub notifications: Option<NotificationsConfiguration>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BluetoothConfiguration {
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_address: Option<String>,
    pub device_address: String,
    pub device_name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NotificationsConfiguration {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_apps: Vec<String>,
}

fn notifications_are_empty(configuration: &Option<NotificationsConfiguration>) -> bool {
    configuration
        .as_ref()
        .is_none_or(|configuration| configuration.ignore_apps.is_empty())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedConfiguration {
    pub adapter: String,
    pub adapter_address: Option<Address>,
    pub device_address: Address,
    pub device_name: String,
    pub ignored_apps: Vec<String>,
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
        let adapter_address = self
            .bluetooth
            .adapter_address
            .as_deref()
            .map(|value| {
                value
                    .parse()
                    .context("invalid configured Bluetooth adapter identity address")
            })
            .transpose()?;
        let device_address = self
            .bluetooth
            .device_address
            .parse()
            .context("invalid configured Bluetooth identity address")?;
        let ignored_apps = self
            .notifications
            .as_ref()
            .map(|configuration| configuration.ignore_apps.as_slice())
            .unwrap_or_default();
        if ignored_apps.len() > MAX_IGNORED_APPS {
            bail!("notifications.ignore_apps exceeds {MAX_IGNORED_APPS} entries");
        }
        for (index, app) in ignored_apps.iter().enumerate() {
            let position = index + 1;
            if app.is_empty() || app.len() > MAX_IGNORED_APP_BYTES {
                bail!(
                    "notifications.ignore_apps entry {position} must contain 1..={MAX_IGNORED_APP_BYTES} UTF-8 bytes"
                );
            }
            if app.chars().any(char::is_control) {
                bail!("notifications.ignore_apps entry {position} must not contain Unicode control characters");
            }
            if app.chars().all(char::is_whitespace) {
                bail!(
                    "notifications.ignore_apps entry {position} must not contain only whitespace"
                );
            }
        }
        Ok(ValidatedConfiguration {
            adapter: self.bluetooth.adapter.clone(),
            adapter_address,
            device_address,
            device_name: self.bluetooth.device_name.clone(),
            ignored_apps: ignored_apps.to_vec(),
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

    pub fn remove(&self) -> Result<bool> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error)
                .with_context(|| format!("removing configuration {}", self.path.display())),
        }
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
                adapter_address: Some("11:22:33:44:55:66".into()),
                device_address: address.into(),
                device_name: "iPhone".into(),
            },
            notifications: None,
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
    fn legacy_configuration_without_adapter_identity_remains_readable() {
        let directory = TestDirectory::new("configuration-legacy-adapter");
        let store = ConfigurationStore::new(directory.path().join("config.toml"));
        fs::write(
            store.path(),
            "schema_version = 1\n[bluetooth]\nadapter = \"hci0\"\ndevice_address = \"AA:BB:CC:DD:EE:FF\"\ndevice_name = \"iPhone\"\n",
        )
        .unwrap();
        let configuration = store.load().unwrap().unwrap();
        assert_eq!(configuration.adapter, "hci0");
        assert_eq!(configuration.adapter_address, None);
        assert!(configuration.ignored_apps.is_empty());
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
        let mut invalid = configuration("AA:BB:CC:DD:EE:FF");
        invalid.notifications = Some(NotificationsConfiguration {
            ignore_apps: vec!["secret.invalid\napp".into()],
        });
        assert!(store.save(&invalid).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), before);
    }

    #[test]
    fn removal_is_idempotent() {
        let directory = TestDirectory::new("configuration-remove");
        let store = ConfigurationStore::new(directory.path().join("config.toml"));
        assert!(!store.remove().unwrap());
        store.save(&configuration("AA:BB:CC:DD:EE:FF")).unwrap();
        assert!(store.remove().unwrap());
        assert!(!store.remove().unwrap());
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
            Configuration {
                bluetooth: BluetoothConfiguration {
                    adapter_address: Some("invalid".into()),
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

    #[test]
    fn notification_policy_roundtrips_without_normalization() {
        let directory = TestDirectory::new("configuration-notifications");
        let store = ConfigurationStore::new(directory.path().join("config.toml"));
        let mut configuration = configuration("AA:BB:CC:DD:EE:FF");
        let apps: Vec<String> = [
            "com.Example.App",
            "com.example.app",
            " café ",
            "cafe\u{301}",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        configuration.notifications = Some(NotificationsConfiguration {
            ignore_apps: apps.clone(),
        });
        let validated = store.save(&configuration).unwrap();
        assert_eq!(validated.ignored_apps, apps);
        assert_eq!(store.load().unwrap(), Some(validated));
        let source = fs::read_to_string(store.path()).unwrap();
        assert!(source.contains("[notifications]"));
        assert_eq!(
            toml::from_str::<Configuration>(&source).unwrap(),
            configuration
        );
    }

    #[test]
    fn absent_and_empty_notification_policies_are_omitted() {
        for notifications in [None, Some(NotificationsConfiguration::default())] {
            let configuration = Configuration {
                notifications,
                ..configuration("AA:BB:CC:DD:EE:FF")
            };
            let source = toml::to_string(&configuration).unwrap();
            assert!(!source.contains("notifications"));
            assert!(!source.contains("ignore_apps"));
            assert!(configuration.validate().unwrap().ignored_apps.is_empty());
        }
        let source = format!(
            "{}\n[notifications]\n",
            toml::to_string(&configuration("AA:BB:CC:DD:EE:FF")).unwrap()
        );
        let parsed: Configuration = toml::from_str(&source).unwrap();
        assert!(parsed.validate().unwrap().ignored_apps.is_empty());
        assert!(!toml::to_string(&parsed).unwrap().contains("notifications"));
    }

    #[test]
    fn notification_policy_enforces_count_and_utf8_byte_boundaries() {
        let mut configuration = configuration("AA:BB:CC:DD:EE:FF");
        configuration.notifications = Some(NotificationsConfiguration {
            ignore_apps: vec!["app".into(); MAX_IGNORED_APPS],
        });
        assert!(configuration.validate().is_ok());
        configuration
            .notifications
            .as_mut()
            .unwrap()
            .ignore_apps
            .push("extra".into());
        let error = configuration.validate().unwrap_err().to_string();
        assert!(error.contains("100"));
        assert!(!error.contains("extra"));

        for valid in ["a".repeat(255), format!("{}a", "é".repeat(127))] {
            configuration.notifications.as_mut().unwrap().ignore_apps = vec![valid];
            assert!(configuration.validate().is_ok());
        }
        for invalid in ["a".repeat(256), "é".repeat(128)] {
            configuration.notifications.as_mut().unwrap().ignore_apps = vec![invalid];
            assert!(configuration.validate().is_err());
        }
    }

    #[test]
    fn notification_policy_errors_identify_index_without_values() {
        for invalid in [
            String::new(),
            "private.app".repeat(24),
            "private\napp".into(),
            "private\u{7f}app".into(),
            "private\u{85}app".into(),
            "private\u{9f}app".into(),
            " \t ".into(),
            " \u{2003}\u{a0}".into(),
        ] {
            let configuration = Configuration {
                notifications: Some(NotificationsConfiguration {
                    ignore_apps: vec!["valid.app".into(), invalid.clone()],
                }),
                ..configuration("AA:BB:CC:DD:EE:FF")
            };
            let error = configuration.validate().unwrap_err().to_string();
            assert!(error.contains("notifications.ignore_apps entry 2"));
            assert!(!error.contains("private"));
            assert!(!error.contains("valid.app"));
            if !invalid.is_empty() {
                assert!(!error.contains(&invalid));
            }
        }
    }
}
