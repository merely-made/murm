// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! A bounded fixture service, shared by two native processes on each carrier.

mod carriers;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use carriers::{Contact, Owner};
use insigne::delegation::{
    CapabilityScope, DelegationCertificate, DelegationParent, SignedDelegationCertificate,
};
use notochord::{
    LocalNetworkPolicy, NetworkId, ProfileRef, ProofBinding, RequestedAction, RevocationLedger,
    SecureIdentityPolicy, ServiceAccess, ServiceRule, SessionHello, TrafficClass, TrustedRoot,
    admit_session_secure, initiate_session, read_frame, write_frame,
};
use personae::delegation::Issue;
use personae::{Ed25519Keypair, IdentityProvider, InMemoryProvider};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use transport::security::{
    PreparedStream, ServiceSecurityPolicy, SessionSecurityFacts, SetupError, SetupGate, SetupGuard,
    SetupLimits,
};
use transport::{Alpn, PeerID};

const PROTOCOL: &str = "mere/secure-session-probe/v1";
const SERVICE: &str = "/services/secure-session-probe";
const DOMAIN: &str = "mere.secure-session-probe";
const NETWORK: NetworkId = NetworkId([51; 32]);
const ROOT: [u8; 32] = [52; 32];
const NOW: u64 = 50;
const SETUP_TIMEOUT: Duration = Duration::from_secs(3);
const APPLICATION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_APPLICATION_FRAME: u32 = 256;

fn server_key() -> Ed25519Keypair {
    Ed25519Keypair::from_seed([1; 32])
}
fn client_key() -> Ed25519Keypair {
    Ed25519Keypair::from_seed([2; 32])
}
fn client_identity() -> InMemoryProvider {
    InMemoryProvider::from_seed([2; 32])
}
fn profile() -> ProfileRef {
    ProfileRef {
        id: "murm.n3-fixture".into(),
        revision: 1,
    }
}
fn limits() -> SetupLimits {
    SetupLimits {
        timeout: SETUP_TIMEOUT,
        max_read_bytes: 16 * 1024,
        max_write_bytes: 16 * 1024,
        max_frame_bytes: 4096,
        max_frames: 16,
        max_io_operations: 1024,
    }
}
fn owner_policy() -> LocalNetworkPolicy {
    let mut policy = LocalNetworkPolicy::closed(NETWORK);
    policy.trusted_roots.push(TrustedRoot {
        authority: ROOT,
        issuer: server_key().public_key().to_bytes(),
    });
    policy.accepted_profiles.push(profile());
    policy.services.insert(
        SERVICE.into(),
        ServiceRule::new(ServiceAccess::MemberOnly, DOMAIN, ["echo"], false, Some(1)),
    );
    policy
}
fn grant() -> SignedDelegationCertificate {
    let issuer = InMemoryProvider::from_seed([1; 32]);
    SignedDelegationCertificate::issue(
        &issuer,
        DelegationCertificate::new(
            DelegationParent::Root(ROOT),
            issuer.master_public_key().to_bytes(),
            client_identity().master_public_key().to_bytes(),
            CapabilityScope {
                domain: DOMAIN.into(),
                resource: NETWORK.0.to_vec(),
                path_prefix: SERVICE.into(),
                actions: ["echo".to_owned()].into_iter().collect(),
            },
            0,
            0,
            Some(100),
            0,
            [53; 32],
        ),
    )
    .expect("fixed fixture grant")
}

#[derive(Default, Serialize, Deserialize)]
struct Step {
    outcome: String,
    detail: Option<String>,
    mutation: Option<String>,
    channel_binding: Option<String>,
    session_id: Option<String>,
    carrier_peer_present: bool,
    shared_link_present: bool,
    pending_after: usize,
}

#[derive(Default, Serialize, Deserialize)]
struct Report {
    pid: u32,
    role: String,
    carrier: String,
    security_mode: String,
    scenario: String,
    steps: Vec<Step>,
    handled_requests: Vec<u8>,
    completed_requests: Vec<u8>,
    pending_after: usize,
    peak_pending: usize,
    timed_out: usize,
    cancelled: usize,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

type Prepared = (carriers::Stream, SessionSecurityFacts, SetupGuard);

fn boxed<S: carriers::Io + 'static>(
    prepared: Result<(PreparedStream<S>, SessionSecurityFacts, SetupGuard), SetupError>,
) -> Result<Prepared, SetupError> {
    prepared.map(|(stream, facts, guard)| (Box::new(stream) as carriers::Stream, facts, guard))
}

