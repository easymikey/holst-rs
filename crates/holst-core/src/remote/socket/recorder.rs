use std::sync::{Mutex, PoisonError};

use futures_util::future;
use yrs::{Doc, Transact as _, Update, updates::decoder::Decode as _};

use crate::{
    domain::{board::EditRight, change::receipt::Delivery},
    effect::{
        board_write::{LoadSource, OpenedBoard, WriteInterpreter},
        board_write_plan::{Release, WritePlan},
        http::BoxFuture,
    },
    error::{Error, Result},
    remote::{client::Client, socket::write::with_free_client_id},
};

/// A board in memory for tests: records each plan, acks and keeps the released.
#[derive(Debug)]
pub(crate) struct WriteRecorder {
    right: EditRight,
    plans: Mutex<Vec<WritePlan>>,
}

impl WriteRecorder {
    #[must_use]
    pub(crate) const fn new(right: EditRight) -> Self {
        Self {
            right,
            plans: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub(crate) fn plans(&self) -> Vec<WritePlan> {
        self.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl WriteInterpreter for WriteRecorder {
    type Connection = ();

    fn open<'a>(
        &'a self,
        _client: &'a Client,
        _board: &'a str,
        seed: Doc,
    ) -> BoxFuture<'a, Result<OpenedBoard<()>>> {
        let plans = self.plans();
        let released: Vec<&[u8]> = plans
            .iter()
            .filter(|plan| plan.release == Release::Released)
            .map(|plan| plan.change.update.as_slice())
            .collect();
        let loaded = released
            .iter()
            .try_for_each(|update| {
                seed.transact_mut()
                    .apply_update(Update::decode_v2(update)?)
                    .map_err(Error::from)
            })
            .and_then(|()| with_free_client_id(&seed));
        let bytes = released.iter().map(|update| update.len()).sum::<usize>();
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        Box::pin(future::ready(loaded.map(|doc| OpenedBoard {
            connection: (),
            document: doc,
            right: self.right,
            source: LoadSource::Downloaded { bytes },
        })))
    }

    fn carry<'a>(
        &'a self,
        _client: &'a Client,
        (): (),
        plan: &'a WritePlan,
    ) -> BoxFuture<'a, Result<Delivery>> {
        self.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(plan.clone());
        let delivery = match plan.release {
            Release::Released => Delivery::Accepted,
            Release::Held => Delivery::Built,
        };
        Box::pin(future::ready(Ok(delivery)))
    }
}
