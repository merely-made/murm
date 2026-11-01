// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use crate::{
    Bootstrap, BoxFuture, CallContext, CallError, Capability, HostObject, Limits, PeerLocator,
    Profile, SessionAuthority, SessionError, SessionId, Value,
    syrup::{self, Item},
    wire::{self, protocol},
};
use ed25519_dalek::SigningKey;
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, watch},
    time::{Instant, timeout},
};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct SessionConfig {
    pub profile: Profile,
    pub limits: Limits,
    pub local_locator: PeerLocator,
    /// A locator policy supplied by the admitted stream's owner. A CapTP
    /// signature alone does not authenticate a locator's real-world owner.
    pub expected_remote: Option<PeerLocator>,
    pub authority: SessionAuthority,
}
impl SessionConfig {
    pub fn new(local_locator: PeerLocator, authority: SessionAuthority) -> Self {
        Self {
            profile: Profile::OcapnSuite31f0b806,
            limits: Limits::default(),
            local_locator,
            expected_remote: None,
            authority,
        }
    }
}

struct Shared {
    id: SessionId,
    limits: Limits,
    commands: mpsc::Sender<Command>,
    releases: mpsc::Sender<Release>,
    closed: watch::Sender<bool>,
    ready: watch::Sender<bool>,
    notify: Notify,
    release_overflow: AtomicBool,
    next: AtomicU64,
    objects: Arc<Semaphore>,
    resolution_lock: Mutex<()>,
}
impl Shared {
    fn next(&self) -> Result<u64, SessionError> {
        let value = self.next.fetch_add(1, Ordering::Relaxed);
        if value > i64::MAX as u64 {
            return Err(SessionError::Limit("slot space"));
        }
        Ok(value)
    }
    fn check_open(&self) -> Result<(), SessionError> {
        if *self.closed.borrow() {
            Err(SessionError::Closed)
        } else {
            Ok(())
        }
    }
    fn release(&self, release: Release) {
        if *self.closed.borrow() {
            return;
        }
        if self.releases.try_send(release).is_err() {
            self.release_overflow.store(true, Ordering::Release);
        }
        self.notify.notify_one();
    }
    fn close(&self) {
        self.closed.send_replace(true);
        self.notify.notify_waiters();
    }
    fn local(self: &Arc<Self>, kind: LocalKind) -> Result<Capability, SessionError> {
        self.check_open()?;
        let permit = self
            .objects
            .clone()
            .try_acquire_owned()
            .map_err(|_| SessionError::Limit("local objects"))?;
        let local = Arc::new(LocalObject {
            id: self.next()?,
            kind,
            _permit: permit,
        });
        Ok(self.local_cap(local))
    }
    fn local_cap(self: &Arc<Self>, local: Arc<LocalObject>) -> Capability {
        Capability(Arc::new(CapInner {
            session: self.id,
            kind: CapKind::Local(local),
            shared: Arc::downgrade(self),
        }))
    }
    fn target(self: &Arc<Self>, slot: u64) -> Capability {
        Capability(Arc::new(CapInner {
            session: self.id,
            kind: CapKind::Remote {
                slot,
                epoch: 0,
                promise: false,
            },
            shared: Arc::downgrade(self),
        }))
    }
}

pub(crate) struct CapInner {
    pub(crate) session: SessionId,
    kind: CapKind,
    shared: Weak<Shared>,
}
enum CapKind {
    Local(Arc<LocalObject>),
    Remote {
        slot: u64,
        epoch: u64,
        promise: bool,
    },
    Answer {
        slot: u64,
        active: Arc<AtomicBool>,
    },
}
impl Drop for CapInner {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            match self.kind {
                CapKind::Remote { slot, epoch, .. } if epoch != 0 => {
                    shared.release(Release::Import { slot, epoch })
                },
                CapKind::Answer { slot, ref active } if active.load(Ordering::Acquire) => {
                    shared.release(Release::Answer(slot))
                },
                _ => (),
            }
        }
    }
}
struct LocalObject {
    id: u64,
    kind: LocalKind,
    _permit: OwnedSemaphorePermit,
}
enum LocalKind {
    Host(Arc<dyn HostObject>),
    Promise(Arc<PromiseCell>),
}
struct PromiseCell {
    state: watch::Sender<Option<Result<Value, CallError>>>,
}
struct ReplyState {
    state: watch::Sender<Option<Result<Value, CallError>>>,
    deadline: Instant,
}

