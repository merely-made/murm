//! `--then expire`: the evidence that the board rolled the candidate back by itself
//! (wall_node_management D22, "Either refusal plus the restored radio").
//!
//! After the lifetime and margin, the tool sends the commit the board should no longer take.
//! `InvalidCommit` means the board already rolled back; `TransactionExpired` means the
//! candidate is past its deadline but the board has not run its expiry yet. Either way the
//! board's `applied` line must show the profile from before the apply, and after
//! `TransactionExpired` it is read again for a bounded time. A commit that succeeds there is
//! a failed run: nothing expired.

use std::fmt;
use std::time::{Duration, Instant};

use postilion::control::verified::{
    CommitToken, ControlClient, ControlClientError, ControlExchange, Mutation, ProvisionalReceipt,
};
use radio_hand::applied::AppliedRadio;
use radio_hand::control::{ConfigGeneration, Refusal};
use serde::Serialize;

use super::applied::{AppliedLineSource, AppliedRecord};
use super::{
    CarrierSilence, CommitTokenHolder, ControllerState, LifecyclePlan, LifecycleRecord, describe,
    hex, read_applied, reserve_both, step,
};

/// How often the `applied` line is read again after `TransactionExpired`.
const RECHECK_INTERVAL: Duration = Duration::from_millis(250);

/// What showed the expiry.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ExpiryRecord {
    /// The board's answer to the late commit: `InvalidCommit` or `TransactionExpired`.
    pub refusal: String,
    /// How many times the `applied` line was read again before it showed the earlier profile.
    pub rechecks: u32,
    pub restored: bool,
}

/// What the expiry evidence needs from the run so far.
pub(super) struct Late<'r, 'h, S, H> {
    pub state: &'r mut S,
    pub holder: &'h mut H,
    pub receipt: &'r mut ProvisionalReceipt,
    pub expected_generation: ConfigGeneration,
    pub earlier: AppliedRadio,
}

/// Wait out the candidate, send the late commit, and require the earlier radio back.
pub(super) async fn evidence<C, S, H>(
    client: &mut ControlClient<'_, C>,
    late: Late<'_, '_, S, H>,
    plan: &LifecyclePlan,
    record: &mut LifecycleRecord,
    report: &mut dyn FnMut(&str),
) -> Result<(), (&'static str, String)>
where
    C: ControlExchange + AppliedLineSource,
    C::Error: CarrierSilence + fmt::Display,
    S: ControllerState,
    H: CommitTokenHolder,
{
    let wait = Duration::from_millis(plan.lifetime_ms) + plan.expiry_margin;
    report(&format!(
        "waiting {} ms past apply; sending nothing",
        wait.as_millis()
    ));
    tokio::time::sleep(wait).await;

    let (counter, sequence) = reserve_both(late.state).map_err(|message| ("late-commit", message))?;
    let Some(token) = late.holder.release() else {
        return Err(("late-commit", "no commit token is held".into()));
    };
    late.receipt.commit_token = token;
    let answer = client
        .commit(
            counter,
            Mutation {
                sequence,
                expected_generation: late.expected_generation,
            },
            late.receipt,
        )
        .await;
    // Dropping the replaced token zeroes it.
    late.receipt.commit_token = CommitToken::default();
    let refusal = match answer {
        Ok(applied) => {
            record.steps.push(step(
                "late-commit",
                counter,
                Some(sequence),
                Some(hex(&applied.transaction.0)),
                "applied",
            ));
            record.known_good_after = Some(applied.known_good_generation.0);
            return Err((
                "late-commit",
                format!(
                    "the board committed the candidate after its deadline, so nothing expired \
                     (known-good generation {})",
                    applied.known_good_generation.0
                ),
            ));
        }
        Err(ControlClientError::Refused(
            reason @ (Refusal::InvalidCommit | Refusal::TransactionExpired),
        )) => reason,
        Err(error) => {
            let message = describe("late-commit", &error);
            record
                .steps
                .push(step("late-commit", counter, Some(sequence), None, &message));
            return Err(("late-commit", message));
        }
    };
    record.steps.push(step(
        "late-commit",
        counter,
        Some(sequence),
        None,
        &format!("refused {refusal:?}"),
    ));
    report(&format!("late-commit refused={refusal:?}"));

    let started = Instant::now();
    let mut rechecks = 0;
    loop {
        let now = read_applied(client)
            .await
            .map_err(|message| ("radio-after", message))?;
        record.applied_after = Some(AppliedRecord::of(&now));
        let restored = now == late.earlier;
        if restored || refusal == Refusal::InvalidCommit || started.elapsed() >= plan.expiry_margin
        {
            record.expiry = Some(ExpiryRecord {
                refusal: format!("{refusal:?}"),
                rechecks,
                restored,
            });
            if restored {
                report(&format!("radio-after restored rechecks={rechecks}"));
                return Ok(());
            }
            return Err((
                "radio-after",
                format!(
                    "the board refused the late commit ({refusal:?}) but its applied line does \
                     not show the profile from before the apply{}",
                    if refusal == Refusal::TransactionExpired {
                        format!(" within {} ms", plan.expiry_margin.as_millis())
                    } else {
                        String::new()
                    }
                ),
            ));
        }
        rechecks += 1;
        tokio::time::sleep(RECHECK_INTERVAL).await;
    }
}
