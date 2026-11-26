//! Executive authority regressions over canonical detention, election and retirement paths.

use super::*;
use crate::core::entity::EntityRef;
use crate::core::id::ArrestId;
use crate::core::invariants::{StateValidationError, validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save};
use crate::legal::arrest_system::{validate_arrest, validate_release_arrest};
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft,
};
use crate::politics::MayoralElection;
use crate::world::world_system::{
    insert_character, insert_organization, validate_set_character_supervisor,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

fn second_executive(state: &mut AppState, owner: OrganizationId) -> CharacterId {
    insert_character(
        state,
        CharacterDraft {
            name: "Second executive after election".into(),
            organization: Some(owner),
            supervisor: None,
            autonomy: AutonomyLevel::Tight,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap()
}

pub(super) fn detain(registry: &Registry, state: &mut AppState, person: CharacterId) -> ArrestId {
    let existing_police = state
        .world()
        .organizations()
        .find(|organization| organization.kind() == OrganizationKind::LawEnforcement)
        .map(|organization| organization.id());
    let police = existing_police.unwrap_or_else(|| {
        insert_organization(
            registry,
            state,
            OrganizationDraft {
                name: "Certification fixture police".into(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .unwrap()
    });
    let investigation = validate_open_investigation(
        state,
        InvestigationDraft {
            owner: police,
            title: "Executive custody before certification".into(),
            subjects: BTreeSet::from([EntityRef::Character(person)]),
        },
    )
    .unwrap()
    .commit(state)
    .unwrap();
    let evidence = [EvidenceKind::Document, EvidenceKind::KnownAssociation]
        .into_iter()
        .map(|kind| {
            validate_add_evidence(
                state,
                EvidenceDraft {
                    investigation,
                    custodian: police,
                    subject: EntityRef::Character(person),
                    origin: None,
                    kind,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: state.now(),
                },
            )
            .unwrap()
            .commit(state)
            .unwrap()
        })
        .collect();
    validate_arrest(
        registry,
        state,
        ArrestDraft {
            character: person,
            investigation,
            evidence,
        },
    )
    .unwrap()
    .commit(state)
    .unwrap()
}

pub(super) fn verify_elected_executive_certification(
    registry: &Registry,
    state: &AppState,
    owner: OrganizationId,
    boss: CharacterId,
    ballot: &MayoralElection,
) {
    assert_eq!(
        political_certification_status(state, owner),
        PoliticalCertificationStatus::Ready
    );
    // Both fresh and held orders must reject a divided command or a detained
    // elected executive, without erasing the genuine election.
    for detain_nominee in [false, true] {
        let mut blocked = state.clone();
        let held = validate_claim_victory(registry, &blocked, VictoryPath::PoliticalPower).unwrap();
        let second = (!detain_nominee).then(|| second_executive(&mut blocked, owner));
        // A detained second root still represents divided command.
        let arrest = detain(registry, &mut blocked, second.unwrap_or(boss));
        let standing = progress(registry, &blocked).unwrap();
        assert!(standing.latest_election_won);
        assert!(!standing.sole_executive_available);
        assert_eq!(
            standing.political_certification,
            PoliticalCertificationStatus::ExecutiveUnavailable
        );
        let before = bincode::serialize(&build_save(registry, &blocked).unwrap()).unwrap();
        assert_eq!(
            validate_claim_victory(registry, &blocked, VictoryPath::PoliticalPower).err(),
            Some(VictoryError::PoliticalExecutiveUnavailable),
            "a won ballot does not override live executive authority",
        );
        assert_eq!(
            held.commit(registry, &mut blocked).err(),
            Some(VictoryError::PoliticalExecutiveUnavailable)
        );
        assert_eq!(
            before,
            bincode::serialize(&build_save(registry, &blocked).unwrap()).unwrap()
        );
        let mut restored = restore_save(registry, build_save(registry, &blocked).unwrap()).unwrap();
        for branch in [&mut blocked, &mut restored] {
            assert_eq!(
                political_certification_status(branch, owner),
                PoliticalCertificationStatus::ExecutiveUnavailable
            );
            validate_release_arrest(branch, arrest)
                .unwrap()
                .commit(branch)
                .unwrap();
            if let Some(second) = second {
                assert_eq!(
                    political_certification_status(branch, owner),
                    PoliticalCertificationStatus::ExecutiveUnavailable
                );
                validate_set_character_supervisor(branch, second, Some(boss))
                    .unwrap()
                    .commit(branch)
                    .unwrap();
            }
            assert_eq!(
                branch
                    .politics()
                    .organization(owner)
                    .unwrap()
                    .latest_election(),
                Some(ballot)
            );
            assert_eq!(
                political_certification_status(branch, owner),
                PoliticalCertificationStatus::Ready
            );
            validate_claim_victory(registry, branch, VictoryPath::PoliticalPower)
                .unwrap()
                .commit(registry, branch)
                .unwrap();
            validate_invariants(branch);
        }
        assert_eq!(
            bincode::serialize(&blocked).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
        restore_save(registry, build_save(registry, &blocked).unwrap()).unwrap();
        // A terminal snapshot with a second root contradicts the certified
        // command, even when its ballot and report are otherwise genuine.
        let event = blocked.campaign_victory().unwrap().id();
        second_executive(&mut blocked, owner);
        assert_eq!(
            validate_state(&blocked),
            Err(StateValidationError::InvalidCampaignVictory { event })
        );
    }
}