#[derive(Clone)]
pub struct SessionHandle {
    shared: Arc<Shared>,
}
impl SessionHandle {
    pub fn session_id(&self) -> SessionId {
        self.shared.id
    }
    pub fn is_closed(&self) -> bool {
        *self.shared.closed.borrow()
    }
    /// The peer's implicit bootstrap capability. This grants no access to a
    /// registered object without its host-selected swiss capability.
    pub fn bootstrap(&self) -> Capability {
        self.shared.target(0)
    }
    pub async fn closed(&self) {
        let mut rx = self.shared.closed.subscribe();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }
    pub async fn wait_ready(&self) -> Result<(), SessionError> {
        let mut ready = self.shared.ready.subscribe();
        loop {
            self.shared.check_open()?;
            if *ready.borrow_and_update() {
                return Ok(());
            }
            tokio::select! { changed = ready.changed() => changed.map_err(|_| SessionError::Closed)?, _ = self.closed() => return Err(SessionError::Closed) }
        }
    }
    pub async fn export(&self, object: Arc<dyn HostObject>) -> Result<Capability, SessionError> {
        self.shared.local(LocalKind::Host(object))
    }
    pub async fn promise_pair(&self) -> Result<(LocalPromise, Resolver), SessionError> {
        let cell = Arc::new(PromiseCell {
            state: watch::channel(None).0,
        });
        let capability = self.shared.local(LocalKind::Promise(cell.clone()))?;
        Ok((
            LocalPromise { capability },
            Resolver {
                cell,
                shared: self.shared.clone(),
            },
        ))
    }
    fn check_args(&self, target: &Capability, args: &[Value]) -> Result<(), SessionError> {
        self.shared.check_open()?;
        if target.session_id() != self.session_id() {
            return Err(SessionError::ForeignCapability);
        }
        if matches!(target.0.kind, CapKind::Local(_)) {
            return Err(SessionError::Protocol("call requires remote target".into()));
        }
        Value::check_sequence(args, &self.shared.limits, self.session_id())
    }
    fn make_call(
        &self,
        target: &Capability,
        args: Vec<Value>,
    ) -> Result<(Command, RemotePromise), SessionError> {
        self.check_args(target, &args)?;
        let answer = self.shared.next()?;
        let resolver = self.shared.next()?;
        let reply = Arc::new(ReplyState {
            state: watch::channel(None).0,
            deadline: Instant::now() + self.shared.limits.request_timeout,
        });
        let pipeline_target = Capability(Arc::new(CapInner {
            session: self.session_id(),
            kind: CapKind::Answer {
                slot: answer,
                active: Arc::new(AtomicBool::new(false)),
            },
            shared: Arc::downgrade(&self.shared),
        }));
        let promise = RemotePromise {
            target: pipeline_target.clone(),
            reply: reply.clone(),
            shared: self.shared.clone(),
        };
        Ok((
            Command::Call {
                target: target.clone(),
                args,
                answer,
                resolver,
                reply,
                lease: pipeline_target,
            },
            promise,
        ))
    }
    pub fn try_call(
        &self,
        target: &Capability,
        args: Vec<Value>,
    ) -> Result<RemotePromise, SessionError> {
        let (command, promise) = self.make_call(target, args)?;
        self.shared
            .commands
            .try_send(command)
            .map_err(queue_error)?;
        Ok(promise)
    }
    pub async fn call(
        &self,
        target: &Capability,
        args: Vec<Value>,
    ) -> Result<RemotePromise, SessionError> {
        let (command, promise) = self.make_call(target, args)?;
        timeout(
            self.shared.limits.request_timeout,
            self.shared.commands.send(command),
        )
        .await
        .map_err(|_| SessionError::Timeout)?
        .map_err(|_| SessionError::Closed)?;
        Ok(promise)
    }
    pub fn try_send(&self, target: &Capability, args: Vec<Value>) -> Result<(), SessionError> {
        self.check_args(target, &args)?;
        self.shared
            .commands
            .try_send(Command::Send {
                target: target.clone(),
                args,
            })
            .map_err(queue_error)
    }
    pub async fn send(&self, target: &Capability, args: Vec<Value>) -> Result<(), SessionError> {
        self.check_args(target, &args)?;
        timeout(
            self.shared.limits.request_timeout,
            self.shared.commands.send(Command::Send {
                target: target.clone(),
                args,
            }),
        )
        .await
        .map_err(|_| SessionError::Timeout)?
        .map_err(|_| SessionError::Closed)
    }
    pub async fn fetch(&self, swiss: &[u8]) -> Result<RemotePromise, SessionError> {
        if swiss.is_empty() || swiss.len() > self.shared.limits.max_swiss_bytes {
            return Err(SessionError::Limit("swiss bytes"));
        }
        self.call(
            &self.bootstrap(),
            vec![Value::symbol("fetch"), Value::Bytes(swiss.to_vec())],
        )
        .await
    }
    pub async fn listen(&self, target: &Capability) -> Result<RemotePromise, SessionError> {
        self.check_args(target, &[])?;
        if !matches!(
            target.0.kind,
            CapKind::Remote { promise: true, .. } | CapKind::Answer { .. }
        ) {
            return Err(protocol("listen target must be a promise"));
        }
        let resolver = self.shared.next()?;
        let reply = Arc::new(ReplyState {
            state: watch::channel(None).0,
            deadline: Instant::now() + self.shared.limits.request_timeout,
        });
        let command = Command::Listen {
            target: target.clone(),
            resolver,
            reply: reply.clone(),
        };
        timeout(
            self.shared.limits.request_timeout,
            self.shared.commands.send(command),
        )
        .await
        .map_err(|_| SessionError::Timeout)?
        .map_err(|_| SessionError::Closed)?;
        Ok(RemotePromise {
            target: target.clone(),
            reply,
            shared: self.shared.clone(),
        })
    }
    pub async fn close(&self, _reason: &str) -> Result<(), SessionError> {
        self.shared.close();
        Ok(())
    }
}
fn queue_error<T>(error: mpsc::error::TrySendError<T>) -> SessionError {
    match error {
        mpsc::error::TrySendError::Full(_) => SessionError::Limit("command queue"),
        mpsc::error::TrySendError::Closed(_) => SessionError::Closed,
    }
}

#[derive(Clone)]
pub struct RemotePromise {
    target: Capability,
    reply: Arc<ReplyState>,
    shared: Arc<Shared>,
}
impl RemotePromise {
    pub fn target(&self) -> Capability {
        self.target.clone()
    }
    pub async fn resolve(&self) -> Result<Value, CallError> {
        let mut rx = self.reply.state.subscribe();
        loop {
            if let Some(result) = rx.borrow_and_update().clone() {
                return result;
            }
            if *self.shared.closed.borrow() {
                return Err(SessionError::Closed.into());
            }
            tokio::select! {
                changed = rx.changed() => changed.map_err(|_| CallError::Session(SessionError::Closed))?,
                _ = wait_closed(&self.shared) => return Err(SessionError::Closed.into()),
                _ = tokio::time::sleep_until(self.reply.deadline) => return Err(SessionError::Timeout.into()),
            }
        }
    }
}
async fn wait_closed(shared: &Shared) {
    let mut rx = shared.closed.subscribe();
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            break;
        }
    }
}
#[derive(Clone)]
pub struct LocalPromise {
    capability: Capability,
}
impl LocalPromise {
    pub fn capability(&self) -> Capability {
        self.capability.clone()
    }
}
#[derive(Clone)]
pub struct Resolver {
    cell: Arc<PromiseCell>,
    shared: Arc<Shared>,
}
impl Resolver {
    fn settle(&self, result: Result<Value, CallError>) -> Result<(), SessionError> {
        self.shared.check_open()?;
        match &result {
            Ok(v) | Err(CallError::Broken(v)) => v.check(&self.shared.limits, self.shared.id)?,
            Err(CallError::Session(_)) => (),
        }
        // Serialize graph inspection and publication across all resolvers in
        // this session, including callers settling from other OS threads.
        // Without this guard two simultaneous p1=>p2 / p2=>p1 settlements
        // could each inspect the other's old unresolved state.
        let _resolution = self
            .shared
            .resolution_lock
            .lock()
            .map_err(|_| protocol("resolution lock poisoned"))?;
        // A remote resolver argument may echo our own promise back to us.
        // Reject direct and indirect local promise cycles before storing a
        // strong reference in the cell; otherwise Arc cycles survive close.
        if let Ok(value) | Err(CallError::Broken(value)) = &result {
            check_promise_graph(value, &self.cell, &self.shared.limits)?;
        }
        let mut already = false;
        self.cell.state.send_if_modified(|state| {
            if state.is_some() {
                already = true;
                false
            } else {
                *state = Some(result);
                true
            }
        });
        if already {
            return Err(protocol("promise already settled"));
        }
        self.shared.notify.notify_one();
        Ok(())
    }
    pub async fn fulfill(&self, value: Value) -> Result<(), SessionError> {
        self.settle(Ok(value))
    }
    pub async fn break_with(&self, reason: Value) -> Result<(), SessionError> {
        self.settle(Err(CallError::Broken(reason)))
    }
}

