// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use futures_util::FutureExt;
use murm_ocapn::{
    Bootstrap, BoxFuture, CallContext, CallError, Capability, HostObject, Limits, PeerLocator,
    SessionAuthority, SessionConfig, SessionError, SessionHandle, Value, attach,
};
use std::{
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, duplex, split},
    sync::Notify,
    task::JoinHandle,
    time::timeout,
};

#[derive(Clone)]
struct Subject(&'static str, u64);
struct Echo;
impl HostObject for Echo {
    fn call<'a>(
        &'a self,
        context: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            let subject = context
                .authority::<Subject>()
                .ok_or_else(|| CallError::message("missing host authority"))?;
            if subject.0 != "server" || subject.1 != 7 {
                return Err(CallError::message("incorrect immutable host authority"));
            }
            Ok(Value::List(args))
        })
    }
}
struct Factory;
impl HostObject for Factory {
    fn call<'a>(
        &'a self,
        context: CallContext,
        _: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move { Ok(Value::Cap(context.session().export(Arc::new(Echo)).await?)) })
    }
}
struct Callback;
impl HostObject for Callback {
    fn call<'a>(
        &'a self,
        context: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            let [Value::Cap(sink)] = args.as_slice() else {
                return Err(CallError::message("callback required"));
            };
            context
                .session()
                .call(sink, vec![Value::string("event")])
                .await?
                .resolve()
                .await
        })
    }
}
struct Sink {
    seen: Arc<Mutex<Vec<String>>>,
    notify: Arc<Notify>,
}
impl HostObject for Sink {
    fn call<'a>(
        &'a self,
        context: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            let subject = context
                .authority::<Subject>()
                .ok_or_else(|| CallError::message("authority"))?;
            if subject.0 != "client" {
                return Err(CallError::message("wrong callback authority"));
            }
            let [Value::String(event)] = args.as_slice() else {
                return Err(CallError::message("event"));
            };
            self.seen.lock().unwrap().push(event.clone());
            self.notify.notify_one();
            Ok(Value::string("received"))
        })
    }
}
struct Never;
impl HostObject for Never {
    fn call<'a>(
        &'a self,
        _: CallContext,
        _: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(std::future::pending())
    }
}
struct PromiseHost;
impl HostObject for PromiseHost {
    fn call<'a>(
        &'a self,
        context: CallContext,
        _: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            let (promise, resolver) = context.session().promise_pair().await?;
            let object = context
                .session()
                .export(Arc::new(Resolve(resolver)))
                .await?;
            Ok(Value::List(vec![
                Value::Cap(promise.capability()),
                Value::Cap(object),
            ]))
        })
    }
}
struct Resolve(murm_ocapn::Resolver);
impl HostObject for Resolve {
    fn call<'a>(
        &'a self,
        _: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            let [value] = args.as_slice() else {
                return Err(CallError::message("value"));
            };
            self.0.fulfill(value.clone()).await?;
            Ok(Value::Bool(true))
        })
    }
}

