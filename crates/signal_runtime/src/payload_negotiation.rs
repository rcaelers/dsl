use super::ports::PortPayload;

pub(crate) fn negotiate<'a>(
    offered: &'a [PortPayload],
    accepted: &[PortPayload],
) -> Option<&'a PortPayload> {
    offered.iter().find(|offer| {
        accepted
            .iter()
            .any(|accept| accept.type_id == offer.type_id)
    })
}