enum Command {
    Call {
        target: Capability,
        args: Vec<Value>,
        answer: u64,
        resolver: u64,
        reply: Arc<ReplyState>,
        lease: Capability,
    },
    Send {
        target: Capability,
        args: Vec<Value>,
    },
    Listen {
        target: Capability,
        resolver: u64,
        reply: Arc<ReplyState>,
    },
}
enum Release {
    Import { slot: u64, epoch: u64 },
    Answer(u64),
}
enum IoEvent {
    Frame(Vec<u8>),
    Failed(SessionError),
}
struct Export {
    kind: ExportKind,
    wire_count: u64,
}
enum ExportKind {
    Local(Arc<LocalObject>),
    Reply(Weak<ReplyState>),
}
struct Import {
    capability: Weak<CapInner>,
    epoch: u64,
    wire_count: u64,
    promise: bool,
}
struct Answer {
    result: Option<Result<Value, CallError>>,
    waiting: VecDeque<Invocation>,
    listeners: Vec<Capability>,
    released: bool,
    deadline: Instant,
}
struct PromiseEntry {
    cell: Arc<PromiseCell>,
    listeners: Vec<Capability>,
    waiting: VecDeque<Invocation>,
    deadline: Instant,
}
#[derive(Clone, Copy)]
enum Target {
    Export(u64),
    Answer(u64),
}
struct Completion {
    answer: Option<u64>,
    resolver: Option<Capability>,
}
struct Invocation {
    target: Target,
    args: Vec<Value>,
    completion: Completion,
}
struct Completed {
    completion: Completion,
    result: Result<Value, CallError>,
}

struct Core {
    shared: Arc<Shared>,
    config: SessionConfig,
    commands: mpsc::Receiver<Command>,
    releases: mpsc::Receiver<Release>,
    outbound: Option<mpsc::Sender<Vec<u8>>>,
    bootstrap: BTreeMap<Vec<u8>, Arc<dyn HostObject>>,
    bootstrap_caps: HashMap<Vec<u8>, Capability>,
    exports: HashMap<u64, Export>,
    export_ids: HashMap<u64, u64>,
    imports: HashMap<u64, Import>,
    answers: HashMap<u64, Answer>,
    promises: HashMap<u64, PromiseEntry>,
    jobs: FuturesUnordered<BoxFuture<'static, Completed>>,
    completions: VecDeque<Completed>,
    draining_completions: bool,
    open: bool,
}

pub struct Driver<R, W> {
    reader: Option<R>,
    writer: Option<W>,
    core: Option<Core>,
    shared: Arc<Shared>,
}
impl<R, W> Drop for Driver<R, W> {
    fn drop(&mut self) {
        self.shared.close();
    }
}

pub fn attach<R, W>(
    reader: R,
    writer: W,
    config: SessionConfig,
    bootstrap: Bootstrap,
) -> Result<(SessionHandle, Driver<R, W>), SessionError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    config.limits.validate()?;
    let locator = config.local_locator.item();
    PeerLocator::parse(&locator)?;
    syrup::encode(&locator, &config.limits)?;
    if let Some(expected) = &config.expected_remote {
        PeerLocator::parse(&expected.item())?;
        syrup::encode(&expected.item(), &config.limits)?;
    }
    if bootstrap.objects.len() > config.limits.max_bootstrap_objects
        || bootstrap
            .objects
            .keys()
            .any(|s| s.len() > config.limits.max_swiss_bytes)
    {
        return Err(SessionError::Limit("bootstrap objects"));
    }
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| protocol("OS entropy unavailable"))?;
    let (commands, command_rx) = mpsc::channel(config.limits.max_command_queue);
    let (releases, release_rx) =
        mpsc::channel(config.limits.max_imports + config.limits.max_answers);
    let shared = Arc::new(Shared {
        id: SessionId(id),
        limits: config.limits.clone(),
        commands,
        releases,
        closed: watch::channel(false).0,
        ready: watch::channel(false).0,
        notify: Notify::new(),
        release_overflow: AtomicBool::new(false),
        next: AtomicU64::new(1),
        objects: Arc::new(Semaphore::new(config.limits.max_exports)),
        resolution_lock: Mutex::new(()),
    });
    let core = Core {
        shared: shared.clone(),
        config,
        commands: command_rx,
        releases: release_rx,
        outbound: None,
        bootstrap: bootstrap.objects,
        bootstrap_caps: HashMap::new(),
        exports: HashMap::new(),
        export_ids: HashMap::new(),
        imports: HashMap::new(),
        answers: HashMap::new(),
        promises: HashMap::new(),
        jobs: FuturesUnordered::new(),
        completions: VecDeque::new(),
        draining_completions: false,
        open: false,
    };
    Ok((
        SessionHandle {
            shared: shared.clone(),
        },
        Driver {
            reader: Some(reader),
            writer: Some(writer),
            core: Some(core),
            shared,
        },
    ))
}

