use crate::atomic_file;
use crate::service::UserServiceControl;
use anyhow::{bail, Context, Result};
use bluer::Address;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

pub const WIREPLUMBER_UNIT: &str = "wireplumber.service";

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedRule {
    path: PathBuf,
    content: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuleChange {
    created: Vec<OwnedRule>,
    removed: Vec<OwnedRule>,
}

impl RuleChange {
    pub fn changed(&self) -> bool {
        !self.created.is_empty() || !self.removed.is_empty()
    }
}

pub fn apply_with_reload(
    rule: &AudioRule,
    services: &dyn UserServiceControl,
) -> Result<RuleChange> {
    reconcile_with_reload(None, Some(rule), services)
}

pub fn remove_with_reload(
    rule: &AudioRule,
    services: &dyn UserServiceControl,
) -> Result<RuleChange> {
    reconcile_with_reload(Some(rule), None, services)
}

pub fn reconcile_with_reload(
    previous: Option<&AudioRule>,
    desired: Option<&AudioRule>,
    services: &dyn UserServiceControl,
) -> Result<RuleChange> {
    let change = reconcile(previous, desired)?;
    if !change.changed() {
        return Ok(change);
    }
    if let Err(restart) = services.restart(WIREPLUMBER_UNIT) {
        if let Err(cleanup) = rollback_change(&change, services) {
            bail!("cleanup-failed after WirePlumber restart failure: {restart:#}; {cleanup:#}");
        }
        return Err(restart).context("audio-restart-failed");
    }
    Ok(change)
}

pub fn rollback_change(change: &RuleChange, services: &dyn UserServiceControl) -> Result<()> {
    if !change.changed() {
        return Ok(());
    }
    rollback_files(change)?;
    services
        .restart(WIREPLUMBER_UNIT)
        .context("reloading WirePlumber after audio-rule rollback")
}

fn reconcile(previous: Option<&AudioRule>, desired: Option<&AudioRule>) -> Result<RuleChange> {
    let mut rules: BTreeMap<PathBuf, (Option<String>, Option<String>)> = BTreeMap::new();
    if let Some(previous) = previous {
        for rule in previous.rules() {
            rules.entry(rule.path).or_default().0 = Some(rule.content);
        }
    }
    if let Some(desired) = desired {
        for rule in desired.rules() {
            rules.entry(rule.path).or_default().1 = Some(rule.content);
        }
    }

    let mut planned = RuleChange::default();
    for (path, (previous, desired)) in &rules {
        let disk = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("reading audio rule {}", path.display()))
            }
        };
        if disk.as_deref() == desired.as_deref().map(str::as_bytes) {
            continue;
        }
        if disk.is_none() {
            if let Some(content) = desired {
                planned.created.push(OwnedRule {
                    path: path.clone(),
                    content: content.clone(),
                });
            }
            continue;
        }
        if disk.as_deref() != previous.as_deref().map(str::as_bytes) {
            bail!("audio-rule-conflict");
        }
        if let Some(content) = previous {
            planned.removed.push(OwnedRule {
                path: path.clone(),
                content: content.clone(),
            });
        }
        if let Some(content) = desired {
            planned.created.push(OwnedRule {
                path: path.clone(),
                content: content.clone(),
            });
        }
    }

    let mut applied = RuleChange::default();
    for rule in &planned.removed {
        if let Err(error) = fs::remove_file(&rule.path) {
            rollback_files(&applied)
                .with_context(|| format!("cleanup-failed after removing audio rule: {error}"))?;
            return Err(error)
                .with_context(|| format!("removing audio rule {}", rule.path.display()));
        }
        applied.removed.push(rule.clone());
    }
    for rule in &planned.created {
        if let Err(error) =
            atomic_file::replace_preserving_directories(&rule.path, rule.content.as_bytes(), 0o700)
        {
            rollback_files(&applied)
                .with_context(|| format!("cleanup-failed after creating audio rule: {error:#}"))?;
            return Err(error);
        }
        applied.created.push(rule.clone());
    }
    Ok(applied)
}

