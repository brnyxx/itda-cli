use super::{BrowserCompletion, BrowserFailure, deadline, fail, remaining};
use crate::transport::{BrowserRequest, BrowserResponse, BrowserTransport};
use itda_types::error::{AppError, AppResult};
use std::{
    collections::{HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};
use url::Url;

const CAPACITY: usize = 4;
type Identity = (String, String);
type Key = (String, String, bool);

pub(super) trait Driver: Send + 'static {
    fn execute(&mut self, request: BrowserRequest) -> AppResult<BrowserResponse>;
}

impl Driver for BrowserTransport {
    fn execute(&mut self, request: BrowserRequest) -> AppResult<BrowserResponse> {
        BrowserTransport::execute(self, request)
    }
}

struct Slot<D> {
    key: Key,
    busy: bool,
    driver: Option<D>,
    touched: Instant,
}

struct State<D> {
    slots: Vec<Slot<D>>,
    epochs: HashMap<Identity, u64>,
    resetting: HashSet<Identity>,
    generation: u64,
    closing: bool,
    closed: bool,
    #[cfg(test)]
    waiters: usize,
}

pub(super) struct Pool<D> {
    state: Mutex<State<D>>,
    changed: Condvar,
    factory: Box<dyn Fn() -> D + Send + Sync>,
}

impl<D: Driver> Pool<D> {
    pub fn new(factory: impl Fn() -> D + Send + Sync + 'static) -> Self {
        Self {
            state: Mutex::new(State {
                slots: Vec::new(),
                epochs: HashMap::new(),
                resetting: HashSet::new(),
                generation: 0,
                closing: false,
                closed: false,
                #[cfg(test)]
                waiters: 0,
            }),
            changed: Condvar::new(),
            factory: Box::new(factory),
        }
    }