struct IoTasks {
    reader: Option<tokio::task::JoinHandle<()>>,
    writer: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for IoTasks {
    fn drop(&mut self) {
        if let Some(t) = &self.reader {
            t.abort();
        }
        if let Some(t) = &self.writer {
            t.abort();
        }
    }
}

impl<R, W> Driver<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    pub async fn run(mut self) -> Result<(), SessionError> {
        let mut reader = self.reader.take().ok_or(SessionError::Closed)?;
        let mut writer = self.writer.take().ok_or(SessionError::Closed)?;
        let mut core = self.core.take().ok_or(SessionError::Closed)?;
        let limits = core.config.limits.clone();
        let (events, mut event_rx) = mpsc::channel(2);
        let (outbound, mut out_rx) = mpsc::channel::<Vec<u8>>(limits.max_outbound_frames);
        core.outbound = Some(outbound);
        let read_events = events.clone();
        let read_limits = limits.clone();
        let read_task = tokio::spawn(async move {
            loop {
                let result = timeout(
                    read_limits.idle_timeout,
                    read_frame(&mut reader, read_limits.max_frame_bytes),
                )
                .await
                .unwrap_or(Err(SessionError::Timeout));
                match result {
                    Ok(frame) => {
                        if read_events.send(IoEvent::Frame(frame)).await.is_err() {
                            break;
                        }
                    },
                    Err(error) => {
                        let _ = read_events.send(IoEvent::Failed(error)).await;
                        break;
                    },
                }
            }
        });
        let write_timeout = limits.request_timeout;
        let write_task = tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                let write = async {
                    writer.write_all(frame.len().to_string().as_bytes()).await?;
                    writer.write_all(b":").await?;
                    writer.write_all(&frame).await?;
                    writer.flush().await
                };
                match timeout(write_timeout, write).await {
                    Ok(Ok(())) => (),
                    result => {
                        let error = match result {
                            Err(_) => SessionError::Timeout,
                            Ok(Err(e)) => SessionError::Io(e.to_string()),
                            _ => unreachable!(),
                        };
                        let _ = events.send(IoEvent::Failed(error)).await;
                        return;
                    },
                }
            }
            let _ = timeout(write_timeout, writer.shutdown()).await;
        });
        let mut tasks = IoTasks {
            reader: Some(read_task),
            writer: Some(write_task),
        };
        let mut seed = Zeroizing::new([0; 32]);
        getrandom::fill(&mut *seed).map_err(|_| protocol("OS entropy unavailable"))?;
        let key = SigningKey::from_bytes(&seed);
        core.queue(wire::start(&key, &core.config.local_locator, &limits)?)?;
        drop(key);
        drop(seed);
        let handshake_deadline = Instant::now() + limits.handshake_timeout;
        let session_deadline = Instant::now() + limits.max_session_duration;
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let result: Result<(), SessionError> = async {
            loop {
                if *core.shared.closed.borrow() { return Ok(()); }
                if core.shared.release_overflow.load(Ordering::Acquire) { return Err(SessionError::Limit("release queue")); }
                core.pump_promises()?;
                let notify_shared = core.shared.clone();
                tokio::select! {
                    event = event_rx.recv() => match event { Some(IoEvent::Frame(frame)) => core.receive(syrup::decode(&frame, &limits)?)?, Some(IoEvent::Failed(error)) => return Err(error), None => return Err(SessionError::Closed) },
                    command = core.commands.recv(), if core.open => match command { Some(command) => core.command(command)?, None => return Ok(()) },
                    release = core.releases.recv() => if let Some(release) = release { core.release(release)?; },
                    completed = core.jobs.next(), if !core.jobs.is_empty() => if let Some(completed) = completed { core.complete(completed)?; },
                    _ = notify_shared.notify.notified() => (),
                    _ = tick.tick() => core.check_deadlines()?,
                    _ = tokio::time::sleep_until(handshake_deadline), if !core.open => return Err(SessionError::Timeout),
                    _ = tokio::time::sleep_until(session_deadline) => return Err(SessionError::Timeout),
                }
            }
        }.await;
        if !matches!(result, Err(SessionError::Closed)) {
            let reason = if result.is_ok() {
                "session closed"
            } else {
                "session failed"
            };
            let _ = core.queue(Item::record("op:abort", vec![Item::String(reason.into())]));
        }
        core.shared.close();
        core.outbound.take();
        drop(core);
        if let Some(task) = tasks.reader.take() {
            task.abort();
        }
        if let Some(mut task) = tasks.writer.take()
            && timeout(limits.request_timeout, &mut task).await.is_err()
        {
            task.abort();
        }
        result
    }
}
async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Vec<u8>, SessionError> {
    let mut length = 0usize;
    let mut digits = 0usize;
    let mut leading_zero = false;
    loop {
        let byte = reader
            .read_u8()
            .await
            .map_err(|e| SessionError::Io(e.to_string()))?;
        if byte == b':' {
            if digits == 0 {
                return Err(protocol("empty frame length"));
            }
            break;
        }
        if !byte.is_ascii_digit() || digits >= 20 || (digits > 0 && leading_zero) {
            return Err(protocol("invalid frame length"));
        }
        leading_zero = digits == 0 && byte == b'0';
        digits += 1;
        length = length
            .checked_mul(10)
            .and_then(|n| n.checked_add(usize::from(byte - b'0')))
            .ok_or(SessionError::Limit("frame length"))?;
        if length > max {
            return Err(SessionError::Limit("frame bytes"));
        }
    }
    if length == 0 {
        return Err(protocol("empty frame"));
    }
    let mut frame = vec![0; length];
    reader
        .read_exact(&mut frame)
        .await
        .map_err(|e| SessionError::Io(e.to_string()))?;
    Ok(frame)
}

impl Drop for Core {
    fn drop(&mut self) {
        self.shared.close();
        for export in self.exports.values() {
            if let ExportKind::Reply(reply) = &export.kind
                && let Some(reply) = reply.upgrade()
                && reply.state.borrow().is_none()
            {
                reply
                    .state
                    .send_replace(Some(Err(SessionError::Closed.into())));
            }
        }
    }
}

