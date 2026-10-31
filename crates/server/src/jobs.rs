//! A small job queue: downloads and moves run one at a time, in order, and report live progress.

use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};

#[derive(Serialize, Clone, Default, Debug)]
pub struct JobInfo {
    pub id: u64,
    /// "download" | "move" | "single" | "collection" | "addpapers" | "update"
    pub kind: String,
    pub course: String,
    pub drive: String,
    pub title: String,
    /// queued | running | done | failed | cancelled
    pub status: String,
    pub started: u64,
    pub finished: u64,
    pub total: u32,
    pub done: u32,
    pub current_index: u32,
    pub current_title: String,
    pub percent: f32,
    pub speed: String,
    pub eta: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub failed: Vec<String>,
    pub warnings: Vec<String>,
    pub message: String,
    pub log: Vec<String>,
}

struct Slot {
    info: JobInfo,
    cancel: Arc<AtomicBool>,
}

pub type Work = Box<dyn FnOnce(&JobCtx) -> anyhow::Result<()> + Send>;

pub struct Jobs {
    inner: Arc<Mutex<Vec<Slot>>>,
    next: AtomicU64,
    tx: mpsc::Sender<(u64, Work)>,
}

pub struct JobCtx {
    inner: Arc<Mutex<Vec<Slot>>>,
    id: u64,
    pub cancel: Arc<AtomicBool>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl JobCtx {
    pub fn update(&self, f: impl FnOnce(&mut JobInfo)) {
        if let Ok(mut g) = self.inner.lock() {
            if let Some(s) = g.iter_mut().find(|s| s.info.id == self.id) {
                f(&mut s.info);
            }
        }
    }

    /// Keep the last 300 lines.
    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        self.update(|i| {
            i.log.push(line);
            if i.log.len() > 300 {
                let cut = i.log.len() - 300;
                i.log.drain(..cut);
            }
        });
    }
}

impl Jobs {
    pub fn new() -> Arc<Jobs> {
        let inner: Arc<Mutex<Vec<Slot>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel::<(u64, Work)>();
        let worker_inner = inner.clone();
        std::thread::spawn(move || {
            for (id, work) in rx {
                let cancel = {
                    let mut g = worker_inner.lock().unwrap();
                    let Some(s) = g.iter_mut().find(|s| s.info.id == id) else { continue };
                    if s.cancel.load(Ordering::SeqCst) {
                        s.info.status = "cancelled".into();
                        s.info.finished = now();
                        continue;
                    }
                    s.info.status = "running".into();
                    s.info.started = now();
                    s.cancel.clone()
                };
                let ctx = JobCtx { inner: worker_inner.clone(), id, cancel: cancel.clone() };
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctx)));
                ctx.update(|i| {
                    i.finished = now();
                    match &res {
                        Ok(Ok(())) if cancel.load(Ordering::SeqCst) => i.status = "cancelled".into(),
                        Ok(Ok(())) if !i.failed.is_empty() => i.status = "failed".into(),
                        Ok(Ok(())) => i.status = "done".into(),
                        Ok(Err(e)) => {
                            let m = format!("{e:#}");
                            i.status = if cancel.load(Ordering::SeqCst) || m.contains("cancelled") { "cancelled".into() } else { "failed".into() };
                            i.message = m;
                        }
                        Err(_) => {
                            i.status = "failed".into();
                            i.message = "the job crashed unexpectedly".into();
                        }
                    }
                });
                // keep only the newest 40 finished jobs
                let mut g = worker_inner.lock().unwrap();
                let finished: Vec<u64> = g.iter().filter(|s| s.info.status != "queued" && s.info.status != "running").map(|s| s.info.id).collect();
                if finished.len() > 40 {
                    let drop_ids: Vec<u64> = finished[..finished.len() - 40].to_vec();
                    g.retain(|s| !drop_ids.contains(&s.info.id));
                }
            }
        });
        Arc::new(Jobs { inner, next: AtomicU64::new(1), tx })
    }

    /// Queue a job. Refuses a second queued/running job for the same course and kind.
    pub fn submit(&self, mut info: JobInfo, work: Work) -> Result<u64, String> {
        let mut g = self.inner.lock().map_err(|_| "job list unavailable".to_string())?;
        if g.iter().any(|s| s.info.course == info.course && s.info.kind == info.kind && (s.info.status == "queued" || s.info.status == "running")) {
            return Err(format!("a {} job for {} is already queued or running", info.kind, info.course));
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        info.id = id;
        info.status = "queued".into();
        g.push(Slot { info, cancel: Arc::new(AtomicBool::new(false)) });
        drop(g);
        self.tx.send((id, work)).map_err(|_| "job worker stopped".to_string())?;
        Ok(id)
    }

    pub fn list(&self) -> Vec<JobInfo> {
        let g = self.inner.lock().map(|g| g.iter().map(|s| s.info.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let mut g = g;
        g.reverse(); // newest first
        g
    }

    pub fn cancel(&self, id: u64) -> bool {
        if let Ok(g) = self.inner.lock() {
            if let Some(s) = g.iter().find(|s| s.info.id == id && (s.info.status == "queued" || s.info.status == "running")) {
                s.cancel.store(true, Ordering::SeqCst);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait_for(j: &Jobs, id: u64, status: &str) -> JobInfo {
        for _ in 0..200 {
            if let Some(i) = j.list().into_iter().find(|i| i.id == id) {
                if i.status == status {
                    return i;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("job {id} never reached {status}: {:?}", j.list());
    }

    #[test]
    fn runs_in_order_reports_and_cancels() {
        let j = Jobs::new();
        let a = j.submit(JobInfo { kind: "download".into(), course: "A".into(), ..Default::default() }, Box::new(|c| { c.update(|i| i.done = 3); c.log("hello"); Ok(()) })).unwrap();
        let i = wait_for(&j, a, "done");
        assert_eq!(i.done, 3);
        assert_eq!(i.log, vec!["hello".to_string()]);
        // failure keeps the message
        let b = j.submit(JobInfo { kind: "download".into(), course: "B".into(), ..Default::default() }, Box::new(|_| anyhow::bail!("boom"))).unwrap();
        assert_eq!(wait_for(&j, b, "failed").message, "boom");
        // a long job can be cancelled, and a duplicate is refused while it runs
        let c = j.submit(JobInfo { kind: "download".into(), course: "C".into(), ..Default::default() }, Box::new(|c| { while !c.cancel.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); } Ok(()) })).unwrap();
        wait_for(&j, c, "running");
        assert!(j.submit(JobInfo { kind: "download".into(), course: "C".into(), ..Default::default() }, Box::new(|_| Ok(()))).is_err());
        assert!(j.cancel(c));
        wait_for(&j, c, "cancelled");
        // a panic does not kill the worker
        let d = j.submit(JobInfo { kind: "move".into(), course: "D".into(), ..Default::default() }, Box::new(|_| panic!("oops"))).unwrap();
        wait_for(&j, d, "failed");
        let e = j.submit(JobInfo { kind: "move".into(), course: "E".into(), ..Default::default() }, Box::new(|_| Ok(()))).unwrap();
        wait_for(&j, e, "done");
    }
}