fn rollback_files(change: &RuleChange) -> Result<()> {
    for rule in change.created.iter().rev() {
        match fs::read(&rule.path) {
            Ok(bytes) if bytes == rule.content.as_bytes() => fs::remove_file(&rule.path)
                .with_context(|| format!("removing created audio rule {}", rule.path.display()))?,
            Ok(_) => bail!("audio-rule-conflict"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading audio rule {}", rule.path.display()))
            }
        }
    }
    for rule in change.removed.iter().rev() {
        match fs::read(&rule.path) {
            Ok(bytes) if bytes == rule.content.as_bytes() => continue,
            Ok(_) => bail!("audio-rule-conflict"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading audio rule {}", rule.path.display()))
            }
        }
        atomic_file::replace_preserving_directories(&rule.path, rule.content.as_bytes(), 0o700)?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioRule {
    path: PathBuf,
    content: String,
    role_path: PathBuf,
    role_content: String,
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
        let role_path = config_home
            .join("wireplumber/wireplumber.conf.d/91-ancs-bridge-bluetooth-output-only.conf");
        let content = format!(
            "monitor.bluez.rules = [\n  {{\n    matches = [\n      {{ device.name = \"bluez_card.{identity}\" }}\n    ]\n    actions = {{\n      update-props = {{\n        device.disabled = true\n      }}\n    }}\n  }}\n]\n"
        );
        let role_content =
            "monitor.bluez.properties = {\n  bluez5.roles = [ a2dp_source bap_source hfp_ag ]\n}\n"
                .into();
        Self {
            path,
            content,
            role_path,
            role_content,
        }
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

    pub fn role_path(&self) -> &Path {
        &self.role_path
    }

    pub fn role_content(&self) -> &str {
        &self.role_content
    }

    pub fn apply(&self) -> Result<RuleChange> {
        reconcile(None, Some(self))
    }

    pub fn remove(&self) -> Result<RuleChange> {
        reconcile(Some(self), None)
    }

    fn rules(&self) -> [OwnedRule; 2] {
        [
            OwnedRule {
                path: self.path.clone(),
                content: self.content.clone(),
            },
            OwnedRule {
                path: self.role_path.clone(),
                content: self.role_content.clone(),
            },
        ]
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
    const OUTPUT_ONLY_GOLDEN: &str = include_str!("../tests/fixtures/audio-output-only-rule.conf");

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
        assert_eq!(
            rule.role_path(),
            Path::new(
                "/tmp/config/wireplumber/wireplumber.conf.d/91-ancs-bridge-bluetooth-output-only.conf"
            )
        );
        assert_eq!(rule.role_content(), OUTPUT_ONLY_GOLDEN);
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
        assert!(rule.apply().unwrap().changed());
        assert!(rule.role_path().exists());
        assert!(!rule.apply().unwrap().changed());
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
        assert!(rule.remove().unwrap().changed());
        assert!(!rule.role_path().exists());
        assert!(!rule.remove().unwrap().changed());
    }

    #[test]
    fn conflicting_content_is_never_overwritten_or_removed() {
        let directory = TestDirectory::new("audio-conflict");
        let rule = AudioRule::new(
            directory.path().to_owned(),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        fs::create_dir_all(rule.path().parent().unwrap()).unwrap();
        fs::write(rule.role_path(), "user content").unwrap();
        assert!(rule.apply().is_err());
        assert!(rule.remove().is_err());
        assert!(!rule.path().exists(), "preflight prevents a partial apply");
        assert_eq!(
            fs::read_to_string(rule.role_path()).unwrap(),
            "user content"
        );
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
        assert!(apply_with_reload(&rule, &services).unwrap().changed());
        assert!(!apply_with_reload(&rule, &services).unwrap().changed());
        assert_eq!(*services.restarts.lock().unwrap(), 1);
        assert!(remove_with_reload(&rule, &services).unwrap().changed());
        assert!(!remove_with_reload(&rule, &services).unwrap().changed());
        assert_eq!(*services.restarts.lock().unwrap(), 2);

        let failing = FakeServices {
            fail: true,
            ..Default::default()
        };
        assert!(apply_with_reload(&rule, &failing).is_err());
        assert!(
            !rule.path().exists(),
            "failed apply removes its new exact-device rule"
        );
        assert!(
            !rule.role_path().exists(),
            "failed apply removes its new output-only rule"
        );
        assert_eq!(*failing.restarts.lock().unwrap(), 2);
    }

    #[test]
    fn failed_apply_preserves_a_preexisting_exact_rule_and_removes_only_new_role_rule() {
        let directory = TestDirectory::new("audio-partial-rollback");
        let rule = AudioRule::new(
            directory.path().to_owned(),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        fs::create_dir_all(rule.path().parent().unwrap()).unwrap();
        fs::write(rule.path(), rule.content()).unwrap();
        let failing = FakeServices {
            fail: true,
            ..Default::default()
        };

        assert!(apply_with_reload(&rule, &failing).is_err());
        assert_eq!(fs::read_to_string(rule.path()).unwrap(), rule.content());
        assert!(!rule.role_path().exists());
    }

    #[test]
    fn identity_change_replaces_only_the_exact_rule_and_is_reversible() {
        let directory = TestDirectory::new("audio-identity-change");
        let old = AudioRule::new(
            directory.path().to_owned(),
            "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        );
        let new = AudioRule::new(
            directory.path().to_owned(),
            "11:22:33:44:55:66".parse().unwrap(),
        );
        old.apply().unwrap();
        let services = FakeServices::default();

        let change = reconcile_with_reload(Some(&old), Some(&new), &services).unwrap();
        assert!(!old.path().exists());
        assert!(new.path().exists());
        assert!(new.role_path().exists());
        assert_eq!(*services.restarts.lock().unwrap(), 1);

        rollback_change(&change, &services).unwrap();
        assert!(old.path().exists());
        assert!(!new.path().exists());
        assert!(old.role_path().exists());
        assert_eq!(*services.restarts.lock().unwrap(), 2);
    }

    struct SequencedServices {
        restarts: Mutex<usize>,
        fail_on: Vec<usize>,
    }

    impl UserServiceControl for SequencedServices {
        fn unit_exists(&self, _: &str) -> Result<bool> {
            Ok(true)
        }

        fn restart(&self, _: &str) -> Result<()> {
            let mut restarts = self.restarts.lock().unwrap();
            *restarts += 1;
            if self.fail_on.contains(&*restarts) {
                bail!("restart failed")
            }
            Ok(())
        }

        fn stop_and_disable(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn failed_removal_restores_files_and_reloads_the_rollback() {
        for fail_on in [vec![1], vec![1, 2]] {
            let directory = TestDirectory::new("audio-remove-rollback");
            let rule = AudioRule::new(
                directory.path().to_owned(),
                "AA:BB:CC:DD:EE:FF".parse().unwrap(),
            );
            rule.apply().unwrap();
            let services = SequencedServices {
                restarts: Mutex::new(0),
                fail_on: fail_on.clone(),
            };

            let error = remove_with_reload(&rule, &services).unwrap_err();
            assert!(rule.path().exists());
            assert!(rule.role_path().exists());
            assert_eq!(*services.restarts.lock().unwrap(), 2);
            assert_eq!(
                error.to_string().contains("cleanup-failed"),
                fail_on.len() == 2
            );
        }
    }
}