impl Core {
    fn queue(&self, item: Item) -> Result<(), SessionError> {
        let frame = syrup::encode(&item, &self.config.limits)?;
        self.outbound
            .as_ref()
            .ok_or(SessionError::Closed)?
            .try_send(frame)
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => SessionError::Limit("outbound frames"),
                _ => SessionError::Closed,
            })
    }
    fn descriptor(&self, cap: &Capability) -> Result<Item, SessionError> {
        if cap.session_id() != self.shared.id {
            return Err(SessionError::ForeignCapability);
        }
        match &cap.0.kind {
            CapKind::Remote { slot, .. } => {
                Ok(Item::record("desc:export", vec![Item::slot(*slot)?]))
            },
            CapKind::Answer { slot, .. } => {
                Ok(Item::record("desc:answer", vec![Item::slot(*slot)?]))
            },
            CapKind::Local(_) => Err(protocol("expected remote target")),
        }
    }
    fn encode_value(&mut self, value: &Value) -> Result<Item, SessionError> {
        Ok(match value {
            Value::Bool(b) => Item::Bool(*b),
            Value::Integer(n) => Item::Int(*n),
            Value::String(s) => Item::String(s.clone()),
            Value::Symbol(s) => Item::Symbol(s.clone()),
            Value::Bytes(b) => Item::Bytes(b.clone()),
            Value::List(xs) => Item::List(
                xs.iter()
                    .map(|v| self.encode_value(v))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Map(xs) => Item::Map(
                xs.iter()
                    .map(|(k, v)| Ok((self.encode_value(k)?, self.encode_value(v)?)))
                    .collect::<Result<_, SessionError>>()?,
            ),
            Value::Record { label, fields } => Item::Record(
                label.clone(),
                fields
                    .iter()
                    .map(|v| self.encode_value(v))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Cap(cap) => {
                if cap.session_id() != self.shared.id {
                    return Err(SessionError::ForeignCapability);
                }
                match &cap.0.kind {
                    CapKind::Remote { slot, .. } => {
                        Item::record("desc:export", vec![Item::slot(*slot)?])
                    },
                    CapKind::Answer { .. } => {
                        return Err(SessionError::Unsupported(
                            "answer reference as argument; resolve or use as target",
                        ));
                    },
                    CapKind::Local(local) => {
                        let slot = if let Some(slot) = self.export_ids.get(&local.id) {
                            *slot
                        } else {
                            if self.exports.len() >= self.config.limits.max_exports {
                                return Err(SessionError::Limit("exports"));
                            }
                            let slot = self.shared.next()?;
                            self.exports.insert(
                                slot,
                                Export {
                                    kind: ExportKind::Local(local.clone()),
                                    wire_count: 0,
                                },
                            );
                            self.export_ids.insert(local.id, slot);
                            slot
                        };
                        let entry = self
                            .exports
                            .get_mut(&slot)
                            .ok_or(SessionError::UnknownReference)?;
                        entry.wire_count = entry
                            .wire_count
                            .checked_add(1)
                            .ok_or(SessionError::Limit("wire count"))?;
                        let label = match &local.kind {
                            LocalKind::Host(_) => "desc:import-object",
                            LocalKind::Promise(cell) => {
                                self.track_promise(local.id, cell.clone())?;
                                "desc:import-promise"
                            },
                        };
                        Item::record(label, vec![Item::slot(slot)?])
                    },
                }
            },
        })
    }
    fn import(&mut self, slot: u64, promise: bool) -> Result<Capability, SessionError> {
        if let Some(entry) = self.imports.get_mut(&slot) {
            if entry.promise != promise {
                return Err(protocol("reference kind changed"));
            }
            if let Some(capability) = entry.capability.upgrade() {
                entry.wire_count = entry
                    .wire_count
                    .checked_add(1)
                    .ok_or(SessionError::Limit("wire count"))?;
                return Ok(Capability(capability));
            }
            let epoch = entry.epoch;
            self.release(Release::Import { slot, epoch })?;
        }
        if self.imports.len() >= self.config.limits.max_imports {
            return Err(SessionError::Limit("imports"));
        }
        let epoch = self.shared.next()?;
        let capability = Arc::new(CapInner {
            session: self.shared.id,
            kind: CapKind::Remote {
                slot,
                epoch,
                promise,
            },
            shared: Arc::downgrade(&self.shared),
        });
        self.imports.insert(
            slot,
            Import {
                capability: Arc::downgrade(&capability),
                epoch,
                wire_count: 1,
                promise,
            },
        );
        Ok(Capability(capability))
    }
    fn decode_value(&mut self, item: &Item) -> Result<Value, SessionError> {
        Ok(match item {
            Item::Bool(b) => Value::Bool(*b),
            Item::Int(n) => Value::Integer(*n),
            Item::String(s) => Value::String(s.clone()),
            Item::Symbol(s) => Value::Symbol(s.clone()),
            Item::Bytes(b) => Value::Bytes(b.clone()),
            Item::List(xs) => Value::List(
                xs.iter()
                    .map(|x| self.decode_value(x))
                    .collect::<Result<_, _>>()?,
            ),
            Item::Map(xs) => Value::Map(
                xs.iter()
                    .map(|(k, v)| Ok((self.decode_value(k)?, self.decode_value(v)?)))
                    .collect::<Result<_, SessionError>>()?,
            ),
            Item::Record(label, fields) => match label.as_str() {
                "desc:import-object" | "desc:import-promise" => {
                    if fields.len() != 1 {
                        return Err(protocol("invalid import descriptor"));
                    }
                    Value::Cap(self.import(fields[0].as_slot()?, label == "desc:import-promise")?)
                },
                "desc:export" => {
                    if fields.len() != 1 {
                        return Err(protocol("invalid export descriptor"));
                    }
                    let slot = fields[0].as_slot()?;
                    let Some(Export {
                        kind: ExportKind::Local(local),
                        ..
                    }) = self.exports.get(&slot)
                    else {
                        return Err(SessionError::UnknownReference);
                    };
                    Value::Cap(self.shared.local_cap(local.clone()))
                },
                _ if label.starts_with("desc:") => {
                    return Err(SessionError::Unsupported(
                        "handoff or unsupported reference descriptor",
                    ));
                },
                _ if label.starts_with("op:")
                    || label.starts_with("ocapn-")
                    || label == "my-location" =>
                {
                    return Err(SessionError::Unsupported(
                        "protocol record as passable value",
                    ));
                },
                _ => Value::Record {
                    label: label.clone(),
                    fields: fields
                        .iter()
                        .map(|x| self.decode_value(x))
                        .collect::<Result<_, _>>()?,
                },
            },
        })
    }
    fn command(&mut self, command: Command) -> Result<(), SessionError> {
        match command {
            Command::Call {
                target,
                args,
                answer,
                resolver,
                reply,
                lease,
            } => {
                self.add_reply(resolver, &reply)?;
                let target = self.descriptor(&target)?;
                let args = self.encode_value(&Value::List(args))?;
                self.queue(Item::record(
                    "op:deliver",
                    vec![
                        target,
                        args,
                        Item::slot(answer)?,
                        Item::record("desc:import-object", vec![Item::slot(resolver)?]),
                    ],
                ))?;
                if let CapKind::Answer { active, .. } = &lease.0.kind {
                    active.store(true, Ordering::Release);
                }
                Ok(())
            },
            Command::Send { target, args } => self.send(&target, args),
            Command::Listen {
                target,
                resolver,
                reply,
            } => {
                self.add_reply(resolver, &reply)?;
                self.queue(Item::record(
                    "op:listen",
                    vec![
                        self.descriptor(&target)?,
                        Item::record("desc:import-object", vec![Item::slot(resolver)?]),
                        Item::Bool(false),
                    ],
                ))
            },
        }
    }
    fn add_reply(&mut self, slot: u64, reply: &Arc<ReplyState>) -> Result<(), SessionError> {
        if self.exports.len() >= self.config.limits.max_exports {
            return Err(SessionError::Limit("exports"));
        }
        if self
            .exports
            .values()
            .filter(|e| matches!(e.kind, ExportKind::Reply(_)))
            .count()
            >= self.config.limits.max_answers
        {
            return Err(SessionError::Limit("outgoing answers"));
        }
        self.exports.insert(
            slot,
            Export {
                kind: ExportKind::Reply(Arc::downgrade(reply)),
                wire_count: 1,
            },
        );
        Ok(())
    }
    fn send(&mut self, target: &Capability, args: Vec<Value>) -> Result<(), SessionError> {
        Value::check_sequence(&args, &self.config.limits, self.shared.id)?;
        let target = self.descriptor(target)?;
        let args = self.encode_value(&Value::List(args))?;
        self.queue(Item::record(
            "op:deliver",
            vec![target, args, Item::Bool(false), Item::Bool(false)],
        ))
    }
    fn release(&mut self, release: Release) -> Result<(), SessionError> {
        match release {
            Release::Answer(slot) => {
                if self.open {
                    self.queue(Item::record(
                        "op:gc-answers",
                        vec![Item::List(vec![Item::slot(slot)?])],
                    ))?;
                }
            },
            Release::Import { slot, epoch } => {
                if self
                    .imports
                    .get(&slot)
                    .is_some_and(|e| e.epoch == epoch && e.capability.strong_count() == 0)
                {
                    let entry = self
                        .imports
                        .remove(&slot)
                        .ok_or(SessionError::UnknownReference)?;
                    if self.open {
                        self.queue(Item::record(
                            "op:gc-exports",
                            vec![
                                Item::List(vec![Item::slot(slot)?]),
                                Item::List(vec![Item::slot(entry.wire_count)?]),
                            ],
                        ))?;
                    }
                }
            },
        }
        Ok(())
    }
    fn receive(&mut self, item: Item) -> Result<(), SessionError> {
        let Item::Record(label, fields) = item else {
            return Err(protocol("expected operation record"));
        };
        if label == "op:abort" {
            if fields.len() != 1 || !matches!(&fields[0], Item::String(_)) {
                return Err(protocol("invalid abort"));
            }
            return Err(SessionError::Closed);
        }
        if label == "op:start-session" {
            if self.open {
                return Err(SessionError::Unsupported(
                    "duplicate session or crossed hellos",
                ));
            }
            wire::verify_start(
                &Item::Record(label, fields),
                self.config.expected_remote.as_ref(),
                &self.config.limits,
            )?;
            self.open = true;
            self.shared.ready.send_replace(true);
            return Ok(());
        }
        if !self.open {
            return Err(protocol("operation before session handshake"));
        }
        match label.as_str() {
            "op:deliver" => {
                if fields.len() != 4 {
                    return Err(protocol("legacy deliver requires four fields"));
                }
                let target = parse_target(&fields[0])?;
                let Item::List(args) = &fields[1] else {
                    return Err(protocol("deliver arguments must be a list"));
                };
                let args = args
                    .iter()
                    .map(|x| self.decode_value(x))
                    .collect::<Result<Vec<_>, _>>()?;
                let answer = if fields[2] == Item::Bool(false) {
                    None
                } else {
                    Some(fields[2].as_slot()?)
                };
                let resolver = if fields[3] == Item::Bool(false) {
                    None
                } else {
                    match self.decode_value(&fields[3])? {
                        Value::Cap(cap) if matches!(cap.0.kind, CapKind::Remote { .. }) => {
                            Some(cap)
                        },
                        _ => return Err(protocol("invalid resolver")),
                    }
                };
                if let Some(slot) = answer {
                    if self.answers.len() >= self.config.limits.max_answers {
                        return Err(SessionError::Limit("incoming answers"));
                    }
                    if self.answers.contains_key(&slot) {
                        return Err(protocol("duplicate answer slot"));
                    }
                    self.answers.insert(
                        slot,
                        Answer {
                            result: None,
                            waiting: VecDeque::new(),
                            listeners: Vec::new(),
                            released: false,
                            deadline: Instant::now() + self.config.limits.request_timeout,
                        },
                    );
                }
                self.dispatch(
                    Invocation {
                        target,
                        args,
                        completion: Completion { answer, resolver },
                    },
                    0,
                )
            },
            "op:listen" => {
                if fields.len() != 3 || fields[2] != Item::Bool(false) {
                    return Err(SessionError::Unsupported(
                        "listen mode; profile supports settling only",
                    ));
                }
                let target = parse_target(&fields[0])?;
                let Value::Cap(resolver) = self.decode_value(&fields[1])? else {
                    return Err(protocol("listen resolver"));
                };
                if !matches!(resolver.0.kind, CapKind::Remote { .. }) {
                    return Err(protocol("listen resolver must be remote"));
                }
                self.listen(target, resolver)
            },
            "op:gc-exports" => {
                if fields.len() != 2 {
                    return Err(protocol("invalid gc exports"));
                }
                let (Item::List(slots), Item::List(deltas)) = (&fields[0], &fields[1]) else {
                    return Err(protocol("invalid gc exports"));
                };
                if slots.len() != deltas.len() {
                    return Err(protocol("GC list lengths differ"));
                }
                for (slot, delta) in slots.iter().zip(deltas) {
                    let slot = slot.as_slot()?;
                    let delta = delta.as_slot()?;
                    let entry = self
                        .exports
                        .get_mut(&slot)
                        .ok_or(SessionError::UnknownReference)?;
                    if delta == 0 || delta > entry.wire_count {
                        return Err(protocol("invalid GC wire delta"));
                    }
                    entry.wire_count -= delta;
                    if entry.wire_count == 0 {
                        let entry = self
                            .exports
                            .remove(&slot)
                            .ok_or(SessionError::UnknownReference)?;
                        if let ExportKind::Local(local) = entry.kind {
                            self.export_ids.remove(&local.id);
                        }
                    }
                }
                Ok(())
            },
            "op:gc-answers" => {
                if fields.len() != 1 {
                    return Err(protocol("invalid gc answers"));
                }
                let Item::List(slots) = &fields[0] else {
                    return Err(protocol("invalid gc answers"));
                };
                for slot in slots {
                    let slot = slot.as_slot()?;
                    let entry = self
                        .answers
                        .get_mut(&slot)
                        .ok_or(SessionError::UnknownReference)?;
                    if entry.result.is_some() {
                        self.answers.remove(&slot);
                    } else {
                        entry.released = true;
                    }
                }
                Ok(())
            },
            _ => Err(SessionError::Unsupported(
                "operation or handoff outside pinned two-party profile",
            )),
        }
    }
    fn dispatch(&mut self, invocation: Invocation, depth: usize) -> Result<(), SessionError> {
        if depth > self.config.limits.max_depth {
            return Err(SessionError::Limit("promise routing depth"));
        }
        match invocation.target {
            Target::Answer(slot) => {
                let Some(answer) = self.answers.get_mut(&slot) else {
                    return Err(SessionError::UnknownReference);
                };
                if let Some(result) = answer.result.clone() {
                    self.route_result(invocation, result, depth + 1)
                } else {
                    if self.pipeline_count() >= self.config.limits.max_pipeline_calls {
                        return Err(SessionError::Limit("pipeline calls"));
                    }
                    self.answers
                        .get_mut(&slot)
                        .ok_or(SessionError::UnknownReference)?
                        .waiting
                        .push_back(invocation);
                    Ok(())
                }
            },
            Target::Export(0) => {
                let result = match invocation.args.as_slice() {
                    [Value::Symbol(method), Value::Bytes(swiss)] if method == "fetch" => {
                        if swiss.len() > self.config.limits.max_swiss_bytes {
                            return Err(SessionError::Limit("swiss bytes"));
                        }
                        if let Some(cap) = self.bootstrap_caps.get(swiss) {
                            Ok(Value::Cap(cap.clone()))
                        } else if let Some(object) = self.bootstrap.get(swiss) {
                            let cap = self.shared.local(LocalKind::Host(object.clone()))?;
                            self.bootstrap_caps.insert(swiss.clone(), cap.clone());
                            Ok(Value::Cap(cap))
                        } else {
                            Err(CallError::message("unknown capability"))
                        }
                    },
                    _ => Err(CallError::message("unsupported bootstrap method")),
                };
                self.complete(Completed {
                    completion: invocation.completion,
                    result,
                })
            },
            Target::Export(slot) => {
                let entry = self
                    .exports
                    .get(&slot)
                    .ok_or(SessionError::UnknownReference)?;
                match &entry.kind {
                    ExportKind::Reply(reply) => {
                        let result = match invocation.args.as_slice() {
                            [Value::Symbol(method), value] if method == "fulfill" => {
                                Ok(value.clone())
                            },
                            [Value::Symbol(method), reason] if method == "break" => {
                                Err(CallError::Broken(reason.clone()))
                            },
                            _ => return Err(protocol("invalid resolver invocation")),
                        };
                        if let Some(reply) = reply.upgrade() {
                            if reply.state.borrow().is_some() {
                                return Err(protocol("resolver already settled"));
                            }
                            reply.state.send_replace(Some(result));
                        }
                        self.complete(Completed {
                            completion: invocation.completion,
                            result: Ok(Value::Bool(false)),
                        })
                    },
                    ExportKind::Local(local) => {
                        let local = local.clone();
                        match &local.kind {
                            LocalKind::Host(object) => self.start_host(object.clone(), invocation),
                            LocalKind::Promise(cell) => {
                                let result = cell.state.borrow().clone();
                                if let Some(result) = result {
                                    self.route_result(invocation, result, depth + 1)
                                } else {
                                    self.track_promise(local.id, cell.clone())?;
                                    if self.pipeline_count()
                                        >= self.config.limits.max_pipeline_calls
                                    {
                                        return Err(SessionError::Limit("pipeline calls"));
                                    }
                                    self.promises
                                        .get_mut(&local.id)
                                        .ok_or(SessionError::UnknownReference)?
                                        .waiting
                                        .push_back(invocation);
                                    Ok(())
                                }
                            },
                        }
                    },
                }
            },
        }
    }
    fn route_result(
        &mut self,
        invocation: Invocation,
        result: Result<Value, CallError>,
        depth: usize,
    ) -> Result<(), SessionError> {
        match result {
            Err(error) => self.complete(Completed {
                completion: invocation.completion,
                result: Err(error),
            }),
            Ok(Value::Cap(cap)) => {
                if cap.session_id() != self.shared.id {
                    return Err(SessionError::ForeignCapability);
                }
                match &cap.0.kind {
                    CapKind::Local(local) => match &local.kind {
                        LocalKind::Host(object) => self.start_host(object.clone(), invocation),
                        LocalKind::Promise(cell) => {
                            let result = cell.state.borrow().clone();
                            if let Some(result) = result {
                                if depth >= self.config.limits.max_depth {
                                    return Err(SessionError::Limit("promise routing depth"));
                                }
                                self.route_result(invocation, result, depth + 1)
                            } else {
                                self.track_promise(local.id, cell.clone())?;
                                if self.pipeline_count() >= self.config.limits.max_pipeline_calls {
                                    return Err(SessionError::Limit("pipeline calls"));
                                }
                                self.promises
                                    .get_mut(&local.id)
                                    .ok_or(SessionError::UnknownReference)?
                                    .waiting
                                    .push_back(invocation);
                                Ok(())
                            }
                        },
                    },
                    CapKind::Remote { .. } | CapKind::Answer { .. } => {
                        if self.jobs.len() >= self.config.limits.max_host_calls {
                            return Err(SessionError::Limit("host calls"));
                        }
                        let handle = SessionHandle {
                            shared: self.shared.clone(),
                        };
                        let deadline = self.config.limits.request_timeout;
                        self.jobs.push(Box::pin(async move {
                            let result = timeout(deadline, async {
                                handle.call(&cap, invocation.args).await?.resolve().await
                            })
                            .await
                            .unwrap_or(Err(SessionError::Timeout.into()));
                            Completed {
                                completion: invocation.completion,
                                result,
                            }
                        }));
                        Ok(())
                    },
                }
            },
            Ok(_) => self.complete(Completed {
                completion: invocation.completion,
                result: Err(CallError::message(
                    "promise did not resolve to a callable capability",
                )),
            }),
        }
    }
    fn start_host(
        &mut self,
        object: Arc<dyn HostObject>,
        invocation: Invocation,
    ) -> Result<(), SessionError> {
        if self.jobs.len() >= self.config.limits.max_host_calls {
            return Err(SessionError::Limit("host calls"));
        }
        let context = CallContext {
            authority: self.config.authority.clone(),
            session: SessionHandle {
                shared: self.shared.clone(),
            },
        };
        let deadline = self.config.limits.request_timeout;
        self.jobs.push(Box::pin(async move {
            let result = timeout(deadline, object.call(context, invocation.args))
                .await
                .unwrap_or(Err(SessionError::Timeout.into()));
            Completed {
                completion: invocation.completion,
                result,
            }
        }));
        Ok(())
    }
    fn complete(&mut self, completed: Completed) -> Result<(), SessionError> {
        let max = self.config.limits.max_answers
            + self.config.limits.max_pipeline_calls
            + self.config.limits.max_host_calls
            + 1;
        if self.completions.len() >= max {
            return Err(SessionError::Limit("completion queue"));
        }
        self.completions.push_back(completed);
        if self.draining_completions {
            return Ok(());
        }
        self.draining_completions = true;
        let result = (|| {
            while let Some(completed) = self.completions.pop_front() {
                self.complete_one(completed)?;
            }
            Ok(())
        })();
        self.draining_completions = false;
        result
    }
    fn complete_one(&mut self, completed: Completed) -> Result<(), SessionError> {
        match &completed.result {
            Ok(value) | Err(CallError::Broken(value)) => {
                value.check(&self.config.limits, self.shared.id)?
            },
            Err(CallError::Session(_)) => (),
        }
        let (waiting, listeners) = if let Some(slot) = completed.completion.answer {
            if let Some(answer) = self.answers.get_mut(&slot) {
                answer.result = Some(completed.result.clone());
                let waiting = std::mem::take(&mut answer.waiting);
                let listeners = std::mem::take(&mut answer.listeners);
                if answer.released {
                    self.answers.remove(&slot);
                }
                (waiting, listeners)
            } else {
                (VecDeque::new(), Vec::new())
            }
        } else {
            (VecDeque::new(), Vec::new())
        };
        if let Some(resolver) = completed.completion.resolver {
            self.reply(&resolver, completed.result.clone())?;
        }
        for listener in listeners {
            self.reply(&listener, completed.result.clone())?;
        }
        for invocation in waiting {
            self.route_result(invocation, completed.result.clone(), 0)?;
        }
        Ok(())
    }
    fn reply(
        &mut self,
        resolver: &Capability,
        result: Result<Value, CallError>,
    ) -> Result<(), SessionError> {
        let (method, value) = match result {
            Ok(value) => ("fulfill", value),
            Err(CallError::Broken(reason)) => ("break", reason),
            Err(CallError::Session(_)) => ("break", Value::string("call failed")),
        };
        self.send(resolver, vec![Value::symbol(method), value])
    }
    fn track_promise(&mut self, id: u64, cell: Arc<PromiseCell>) -> Result<(), SessionError> {
        if !self.promises.contains_key(&id) {
            if self.promises.len() >= self.config.limits.max_answers {
                return Err(SessionError::Limit("local promises"));
            }
            self.promises.insert(
                id,
                PromiseEntry {
                    cell,
                    listeners: Vec::new(),
                    waiting: VecDeque::new(),
                    deadline: Instant::now() + self.config.limits.request_timeout,
                },
            );
        }
        Ok(())
    }
    fn listen(&mut self, target: Target, resolver: Capability) -> Result<(), SessionError> {
        match target {
            Target::Answer(slot) => {
                let answer = self
                    .answers
                    .get(&slot)
                    .ok_or(SessionError::UnknownReference)?;
                if let Some(result) = answer.result.clone() {
                    return self.reply(&resolver, result);
                }
                if self.listener_count() >= self.config.limits.max_listeners {
                    return Err(SessionError::Limit("listeners"));
                }
                self.answers
                    .get_mut(&slot)
                    .ok_or(SessionError::UnknownReference)?
                    .listeners
                    .push(resolver);
                Ok(())
            },
            Target::Export(slot) => {
                let entry = self
                    .exports
                    .get(&slot)
                    .ok_or(SessionError::UnknownReference)?;
                let ExportKind::Local(local) = &entry.kind else {
                    return Err(protocol("listen target is not a promise"));
                };
                let LocalKind::Promise(cell) = &local.kind else {
                    return Err(protocol("listen target is not a promise"));
                };
                let id = local.id;
                let cell = cell.clone();
                let result = cell.state.borrow().clone();
                if let Some(result) = result {
                    self.reply(&resolver, result)
                } else {
                    if self.listener_count() >= self.config.limits.max_listeners {
                        return Err(SessionError::Limit("listeners"));
                    }
                    self.track_promise(id, cell)?;
                    self.promises
                        .get_mut(&id)
                        .ok_or(SessionError::UnknownReference)?
                        .listeners
                        .push(resolver);
                    Ok(())
                }
            },
        }
    }
    fn pump_promises(&mut self) -> Result<(), SessionError> {
        let settled = self
            .promises
            .iter()
            .filter_map(|(id, p)| p.cell.state.borrow().clone().map(|result| (*id, result)))
            .collect::<Vec<_>>();
        for (id, result) in settled {
            let entry = self
                .promises
                .get_mut(&id)
                .ok_or(SessionError::UnknownReference)?;
            let listeners = std::mem::take(&mut entry.listeners);
            let waiting = std::mem::take(&mut entry.waiting);
            for listener in listeners {
                self.reply(&listener, result.clone())?;
            }
            for invocation in waiting {
                self.route_result(invocation, result.clone(), 0)?;
            }
        }
        self.promises.retain(|_, p| {
            !(p.listeners.is_empty() && p.waiting.is_empty() && Arc::strong_count(&p.cell) == 1)
        });
        Ok(())
    }
    fn pipeline_count(&self) -> usize {
        self.answers
            .values()
            .map(|a| a.waiting.len())
            .sum::<usize>()
            + self
                .promises
                .values()
                .map(|p| p.waiting.len())
                .sum::<usize>()
    }
    fn listener_count(&self) -> usize {
        self.promises
            .values()
            .map(|p| p.listeners.len())
            .sum::<usize>()
            + self
                .answers
                .values()
                .map(|a| a.listeners.len())
                .sum::<usize>()
    }
    fn check_deadlines(&self) -> Result<(), SessionError> {
        let now = Instant::now();
        if self
            .answers
            .values()
            .any(|a| a.result.is_none() && a.deadline <= now)
            || self.promises.values().any(|p| {
                (!p.listeners.is_empty() || !p.waiting.is_empty())
                    && p.cell.state.borrow().is_none()
                    && p.deadline <= now
            })
            || self.exports.values().any(|e| {
                if let ExportKind::Reply(reply) = &e.kind {
                    reply
                        .upgrade()
                        .is_some_and(|r| r.state.borrow().is_none() && r.deadline <= now)
                } else {
                    false
                }
            })
        {
            return Err(SessionError::Timeout);
        }
        Ok(())
    }
}
fn parse_target(item: &Item) -> Result<Target, SessionError> {
    match item {
        Item::Record(label, fields) if fields.len() == 1 => match label.as_str() {
            "desc:export" => Ok(Target::Export(fields[0].as_slot()?)),
            "desc:answer" => Ok(Target::Answer(fields[0].as_slot()?)),
            _ => Err(SessionError::Unsupported("target descriptor")),
        },
        _ => Err(protocol("invalid target descriptor")),
    }
}

fn check_promise_graph(
    value: &Value,
    target: &Arc<PromiseCell>,
    limits: &Limits,
) -> Result<(), SessionError> {
    fn visit(
        value: &Value,
        target: &Arc<PromiseCell>,
        limits: &Limits,
        depth: usize,
        nodes: &mut usize,
        path: &mut HashSet<usize>,
    ) -> Result<(), SessionError> {
        *nodes += 1;
        if depth > limits.max_depth || *nodes > limits.max_value_nodes {
            return Err(SessionError::Limit("promise graph"));
        }
        match value {
            Value::Cap(cap) => {
                if let CapKind::Local(local) = &cap.0.kind
                    && let LocalKind::Promise(cell) = &local.kind
                {
                    let id = Arc::as_ptr(cell) as usize;
                    if Arc::ptr_eq(cell, target) || !path.insert(id) {
                        return Err(protocol("cyclic promise resolution"));
                    }
                    let state = cell.state.borrow().clone();
                    if let Some(Ok(value) | Err(CallError::Broken(value))) = state {
                        visit(&value, target, limits, depth + 1, nodes, path)?;
                    }
                    path.remove(&id);
                }
            },
            Value::List(xs) | Value::Record { fields: xs, .. } => {
                for value in xs {
                    visit(value, target, limits, depth + 1, nodes, path)?;
                }
            },
            Value::Map(xs) => {
                for (k, v) in xs {
                    visit(k, target, limits, depth + 1, nodes, path)?;
                    visit(v, target, limits, depth + 1, nodes, path)?;
                }
            },
            _ => (),
        }
        Ok(())
    }
    visit(value, target, limits, 0, &mut 0, &mut HashSet::new())
}
