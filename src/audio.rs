use crate::atomic_file;
use crate::service::UserServiceControl;
use anyhow::{bail, Context, Result};
use bluer::Address;
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

pub const WIREPLUMBER_UNIT: &str = "wireplumber.service";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleChange {
    Unchanged,
    Created,
    Removed,
}

pub fn apply_with_reload(
    rule: &AudioRule,
    services: &dyn UserServiceControl,
) -> Result<RuleChange> {
    let change = rule.apply()?;
    if change == RuleChange::Created {
        if let Err(restart) = services.restart(WIREPLUMBER_UNIT) {
            let cleanup = rollback_created(rule, services);
            if let Err(cleanup) = cleanup {
                bail!("cleanup-failed after WirePlumber restart failure: {restart:#}; {cleanup:#}");
            }
            return Err(restart).context("audio-restart-failed");
        }
    }
    Ok(change)
}

pub fn remove_with_reload(
    rule: &AudioRule,
    services: &dyn UserServiceControl,
) -> Result<RuleChange> {
    let change = rule.remove()?;
    if change == RuleChange::Removed {
        if let Err(restart) = services.restart(WIREPLUMBER_UNIT) {
            // Restore the exact owned rule so teardown remains retry-safe.
            rule.apply()
                .context("restoring audio rule after reload failure")?;
            return Err(restart).context("audio-restart-failed");
        }
    }
    Ok(change)
}