fn registry() -> Bootstrap {
    let mut bootstrap = Bootstrap::new();
    for (swiss, object) in [
        (b"echo".as_slice(), Arc::new(Echo) as Arc<dyn HostObject>),
        (b"factory", Arc::new(Factory)),
        (b"callback", Arc::new(Callback)),
        (b"never", Arc::new(Never)),
        (b"promise", Arc::new(PromiseHost)),
    ] {
        bootstrap.register(swiss, object).unwrap();
    }
    bootstrap
}
struct Pair {
    client: SessionHandle,
    server: SessionHandle,
    tasks: Vec<JoinHandle<Result<(), SessionError>>>,
}
impl Drop for Pair {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
async fn pair() -> Pair {
    pair_with_limits(Limits::default()).await
}
async fn pair_with_limits(server_limits: Limits) -> Pair {
    pair_configured(server_limits, registry()).await
}
async fn pair_configured(server_limits: Limits, bootstrap: Bootstrap) -> Pair {
    let (client_io, server_io) = duplex(65536);
    let (cr, cw) = split(client_io);
    let (sr, sw) = split(server_io);
    let mut c = SessionConfig::new(
        PeerLocator::new("test-admitted", "client"),
        SessionAuthority::new(Subject("client", 3)),
    );
    c.expected_remote = Some(PeerLocator::new("test-admitted", "server"));
    let mut s = SessionConfig::new(
        PeerLocator::new("test-admitted", "server"),
        SessionAuthority::new(Subject("server", 7)),
    );
    s.expected_remote = Some(PeerLocator::new("test-admitted", "client"));
    s.limits = server_limits;
    let (client, cd) = attach(cr, cw, c, Bootstrap::new()).unwrap();
    let (server, sd) = attach(sr, sw, s, bootstrap).unwrap();
    let tasks = vec![tokio::spawn(cd.run()), tokio::spawn(sd.run())];
    timeout(Duration::from_secs(2), client.wait_ready())
        .await
        .unwrap()
        .unwrap();
    Pair {
        client,
        server,
        tasks,
    }
}
async fn fetch(handle: &SessionHandle, swiss: &[u8]) -> Capability {
    let value = handle.fetch(swiss).await.unwrap().resolve().await.unwrap();
    let Value::Cap(cap) = value else {
        panic!("expected cap")
    };
    cap
}

#[tokio::test]
async fn pipeline_callback_authority_and_disconnect() {
    let pair = pair().await;
    let factory = pair.client.fetch(b"factory").await.unwrap();
    let echo = pair.client.call(&factory.target(), vec![]).await.unwrap();
    let answer = pair
        .client
        .call(
            &echo.target(),
            vec![
                Value::Integer(42),
                Value::string("caller cannot replace server subject"),
            ],
        )
        .await
        .unwrap();
    let Value::List(values) = timeout(Duration::from_secs(2), answer.resolve())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected list")
    };
    assert!(matches!(values[0], Value::Integer(42)));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let notify = Arc::new(Notify::new());
    let sink = pair
        .client
        .export(Arc::new(Sink {
            seen: seen.clone(),
            notify: notify.clone(),
        }))
        .await
        .unwrap();
    let callback = fetch(&pair.client, b"callback").await;
    let result = pair
        .client
        .call(&callback, vec![Value::Cap(sink)])
        .await
        .unwrap()
        .resolve()
        .await
        .unwrap();
    assert!(matches!(result,Value::String(ref s)if s=="received"));
    assert_eq!(seen.lock().unwrap().as_slice(), ["event"]);
    let never = fetch(&pair.client, b"never").await;
    let pending = pair.client.call(&never, vec![]).await.unwrap();
    pair.server.close("revoked").await.unwrap();
    timeout(Duration::from_secs(2), pair.client.closed())
        .await
        .unwrap();
    assert!(pending.resolve().await.is_err());
}

#[tokio::test]
async fn promise_listener_and_foreign_reference_rejection() {
    let other = pair().await;
    let pair = pair().await;
    let promise_factory = fetch(&pair.client, b"promise").await;
    let Value::List(mut values) = pair
        .client
        .call(&promise_factory, vec![])
        .await
        .unwrap()
        .resolve()
        .await
        .unwrap()
    else {
        panic!()
    };
    let Value::Cap(resolver) = values.pop().unwrap() else {
        panic!()
    };
    let Value::Cap(promise) = values.pop().unwrap() else {
        panic!()
    };
    let listening = pair.client.listen(&promise).await.unwrap();
    pair.client
        .send(&resolver, vec![Value::string("settled")])
        .await
        .unwrap();
    assert!(
        matches!(timeout(Duration::from_secs(2),listening.resolve()).await.unwrap().unwrap(),Value::String(ref s)if s=="settled")
    );
    let late = pair.client.listen(&promise).await.unwrap();
    assert!(matches!(late.resolve().await.unwrap(),Value::String(ref s)if s=="settled"));
    let foreign = fetch(&other.client, b"echo").await;
    assert_eq!(
        pair.client.try_send(&foreign, vec![]).unwrap_err(),
        SessionError::ForeignCapability
    );
    let echo = fetch(&pair.client, b"echo").await;
    assert_eq!(
        pair.client
            .try_send(&echo, vec![Value::List(vec![Value::Cap(foreign)])])
            .unwrap_err(),
        SessionError::ForeignCapability
    );
}

