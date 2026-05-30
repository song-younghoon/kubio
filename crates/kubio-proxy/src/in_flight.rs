use kubio_observe::Observer;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::state::ProxyState;

pub(crate) struct ObservedInFlightPermit {
    permit: Option<OwnedSemaphorePermit>,
    semaphore: Arc<Semaphore>,
    observer: Arc<Observer>,
    max: usize,
}

impl ObservedInFlightPermit {
    pub(crate) fn new(state: &ProxyState, permit: OwnedSemaphorePermit) -> Self {
        let current = state
            .config
            .performance
            .max_in_flight_requests
            .saturating_sub(state.in_flight.available_permits());
        state
            .observer
            .record_in_flight(current, state.config.performance.max_in_flight_requests);
        Self {
            permit: Some(permit),
            semaphore: state.in_flight.clone(),
            observer: state.observer.clone(),
            max: state.config.performance.max_in_flight_requests,
        }
    }
}

impl Drop for ObservedInFlightPermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        let current = self.max.saturating_sub(self.semaphore.available_permits());
        self.observer.record_in_flight(current, self.max);
    }
}
