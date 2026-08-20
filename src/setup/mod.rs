use crate::{
    clock::Clock,
    machine::{
        parse_command, validate_command, ConfirmationKind, ProtocolError, SetupCommand, SetupEvent,
        SetupFailure, SetupState, API_VERSION,
    },
};
use async_trait::async_trait;
use serde::Serialize;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

pub mod production;

pub const CANDIDATE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);
pub const ANCS_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub kind: ConfirmationKind,
    pub address: String,
    pub device_name: String,
    pub passkey: Option<String>,
}

pub enum Preparation {
    Existing(Candidate),
    Fresh,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SetupOptions {
    pub disable_phone_audio: bool,
    pub repair: bool,
}

#[async_trait]
pub trait SetupBackend: Send {
    async fn prepare(&mut self, options: SetupOptions) -> Result<Preparation, SetupFailure>;
    async fn wait_for_candidate(&mut self) -> Result<Candidate, SetupFailure>;
    async fn answer_confirmation(
        &mut self,
        candidate: &Candidate,
        accept: bool,
    ) -> Result<(), SetupFailure>;
    async fn verify_ancs(&mut self, candidate: &Candidate) -> Result<(), SetupFailure>;
    async fn cleanup_temporary(&mut self) -> Result<(), SetupFailure>;
    async fn commit(
        &mut self,
        candidate: &Candidate,
        options: SetupOptions,
    ) -> Result<(), SetupFailure>;
    fn final_address(&self, candidate: &Candidate) -> String {
        candidate.address.clone()
    }
}

pub struct SetupProtocol<B, C> {
    backend: B,
    clock: C,
    request_sequence: u64,
}

#[async_trait]
pub trait SetupCommandInput: Send {
    async fn next_command(&mut self) -> Result<SetupCommand, SetupFailure>;
}

#[async_trait]
impl<R> SetupCommandInput for R
where
    R: AsyncBufRead + Unpin + Send,
{
    async fn next_command(&mut self) -> Result<SetupCommand, SetupFailure> {
        let mut line = String::new();
        let count = self
            .read_line(&mut line)
            .await
            .map_err(|_| SetupFailure::InvalidProtocol)?;
        if count == 0 {
            return Err(SetupFailure::StdinClosed);
        }
        parse_command(line.trim_end()).map_err(protocol_failure)
    }
}

/// Terminal stdin adapter whose detached blocking reader cannot hold the Tokio
/// runtime open after setup has emitted its final event.
pub struct StdinCommandInput {
    receiver: mpsc::UnboundedReceiver<Result<String, ()>>,
}

impl StdinCommandInput {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            loop {
                let mut line = String::new();
                match std::io::BufRead::read_line(&mut input, &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(Err(()));
                        break;
                    }
                }
            }
        });
        Self { receiver }
    }
}