#[tokio::test]
async fn queue_limits_cancel_and_oversized_wire_are_terminal() {
    let (io, _peer) = duplex(1024);
    let (r, w) = split(io);
    let mut config =
        SessionConfig::new(PeerLocator::new("test", "local"), SessionAuthority::new(()));
    config.limits.max_command_queue = 1;
    let (handle, driver) = attach(r, w, config, Bootstrap::new()).unwrap();
    let first = handle.fetch(b"unused").await.unwrap();
    assert!(matches!(
        handle.try_send(&first.target(), vec![]),
        Err(SessionError::Limit("command queue"))
    ));
    drop(driver);
    assert!(handle.is_closed());
    assert!(first.resolve().await.is_err());
    let (io, mut peer) = duplex(1024);
    let (r, w) = split(io);
    let mut config =
        SessionConfig::new(PeerLocator::new("test", "local"), SessionAuthority::new(()));
    config.limits.max_frame_bytes = 512;
    let (handle, driver) = attach(r, w, config, Bootstrap::new()).unwrap();
    let task = tokio::spawn(driver.run());
    peer.write_all(b"999999999999:").await.unwrap();
    timeout(Duration::from_secs(2), handle.closed())
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(SessionError::Limit("frame bytes"))
    ));
}

#[tokio::test]
async fn cancellation_breaks_pending_promises() {
    let pair = pair().await;
    let never = fetch(&pair.client, b"never").await;
    let pending = pair.client.call(&never, vec![]).await.unwrap();
    pair.tasks[0].abort();
    timeout(Duration::from_secs(2), pair.client.closed())
        .await
        .unwrap();
    assert!(pending.resolve().await.is_err());
}

async fn read_raw(peer: &mut DuplexStream) -> Vec<u8> {
    let mut len = String::new();
    loop {
        let b = peer.read_u8().await.unwrap();
        if b == b':' {
            break;
        }
        len.push(char::from(b));
    }
    let mut bytes = vec![0; len.parse::<usize>().unwrap()];
    peer.read_exact(&mut bytes).await.unwrap();
    bytes
}
async fn write_raw(peer: &mut DuplexStream, bytes: &[u8]) {
    peer.write_all(bytes.len().to_string().as_bytes())
        .await
        .unwrap();
    peer.write_all(b":").await.unwrap();
    peer.write_all(bytes).await.unwrap();
}
// These tests exercise protocol rejection after a valid signature, without
// claiming transport authentication. The loopback peer reflects the fresh
// test handshake; production service hosts bind expected_remote themselves.
async fn raw_peer() -> (
    SessionHandle,
    DuplexStream,
    Vec<u8>,
    JoinHandle<Result<(), SessionError>>,
) {
    let (io, mut peer) = duplex(65536);
    let (r, w) = split(io);
    let (handle, driver) = attach(
        r,
        w,
        SessionConfig::new(
            PeerLocator::new("test-only", "raw"),
            SessionAuthority::new(()),
        ),
        Bootstrap::new(),
    )
    .unwrap();
    let task = tokio::spawn(driver.run());
    let hello = read_raw(&mut peer).await;
    write_raw(&mut peer, &hello).await;
    handle.wait_ready().await.unwrap();
    (handle, peer, hello, task)
}
fn sym(s: &str) -> String {
    format!("{}'{s}", s.len())
}
fn record(label: &str, fields: &str) -> Vec<u8> {
    format!("<{}{fields}>", sym(label)).into_bytes()
}

#[tokio::test]
async fn unsupported_and_malformed_peer_messages_fail_closed() {
    for bytes in [
        record("op:deliver", &format!("<{}999+>[]ff", sym("desc:export"))),
        record(
            "op:deliver",
            &format!(
                "<{}0+>[<{}>]ff",
                sym("desc:export"),
                sym("desc:handoff-give")
            ),
        ),
        record("op:deliver-only", ""),
        b"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]".to_vec(),
        b"<1\"x>".to_vec(),
    ] {
        let (handle, mut peer, _, task) = raw_peer().await;
        write_raw(&mut peer, &bytes).await;
        timeout(Duration::from_secs(2), handle.closed())
            .await
            .unwrap();
        assert!(task.await.unwrap().is_err());
    }
    let (handle, mut peer, hello, task) = raw_peer().await;
    write_raw(&mut peer, &hello).await;
    timeout(Duration::from_secs(2), handle.closed())
        .await
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(SessionError::Unsupported(
            "duplicate session or crossed hellos"
        ))
    ));
}