fn security_policy(carrier: &str, protocol: Alpn, peer: PeerID) -> ServiceSecurityPolicy {
    if carrier == "iroh-native" {
        ServiceSecurityPolicy::native(protocol, Some(peer), limits())
    } else {
        ServiceSecurityPolicy::noise(protocol, Some(peer), limits())
    }
}

async fn prepare_client(
    owner: &Owner,
    contact: &Contact,
    protocol: &Alpn,
    security: &ServiceSecurityPolicy,
    key: &Ed25519Keypair,
    gate: &SetupGate,
) -> Result<(ProofBinding, Arc<AtomicBool>, Result<Prepared, SetupError>)> {
    let guard = gate.reserve(security)?;
    if contact.carrier == "iroh-native" {
        let outbound = guard.run(owner.connect_native(contact, protocol)).await??;
        let binding = outbound.proof_binding();
        Ok((
            binding,
            Arc::new(AtomicBool::new(false)),
            boxed(guard.native(outbound.stream)),
        ))
    } else {
        let outgoing = guard.run(owner.connect(contact, protocol)).await??;
        let prepared = boxed(guard.noise_initiator(key, outgoing.stream).await);
        Ok((outgoing.binding, outgoing.flip_next_ciphertext, prepared))
    }
}

async fn server(carrier: &str, scenario: &str, contact_path: &Path, output: &Path) -> Result<()> {
    let protocol = Alpn::new(PROTOCOL);
    let (owner, contact) = Owner::listen(carrier, &protocol).await?;
    write_json(contact_path, &contact)?;
    let gate = SetupGate::new(1);
    let security = security_policy(
        carrier,
        protocol.clone(),
        PeerID::from_public_key(client_key().public_key()),
    );
    let policy = owner_policy();
    let ledger = RevocationLedger::new();
    let mut report = Report {
        pid: std::process::id(),
        role: "server".into(),
        carrier: carrier.into(),
        security_mode: if carrier == "iroh-native" {
            "native"
        } else {
            "noise"
        }
        .into(),
        scenario: scenario.into(),
        ..Default::default()
    };
    for _ in 0..if scenario == "reconnect" { 3 } else { 1 } {
        let (carrier_facts, prepared) = if carrier == "iroh-native" {
            let accepted = tokio::time::timeout(
                if scenario == "wrong_server" {
                    SETUP_TIMEOUT
                } else {
                    Duration::from_secs(30)
                },
                owner.accept_native(&protocol),
            )
            .await;
            if accepted.is_err() && scenario == "wrong_server" {
                // A native client checks its expected peer before writing any
                // stream data. QUIC's lazy stream therefore remains undisclosed.
                report.steps.push(Step {
                    outcome: "no_stream".into(),
                    ..Default::default()
                });
                continue;
            }
            let (stream, facts) = accepted.context("native carrier accept deadline")??;
            (facts, boxed(gate.reserve(&security)?.native(stream)))
        } else {
            let incoming = tokio::time::timeout(Duration::from_secs(30), owner.accept(&protocol))
                .await
                .context("carrier accept deadline")??;
            let prepared = boxed(
                gate.noise_responder(&server_key(), incoming.stream, &security)
                    .await,
            );
            (incoming.facts, prepared)
        };
        let mut step = Step {
            carrier_peer_present: carrier_facts.authenticated_initiator.is_some(),
            shared_link_present: carrier_facts.ingress.shared_link.is_some(),
            ..Default::default()
        };
        match prepared {
            Err(error) => {
                step.outcome = "setup_refused".into();
                step.detail = Some(format!("{error:?}"));
            }
            Ok((stream, facts, guard)) => {
                let channel = facts
                    .secure_channel_facts()
                    .context("secure facts absent")?;
                step.channel_binding = Some(hex(&channel.handshake_hash));
                let admitted = guard
                    .run(admit_session_secure(
                        stream,
                        &policy,
                        &ledger,
                        &carrier_facts,
                        &channel,
                        SecureIdentityPolicy::MatchCarrierWhenPresent,
                        NOW,
                        0,
                    ))
                    .await;
                match admitted {
                    Err(error) => {
                        step.outcome = "admission_bounded".into();
                        step.detail = Some(format!("{error:?}"));
                    }
                    Ok(Err(error)) => {
                        step.outcome = "admission_io".into();
                        step.detail = Some(error.to_string());
                    }
                    Ok(Ok(Err(reason))) => {
                        step.outcome = "admission_refused".into();
                        step.detail = Some(reason.to_string());
                        guard.refused();
                    }
                    Ok(Ok(Ok(mut admitted))) => {
                        step.session_id = Some(hex(&admitted.session.principal.session_id));
                        ensure!(
                            admitted.session.facts == carrier_facts,
                            "carrier facts changed"
                        );
                        guard.admitted()?;
                        let application = tokio::time::timeout(APPLICATION_TIMEOUT, async {
                            let request =
                                read_frame(&mut admitted.session.stream, MAX_APPLICATION_FRAME)
                                    .await?;
                            ensure!(request.len() == 1, "one bounded request id required");
                            report.handled_requests.push(request[0]);
                            write_frame(
                                &mut admitted.session.stream,
                                &request,
                                MAX_APPLICATION_FRAME,
                            )
                            .await?;
                            let ack =
                                read_frame(&mut admitted.session.stream, MAX_APPLICATION_FRAME)
                                    .await
                                    .context("read reader acknowledgement")?;
                            ensure!(ack == b"done", "reader acknowledgement required");
                            admitted
                                .session
                                .stream
                                .shutdown()
                                .await
                                .context("finish service stream")?;
                            report.completed_requests.push(request[0]);
                            Ok::<_, anyhow::Error>(())
                        })
                        .await;
                        match application {
                            Ok(Ok(())) => step.outcome = "responded".into(),
                            Ok(Err(error)) => {
                                step.outcome = "application_refused".into();
                                step.detail = Some(error.to_string());
                            }
                            Err(_) => step.outcome = "application_timeout".into(),
                        }
                    }
                }
            }
        }
        step.pending_after = gate.pending();
        ensure!(step.pending_after == 0, "setup lease leaked");
        report.steps.push(step);
    }
    report.pending_after = gate.pending();
    let stats = gate.stats();
    report.peak_pending = stats.peak_pending;
    report.timed_out = stats.timed_out;
    report.cancelled = stats.cancelled;
    write_json(output, &report)?;
    owner.close().await?;
    Ok(())
}