impl Default for StdinCommandInput {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SetupCommandInput for StdinCommandInput {
    async fn next_command(&mut self) -> Result<SetupCommand, SetupFailure> {
        let line = self
            .receiver
            .recv()
            .await
            .ok_or(SetupFailure::StdinClosed)?
            .map_err(|_| SetupFailure::InvalidProtocol)?;
        parse_command(line.trim_end()).map_err(protocol_failure)
    }
}

impl<B, C> SetupProtocol<B, C>
where
    B: SetupBackend,
    C: Clock,
{
    pub fn new(backend: B, clock: C) -> Self {
        Self {
            backend,
            clock,
            request_sequence: 0,
        }
    }

    pub async fn run<R, W>(&mut self, reader: &mut R, writer: &mut W, options: SetupOptions) -> bool
    where
        R: SetupCommandInput,
        W: AsyncWrite + Unpin + Send,
    {
        if write_event(writer, &SetupEvent::state(SetupState::CheckingEnvironment))
            .await
            .is_err()
        {
            return false;
        }

        let preparation = tokio::select! {
            biased;
            command = read_command(reader) => {
                let failure = command_failure(command, None);
                return self.fail_with_cleanup(writer, failure).await;
            }
            () = shutdown_signal() => {
                return self.fail_with_cleanup(writer, SetupFailure::Cancelled).await;
            }
            result = self.backend.prepare(options) => match result {
                Ok(value) => value,
                Err(failure) => return self.fail_with_cleanup(writer, failure).await,
            }
        };
        let candidate = match preparation {
            Preparation::Existing(candidate) => candidate,
            Preparation::Fresh => {
                if write_event(writer, &SetupEvent::state(SetupState::WaitingForIphone))
                    .await
                    .is_err()
                {
                    let _ = self.backend.cleanup_temporary().await;
                    return false;
                }
                tokio::select! {
                    biased;
                    result = self.backend.wait_for_candidate() => match result {
                        Ok(candidate) => candidate,
                        Err(failure) => return self.fail_with_cleanup(writer, failure).await,
                    },
                    () = self.clock.sleep(CANDIDATE_TIMEOUT) => {
                        return self.fail_with_cleanup(writer, SetupFailure::CandidateTimeout).await;
                    }
                    command = read_command(reader) => {
                        let failure = command_failure(command, None);
                        return self.fail_with_cleanup(writer, failure).await;
                    }
                    () = shutdown_signal() => {
                        return self.fail_with_cleanup(writer, SetupFailure::Cancelled).await;
                    }
                }
            }
        };

        self.request_sequence += 1;
        let request_id = format!("setup-{}", self.request_sequence);
        let confirmation = SetupEvent::ConfirmationRequest {
            v: API_VERSION,
            kind: candidate.kind,
            request_id: request_id.clone(),
            device_name: candidate.device_name.clone(),
            address: candidate.address.clone(),
            passkey: candidate.passkey.clone(),
        };
        if write_event(writer, &confirmation).await.is_err() {
            let _ = self.backend.cleanup_temporary().await;
            return false;
        }
        let accepted = tokio::select! {
            command = read_command(reader) => match command {
                Ok(command) => {
                    if let Err(error) = validate_command(&command, Some(&request_id)) {
                        return self.fail_with_cleanup(writer, protocol_failure(error)).await;
                    }
                    match command {
                        SetupCommand::Confirm { accept, .. } => accept,
                        SetupCommand::Cancel => return self.fail_with_cleanup(writer, SetupFailure::Cancelled).await,
                    }
                }
                Err(failure) => return self.fail_with_cleanup(writer, failure).await,
            },
            () = self.clock.sleep(CONFIRMATION_TIMEOUT) => {
                return self.fail_with_cleanup(writer, SetupFailure::ConfirmationTimeout).await;
            }
            () = shutdown_signal() => {
                return self.fail_with_cleanup(writer, SetupFailure::Cancelled).await;
            }
        };
        if !accepted {
            let _ = self.backend.answer_confirmation(&candidate, false).await;
            return self.fail_with_cleanup(writer, SetupFailure::Rejected).await;
        }
        tokio::select! {
            result = self.backend.answer_confirmation(&candidate, true) => if let Err(failure) = result {
                return self.fail_with_cleanup(writer, failure).await;
            },
            () = shutdown_signal() => {
                return self.fail_with_cleanup(writer, SetupFailure::Cancelled).await;
            }
        }
        if write_event(writer, &SetupEvent::state(SetupState::VerifyingAncs))
            .await
            .is_err()
        {
            let _ = self.backend.cleanup_temporary().await;
            return false;
        }
        tokio::select! {
            biased;
            result = self.backend.verify_ancs(&candidate) => if let Err(failure) = result {
                return self.fail_with_cleanup(writer, failure).await;
            },
            () = self.clock.sleep(ANCS_TIMEOUT) => {
                return self.fail_with_cleanup(writer, SetupFailure::AncsTimeout).await;
            }
            command = read_command(reader) => {
                let failure = command_failure(command, None);
                return self.fail_with_cleanup(writer, failure).await;
            }
            () = shutdown_signal() => {
                return self.fail_with_cleanup(writer, SetupFailure::Cancelled).await;
            }
        }
        if let Err(failure) = self.backend.cleanup_temporary().await {
            return self.fail(writer, failure).await;
        }
        if write_event(
            writer,
            &SetupEvent::state(SetupState::ApplyingConfiguration),
        )
        .await
        .is_err()
        {
            return false;
        }
        tokio::select! {
            biased;
            command = read_command(reader) => {
                return self.fail(writer, command_failure(command, None)).await;
            }
            () = shutdown_signal() => {
                return self.fail(writer, SetupFailure::Cancelled).await;
            }
            result = self.backend.commit(&candidate, options) => if let Err(failure) = result {
                return self.fail(writer, failure).await;
            }
        }
        let final_address = self.backend.final_address(&candidate);
        write_event(
            writer,
            &SetupEvent::Complete {
                v: API_VERSION,
                address: final_address,
            },
        )
        .await
        .is_ok()
    }

    async fn fail_with_cleanup<W>(&mut self, writer: &mut W, failure: SetupFailure) -> bool
    where
        W: AsyncWrite + Unpin + Send,
    {
        let failure = match self.backend.cleanup_temporary().await {
            Ok(()) => failure,
            Err(_) => SetupFailure::CleanupFailed,
        };
        self.fail(writer, failure).await
    }

    async fn fail<W>(&mut self, writer: &mut W, failure: SetupFailure) -> bool
    where
        W: AsyncWrite + Unpin + Send,
    {
        let _ = write_event(writer, &SetupEvent::error(failure)).await;
        false
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn read_command<R>(reader: &mut R) -> Result<SetupCommand, SetupFailure>
where
    R: SetupCommandInput,
{
    reader.next_command().await
}

fn command_failure(
    command: Result<SetupCommand, SetupFailure>,
    active_confirmation: Option<&str>,
) -> SetupFailure {
    match command {
        Err(failure) => failure,
        Ok(SetupCommand::Cancel) => SetupFailure::Cancelled,
        Ok(command) => validate_command(&command, active_confirmation)
            .err()
            .map(protocol_failure)
            .unwrap_or(SetupFailure::InvalidProtocol),
    }
}

fn protocol_failure(error: ProtocolError) -> SetupFailure {
    match error {
        ProtocolError::UnsupportedVersion => SetupFailure::UnsupportedApiVersion,
        ProtocolError::Malformed
        | ProtocolError::WrongState
        | ProtocolError::ConfirmationMismatch => SetupFailure::InvalidProtocol,
    }
}

async fn write_event<W>(writer: &mut W, event: &impl Serialize) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut line = serde_json::to_vec(event).expect("setup events are serializable");
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use std::sync::{Arc, Mutex};
    use tokio::io::BufReader;

    #[derive(Clone)]
    struct FakeBackend {
        state: Arc<Mutex<FakeState>>,
    }

    struct FakeState {
        preparation: Result<Preparation, SetupFailure>,
        candidate: Result<Candidate, SetupFailure>,
        verify: Result<(), SetupFailure>,
        cleanup: Result<(), SetupFailure>,
        commit: Result<(), SetupFailure>,
        calls: Vec<&'static str>,
        answers: Vec<bool>,
        wait_candidate: bool,
        wait_verify: bool,
        wait_prepare: bool,
        wait_commit: bool,
    }

    impl FakeBackend {
        fn fresh() -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeState {
                    preparation: Ok(Preparation::Fresh),
                    candidate: Ok(candidate(ConfirmationKind::Pairing)),
                    verify: Ok(()),
                    cleanup: Ok(()),
                    commit: Ok(()),
                    calls: Vec::new(),
                    answers: Vec::new(),
                    wait_candidate: false,
                    wait_verify: false,
                    wait_prepare: false,
                    wait_commit: false,
                })),
            }
        }
    }

    #[async_trait]
    impl SetupBackend for FakeBackend {
        async fn prepare(&mut self, _: SetupOptions) -> Result<Preparation, SetupFailure> {
            let (wait, preparation) = {
                let mut state = self.state.lock().unwrap();
                state.calls.push("prepare");
                (
                    state.wait_prepare,
                    std::mem::replace(&mut state.preparation, Err(SetupFailure::BackendFailed)),
                )
            };
            if wait {
                std::future::pending().await
            } else {
                preparation
            }
        }
        async fn wait_for_candidate(&mut self) -> Result<Candidate, SetupFailure> {
            let (wait, candidate) = {
                let mut state = self.state.lock().unwrap();
                state.calls.push("candidate");
                (state.wait_candidate, state.candidate.clone())
            };
            if wait {
                std::future::pending().await
            } else {
                candidate
            }
        }
        async fn answer_confirmation(
            &mut self,
            _: &Candidate,
            accept: bool,
        ) -> Result<(), SetupFailure> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("answer");
            state.answers.push(accept);
            Ok(())
        }
        async fn verify_ancs(&mut self, _: &Candidate) -> Result<(), SetupFailure> {
            let (wait, verify) = {
                let mut state = self.state.lock().unwrap();
                state.calls.push("verify");
                (state.wait_verify, state.verify)
            };
            if wait {
                std::future::pending().await
            } else {
                verify
            }
        }
        async fn cleanup_temporary(&mut self) -> Result<(), SetupFailure> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("cleanup");
            state.cleanup
        }
        async fn commit(&mut self, _: &Candidate, _: SetupOptions) -> Result<(), SetupFailure> {
            let (wait, commit) = {
                let mut state = self.state.lock().unwrap();
                state.calls.push("commit");
                (state.wait_commit, state.commit)
            };
            if wait {
                std::future::pending().await
            } else {
                commit
            }
        }
    }

    fn candidate(kind: ConfirmationKind) -> Candidate {
        Candidate {
            kind,
            address: "AA:BB:CC:DD:EE:FF".into(),
            device_name: "iPhone".into(),
            passkey: (kind == ConfirmationKind::Pairing).then(|| "123456".into()),
        }
    }

    async fn run(backend: FakeBackend, input: &str) -> (bool, Vec<serde_json::Value>) {
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut server_read = BufReader::new(server_read);
        let task = tokio::spawn(async move {
            let mut protocol = SetupProtocol::new(backend, FakeClock::default());
            protocol
                .run(&mut server_read, &mut server_write, SetupOptions::default())
                .await
        });
        if input.is_empty() {
            client_write.shutdown().await.unwrap();
        }
        let mut reader = BufReader::new(client_read);
        let mut events = Vec::new();
        let mut sent = input.is_empty();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            let event: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
            if !sent && event["event"] == "confirmation-request" {
                client_write.write_all(input.as_bytes()).await.unwrap();
                sent = true;
            }
            events.push(event);
        }
        (task.await.unwrap(), events)
    }

    #[tokio::test]
    async fn fresh_success_cleans_before_commit_and_complete() {
        let backend = FakeBackend::fresh();
        let state = backend.state.clone();
        let (ok, events) = run(
            backend,
            "{\"v\":1,\"command\":\"confirm\",\"requestId\":\"setup-1\",\"accept\":true}\n",
        )
        .await;
        assert!(ok);
        assert_eq!(events.last().unwrap()["event"], "complete");
        assert_eq!(
            state.lock().unwrap().calls,
            vec![
                "prepare",
                "candidate",
                "answer",
                "verify",
                "cleanup",
                "commit"
            ]
        );
    }

    #[tokio::test]
    async fn existing_bond_reuse_uses_same_confirmation_gate() {
        let backend = FakeBackend::fresh();
        backend.state.lock().unwrap().preparation = Ok(Preparation::Existing(candidate(
            ConfirmationKind::ExistingBond,
        )));
        let (ok, events) = run(
            backend,
            "{\"v\":1,\"command\":\"confirm\",\"requestId\":\"setup-1\",\"accept\":true}\n",
        )
        .await;
        assert!(ok);
        assert_eq!(events[1]["kind"], "existing-bond");
        assert!(events[1]["passkey"].is_null());
    }

    #[tokio::test]
    async fn rejection_cancel_mismatch_malformed_and_stdin_close_are_fatal_and_clean() {
        let cases = [
            (
                "{\"v\":1,\"command\":\"confirm\",\"requestId\":\"setup-1\",\"accept\":false}\n",
                "rejected",
            ),
            ("{\"v\":1,\"command\":\"cancel\"}\n", "cancelled"),
            (
                "{\"v\":1,\"command\":\"confirm\",\"requestId\":\"wrong\",\"accept\":true}\n",
                "invalid-protocol",
            ),
            ("not-json\n", "invalid-protocol"),
            ("", "stdin-closed"),
        ];
        for (input, code) in cases {
            let backend = FakeBackend::fresh();
            let state = backend.state.clone();
            let (ok, events) = run(backend, input).await;
            assert!(!ok);
            assert_eq!(events.last().unwrap()["code"], code);
            assert!(state.lock().unwrap().calls.contains(&"cleanup"));
            assert!(!state.lock().unwrap().calls.contains(&"commit"));
        }
    }

    #[tokio::test]
    async fn backend_failure_is_reported_after_cleanup() {
        let backend = FakeBackend::fresh();
        backend.state.lock().unwrap().verify = Err(SetupFailure::BackendFailed);
        let (ok, events) = run(
            backend,
            "{\"v\":1,\"command\":\"confirm\",\"requestId\":\"setup-1\",\"accept\":true}\n",
        )
        .await;
        assert!(!ok);
        assert_eq!(events.last().unwrap()["code"], "backend-failed");
    }

    async fn timeout_case(
        backend: FakeBackend,
        trigger_event: &str,
        input: Option<&str>,
        advance: Duration,
    ) -> Vec<serde_json::Value> {
        let clock = FakeClock::default();
        let advance_clock = clock.clone();
        let (client, server) = tokio::io::duplex(4096);
        let (client_read, mut client_write) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let mut server_read = BufReader::new(server_read);
        let task = tokio::spawn(async move {
            SetupProtocol::new(backend, clock)
                .run(&mut server_read, &mut server_write, SetupOptions::default())
                .await
        });
        let mut output = BufReader::new(client_read);
        let mut events = Vec::new();
        let mut sent = false;
        loop {
            let mut line = String::new();
            if output.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            let event: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
            if !sent && event["event"] == "confirmation-request" {
                if let Some(input) = input {
                    client_write.write_all(input.as_bytes()).await.unwrap();
                }
                sent = true;
            }
            let should_advance = event["state"] == trigger_event || event["event"] == trigger_event;
            events.push(event);
            if should_advance {
                tokio::task::yield_now().await;
                advance_clock.advance(advance);
            }
        }
        assert!(!task.await.unwrap());
        events
    }

    #[tokio::test]
    async fn all_three_setup_deadlines_are_injectable_and_stable() {
        let candidate_backend = FakeBackend::fresh();
        candidate_backend.state.lock().unwrap().wait_candidate = true;
        let events = timeout_case(
            candidate_backend,
            "waiting-for-iphone",
            None,
            CANDIDATE_TIMEOUT,
        )
        .await;
        assert_eq!(events.last().unwrap()["code"], "candidate-timeout");

        let confirmation_backend = FakeBackend::fresh();
        let events = timeout_case(
            confirmation_backend,
            "confirmation-request",
            None,
            CONFIRMATION_TIMEOUT,
        )
        .await;
        assert_eq!(events.last().unwrap()["code"], "confirmation-timeout");

        let ancs_backend = FakeBackend::fresh();
        ancs_backend.state.lock().unwrap().wait_verify = true;
        let events = timeout_case(
            ancs_backend,
            "verifying-ancs",
            Some("{\"v\":1,\"command\":\"confirm\",\"requestId\":\"setup-1\",\"accept\":true}\n"),
            ANCS_TIMEOUT,
        )
        .await;
        assert_eq!(events.last().unwrap()["code"], "ancs-timeout");
    }

    #[tokio::test]
    async fn cancellation_is_honored_in_every_published_state() {
        for state_name in [
            "checking-environment",
            "waiting-for-iphone",
            "verifying-ancs",
            "applying-configuration",
        ] {
            let backend = FakeBackend::fresh();
            {
                let mut state = backend.state.lock().unwrap();
                match state_name {
                    "checking-environment" => state.wait_prepare = true,
                    "waiting-for-iphone" => state.wait_candidate = true,
                    "verifying-ancs" => state.wait_verify = true,
                    "applying-configuration" => state.wait_commit = true,
                    _ => unreachable!(),
                }
            }
            let (client, server) = tokio::io::duplex(4096);
            let (client_read, mut client_write) = tokio::io::split(client);
            let (server_read, mut server_write) = tokio::io::split(server);
            let mut server_read = BufReader::new(server_read);
            let task = tokio::spawn(async move {
                SetupProtocol::new(backend, FakeClock::default())
                    .run(&mut server_read, &mut server_write, SetupOptions::default())
                    .await
            });
            let mut output = BufReader::new(client_read);
            let mut last = serde_json::Value::Null;
            loop {
                let mut line = String::new();
                if output.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let event: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
                if event["event"] == "confirmation-request" {
                    client_write
                        .write_all(b"{\"v\":1,\"command\":\"confirm\",\"requestId\":\"setup-1\",\"accept\":true}\n")
                        .await
                        .unwrap();
                }
                if event["state"] == state_name {
                    client_write
                        .write_all(b"{\"v\":1,\"command\":\"cancel\"}\n")
                        .await
                        .unwrap();
                }
                last = event;
            }
            assert!(!task.await.unwrap());
            assert_eq!(last["code"], "cancelled", "state {state_name}");
        }
    }
}