#[tokio::test]
async fn promise_cycles_are_rejected_and_resolution_is_one_shot() {
    let pair = pair().await;
    let (p1, r1) = pair.client.promise_pair().await.unwrap();
    let (p2, r2) = pair.client.promise_pair().await.unwrap();
    assert!(r1.fulfill(Value::Cap(p1.capability())).await.is_err());
    assert!(r1.break_with(Value::Cap(p1.capability())).await.is_err());
    r1.fulfill(Value::Cap(p2.capability())).await.unwrap();
    assert!(r2.fulfill(Value::Cap(p1.capability())).await.is_err());
    r2.fulfill(Value::string("settled")).await.unwrap();
    assert!(r2.fulfill(Value::string("twice")).await.is_err());
}

#[tokio::test]
async fn slow_consumer_writer_has_a_deadline_without_pending_requests() {
    let (io, mut peer) = duplex(1024);
    let (r, w) = split(io);
    let mut config = SessionConfig::new(
        PeerLocator::new("test-only", "slow"),
        SessionAuthority::new(()),
    );
    config.limits.request_timeout = Duration::from_millis(50);
    let (handle, driver) = attach(r, w, config, Bootstrap::new()).unwrap();
    let task = tokio::spawn(driver.run());
    let hello = read_raw(&mut peer).await;
    write_raw(&mut peer, &hello).await;
    handle.wait_ready().await.unwrap();
    handle
        .send(&handle.bootstrap(), vec![Value::Bytes(vec![7; 32768])])
        .await
        .unwrap();
    // No reply was requested. Only the writer deadline (not a response
    // deadline) can produce this timeout while the peer stops reading.
    timeout(Duration::from_secs(2), handle.closed())
        .await
        .unwrap();
    assert!(matches!(task.await.unwrap(), Err(SessionError::Timeout)));
}

#[tokio::test]
async fn host_listener_pipeline_and_reference_limits_fail_closed() {
    for lane in ["host", "listener", "pipeline", "imports", "objects"] {
        let mut limits = Limits::default();
        match lane {
            "host" => limits.max_host_calls = 1,
            "listener" => limits.max_listeners = 1,
            "pipeline" => limits.max_pipeline_calls = 1,
            "imports" => limits.max_imports = 1,
            "objects" => limits.max_exports = 1,
            _ => unreachable!(),
        }
        let mut pair = pair_with_limits(limits).await;
        match lane {
            "host" | "pipeline" => {
                let never = fetch(&pair.client, b"never").await;
                let pending = pair.client.call(&never, vec![]).await.unwrap();
                let target = if lane == "host" {
                    never
                } else {
                    pending.target()
                };
                let _second = pair.client.call(&target, vec![]).await.unwrap();
                if lane == "pipeline" {
                    let _third = pair.client.call(&target, vec![]).await.unwrap();
                    timeout(Duration::from_secs(2), pair.server.closed())
                        .await
                        .unwrap();
                }
            },
            "listener" => {
                let factory = fetch(&pair.client, b"promise").await;
                let Value::List(values) = pair
                    .client
                    .call(&factory, vec![])
                    .await
                    .unwrap()
                    .resolve()
                    .await
                    .unwrap()
                else {
                    panic!()
                };
                let Value::Cap(promise) = &values[0] else {
                    panic!()
                };
                let _first = pair.client.listen(promise).await.unwrap();
                let _second = pair.client.listen(promise).await.unwrap();
                timeout(Duration::from_secs(2), pair.server.closed())
                    .await
                    .unwrap();
            },
            "imports" => {
                let echo = fetch(&pair.client, b"echo").await;
                let a = pair.client.export(Arc::new(Never)).await.unwrap();
                let b = pair.client.export(Arc::new(Never)).await.unwrap();
                pair.client
                    .send(&echo, vec![Value::Cap(a), Value::Cap(b)])
                    .await
                    .unwrap();
            },
            "objects" => {
                let _echo = fetch(&pair.client, b"echo").await;
                let request = pair.client.fetch(b"factory").await.unwrap();
                assert!(request.resolve().await.is_err());
            },
            _ => unreachable!(),
        }
        timeout(Duration::from_secs(2), pair.server.closed())
            .await
            .unwrap();
        assert!(pair.server.is_closed(), "{lane}");
        let result = timeout(Duration::from_secs(2), &mut pair.tasks[1])
            .await
            .unwrap()
            .unwrap();
        let expected = match lane {
            "host" => "host calls",
            "listener" => "listeners",
            "pipeline" => "pipeline calls",
            "imports" => "imports",
            "objects" => "local objects",
            _ => unreachable!(),
        };
        assert_eq!(result, Err(SessionError::Limit(expected)), "{lane}");
    }
}