    pub fn execute(&self, mut request: BrowserRequest) -> Result<BrowserResponse, BrowserFailure> {
        let deadline = deadline(request.timeout).map_err(BrowserFailure::not_started)?;
        let url = Url::parse(&request.url)
            .map_err(|_| BrowserFailure::not_started(AppError::api("bad_url", 400)))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(BrowserFailure::not_started(AppError::api("bad_url", 400)));
        }
        let origin = url.origin().ascii_serialization();
        let identity = (origin.clone(), request.account_id.clone());
        let key = (origin, request.account_id.clone(), request.allow_local);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = *state.epochs.entry(identity.clone()).or_default();
        let generation = state.generation;
        let (cached, evicted) = loop {
            if state.epochs.get(&identity) != Some(&epoch)
                || state.resetting.contains(&identity)
                || state.generation != generation
                || state.closing
                || state.closed
            {
                return Err(BrowserFailure::not_started(AppError::api(
                    "browser_session_reset",
                    409,
                )));
            }
            remaining(deadline).map_err(BrowserFailure::not_started)?;
            if let Some(index) = state.slots.iter().position(|slot| slot.key == key) {
                let slot = &mut state.slots[index];
                if !slot.busy {
                    slot.busy = true;
                    break (slot.driver.take(), None);
                }
            } else if state.slots.len() < CAPACITY {
                state.slots.push(Slot {
                    key: key.clone(),
                    busy: true,
                    driver: None,
                    touched: Instant::now(),
                });
                break (None, None);
            } else if let Some(index) = state
                .slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| !slot.busy)
                .min_by_key(|(_, slot)| slot.touched)
                .map(|(index, _)| index)
            {
                let old = std::mem::replace(
                    &mut state.slots[index],
                    Slot {
                        key: key.clone(),
                        busy: true,
                        driver: None,
                        touched: Instant::now(),
                    },
                );
                break (None, old.driver);
            }
            let left = remaining(deadline).map_err(BrowserFailure::not_started)?;
            #[cfg(test)]
            {
                state.waiters += 1;
                self.changed.notify_all();
            }
            (state, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
            #[cfg(test)]
            {
                state.waiters -= 1;
            }
        };
        drop(state);
        drop(evicted);
        let mut driver = cached.unwrap_or_else(|| (self.factory)());
        let result = match remaining(deadline) {
            Ok(left) => {
                request.timeout = left;
                match catch_unwind(AssertUnwindSafe(|| driver.execute(request))) {
                    Ok(result) => result.map_err(|error| BrowserFailure {
                        error,
                        completion: BrowserCompletion::Quiescent,
                    }),
                    Err(_) => Err(BrowserFailure::unknown(fail())),
                }
            }
            Err(error) => Err(BrowserFailure::not_started(error)),
        };
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // A driver that failed mid-request is thrown away; one that never
        // started (the deadline ran out first) is as good as before.
        let discard = state.epochs.get(&identity) != Some(&epoch)
            || state.generation != generation
            || result
                .as_ref()
                .is_err_and(|failure| failure.completion != BrowserCompletion::NotStarted);
        if discard {
            drop(state);
            drop(driver);
            state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            state.slots.retain(|slot| slot.key != key);
        } else if let Some(slot) = state.slots.iter_mut().find(|slot| slot.key == key) {
            slot.driver = Some(driver);
            slot.busy = false;
            slot.touched = Instant::now();
        }
        self.changed.notify_all();
        result
    }

    pub fn reset(&self, origin: &str, account: &str, timeout: Duration) -> AppResult<()> {
        let deadline = deadline(timeout)?;
        let identity = (origin.to_string(), account.to_string());
        let matches = |key: &Key| key.0 == origin && key.1 == account;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.resetting.contains(&identity) {
            let left = remaining(deadline)?;
            (state, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
        }
        let epoch = state.epochs.entry(identity.clone()).or_default();
        *epoch = epoch.wrapping_add(1);
        state.resetting.insert(identity.clone());
        self.changed.notify_all();
        let mut idle = Vec::new();
        for slot in &mut state.slots {
            if matches(&slot.key) && !slot.busy {
                slot.busy = true;
                idle.push((slot.key.clone(), slot.driver.take()));
            }
        }
        drop(state);
        let keys: Vec<_> = idle.iter().map(|(key, _)| key.clone()).collect();
        drop(idle);
        state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.slots.retain(|slot| !keys.contains(&slot.key));
        let result = loop {
            if !state.slots.iter().any(|slot| matches(&slot.key)) {
                break Ok(());
            }
            let left = match remaining(deadline) {
                Ok(left) => left,
                Err(error) => break Err(error),
            };
            (state, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
        };
        state.resetting.remove(&identity);
        self.changed.notify_all();
        result
    }

    pub fn shutdown(&self, timeout: Duration, permanent: bool) -> AppResult<()> {
        let deadline = deadline(timeout)?;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.closing {
            let left = remaining(deadline)?;
            (state, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
        }
        state.closing = true;
        state.generation = state.generation.wrapping_add(1);
        self.changed.notify_all();
        let mut idle = Vec::new();
        for slot in &mut state.slots {
            if !slot.busy {
                slot.busy = true;
                idle.push((slot.key.clone(), slot.driver.take()));
            }
        }
        drop(state);
        let keys: Vec<_> = idle.iter().map(|(key, _)| key.clone()).collect();
        drop(idle);
        state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.slots.retain(|slot| !keys.contains(&slot.key));
        let result = loop {
            if state.slots.is_empty() {
                break Ok(());
            }
            let left = match remaining(deadline) {
                Ok(left) => left,
                Err(error) => break Err(error),
            };
            (state, _) = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner());
        };
        state.closed |= permanent;
        state.closing = false;
        self.changed.notify_all();
        result
    }

    #[cfg(test)]
    pub fn wait_for_waiters(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.lock().expect("state");
        while state.waiters < count {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("waiter deadline");
            (state, _) = self.changed.wait_timeout(state, left).expect("wait");
        }
    }
}
