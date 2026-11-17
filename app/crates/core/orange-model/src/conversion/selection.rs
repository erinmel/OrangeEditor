//! Deterministic direct-converter selection for a prospective connection.

use std::collections::{BTreeMap, BTreeSet};

use crate::id::TypeExpr;

use super::{
    ConversionClass, ConverterKey, ConverterProvider, ConverterRegistrationError,
    ConverterRegistry, NamedPolicy,
};

/// Result of resolving a direct connection conversion without mutating a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// The endpoint types are already equal; no converter node is needed.
    Compatible,
    /// A unique lossless-total entry and exact provider can be inserted directly.
    Automatic(ResolvedConverter),
    /// The user must choose a candidate, provider, or required finite policy value.
    ChoiceRequired {
        candidates: Vec<ConverterCandidate>,
        reason: ChoiceReason,
    },
    /// There is no valid direct selection for this request.
    Rejected(SelectionRejection),
}

/// Exact converter entry, provider, and validated options to bind to a new node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConverter {
    pub entry: ConverterKey,
    pub provider: ConverterProvider,
    pub options: BTreeMap<String, String>,
}

/// One stable selectable entry/provider pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConverterCandidate {
    pub entry: ConverterKey,
    pub provider: ConverterProvider,
    pub class: ConversionClass,
    pub reasons: BTreeSet<CandidateReason>,
    pub policies: Vec<NamedPolicy>,
}

/// Why a candidate needs an explicit choice.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateReason {
    CheckedPartial,
    Lossy,
    MultipleSemanticEntries,
    MultipleProviders,
    RequiredPolicy(String),
    RequiredPoliciesUnresolved,
}

/// Why the planner could not choose a converter automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceReason {
    OnlyPartialOrLossyEntries,
    MultipleLosslessEntries,
    MissingRequiredPolicies,
    ProviderAmbiguity,
    UniqueLosslessProviderUnavailable,
}

/// Rejection details from selection or explicit candidate resolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectionRejection {
    #[error("no direct converter is registered for `{from}` → `{to}`")]
    NoDirectConverter { from: TypeExpr, to: TypeExpr },
    #[error("no implementation provider is registered for converter `{entry}`")]
    NoProvider { entry: ConverterKey },
    #[error("the requested converter choice is stale or does not match this endpoint pair")]
    StaleChoice,
    #[error(transparent)]
    InvalidOptions(#[from] ConverterRegistrationError),
}

/// User-selected exact candidate; identity is never re-resolved by pair alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitConverterChoice {
    pub entry: ConverterKey,
    pub provider: ConverterProvider,
    pub options: BTreeMap<String, String>,
}

