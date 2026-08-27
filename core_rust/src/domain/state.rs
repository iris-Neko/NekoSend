use thiserror::Error;

use super::{TransferFailureReason, TransferState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferTransition {
    pub from: TransferState,
    pub to: TransferState,
    pub reason: Option<TransferFailureReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransferTransitionError {
    #[error("transition from {from:?} to {to:?} is not allowed")]
    InvalidState {
        from: TransferState,
        to: TransferState,
    },
    #[error("transition from {from:?} to {to:?} requires a failure reason")]
    MissingReason {
        from: TransferState,
        to: TransferState,
    },
    #[error("failure reason {reason:?} is invalid for transition to {to:?}")]
    InvalidReason {
        to: TransferState,
        reason: TransferFailureReason,
    },
    #[error("failure reason {reason:?} cannot be recovered in the existing task")]
    FinalFailure { reason: TransferFailureReason },
}

impl TransferFailureReason {
    pub const fn is_recoverable(self) -> bool {
        matches!(
            self,
            Self::PeerOffline
                | Self::SourceChanged
                | Self::NotEnoughSpace
                | Self::ConnectionError
                | Self::InvalidPath
                | Self::PermissionLost
        )
    }
}

pub fn validate_transfer_transition(
    transition: TransferTransition,
) -> Result<(), TransferTransitionError> {
    use TransferState as S;

    let allowed = matches!(
        (transition.from, transition.to),
        (S::Queued, S::Offered | S::Cancelled | S::Failed)
            | (
                S::Offered,
                S::Accepted | S::Queued | S::Failed | S::Cancelled
            )
            | (
                S::Accepted,
                S::Transferring | S::Queued | S::Paused | S::Failed | S::Cancelled
            )
            | (
                S::Transferring,
                S::Verifying | S::Queued | S::Paused | S::Failed | S::Cancelled
            )
            | (
                S::Paused,
                S::Queued | S::Accepted | S::Cancelled | S::Failed
            )
            | (S::Verifying, S::Completed | S::Failed)
            | (S::Failed, S::Queued | S::Accepted | S::Cancelled)
    );
    if !allowed {
        return Err(TransferTransitionError::InvalidState {
            from: transition.from,
            to: transition.to,
        });
    }

    match transition.to {
        S::Failed => {
            let reason = transition
                .reason
                .ok_or(TransferTransitionError::MissingReason {
                    from: transition.from,
                    to: transition.to,
                })?;
            if matches!(
                reason,
                TransferFailureReason::PeerOffline
                    | TransferFailureReason::ConnectionError
                    | TransferFailureReason::UserCancelled
            ) {
                return Err(TransferTransitionError::InvalidReason {
                    to: transition.to,
                    reason,
                });
            }
        }
        S::Cancelled => {
            if transition.reason != Some(TransferFailureReason::UserCancelled) {
                return Err(TransferTransitionError::MissingReason {
                    from: transition.from,
                    to: transition.to,
                });
            }
        }
        S::Queued if transition.from == S::Failed => {
            let reason = transition
                .reason
                .ok_or(TransferTransitionError::MissingReason {
                    from: transition.from,
                    to: transition.to,
                })?;
            if !reason.is_recoverable() {
                return Err(TransferTransitionError::FinalFailure { reason });
            }
        }
        S::Accepted if transition.from == S::Failed => {
            let reason = transition
                .reason
                .ok_or(TransferTransitionError::MissingReason {
                    from: transition.from,
                    to: transition.to,
                })?;
            if !reason.is_recoverable() {
                return Err(TransferTransitionError::FinalFailure { reason });
            }
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(
        from: TransferState,
        to: TransferState,
        reason: Option<TransferFailureReason>,
    ) -> TransferTransition {
        TransferTransition { from, to, reason }
    }

    #[test]
    fn every_documented_happy_path_is_allowed() {
        let cases = [
            transition(TransferState::Queued, TransferState::Offered, None),
            transition(TransferState::Offered, TransferState::Accepted, None),
            transition(TransferState::Accepted, TransferState::Transferring, None),
            transition(TransferState::Transferring, TransferState::Verifying, None),
            transition(TransferState::Verifying, TransferState::Completed, None),
            transition(
                TransferState::Transferring,
                TransferState::Queued,
                Some(TransferFailureReason::ConnectionError),
            ),
            transition(
                TransferState::Paused,
                TransferState::Cancelled,
                Some(TransferFailureReason::UserCancelled),
            ),
        ];
        for case in cases {
            assert_eq!(validate_transfer_transition(case), Ok(()));
        }
    }

    #[test]
    fn terminal_states_cannot_transition() {
        for state in [TransferState::Completed, TransferState::Cancelled] {
            assert!(
                validate_transfer_transition(transition(state, TransferState::Queued, None))
                    .is_err()
            );
        }
    }

    #[test]
    fn final_failures_cannot_retry_existing_task() {
        for reason in [
            TransferFailureReason::Rejected,
            TransferFailureReason::Unsupported,
        ] {
            assert_eq!(
                validate_transfer_transition(transition(
                    TransferState::Failed,
                    TransferState::Queued,
                    Some(reason),
                )),
                Err(TransferTransitionError::FinalFailure { reason })
            );
        }
    }

    #[test]
    fn network_interruption_must_queue_instead_of_fail() {
        assert!(
            validate_transfer_transition(transition(
                TransferState::Transferring,
                TransferState::Failed,
                Some(TransferFailureReason::ConnectionError),
            ))
            .is_err()
        );
        assert!(
            validate_transfer_transition(transition(
                TransferState::Transferring,
                TransferState::Queued,
                Some(TransferFailureReason::ConnectionError),
            ))
            .is_ok()
        );
    }
}
