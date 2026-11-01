// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Loopback-only independent-suite fixtures. Not part of production bootstrap.
use murm_ocapn::{
    Bootstrap, BoxFuture, CallContext, CallError, HostObject, PeerLocator, Resolver,
    SessionAuthority, SessionConfig, Value, attach,
};
use std::{path::PathBuf, sync::Arc};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet};

struct TestOnlyAuthority;
enum Fixture {
    Builder,
    Factory,
    Car(String),
    Echo,
    Greeter,
    PromiseMaker,
    Resolver(Resolver),
}
impl HostObject for Fixture {
    fn call<'a>(
        &'a self,
        context: CallContext,
        args: Vec<Value>,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            if context.authority::<TestOnlyAuthority>().is_none() {
                return Err(CallError::message("test-only authority required"));
            }
            match self {
                Fixture::Builder if args.is_empty() => Ok(Value::Cap(
                    context.session().export(Arc::new(Fixture::Factory)).await?,
                )),
                Fixture::Factory => {
                    let [Value::List(spec)] = args.as_slice() else {
                        return Err(CallError::message("expected one car specification"));
                    };
                    let [Value::Symbol(color), Value::Symbol(model)] = spec.as_slice() else {
                        return Err(CallError::message("expected color and model symbols"));
                    };
                    Ok(Value::Cap(
                        context
                            .session()
                            .export(Arc::new(Fixture::Car(format!(
                                "Vroom! I am a {color} {model} car!"
                            ))))
                            .await?,
                    ))
                },
                Fixture::Car(message) if args.is_empty() => Ok(Value::String(message.clone())),
                Fixture::Echo => Ok(Value::List(args)),
                Fixture::Greeter => {
                    let [Value::Cap(target)] = args.as_slice() else {
                        return Err(CallError::message("expected greeting callback"));
                    };
                    let reply = context
                        .session()
                        .call(target, vec![Value::string("Hello")])
                        .await?;
                    reply.resolve().await?;
                    drop(reply);
                    Ok(Value::Bool(false))
                },
                Fixture::PromiseMaker if args.is_empty() => {
                    let (promise, resolver) = context.session().promise_pair().await?;
                    let resolver = context
                        .session()
                        .export(Arc::new(Fixture::Resolver(resolver)))
                        .await?;
                    Ok(Value::List(vec![
                        Value::Cap(promise.capability()),
                        Value::Cap(resolver),
                    ]))
                },
                Fixture::Resolver(resolver) => {
                    let [Value::Symbol(method), value] = args.as_slice() else {
                        return Err(CallError::message("expected resolver method and value"));
                    };
                    match method.as_str() {
                        "fulfill" => resolver.fulfill(value.clone()).await?,
                        "break" => resolver.break_with(value.clone()).await?,
                        _ => return Err(CallError::message("unsupported resolver method")),
                    }
                    Ok(Value::Bool(false))
                },
                _ => Err(CallError::message("invalid fixture invocation")),
            }
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: suite_server LOCATOR_FILE")?,
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let mut local = PeerLocator::new("tcp-testing-only", "murm-ocapn-suite-fixture");
    local.hints = vec![
        ("host".into(), "127.0.0.1".into()),
        ("port".into(), port.to_string()),
    ];
    let locator =
        format!("ocapn://murm-ocapn-suite-fixture.tcp-testing-only?host=127.0.0.1&port={port}");
    let mut bootstrap = Bootstrap::new();
    for (swiss, fixture) in [
        ("JadQ0++RzsD4M+40uLxTWVaVqM10DcBJ", Fixture::Builder),
        ("IO58l1laTyhcrgDKbEzFOO32MDd6zE5w", Fixture::Echo),
        ("VMDDd1voKWarCe2GvgLbxbVFysNzRPzx", Fixture::Greeter),
        ("IokCxYmMj04nos2JN1TDoY1bT8dXh6Lr", Fixture::PromiseMaker),
    ] {
        bootstrap.register(swiss.as_bytes(), Arc::new(fixture))?;
    }
    std::fs::write(path, format!("{locator}\n"))?;
    println!("{locator}");
    let permits = Arc::new(Semaphore::new(32));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream,_) = accepted?;
                let Ok(permit) = permits.clone().try_acquire_owned() else { drop(stream); continue; };
                let (reader,writer) = stream.into_split();
                let config = SessionConfig::new(local.clone(),SessionAuthority::new(TestOnlyAuthority));
                let (_,driver) = attach(reader,writer,config,bootstrap.clone())?;
                tasks.spawn(async move { let _permit=permit; if let Err(error)=driver.run().await { eprintln!("fixture session ended: {error}"); } });
            }
            _ = tasks.join_next(), if !tasks.is_empty() => (),
        }
    }
}
