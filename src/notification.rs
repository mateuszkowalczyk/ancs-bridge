use anyhow::Context;
use async_trait::async_trait;
use notify_rust::Notification;
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
};
use tokio::sync::oneshot;

/// Sensitive values used only for delivery. Intentionally not debuggable or serializable.
pub struct NotificationPayload {
    app_name: String,
    title: String,
    message: String,
}

impl NotificationPayload {
    pub fn new(app_name: String, title: String, message: String) -> Self {
        Self {
            app_name,
            title,
            message,
        }
    }

    fn summary(&self) -> &str {
        if self.title.is_empty() {
            &self.app_name
        } else {
            &self.title
        }
    }

    #[cfg(test)]
    pub(crate) fn test_app_name(&self) -> &str {
        &self.app_name
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesktopHandle(pub u32);

#[async_trait]
pub trait NotificationSink: Send {
    async fn create(&mut self, payload: NotificationPayload) -> anyhow::Result<DesktopHandle>;
    async fn replace(
        &mut self,
        handle: DesktopHandle,
        payload: NotificationPayload,
    ) -> anyhow::Result<()>;
    async fn close(&mut self, handle: DesktopHandle) -> anyhow::Result<()>;
}

pub struct FreedesktopSink {
    commands: mpsc::Sender<SinkCommand>,
}

enum SinkCommand {
    Create {
        payload: NotificationPayload,
        reply: oneshot::Sender<Result<DesktopHandle, String>>,
    },
    Replace {
        handle: DesktopHandle,
        payload: NotificationPayload,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Close {
        handle: DesktopHandle,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

impl Default for FreedesktopSink {
    fn default() -> Self {
        let (commands, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("ancs-notification-sink".into())
            .spawn(move || notification_actor(receiver))
            .expect("spawning notification sink actor");
        Self { commands }
    }
}

fn notification_actor(receiver: mpsc::Receiver<SinkCommand>) {
    let mut handles = HashMap::new();
    while let Ok(command) = receiver.recv() {
        match command {
            SinkCommand::Create { payload, reply } => {
                let result = Notification::new()
                    .appname("ancs-bridge")
                    .summary(payload.summary())
                    .body(&payload.message)
                    .show()
                    .map_err(|error| error.to_string())
                    .map(|handle| {
                        let id = DesktopHandle(handle.id());
                        handles.insert(id, handle);
                        id
                    });
                let _ = reply.send(result);
            }
            SinkCommand::Replace {
                handle,
                payload,
                reply,
            } => {
                let result = Notification::new()
                    .appname("ancs-bridge")
                    .summary(payload.summary())
                    .body(&payload.message)
                    .id(handle.0)
                    .show()
                    .map_err(|error| error.to_string())
                    .map(|replacement| {
                        handles.insert(handle, replacement);
                    });
                let _ = reply.send(result);
            }
            SinkCommand::Close { handle, reply } => {
                if let Some(value) = handles.remove(&handle) {
                    value.close();
                }
                let _ = reply.send(Ok(()));
            }
        }
    }
}

#[async_trait]
impl NotificationSink for FreedesktopSink {
    async fn create(&mut self, payload: NotificationPayload) -> anyhow::Result<DesktopHandle> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(SinkCommand::Create { payload, reply })
            .context("notification actor stopped")?;
        response
            .await
            .context("notification actor dropped create response")?
            .map_err(anyhow::Error::msg)
    }

    async fn replace(
        &mut self,
        handle: DesktopHandle,
        payload: NotificationPayload,
    ) -> anyhow::Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(SinkCommand::Replace {
                handle,
                payload,
                reply,
            })
            .context("notification actor stopped")?;
        response
            .await
            .context("notification actor dropped replace response")?
            .map_err(anyhow::Error::msg)
    }

    async fn close(&mut self, handle: DesktopHandle) -> anyhow::Result<()> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(SinkCommand::Close { handle, reply })
            .context("notification actor stopped")?;
        response
            .await
            .context("notification actor dropped close response")?
            .map_err(anyhow::Error::msg)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SinkCall {
    Create(DesktopHandle),
    Replace(DesktopHandle),
    Close(DesktopHandle),
}

#[derive(Clone, Default)]
pub struct FakeNotificationSink {
    calls: Arc<Mutex<Vec<SinkCall>>>,
    fail_next: Arc<Mutex<bool>>,
    next_id: Arc<Mutex<u32>>,
}

impl FakeNotificationSink {
    pub fn calls(&self) -> Vec<SinkCall> {
        self.calls.lock().expect("fake sink poisoned").clone()
    }

    pub fn fail_next(&self) {
        *self.fail_next.lock().expect("fake sink poisoned") = true;
    }

    fn maybe_fail(&self) -> anyhow::Result<()> {
        let mut fail = self.fail_next.lock().expect("fake sink poisoned");
        if std::mem::take(&mut *fail) {
            anyhow::bail!("injected notification failure");
        }
        Ok(())
    }
}

#[async_trait]
impl NotificationSink for FakeNotificationSink {
    async fn create(&mut self, _payload: NotificationPayload) -> anyhow::Result<DesktopHandle> {
        self.maybe_fail()?;
        let mut next = self.next_id.lock().expect("fake sink poisoned");
        *next += 1;
        let handle = DesktopHandle(*next);
        self.calls
            .lock()
            .expect("fake sink poisoned")
            .push(SinkCall::Create(handle));
        Ok(handle)
    }

    async fn replace(
        &mut self,
        handle: DesktopHandle,
        _payload: NotificationPayload,
    ) -> anyhow::Result<()> {
        self.maybe_fail()?;
        self.calls
            .lock()
            .expect("fake sink poisoned")
            .push(SinkCall::Replace(handle));
        Ok(())
    }

    async fn close(&mut self, handle: DesktopHandle) -> anyhow::Result<()> {
        self.maybe_fail()?;
        self.calls
            .lock()
            .expect("fake sink poisoned")
            .push(SinkCall::Close(handle));
        Ok(())
    }
}