pub fn rollback_created(rule: &AudioRule, services: &dyn UserServiceControl) -> Result<()> {
    match rule.remove()? {
        RuleChange::Removed => services
            .restart(WIREPLUMBER_UNIT)
            .context("reloading WirePlumber after audio-rule rollback"),
        RuleChange::Unchanged => Ok(()),
        RuleChange::Created => unreachable!(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioRule {
    path: PathBuf,
    content: String,
}

impl AudioRule {
    pub fn from_environment(address: Address) -> Result<Self> {
        Ok(Self::new(config_home_from_environment()?, address))
    }

    pub fn from_environment_values(
        address: Address,
        xdg_config_home: Option<OsString>,
        home: Option<OsString>,
    ) -> Result<Self> {
        let base = absolute(xdg_config_home.as_deref())
            .or_else(|| absolute(home.as_deref()).map(|path| path.join(".config")))
            .context("no absolute XDG_CONFIG_HOME or HOME is available")?;
        Ok(Self::new(base, address))
    }

    pub fn new(config_home: PathBuf, address: Address) -> Self {
        let identity = address.to_string().replace(':', "_");
        let path = config_home.join(format!(
            "wireplumber/wireplumber.conf.d/90-ancs-bridge-{identity}.conf"
        ));
        let content = format!(
            "monitor.bluez.rules = [\n  {{\n    matches = [\n      {{ device.name = \"bluez_card.{identity}\" }}\n    ]\n    actions = {{\n      update-props = {{\n        device.disabled = true\n      }}\n    }}\n  }}\n]\n"
        );
        Self { path, content }
    }

    pub fn parse(config_home: PathBuf, address: &str) -> Result<Self> {
        let address: Address = address
            .parse()
            .context("invalid Bluetooth identity for audio rule")?;
        Ok(Self::new(config_home, address))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn apply(&self) -> Result<RuleChange> {
        match fs::read(&self.path) {
            Ok(bytes) if bytes == self.content.as_bytes() => return Ok(RuleChange::Unchanged),
            Ok(_) => bail!("audio-rule-conflict"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading audio rule {}", self.path.display()))
            }
        }
        atomic_file::replace_preserving_directories(&self.path, self.content.as_bytes(), 0o700)?;
        Ok(RuleChange::Created)
    }

    pub fn remove(&self) -> Result<RuleChange> {
        match fs::read(&self.path) {
            Ok(bytes) if bytes != self.content.as_bytes() => bail!("audio-rule-conflict"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RuleChange::Unchanged)
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading audio rule {}", self.path.display()))
            }
        }
        fs::remove_file(&self.path)
            .with_context(|| format!("removing audio rule {}", self.path.display()))?;
        Ok(RuleChange::Removed)
    }
}

pub fn config_home_from_environment() -> Result<PathBuf> {
    config_home_from_environment_values(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

pub fn config_home_from_environment_values(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf> {
    absolute(xdg_config_home.as_deref())
        .or_else(|| absolute(home.as_deref()).map(|path| path.join(".config")))
        .context("no absolute XDG_CONFIG_HOME or HOME is available")
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
    use std::sync::Mutex;

    const GOLDEN: &str = include_str!("../tests/fixtures/audio-rule-AA_BB_CC_DD_EE_FF.conf");

    #[test]
    fn canonical_path_and_content_match_golden_fixture() {
        let rule = AudioRule::parse(PathBuf::from("/tmp/config"), "aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(
            rule.path(),
            Path::new(
                "/tmp/config/wireplumber/wireplumber.conf.d/90-ancs-bridge-AA_BB_CC_DD_EE_FF.conf"
            )
        );
        assert_eq!(rule.content(), GOLDEN);
        for invalid in ["bad", "../../escape", "AA:BB:CC:DD:EE:FF/other"] {
            assert!(AudioRule::parse(PathBuf::from("/tmp/config"), invalid).is_err());
        }
    }

    #[test]
    fn apply_and_remove_are_exact_idempotent_and_private() {
        let directory = TestDirectory::new("audio-rule");
        let existing_parent = directory.path().join("existing");
        fs::create_dir(&existing_parent).unwrap();
        fs::set_permissions(&existing_parent, fs::Permissions::from_mode(0o755)).unwrap();
        let rule = AudioRule::new(existing_parent, "AA:BB:CC:DD:EE:FF".parse().unwrap());
        assert_eq!(rule.apply().unwrap(), RuleChange::Created);
        assert_eq!(rule.apply().unwrap(), RuleChange::Unchanged);
        assert_eq!(
            fs::metadata(rule.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(rule.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(rule.remove().unwrap(), RuleChange::Removed);
        assert_eq!(rule.remove().unwrap(), RuleChange::Unchanged);
    }

    #[test]
    fn conflicting_content_is_never_overwritten_or_removed() {
        let directory = TestDirectory::new("audio-conflict");
        let rule = AudioRule::new(
            directory.path().to_owned(),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        fs::create_dir_all(rule.path().parent().unwrap()).unwrap();
        fs::write(rule.path(), "user content").unwrap();
        assert!(rule.apply().is_err());
        assert!(rule.remove().is_err());
        assert_eq!(fs::read_to_string(rule.path()).unwrap(), "user content");
    }

    #[derive(Default)]
    struct FakeServices {
        restarts: Mutex<usize>,
        fail: bool,
    }

    impl UserServiceControl for FakeServices {
        fn unit_exists(&self, _: &str) -> Result<bool> {
            Ok(true)
        }
        fn restart(&self, _: &str) -> Result<()> {
            *self.restarts.lock().unwrap() += 1;
            if self.fail {
                bail!("restart failed")
            } else {
                Ok(())
            }
        }
        fn stop_and_disable(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn service_restarts_only_on_changes_and_apply_failure_rolls_back() {
        let directory = TestDirectory::new("audio-reload");
        let rule = AudioRule::new(
            directory.path().to_owned(),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        let services = FakeServices::default();
        assert_eq!(
            apply_with_reload(&rule, &services).unwrap(),
            RuleChange::Created
        );
        assert_eq!(
            apply_with_reload(&rule, &services).unwrap(),
            RuleChange::Unchanged
        );
        assert_eq!(*services.restarts.lock().unwrap(), 1);
        assert_eq!(
            remove_with_reload(&rule, &services).unwrap(),
            RuleChange::Removed
        );
        assert_eq!(
            remove_with_reload(&rule, &services).unwrap(),
            RuleChange::Unchanged
        );
        assert_eq!(*services.restarts.lock().unwrap(), 2);

        let failing = FakeServices {
            fail: true,
            ..Default::default()
        };
        assert!(apply_with_reload(&rule, &failing).is_err());
        assert!(
            !rule.path().exists(),
            "failed apply removes only its new rule"
        );
        assert_eq!(*failing.restarts.lock().unwrap(), 2);
    }
}