#[tokio::test]
async fn unanswered_request_expires_after_its_frame_was_delivered() {
    let (io, mut peer) = duplex(4096);
    let (r, w) = split(io);
    let mut config = SessionConfig::new(
        PeerLocator::new("test-only", "unanswered"),
        SessionAuthority::new(()),
    );
    config.limits.request_timeout = Duration::from_millis(50);
    let (handle, driver) = attach(r, w, config, Bootstrap::new()).unwrap();
    let task = tokio::spawn(driver.run());
    let hello = read_raw(&mut peer).await;
    write_raw(&mut peer, &hello).await;
    handle.wait_ready().await.unwrap();
    let pending = handle.fetch(b"unanswered").await.unwrap();
    let delivered = read_raw(&mut peer).await;
    assert!(delivered.starts_with(b"<10'op:deliver"));
    assert!(matches!(
        pending.resolve().await,
        Err(CallError::Session(SessionError::Timeout))
    ));
    timeout(Duration::from_secs(2), handle.closed())
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(SessionError::Timeout));
}

#[tokio::test]
async fn idle_peer_expires() {
    let limits = Limits {
        idle_timeout: Duration::from_millis(50),
        ..Limits::default()
    };
    let pair = pair_with_limits(limits).await;
    timeout(Duration::from_secs(2), pair.server.closed())
        .await
        .unwrap();
    timeout(Duration::from_secs(2), pair.client.closed())
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_cross_resolution_cannot_form_a_retained_cycle() {
    let pair = pair().await;
    for _ in 0..32 {
        let (p1, r1) = pair.client.promise_pair().await.unwrap();
        let (p2, r2) = pair.client.promise_pair().await.unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let successes = std::thread::scope(|scope| {
            let a = barrier.clone();
            let p2 = p2.capability();
            let first = scope.spawn(move || {
                a.wait();
                r1.fulfill(Value::Cap(p2)).now_or_never().unwrap().is_ok()
            });
            let b = barrier;
            let p1 = p1.capability();
            let second = scope.spawn(move || {
                b.wait();
                r2.fulfill(Value::Cap(p1)).now_or_never().unwrap().is_ok()
            });
            usize::from(first.join().unwrap()) + usize::from(second.join().unwrap())
        });
        assert_eq!(successes, 1);
    }
    assert!(!pair.client.is_closed());
}

struct TrackedFactory {
    drops: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}
struct Tracked {
    drops: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}
impl Drop for Tracked {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_one();
    }
}
impl HostObject for Tracked {
    fn call<'a>(
        &'a self,
        _: CallContext,
        _: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async { Ok(Value::Bool(true)) })
    }
}
impl HostObject for TrackedFactory {
    fn call<'a>(
        &'a self,
        ctx: CallContext,
        _: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            Ok(Value::Cap(
                ctx.session()
                    .export(Arc::new(Tracked {
                        drops: self.drops.clone(),
                        notify: self.notify.clone(),
                    }))
                    .await?,
            ))
        })
    }
}
#[tokio::test]
async fn remote_reference_and_answer_release_collect_host_object_before_close() {
    let drops = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(Notify::new());
    let mut bootstrap = Bootstrap::new();
    bootstrap
        .register(
            b"tracked",
            Arc::new(TrackedFactory {
                drops: drops.clone(),
                notify: notify.clone(),
            }),
        )
        .unwrap();
    let pair = pair_configured(Limits::default(), bootstrap).await;
    let factory = fetch(&pair.client, b"tracked").await;
    let pending = pair.client.call(&factory, vec![]).await.unwrap();
    let child = pending.resolve().await.unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(pending);
    drop(child);
    timeout(Duration::from_secs(2), notify.notified())
        .await
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(!pair.server.is_closed());
}
