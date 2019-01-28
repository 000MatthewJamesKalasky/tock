use kernel::common::cells::OptionalCell;

use kernel::common::{deferred_call::DeferredCall, deferred_call_mux::*};
use crate::deferred_call_tasks::DeferredCallTask;

static DEFERRED_CALL: DeferredCall<DeferredCallTask> =
    unsafe { DeferredCall::new(DeferredCallTask::MuxBackend) };

pub static mut MUXBACKEND: Nrf52DeferredCallMuxBackend =
    Nrf52DeferredCallMuxBackend::new();

pub struct Nrf52DeferredCallMuxBackend {
    client: OptionalCell<&'static DeferredCallMuxBackendClient>,
}

impl Nrf52DeferredCallMuxBackend {
    pub const fn new() -> Nrf52DeferredCallMuxBackend {
        Nrf52DeferredCallMuxBackend {
            client: OptionalCell::empty(),
        }
    }

    pub fn handle_interrupt(&self) {
        self.client.map(|c| c.call());
    }
}

impl DeferredCallMuxBackend for Nrf52DeferredCallMuxBackend {
    fn set(&self) {
        DEFERRED_CALL.set();
    }

    fn set_client(&self, client: &'static DeferredCallMuxBackendClient) {
        self.client.set(client);
    }
}
