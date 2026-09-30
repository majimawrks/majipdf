//! One job at a time. Every long-running command (compress, merge, split, organize, word, excel)
//! takes a `JobGuard` *before* spawning its worker and keeps it until the worker has finished its
//! cleanup, so the shared `CANCEL` flag / child-process slots always belong to exactly one job.
//! A second request while one runs is rejected with `BUSY` (the UI blocks this anyway).
use crate::compress::CANCEL;
use std::sync::atomic::{AtomicBool, Ordering};

pub const BUSY: &str = "busy";
static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct JobGuard(());

/// Claims the single job slot and clears any stale cancel request.
pub fn begin() -> Result<JobGuard, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(BUSY.into());
    }
    CANCEL.store(false, Ordering::SeqCst);
    Ok(JobGuard(()))
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_slot() {
        let g = begin().unwrap();
        assert_eq!(begin().err().as_deref(), Some(BUSY));
        drop(g);
        assert!(begin().is_ok());
    }
}
