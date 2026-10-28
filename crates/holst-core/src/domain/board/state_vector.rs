use yrs::StateVector;

/// Whether `have` holds every update `server` announces.
#[must_use]
pub fn holds(have: &StateVector, server: &StateVector) -> bool {
    server
        .iter()
        .all(|(client, clock)| have.get(client) >= *clock)
}

/// Whether `have` holds an update `server` lacks: a copy from before a
/// restored backup or from another server, which yrs would trust over the
/// server's blocks of the same ids.
///
/// ```
/// use holst_core::domain::board::state_vector::ahead;
/// use yrs::{ClientID, StateVector};
///
/// let at = |pairs: &[(u64, u32)]| {
///     StateVector::from_iter(
///         pairs.iter().map(|&(id, clock)| (ClientID::new(id), clock)),
///     )
/// };
/// assert!(!ahead(&at(&[(1, 5)]), &at(&[(1, 9), (2, 3)])));
/// assert!(ahead(&at(&[(1, 10)]), &at(&[(1, 9)])));
/// assert!(ahead(&at(&[(7, 1)]), &at(&[(1, 9)])));
/// ```
#[must_use]
pub fn ahead(have: &StateVector, server: &StateVector) -> bool {
    have.iter()
        .any(|(client, clock)| server.get(client) < *clock)
}
