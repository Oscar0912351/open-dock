use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
    thread::JoinHandle,
    time::Duration,
};

use parking_lot::{Condvar, Mutex};

/// Stop-signal for én worker-tråd. Hver start får sit eget token, så en gammel
/// tråd der stadig er ved at lukke aldrig forveksles med den nye.
#[derive(Clone, Default)]
pub struct StopToken(Arc<(Mutex<bool>, Condvar)>);

impl StopToken {
    pub fn is_stopped(&self) -> bool {
        *(self.0).0.lock()
    }

    pub fn stop(&self) {
        let (lock, cvar) = &*self.0;
        *lock.lock() = true;
        cvar.notify_all();
    }

    /// Sover op til `dur` eller til stop. Returnerer `true` hvis tråden skal stoppe.
    pub fn wait(&self, dur: Duration) -> bool {
        let (lock, cvar) = &*self.0;
        let mut stopped = lock.lock();
        if *stopped {
            return true;
        }
        let _ = cvar.wait_for(&mut stopped, dur);
        *stopped
    }
}

/// En worker-tråd joines bevidst ikke: stop kaldes fra scheduleren, som kan være
/// main-tråden, og et scan kan tage sekunder. Tokenet sikrer i stedet, at den
/// gamle tråd stopper med at publicere, så snart den observerer stop.
pub struct Worker {
    token: StopToken,
    join: JoinHandle<()>,
}

impl Worker {
    /// Starter en navngivet tråd. Panics fanges og logges under `target`.
    pub fn spawn(
        name: &str,
        target: &'static str,
        f: impl FnOnce(StopToken) + Send + 'static,
    ) -> Option<Self> {
        let token = StopToken::default();
        let thread_token = token.clone();
        let thread_name = name.to_string();
        let join = match std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| f(thread_token)));
                if let Err(payload) = result {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("ukendt panic");
                    log::error!(target: target, "{thread_name}-tråd afbrudt af panic: {msg}");
                }
            }) {
            Ok(join) => join,
            Err(err) => {
                log::warn!(target: target, "Kunne ikke starte {name}-tråd: {err}");
                return None;
            }
        };
        Some(Self { token, join })
    }

    pub fn is_finished(&self) -> bool {
        self.join.is_finished()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.token.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Instant,
    };

    #[test]
    fn wait_times_out_when_not_stopped() {
        let token = StopToken::default();
        assert!(!token.wait(Duration::from_millis(20)));
    }

    #[test]
    fn wait_returns_immediately_after_stop() {
        let token = StopToken::default();
        token.stop();
        let started = Instant::now();
        assert!(token.wait(Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn dropping_worker_stops_token() {
        let exited = Arc::new(AtomicBool::new(false));
        let exited_thread = Arc::clone(&exited);
        let worker = Worker::spawn("od-worker-test", "provider:test", move |stop| {
            while !stop.wait(Duration::from_millis(10)) {}
            exited_thread.store(true, Ordering::Release);
        });
        let Some(worker) = worker else {
            panic!("test-worker kunne ikke starte");
        };
        drop(worker);

        let deadline = Instant::now() + Duration::from_secs(1);
        while !exited.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(exited.load(Ordering::Acquire));
    }
}