impl ConverterRegistry {
    /// Selects a direct converter for a new connection.
    ///
    /// Only one lossless-total semantic entry can be automatic. Partial and lossy entries
    /// remain explicit choices, even when they are the only entries for the pair.
    /// `supplied_options` applies only to a uniquely selected lossless entry; options for
    /// an ambiguous candidate are validated later with `resolve_explicit_choice`.
    /// `from` and `to` must be canonical, concrete `TypeExpr`s already checked by the
    /// caller's typing phase. The registry performs exact identity matching and no
    /// endpoint resolution or assignability search.
    pub fn select_for_connection(
        &self,
        from: &TypeExpr,
        to: &TypeExpr,
        supplied_options: &BTreeMap<String, String>,
    ) -> SelectionOutcome {
        if from == to {
            return SelectionOutcome::Compatible;
        }

        let entries: Vec<_> = self.entries_for_pair(from, to).collect();
        if entries.is_empty() {
            return SelectionOutcome::Rejected(SelectionRejection::NoDirectConverter {
                from: from.clone(),
                to: to.clone(),
            });
        }

        let lossless: Vec<_> = entries
            .iter()
            .filter(|(_, definition)| definition.class == ConversionClass::LosslessTotal)
            .collect();
        if lossless.len() == 1 {
            let (key, definition) = lossless[0];
            let providers: Vec<_> = self.providers(key).into_iter().flatten().cloned().collect();
            if providers.is_empty() {
                let candidates = self.candidates_for_entries(&entries);
                if candidates.is_empty() {
                    return SelectionOutcome::Rejected(SelectionRejection::NoProvider {
                        entry: (*key).clone(),
                    });
                }
                return SelectionOutcome::ChoiceRequired {
                    candidates,
                    reason: ChoiceReason::UniqueLosslessProviderUnavailable,
                };
            }
            let provider = if providers.len() == 1 {
                providers[0].clone()
            } else if let Some(default) = self.new_binding_default(key) {
                default.clone()
            } else {
                return SelectionOutcome::ChoiceRequired {
                    candidates: self.candidates_for_entries(&entries),
                    reason: ChoiceReason::ProviderAmbiguity,
                };
            };
            match definition.resolve_options(supplied_options) {
                Ok(options) => {
                    return SelectionOutcome::Automatic(ResolvedConverter {
                        entry: (*key).clone(),
                        provider,
                        options,
                    });
                }
                Err(ConverterRegistrationError::MissingPolicy { name, .. }) => {
                    let mut candidates = self.candidates_for_entries(&entries);
                    if let Some(candidate) = candidates.iter_mut().find(|candidate| {
                        candidate.entry == **key && candidate.provider == provider
                    }) {
                        candidate
                            .reasons
                            .insert(CandidateReason::RequiredPolicy(name));
                    }
                    return SelectionOutcome::ChoiceRequired {
                        candidates,
                        reason: ChoiceReason::MissingRequiredPolicies,
                    };
                }
                Err(error) => {
                    return SelectionOutcome::Rejected(SelectionRejection::InvalidOptions(error));
                }
            }
        }

        let reason = if lossless.is_empty() {
            ChoiceReason::OnlyPartialOrLossyEntries
        } else {
            ChoiceReason::MultipleLosslessEntries
        };
        let candidates = self.candidates_for_entries(&entries);
        if candidates.is_empty() {
            return SelectionOutcome::Rejected(SelectionRejection::NoProvider {
                entry: entries[0].0.clone(),
            });
        }
        SelectionOutcome::ChoiceRequired { candidates, reason }
    }

    /// Resolves the exact candidate previously shown to the user.
    ///
    /// A stale entry/provider selection fails rather than falling back to a new pair
    /// default. Required policy omissions are reported so the caller can request values.
    pub fn resolve_explicit_choice(
        &self,
        from: &TypeExpr,
        to: &TypeExpr,
        choice: ExplicitConverterChoice,
    ) -> Result<ResolvedConverter, SelectionRejection> {
        if from == to {
            return Err(SelectionRejection::StaleChoice);
        }
        let Some(definition) = self.entry(&choice.entry) else {
            return Err(SelectionRejection::StaleChoice);
        };
        if &definition.from != from || &definition.to != to {
            return Err(SelectionRejection::StaleChoice);
        }
        let Some(providers) = self.providers(&choice.entry) else {
            return Err(SelectionRejection::StaleChoice);
        };
        if !providers
            .into_iter()
            .any(|provider| provider == &choice.provider)
        {
            return Err(SelectionRejection::StaleChoice);
        }
        let options = definition.resolve_options(&choice.options)?;
        Ok(ResolvedConverter {
            entry: choice.entry,
            provider: choice.provider,
            options,
        })
    }

    fn candidates_for_entries(
        &self,
        entries: &[(&ConverterKey, &super::ConverterDefinition)],
    ) -> Vec<ConverterCandidate> {
        let multiple_entries = entries.len() > 1;
        let mut candidates = Vec::new();
        for (key, definition) in entries {
            let providers: Vec<_> = self.providers(key).into_iter().flatten().cloned().collect();
            for provider in &providers {
                let mut reasons = BTreeSet::new();
                if multiple_entries {
                    reasons.insert(CandidateReason::MultipleSemanticEntries);
                }
                match definition.class {
                    ConversionClass::LosslessTotal => {}
                    ConversionClass::CheckedPartial => {
                        reasons.insert(CandidateReason::CheckedPartial);
                    }
                    ConversionClass::Lossy => {
                        reasons.insert(CandidateReason::Lossy);
                    }
                }
                if providers.len() > 1 && self.new_binding_default(key).is_none() {
                    reasons.insert(CandidateReason::MultipleProviders);
                }
                for policy in &definition.policies {
                    if policy.default.is_none() {
                        reasons.insert(CandidateReason::RequiredPolicy(policy.name.clone()));
                    }
                }
                candidates.push(ConverterCandidate {
                    entry: (*key).clone(),
                    provider: provider.clone(),
                    class: definition.class,
                    reasons,
                    policies: definition.policies.clone(),
                });
            }
        }
        candidates
    }
}
