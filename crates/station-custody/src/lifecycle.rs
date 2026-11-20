//! The controller lifecycle in one invocation (wall_node_management D17, D18): signed Status,
//! ProvisionalApply of a candidate built from arguments, signed Status as evidence, then
//! exactly one of Commit, Revert, or a wait past the board's deadline and a Status read.
//!
//! The controller key stays inside this crate, released by the caller's authority. The commit
//! token lives only in a [`CommitTokenHolder`] for the length of the run. Every outer counter
//! and mutation sequence is spent durably in [`ControllerState`] before the frame carrying it
//! is sent. A controller-side timeout is never reported as an expiry: only what the board
//! answers after its deadline is evidence.

use std::fmt;
use std::time::Duration;

use personae::IdentityProvider;
use postilion::control::verified::{
    CommitToken, ControlClient, ControlClientError, ControlExchange, Mutation, UsbControlError,
    VerifiedStatus,
};
use radio_hand::control::{ChangeId, NodeId, PublicConfigurationV1, Refusal};

use crate::FirstOwnerCredential;

mod record;
mod state;
mod token;

pub use record::{CandidateRecord, LifecycleRecord, StatusRecord, StepRecord};
pub use state::{ControllerState, FileControllerState};
pub use token::{CommitTokenHolder, InMemoryTokenHolder};

/// What follows the apply and its status evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Commit,
    Revert,
    /// Send nothing until the lifetime and margin have passed, then read Status.
    Expire,
}

impl Then {
    const fn name(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Revert => "revert",
            Self::Expire => "expire",
        }
    }
}

/// One lifecycle run.
#[derive(Clone, Debug)]
pub struct LifecyclePlan {
    pub node: NodeId,
    pub candidate: PublicConfigurationV1,
    pub lifetime_ms: u64,
    pub then: Then,
    /// For [`Then::Expire`]: how long past the lifetime to wait before reading Status.
    pub expiry_margin: Duration,
}

/// Whether a carrier error is the board's silence. The board answers every outer refusal
/// with nothing, so silence needs a list of things to check rather than a bare timeout.
pub trait CarrierSilence {
    fn is_silence(&self) -> bool;
}

impl CarrierSilence for UsbControlError {
    fn is_silence(&self) -> bool {
        matches!(self, Self::Timeout)
    }
}

/// A run that stopped, with the record as far as it got. Its text names no secret.
pub struct LifecycleFailure {
    pub step: &'static str,
    pub message: String,
    pub record: Box<LifecycleRecord>,
}

impl fmt::Debug for LifecycleFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LifecycleFailure")
            .field("step", &self.step)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for LifecycleFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed: {}", self.step, self.message)
    }
}

impl std::error::Error for LifecycleFailure {}

const SILENCE_CHECKS: &str = "the board answered nothing. Worth checking: the controller scope is \
the one the board was claimed with; the counter record is not behind the board (a counter it \
already accepted earns silence); the radio was not mid-frame (the board will not quiet it then; \
run again); the port is this board's";
const APPLY_SILENCE_CHECKS: &str = "; and the profile itself: the V4 refuses a profile it cannot \
apply (a frequency outside the region, or a value the radio does not support) with silence too";

fn describe<E: CarrierSilence + fmt::Display>(step: &str, error: &ControlClientError<E>) -> String {
    match error {
        ControlClientError::Carrier(carrier) if carrier.is_silence() => match step {
            "apply" => format!("{SILENCE_CHECKS}{APPLY_SILENCE_CHECKS}"),
            _ => SILENCE_CHECKS.to_string(),
        },
        ControlClientError::Carrier(carrier) => format!("carrier failed: {carrier}"),
        ControlClientError::Refused(reason) => refusal(*reason),
        // The remaining variants name no key or token material.
        other => other.to_string(),
    }
}

fn refusal(reason: Refusal) -> String {
    let hint = match reason {
        Refusal::StaleGeneration => {
            "the known-good generation moved since the status read; run again"
        }
        Refusal::TransactionExpired => {
            "the mutation sequence record is behind the board's, or the candidate's deadline passed"
        }
        Refusal::TransactionTooFar => {
            "the mutation sequence record is too far ahead of the board's"
        }
        Refusal::TransactionConflict => "a candidate is already armed; let it expire or revert it",
        Refusal::InvalidCommit => "no armed candidate matches this change",
        Refusal::InvalidArguments => "the lifetime or configuration is out of range",
        Refusal::Unauthorized => "this controller's grant does not permit the change",
        _ => "see the refusal name",
    };
    format!("the board refused: {reason:?} ({hint})")
}

