use crate::{
    audio::{remove_with_reload, AudioRule},
    config::ConfigurationStore,
    service::UserServiceControl,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bluer::{Address, Session};
use std::{path::PathBuf, sync::Arc};

pub const BRIDGE_UNIT: &str = "ancs-bridge.service";

#[async_trait]
pub trait BondCleanup: Send + Sync {
    async fn remove_exact(
        &self,
        adapter: &str,
        adapter_address: Option<Address>,
        address: Address,
    ) -> Result<()>;
}

#[derive(Default)]
pub struct BluezBondCleanup;

#[async_trait]
impl BondCleanup for BluezBondCleanup {
    async fn remove_exact(
        &self,
        adapter_name: &str,
        adapter_address: Option<Address>,
        address: Address,
    ) -> Result<()> {
        let session = Session::new().await.context("connecting to BlueZ")?;
        let adapter = if let Some(identity) = adapter_address {
            crate::bluetooth::transport::adapter_by_identity(&session, identity)
                .await?
                .context("locating configured adapter identity")?
        } else {
            session
                .adapter(adapter_name)
                .context("locating configured adapter")?
        };
        if let Some(device) =
            crate::bluetooth::transport::device_by_identity(&adapter, address).await?
        {
            adapter
                .remove_device(device.address())
                .await
                .context("removing configured Bluetooth bond")?;
        }
        Ok(())
    }
}

pub struct Teardown {
    store: ConfigurationStore,
    config_home: PathBuf,
    services: Arc<dyn UserServiceControl>,
    bonds: Arc<dyn BondCleanup>,
}

impl Teardown {
    pub fn new(
        store: ConfigurationStore,
        config_home: PathBuf,
        services: Arc<dyn UserServiceControl>,
        bonds: Arc<dyn BondCleanup>,
    ) -> Self {
        Self {
            store,
            config_home,
            services,
            bonds,
        }
    }

    pub async fn run(&self, forget_device: bool) -> Result<()> {
        let Some(configuration) = self.store.load()? else {
            return Ok(());
        };
        let mut failures = Vec::new();
        if let Err(error) = self.services.stop_and_disable(BRIDGE_UNIT) {
            failures.push(format!("service cleanup: {error:#}"));
        }
        let rule = AudioRule::new(self.config_home.clone(), configuration.device_address);
        match remove_with_reload(&rule, self.services.as_ref()) {
            Ok(_) => {}
            Err(error) => failures.push(format!("audio rule cleanup: {error:#}")),
        }
        if forget_device {
            if let Err(error) = self
                .bonds
                .remove_exact(
                    &configuration.adapter,
                    configuration.adapter_address,
                    configuration.device_address,
                )
                .await
            {
                failures.push(format!("Bluetooth bond cleanup: {error:#}"));
            }
        }
        if failures.is_empty() {
            self.store.remove()?;
            Ok(())
        } else {
            Err(anyhow!(failures.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        atomic_file::test_support::TestDirectory,
        config::{BluetoothConfiguration, Configuration, CONFIG_SCHEMA_VERSION},
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeServices {
        calls: Mutex<Vec<String>>,
        fail_stop: bool,
        fail_restart: bool,
    }

    impl UserServiceControl for FakeServices {
        fn unit_exists(&self, _: &str) -> Result<bool> {
            Ok(true)
        }
        fn restart(&self, unit: &str) -> Result<()> {
            self.calls.lock().unwrap().push(format!("restart:{unit}"));
            if self.fail_restart {
                Err(anyhow!("restart failed"))
            } else {
                Ok(())
            }
        }
        fn stop_and_disable(&self, unit: &str) -> Result<()> {
            self.calls.lock().unwrap().push(format!("stop:{unit}"));
            if self.fail_stop {
                Err(anyhow!("stop failed"))
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct FakeBonds {
        calls: Mutex<Vec<(String, Address)>>,
        fail: bool,
    }

    #[async_trait]
    impl BondCleanup for FakeBonds {
        async fn remove_exact(
            &self,
            adapter: &str,
            _: Option<Address>,
            address: Address,
        ) -> Result<()> {
            self.calls.lock().unwrap().push((adapter.into(), address));
            if self.fail {
                Err(anyhow!("bond failed"))
            } else {
                Ok(())
            }
        }
    }

    fn configuration() -> Configuration {
        Configuration {
            schema_version: CONFIG_SCHEMA_VERSION,
            bluetooth: BluetoothConfiguration {
                adapter: "hci0".into(),
                adapter_address: Some("11:22:33:44:55:66".into()),
                device_address: "AA:BB:CC:DD:EE:FF".into(),
                device_name: "iPhone".into(),
            },
        }
    }

    fn harness(
        directory: &TestDirectory,
        services: Arc<FakeServices>,
        bonds: Arc<FakeBonds>,
    ) -> (Teardown, ConfigurationStore) {
        let store = ConfigurationStore::new(directory.path().join("ancs-bridge/config.toml"));
        (
            Teardown::new(store.clone(), directory.path().to_owned(), services, bonds),
            store,
        )
    }

    #[tokio::test]
    async fn absent_configuration_is_a_noop_without_guessing() {
        let directory = TestDirectory::new("teardown-empty");
        let services = Arc::new(FakeServices::default());
        let bonds = Arc::new(FakeBonds::default());
        let (teardown, _) = harness(&directory, services.clone(), bonds.clone());
        teardown.run(true).await.unwrap();
        assert!(services.calls.lock().unwrap().is_empty());
        assert!(bonds.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn retained_and_forgotten_bond_follow_exact_order_and_are_retry_safe() {
        for forget in [false, true] {
            let directory = TestDirectory::new("teardown-success");
            let services = Arc::new(FakeServices::default());
            let bonds = Arc::new(FakeBonds::default());
            let (teardown, store) = harness(&directory, services.clone(), bonds.clone());
            store.save(&configuration()).unwrap();
            let rule = AudioRule::new(
                directory.path().to_owned(),
                "AA:BB:CC:DD:EE:FF".parse().unwrap(),
            );
            rule.apply().unwrap();
            teardown.run(forget).await.unwrap();
            assert!(store.load().unwrap().is_none());
            assert!(!rule.path().exists());
            assert!(!rule.role_path().exists());
            assert_eq!(bonds.calls.lock().unwrap().len(), usize::from(forget));
            teardown.run(forget).await.unwrap();
        }
    }

    #[tokio::test]
    async fn failures_preserve_configuration_for_retry() {
        let variants = [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ];
        for (fail_stop, fail_restart, fail_bond) in variants {
            let directory = TestDirectory::new("teardown-failure");
            let services = Arc::new(FakeServices {
                fail_stop,
                fail_restart,
                ..Default::default()
            });
            let bonds = Arc::new(FakeBonds {
                fail: fail_bond,
                ..Default::default()
            });
            let (teardown, store) = harness(&directory, services, bonds);
            store.save(&configuration()).unwrap();
            let rule = AudioRule::new(
                directory.path().to_owned(),
                "AA:BB:CC:DD:EE:FF".parse().unwrap(),
            );
            rule.apply().unwrap();
            assert!(teardown.run(true).await.is_err());
            assert!(store.load().unwrap().is_some());
            if fail_restart {
                assert!(rule.path().exists());
                assert!(rule.role_path().exists());
            }
        }
    }

    #[tokio::test]
    async fn conflicting_rule_and_invalid_configuration_are_preserved() {
        let directory = TestDirectory::new("teardown-conflict");
        let (teardown, store) = harness(
            &directory,
            Arc::new(FakeServices::default()),
            Arc::new(FakeBonds::default()),
        );
        store.save(&configuration()).unwrap();
        let rule = AudioRule::new(
            directory.path().to_owned(),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        std::fs::create_dir_all(rule.path().parent().unwrap()).unwrap();
        std::fs::write(rule.path(), "not ours").unwrap();
        assert!(teardown.run(false).await.is_err());
        assert!(store.load().unwrap().is_some());

        std::fs::write(store.path(), "invalid = [").unwrap();
        assert!(teardown.run(true).await.is_err());
        assert!(store.path().exists());
    }
}
