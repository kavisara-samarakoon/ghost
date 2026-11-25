//! P1-01 inert native contracts. Registered only by `lib.rs` under `cfg(test)`.
//! Identifiers, state descriptions and reservations confer no consent or authority.
#[cfg(test)]
mod adapters;
#[cfg(test)]
mod audit;
mod clock;
#[cfg(test)]
mod consent;
// P1-07 egress is registered privately inside consent to preserve its opaque
// grant/validation boundary. Both modules remain excluded from production.
#[cfg(test)]
mod host;
mod limits;
#[cfg(test)]
mod microphone;
mod model;
#[cfg(test)]
mod native_speech;
#[cfg(test)]
mod preservation;
#[cfg(test)]
mod provenance;
mod session;
#[cfg(test)]
mod settings;
#[cfg(test)]
mod speech;
#[cfg(test)]
mod speech_output_ownership;
#[cfg(test)]
mod stop;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wake;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractError {
    InvalidIdentifier,
    Overflow,
    LimitExceeded,
    AlreadyConsumed,
    DuplicateSession,
    UnknownSession,
    DuplicateTurn,
    UnknownTurn,
    DuplicateProviderRole,
    PlanningNotConversationRole,
    TranscriptionRouteRequired,
    DuplicateJob,
    StaleSpeechLease,
    ClockDomainMismatch,
    TimeReversed,
}

type Result<T> = std::result::Result<T, ContractError>;
