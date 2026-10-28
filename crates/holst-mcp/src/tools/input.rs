//! Checks and conversions of tool arguments. Pure: no requests, no state.

use holst_core::domain::id::is_uuid;

pub(crate) fn lower_uuid(reference: String) -> String {
    match reference {
        id if is_uuid(&id) => id.to_ascii_lowercase(),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::input::lower_uuid;

    #[test]
    fn uuids_are_lowercase() {
        let id = "AE1F0A2B-C3D4-4F19-8A7B-1C2D3E4F5A6B";
        assert_eq!(lower_uuid(id.to_owned()), id.to_ascii_lowercase());
        assert_eq!(lower_uuid("Plans".to_owned()), "Plans");
    }
}