/// Run one lifecycle against `carrier` as the first-owner controller of `scope`.
///
/// `report` receives one line per step for the operator; none carries a secret.
///
/// # Errors
/// A [`LifecycleFailure`] names the step that stopped, why (silence with what to check, the
/// board's refusal, a carrier or controller-state fault) and carries the record so far. Every
/// counter and sequence it lists stays spent.
#[allow(clippy::too_many_lines)]
pub async fn run_lifecycle<C, S, H>(
    provider: &dyn IdentityProvider,
    scope: &[u8],
    carrier: C,
    plan: &LifecyclePlan,
    state: &mut S,
    holder: &mut H,
    report: &mut dyn FnMut(&str),
) -> Result<LifecycleRecord, LifecycleFailure>
where
    C: ControlExchange,
    C::Error: CarrierSilence + fmt::Display,
    S: ControllerState,
    H: CommitTokenHolder,
{
    let change = ChangeId(uuid::Uuid::new_v4().into_bytes());
    let mut record = LifecycleRecord {
        schema: "station-claim.lifecycle/v1",
        node: hex(&plan.node.0),
        change: hex(&change.0),
        action: plan.then.name(),
        candidate: CandidateRecord::of(&plan.candidate),
        lifetime_ms: plan.lifetime_ms,
        steps: Vec::new(),
        status_before: None,
        candidate_generation: None,
        deadline_ms: None,
        applied_reported: None,
        status_during: None,
        status_after: None,
        known_good_after: None,
        outcome: "incomplete".into(),
        notes: vec![
            "the provisional answer carries no applied configuration; signed Status carries no \
             radio fields"
                .into(),
        ],
    };
    macro_rules! fail {
        ($step:expr, $message:expr) => {{
            holder.discard();
            record.outcome = format!("failed at {}", $step);
            return Err(LifecycleFailure {
                step: $step,
                message: $message,
                record: Box::new(record),
            });
        }};
    }
    let Ok(credential) = FirstOwnerCredential::derive(provider, scope) else {
        fail!(
            "identity",
            "the first-owner controller credential could not be derived".into()
        )
    };
    let mut client = ControlClient::new(carrier, &credential.identity, plan.node);

    // Status before: the known-good generation every mutation names.
    let before = match status(&mut client, state, &mut record, "status-before").await {
        Ok(status) => status,
        Err((step, message)) => fail!(step, message),
    };
    report(&format!(
        "status-before known-good-generation={} watermark={}",
        before.status.known_good_generation().0,
        before.status.generation_watermark().0
    ));

    // Apply. Counter and sequence are spent before the frame leaves.
    let (counter, sequence) = match reserve_both(state) {
        Ok(values) => values,
        Err(message) => fail!("apply", message),
    };
    let mutation = Mutation {
        sequence,
        expected_generation: before.known_good_generation,
    };
    let applied = client
        .provisional_apply(counter, mutation, change, plan.candidate, plan.lifetime_ms)
        .await;
    let mut receipt = match applied {
        Ok(receipt) => receipt,
        Err(error) => {
            let message = describe("apply", &error);
            record
                .steps
                .push(step("apply", counter, Some(sequence), None, &message));
            fail!("apply", message)
        }
    };
    holder.keep(std::mem::take(&mut receipt.commit_token));
    record.steps.push(step(
        "apply",
        counter,
        Some(sequence),
        Some(hex(&receipt.transaction.0)),
        "provisional",
    ));
    record.candidate_generation = Some(receipt.candidate_generation.0);
    record.deadline_ms = Some(receipt.deadline_ms);
    report(&format!(
        "applied change={} candidate-generation={} board-deadline-ms={}",
        record.change, receipt.candidate_generation.0, receipt.deadline_ms
    ));

    // Status as evidence that the candidate is armed.
    match status(&mut client, state, &mut record, "status-during").await {
        Ok(during) => report(&format!(
            "status-during known-good-generation={} watermark={}",
            during.status.known_good_generation().0,
            during.status.generation_watermark().0
        )),
        Err((step, message)) => fail!(step, message),
    }

    match plan.then {
        Then::Commit => {
            let (counter, sequence) = match reserve_both(state) {
                Ok(values) => values,
                Err(message) => fail!("commit", message),
            };
            let Some(token) = holder.release() else {
                fail!("commit", "no commit token is held".into())
            };
            receipt.commit_token = token;
            let committed = client
                .commit(
                    counter,
                    Mutation {
                        sequence,
                        expected_generation: before.known_good_generation,
                    },
                    &receipt,
                )
                .await;
            // Dropping the replaced token zeroes it.
            receipt.commit_token = CommitToken::default();
            match committed {
                Ok(applied) => {
                    record.steps.push(step(
                        "commit",
                        counter,
                        Some(sequence),
                        Some(hex(&applied.transaction.0)),
                        "applied",
                    ));
                    record.known_good_after = Some(applied.known_good_generation.0);
                }
                Err(error) => {
                    let message = describe("commit", &error);
                    record
                        .steps
                        .push(step("commit", counter, Some(sequence), None, &message));
                    fail!("commit", message)
                }
            }
        }
        Then::Revert => {
            holder.discard();
            let (counter, sequence) = match reserve_both(state) {
                Ok(values) => values,
                Err(message) => fail!("revert", message),
            };
            let mutation = Mutation {
                sequence,
                expected_generation: before.known_good_generation,
            };
            match client.revert(counter, mutation, change).await {
                Ok(applied) => {
                    record.steps.push(step(
                        "revert",
                        counter,
                        Some(sequence),
                        Some(hex(&applied.transaction.0)),
                        "applied",
                    ));
                    record.known_good_after = Some(applied.known_good_generation.0);
                }
                Err(error) => {
                    let message = describe("revert", &error);
                    record
                        .steps
                        .push(step("revert", counter, Some(sequence), None, &message));
                    fail!("revert", message)
                }
            }
        }
        Then::Expire => {
            holder.discard();
            let wait = Duration::from_millis(plan.lifetime_ms) + plan.expiry_margin;
            report(&format!(
                "waiting {} ms past apply; sending nothing",
                wait.as_millis()
            ));
            tokio::time::sleep(wait).await;
            match status(&mut client, state, &mut record, "status-after").await {
                Ok(after) => {
                    record.status_after = Some(StatusRecord::of(&after.status));
                    record.known_good_after = Some(after.known_good_generation.0);
                    record.notes.push(
                        "status v1 carries no armed-candidate field: after an expiry it shows \
                         the same known-good generation and watermark as while armed"
                            .into(),
                    );
                }
                Err((step, message)) => fail!(step, message),
            }
        }
    }
    record.outcome = format!("{} completed", plan.then.name());
    report(&format!(
        "{} known-good-generation={}",
        plan.then.name(),
        record.known_good_after.unwrap_or_default()
    ));
    Ok(record)
}