async fn client(scenario: &str, contact_path: &Path, output: &Path) -> Result<()> {
    let contact: Contact = serde_json::from_slice(&std::fs::read(contact_path)?)?;
    let protocol = Alpn::new(PROTOCOL);
    let owner = Owner::connector(&contact, &protocol, scenario == "wrong_client").await?;
    let gate = SetupGate::new(1);
    let expected = if scenario == "wrong_server" {
        Ed25519Keypair::from_seed([3; 32]).public_key().to_bytes()
    } else {
        contact.publisher
    };
    let security = security_policy(
        &contact.carrier,
        protocol.clone(),
        PeerID::from_bytes(&expected).unwrap(),
    );
    let mut report = Report {
        pid: std::process::id(),
        role: "client".into(),
        carrier: contact.carrier.clone(),
        security_mode: if contact.carrier == "iroh-native" {
            "native"
        } else {
            "noise"
        }
        .into(),
        scenario: scenario.into(),
        ..Default::default()
    };
    let mut prior_hello: Option<SessionHello> = None;
    let mut retained_streams: Vec<carriers::Stream> = Vec::new();
    for index in 0..if scenario == "reconnect" { 3 } else { 1 } {
        if scenario == "cancel" || scenario == "setup_stall" {
            let mut outgoing =
                tokio::time::timeout(Duration::from_secs(30), owner.connect(&contact, &protocol))
                    .await
                    .context("carrier connect deadline")??;
            let mut step = Step {
                carrier_peer_present: outgoing.binding.initiator_transport_identity.is_some(),
                shared_link_present: outgoing.binding.shared_link.is_some(),
                ..Default::default()
            };
            // Materialize an iroh bidirectional stream and leave a deliberately
            // incomplete setup prefix, rather than testing an unseen stream.
            outgoing.stream.write_all(&[0]).await?;
            outgoing.stream.flush().await?;
            if scenario == "setup_stall" {
                tokio::time::sleep(SETUP_TIMEOUT + Duration::from_millis(500)).await;
            }
            let _ = outgoing.stream.shutdown().await;
            step.outcome = "peer_closed".into();
            report.steps.push(step);
            break;
        }
        let key = if scenario == "wrong_client" {
            Ed25519Keypair::from_seed([3; 32])
        } else {
            client_key()
        };
        let (binding, flip_next_ciphertext, prepared) =
            prepare_client(&owner, &contact, &protocol, &security, &key, &gate).await?;
        let mut step = Step {
            carrier_peer_present: binding.initiator_transport_identity.is_some(),
            shared_link_present: binding.shared_link.is_some(),
            ..Default::default()
        };
        match prepared {
            Err(error) => {
                step.outcome = "setup_refused".into();
                step.detail = Some(format!("{error:?}"));
            }
            Ok((mut stream, facts, guard)) => {
                let channel = facts
                    .secure_channel_facts()
                    .context("secure facts absent")?;
                step.channel_binding = Some(hex(&channel.handshake_hash));
                let mut hello = if scenario == "reconnect" && index == 1 {
                    prior_hello.clone().context("prior hello absent")?
                } else {
                    SessionHello::issue_secure(
                        &client_identity(),
                        NETWORK,
                        profile(),
                        RequestedAction {
                            domain: DOMAIN.into(),
                            path: SERVICE.into(),
                            action: "echo".into(),
                        },
                        TrafficClass::Interactive,
                        [index as u8 + 61; 32],
                        &binding.with_secure_channel(channel),
                        vec![grant()],
                    )?
                };
                if scenario == "reconnect" && index == 0 {
                    prior_hello = Some(hello.clone());
                }
                if scenario == "tamper_hello" {
                    hello.transcript_signature[0] ^= 1;
                    step.mutation = Some("signed_hello".into());
                }
                if scenario == "tamper_ciphertext" {
                    ensure!(
                        contact.carrier != "iroh-native",
                        "fixture cannot mutate below native TLS"
                    );
                    flip_next_ciphertext.store(true, Ordering::SeqCst);
                }
                match guard
                    .run(initiate_session(&mut stream, &hello, &Default::default()))
                    .await
                {
                    Err(error) => {
                        step.outcome = "admission_bounded".into();
                        step.detail = Some(format!("{error:?}"));
                    }
                    Ok(Err(error)) => {
                        step.outcome = "admission_io".into();
                        step.detail = Some(error.to_string());
                    }
                    Ok(Ok(reply)) if !reply.is_accept() => {
                        step.outcome = "admission_refused".into();
                        guard.refused();
                    }
                    Ok(Ok(reply)) => {
                        if let notochord::SessionReply::Accept { session_id, .. } = reply {
                            step.session_id = Some(hex(&session_id));
                        }
                        guard.admitted()?;
                        let request = [index as u8 + 1];
                        write_frame(&mut stream, &request, MAX_APPLICATION_FRAME).await?;
                        if scenario == "slow_consumer" {
                            tokio::time::sleep(APPLICATION_TIMEOUT + Duration::from_millis(500))
                                .await;
                            let _ = stream.shutdown().await;
                            step.outcome = "consumer_stalled".into();
                        } else {
                            let response = tokio::time::timeout(
                                APPLICATION_TIMEOUT,
                                read_frame(&mut stream, MAX_APPLICATION_FRAME),
                            )
                            .await??;
                            ensure!(response == request, "service response differed");
                            write_frame(&mut stream, b"done", MAX_APPLICATION_FRAME).await?;
                            stream.shutdown().await?;
                            report.completed_requests.push(request[0]);
                            step.outcome = "responded".into();
                        }
                    }
                }
                if scenario == "tamper_ciphertext" {
                    ensure!(
                        !flip_next_ciphertext.load(Ordering::SeqCst),
                        "ciphertext mutator did not write its changed byte"
                    );
                    step.mutation = Some("ciphertext".into());
                }
                // Keep each connection handle through the peer's bounded
                // completion witness, including after write-side shutdown.
                retained_streams.push(stream);
            }
        }
        step.pending_after = gate.pending();
        ensure!(step.pending_after == 0, "client setup lease leaked");
        report.steps.push(step);
    }
    report.pending_after = gate.pending();
    let stats = gate.stats();
    report.peak_pending = stats.peak_pending;
    report.timed_out = stats.timed_out;
    report.cancelled = stats.cancelled;
    // Keep the carrier endpoint alive until the other process has finished
    // its final stream shutdown. Receiving a response does not establish that
    // the peer has consumed our acknowledgement or completed its own close.
    let server_result = output.with_file_name("server.json");
    tokio::time::timeout(Duration::from_secs(8), async {
        while !server_result.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("server completion deadline")?;
    drop(retained_streams);
    write_json(output, &report)?;
    owner.close().await?;
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("server") if args.len() == 6 => {
            server(&args[2], &args[3], Path::new(&args[4]), Path::new(&args[5])).await
        }
        Some("client") if args.len() == 5 => {
            client(&args[2], Path::new(&args[3]), Path::new(&args[4])).await
        }
        _ => anyhow::bail!(
            "usage: secure-sessions-n3 server CARRIER SCENARIO CONTACT RESULT | client SCENARIO CONTACT RESULT"
        ),
    }
}
