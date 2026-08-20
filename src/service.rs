use anyhow::{bail, Context, Result};
use std::process::{Command, Output};

pub trait UserServiceControl: Send + Sync {
    fn unit_exists(&self, unit: &str) -> Result<bool>;
    fn restart(&self, unit: &str) -> Result<()>;
    fn stop_and_disable(&self, unit: &str) -> Result<()>;
}

#[derive(Default)]
pub struct SystemdUserServiceControl;

impl SystemdUserServiceControl {
    fn run(arguments: &[&str]) -> Result<Output> {
        Command::new("systemctl")
            .arg("--user")
            .args(arguments)
            .output()
            .with_context(|| format!("running systemctl --user {}", arguments.join(" ")))
    }

    fn require_success(arguments: &[&str]) -> Result<()> {
        let output = Self::run(arguments)?;
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            bail!(
                "systemctl --user {} failed: {}",
                arguments.join(" "),
                diagnostic.trim()
            );
        }
        Ok(())
    }
}

impl UserServiceControl for SystemdUserServiceControl {
    fn unit_exists(&self, unit: &str) -> Result<bool> {
        let output = Self::run(&["show", "--property=LoadState", "--value", unit])?;
        let state = String::from_utf8_lossy(&output.stdout);
        if state.trim() == "not-found" {
            return Ok(false);
        }
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            bail!("querying user unit {unit} failed: {}", diagnostic.trim());
        }
        Ok(true)
    }

    fn restart(&self, unit: &str) -> Result<()> {
        Self::require_success(&["restart", unit])
    }

    fn stop_and_disable(&self, unit: &str) -> Result<()> {
        if self.unit_exists(unit)? {
            Self::require_success(&["disable", "--now", unit])?;
        }
        Ok(())
    }
}