/// Spend a counter, read signed Status, and record both.
async fn status<C, S>(
    client: &mut ControlClient<'_, C>,
    state: &mut S,
    record: &mut LifecycleRecord,
    name: &'static str,
) -> Result<VerifiedStatus, (&'static str, String)>
where
    C: ControlExchange,
    C::Error: CarrierSilence + fmt::Display,
    S: ControllerState,
{
    let counter = state
        .reserve_counter()
        .map_err(|error| (name, format!("controller state: {error}")))?;
    match client.status(counter).await {
        Ok(status) => {
            record.steps.push(step(
                name,
                counter,
                None,
                Some(hex(&status.transaction.0)),
                "observed",
            ));
            let fields = Some(StatusRecord::of(&status.status));
            match name {
                "status-before" => record.status_before = fields,
                "status-during" => record.status_during = fields,
                _ => {}
            }
            Ok(status)
        }
        Err(error) => {
            let message = describe(name, &error);
            record.steps.push(step(name, counter, None, None, &message));
            Err((name, message))
        }
    }
}

fn reserve_both<S: ControllerState>(state: &mut S) -> Result<(u64, u64), String> {
    let counter = state
        .reserve_counter()
        .map_err(|error| format!("controller state: {error}"))?;
    let sequence = state
        .reserve_sequence()
        .map_err(|error| format!("controller state: {error}"))?;
    Ok((counter, sequence))
}

fn step(
    name: &'static str,
    counter: u64,
    sequence: Option<u64>,
    transaction: Option<String>,
    outcome: &str,
) -> StepRecord {
    StepRecord {
        step: name,
        counter,
        sequence,
        transaction,
        outcome: outcome.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
