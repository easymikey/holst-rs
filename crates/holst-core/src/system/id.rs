use crate::domain::id_source::IdSource;

/// A random UUID v4.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn new_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[cfg(test)]
mod tests {

    use crate::{domain::id_source::IdSource, system::id::RandomIds};

    #[test]
    fn uuid_ids_are_unique_uuids() {
        let (a, b) = (RandomIds.new_id(), RandomIds.new_id());
        assert_ne!(a, b);
        assert!(crate::domain::id::is_uuid(&a), "{a}");
    }
}
