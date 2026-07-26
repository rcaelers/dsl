//! Compiler-owned assembly of payload inventory submissions.

use std::collections::HashSet;
use std::sync::Arc;

use logic_analyzer_graph_api::node::PayloadRegistration;
use signal_processing::PayloadRegistrationError;

use super::graph::{BuilderRegistry, PayloadSubscription};

pub(crate) fn payload_registrations() -> Vec<&'static PayloadRegistration> {
    let mut registrations = inventory::iter::<PayloadRegistration>
        .into_iter()
        .collect::<Vec<_>>();
    validate_payload_registrations(&mut registrations);
    registrations
}

pub(crate) fn apply_payload_registration(
    registration: &PayloadRegistration,
    registry: &mut BuilderRegistry,
) -> Result<(), PayloadRegistrationError> {
    let kind = registration.kind();
    kind.register_runtime_type();
    registry
        .payloads
        .register_erased(kind.type_id(), registration.stable_id())?;
    registry.payloads.register_adapter_erased(
        kind.type_id(),
        kind.name(),
        registration.adapter(),
    )?;
    registry.payload_subscriptions.push(PayloadSubscription {
        kind,
        diagnostic_name: kind.name().to_owned(),
        presentation: registration.presentation(),
        persistent_cache: registration.persistent_cache(),
        configure_request: Arc::new(registration.configure_request()),
    });
    Ok(())
}

fn validate_payload_registrations(registrations: &mut Vec<&PayloadRegistration>) {
    registrations.sort_by_key(|registration| registration.stable_id());
    let mut stable_ids = HashSet::new();
    let mut type_ids = HashSet::new();
    for registration in registrations {
        assert!(
            !registration.stable_id().trim().is_empty(),
            "payload inventory contains an empty stable ID"
        );
        assert!(
            stable_ids.insert(registration.stable_id()),
            "duplicate payload inventory stable ID '{}'",
            registration.stable_id()
        );
        assert!(
            type_ids.insert(registration.kind().type_id()),
            "duplicate payload inventory type '{}'",
            registration.kind().name()
        );
    }
}

#[cfg(test)]
mod payload_registration_tests {
    use logic_analyzer_graph_api::node_support::{
        DefaultLanePresentationDescriptor, LaneBadgeDescriptor,
    };
    use signal_processing::Sample;

    use super::*;

    fn test_presentation() -> DefaultLanePresentationDescriptor {
        DefaultLanePresentationDescriptor::new(
            LaneBadgeDescriptor::new("T", [0, 0, 0]),
            "org.logicconduit.compiler-test.renderer/v1",
        )
    }

    #[test]
    fn registrations_are_stably_ordered_and_unique() {
        let registrations = payload_registrations();
        assert!(
            registrations
                .windows(2)
                .all(|pair| pair[0].stable_id() < pair[1].stable_id())
        );
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let registration = PayloadRegistration::subscribable::<Sample>(
            "org.logicconduit.compiler-test.payload/v1",
            signal_processing::digital_payload_adapter,
            test_presentation,
        );
        let mut registrations = vec![&registration, &registration];
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                validate_payload_registrations(&mut registrations)
            }))
            .is_err()
        );
    }
}
