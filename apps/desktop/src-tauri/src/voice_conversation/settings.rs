//! P1-12 synthetic settings schema and native review contract only.
//! No filesystem, credentials, grants, device permissions or runtime services.
//! Reference catalogs are explicit finite fixtures, not installed/approved live
//! providers or models. Change requirements are NOT completed runtime fanout.
use super::clock::{Deadline, MonotonicTime};
use super::model::{ConfigurationRevision, ContractIdentity, ContractVersion, ProviderRole};
use super::ContractError;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

const REVIEW_LIFETIME: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Contract(ContractError),
    Malformed,
    TooLarge,
    InvalidReference,
    Stale,
    Busy,
    Closed,
    Denied,
    Expired,
    Untrusted,
}
impl From<ContractError> for Error {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}
type Result<T> = std::result::Result<T, Error>;

macro_rules! reference {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        struct $name(NonZeroU64);
    )+};
}
reference!(
    ProfileId,
    ModelId,
    CredentialId,
    VoiceId,
    WakeModelId,
    LanguageId,
    ThresholdId
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialReference {
    id: CredentialId,
    revision: NonZeroU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileReference {
    profile: ProfileId,
    model: ModelId,
    credential: Option<CredentialReference>,
    privacy_revision: NonZeroU64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profiles {
    stt: Option<ProfileReference>,
    conversation: Option<ProfileReference>,
    planning: Option<ProfileReference>,
    tts: Option<ProfileReference>,
}
impl Profiles {
    fn entries(self) -> [(ProviderRole, Option<ProfileReference>); 4] {
        [
            (ProviderRole::Stt, self.stt),
            (ProviderRole::Conversation, self.conversation),
            (ProviderRole::Planning, self.planning),
            (ProviderRole::Tts, self.tts),
        ]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WakeReference {
    model: WakeModelId,
    checksum: [u8; 32],
    thresholds: ThresholdId,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeatureVisibility {
    conversation: bool,
    wake: bool,
    speech: bool,
}

/// A reference to all fixed P1-01 bounds; no new configurable thresholds/limits.
/// Engine-specific threshold calibration stays in the native fixture catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovedBounds {
    Phase0V1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConsentPresentation {
    CompleteNativeReview,
}

/// No strings, endpoints, secrets, stored permission/grant, armed state, history,
/// background-enabled state or automatic-renewal setting is representable here.
/// Visibility and speaker preferences never enable any runtime operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDescription {
    version: ContractVersion,
    revision: ConfigurationRevision,
    profiles: Profiles,
    installed_voice: Option<VoiceId>,
    wake_model: Option<WakeReference>,
    language: Option<LanguageId>,
    bounds: ApprovedBounds,
    visibility: FeatureVisibility,
    speaker_output_preference: bool,
    consent_presentation: ConsentPresentation,
}
impl SettingsDescription {
    fn privacy_default() -> Self {
        Self {
            version: ContractVersion::V1,
            revision: ConfigurationRevision::from_native_sequence(1).unwrap(),
            profiles: Profiles::default(),
            installed_voice: None,
            wake_model: None,
            language: None,
            bounds: ApprovedBounds::Phase0V1,
            visibility: FeatureVisibility::default(),
            speaker_output_preference: false,
            consent_presentation: ConsentPresentation::CompleteNativeReview,
        }
    }
}
/// Decoding creates data only. It cannot produce a confirmed review or grant.
struct UntrustedSettings(SettingsDescription);
fn decode(bytes: &[u8], fixture_input_limit: usize) -> Result<UntrustedSettings> {
    if bytes.len() > fixture_input_limit {
        return Err(Error::TooLarge);
    }
    serde_json::from_slice(bytes)
        .map(UntrustedSettings)
        .map_err(|_| Error::Malformed)
}

struct RegisteredProfile {
    role: ProviderRole,
    reference: ProfileReference,
}
/// Borrowed, finite, explicitly synthetic registries. No account/Keychain/model
/// lookup occurs and no real provider, voice or wake artifact is approved here.
struct FixtureCatalog<'a> {
    profiles: &'a [RegisteredProfile],
    voices: &'a [VoiceId],
    wake_models: &'a [WakeReference],
    languages: &'a [LanguageId],
}
impl FixtureCatalog<'_> {
    fn validate(&self, settings: &SettingsDescription) -> Result<()> {
        for (role, reference) in settings.profiles.entries() {
            if let Some(reference) = reference {
                if !self
                    .profiles
                    .iter()
                    .any(|p| p.role == role && p.reference == reference)
                {
                    return Err(Error::InvalidReference);
                }
            }
        }
        if settings
            .installed_voice
            .is_some_and(|id| !self.voices.contains(&id))
            || settings
                .wake_model
                .is_some_and(|id| !self.wake_models.contains(&id))
            || settings
                .language
                .is_some_and(|id| !self.languages.contains(&id))
        {
            return Err(Error::InvalidReference);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativeContext {
    identity: ContractIdentity,
    os_session: NonZeroU64,
    permission_revision: NonZeroU64,
    policy_revision: NonZeroU64,
}
struct ReviewData {
    candidate: SettingsDescription,
    context: NativeContext,
    sequence: u64,
    deadline: Deadline,
    owner: Arc<()>,
}
#[derive(Clone)]
struct Review(Arc<ReviewData>);

mod presenter {
    use super::*;
    pub(super) struct TestPresenter(Arc<()>);
    pub(super) enum Decision {
        Confirm,
        Cancel,
    }
    pub(super) struct Receipt {
        presenter: Arc<()>,
        review: Review,
        decision: Decision,
    }
    impl TestPresenter {
        pub(super) fn synthetic() -> Self {
            Self(Arc::new(()))
        }
        pub(super) fn receipt(&self, review: &Review, decision: Decision) -> Receipt {
            Receipt {
                presenter: self.0.clone(),
                review: review.clone(),
                decision,
            }
        }
        pub(super) fn key(&self) -> Arc<()> {
            self.0.clone()
        }
    }
    impl Receipt {
        pub(super) fn verifies(&self, presenter: &Arc<()>, review: &Review) -> bool {
            Arc::ptr_eq(&self.presenter, presenter) && Arc::ptr_eq(&self.review.0, &review.0)
        }
        pub(super) fn confirmed(&self) -> bool {
            matches!(self.decision, Decision::Confirm)
        }
    }
}

/// Declarative obligations, never acknowledgements. Every new configuration
/// revision invalidates revision-bound reviews/grants/jobs under P1-06/P1-07.
/// Existing controllers retain sole authority over revocation and release ACKs.
#[derive(Debug, PartialEq, Eq)]
struct ChangeRequirements {
    previous: ContractIdentity,
    next: ContractIdentity,
    invalidate_reviews_grants_and_jobs: bool,
    cancel_wake_input_and_speech: bool,
    clear_ephemeral_context: bool,
}
struct NativeSettings {
    current: SettingsDescription,
    context: NativeContext,
    owner: Arc<()>,
    presenter: Arc<()>,
    sequence: u64,
    pending: Option<Review>,
    last_time: MonotonicTime,
    closed: bool,
}
impl NativeSettings {
    fn synthetic(
        context: NativeContext,
        presenter: &presenter::TestPresenter,
        now: MonotonicTime,
    ) -> Self {
        let mut current = SettingsDescription::privacy_default();
        current.revision = context.identity.configuration_revision;
        Self {
            current,
            context,
            owner: Arc::new(()),
            presenter: presenter.key(),
            sequence: 0,
            pending: None,
            last_time: now,
            closed: false,
        }
    }
    fn observe(&mut self, now: &MonotonicTime) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        if let Err(error) = now.checked_duration_since(&self.last_time) {
            self.pending = None;
            self.closed = true;
            return Err(error.into());
        }
        self.last_time = now.clone();
        Ok(())
    }
    fn prepare(
        &mut self,
        candidate: UntrustedSettings,
        context: NativeContext,
        catalog: &FixtureCatalog<'_>,
        now: &MonotonicTime,
    ) -> Result<Review> {
        self.observe(now)?;
        if context != self.context {
            return Err(Error::Stale);
        }
        if let Some(pending) = &self.pending {
            match pending.0.deadline.is_expired(now) {
                Ok(true) => self.pending = None,
                Ok(false) => (),
                Err(error) => {
                    self.pending = None;
                    self.closed = true;
                    return Err(error.into());
                }
            }
        }
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let revision = self.current.revision.checked_next().map_err(|e| {
            self.closed = true;
            Error::Contract(e)
        })?;
        if candidate.0.revision != revision {
            return Err(Error::Stale);
        }
        catalog.validate(&candidate.0)?;
        let sequence = self.sequence.checked_add(1).ok_or_else(|| {
            self.closed = true;
            Error::Contract(ContractError::Overflow)
        })?;
        let deadline = Deadline::after(now.clone(), REVIEW_LIFETIME)?;
        let review = Review(Arc::new(ReviewData {
            candidate: candidate.0,
            context,
            sequence,
            deadline,
            owner: self.owner.clone(),
        }));
        self.sequence = sequence;
        self.pending = Some(review.clone());
        Ok(review)
    }
    fn confirm(
        &mut self,
        receipt: &presenter::Receipt,
        context: NativeContext,
        catalog: &FixtureCatalog<'_>,
        now: &MonotonicTime,
    ) -> Result<ChangeRequirements> {
        let review = self.pending.take().ok_or(Error::Stale)?;
        self.observe(now)?;
        if !receipt.verifies(&self.presenter, &review) {
            return Err(Error::Untrusted);
        }
        if !Arc::ptr_eq(&review.0.owner, &self.owner)
            || review.0.sequence != self.sequence
            || context != self.context
            || review.0.context != context
            || review.0.candidate.revision != self.current.revision.checked_next()?
        {
            return Err(Error::Stale);
        }
        if review.0.deadline.is_expired(now)? {
            return Err(Error::Expired);
        }
        if !receipt.confirmed() {
            return Err(Error::Denied);
        }
        catalog.validate(&review.0.candidate)?; // a changed native registry invalidates the review
        let generation = context.identity.generation.checked_next().map_err(|e| {
            self.closed = true;
            Error::Contract(e)
        })?;
        let previous = self.context.identity;
        self.context.identity.configuration_revision = review.0.candidate.revision;
        self.context.identity.generation = generation;
        self.current = review.0.candidate.clone();
        Ok(ChangeRequirements {
            previous,
            next: self.context.identity,
            invalidate_reviews_grants_and_jobs: true,
            cancel_wake_input_and_speech: true,
            clear_ephemeral_context: true,
        })
    }
    /// Native revision metadata only; this issues no permission or policy grant.
    fn native_context_changed(
        &mut self,
        permission_revision: NonZeroU64,
        policy_revision: NonZeroU64,
    ) -> Result<()> {
        self.pending = None;
        if self.closed {
            return Err(Error::Closed);
        }
        if permission_revision < self.context.permission_revision
            || policy_revision < self.context.policy_revision
        {
            return Err(Error::Stale);
        }
        self.context.permission_revision = permission_revision;
        self.context.policy_revision = policy_revision;
        Ok(())
    }
    fn untrusted_confirmation(&mut self, _bytes: &[u8]) -> Result<ChangeRequirements> {
        self.pending = None;
        Err(Error::Untrusted)
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
