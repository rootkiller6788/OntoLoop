use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyscallKind {
    CapabilityCall,
}

#[derive(Debug, Clone)]
pub struct SyscallRequest {
    pub syscall_id: u64,
    pub step_id: String,
    pub kind: SyscallKind,
    pub capability: String,
    pub input: String,
    pub priority: u8,
    pub seq: u64,
}

#[derive(Debug, Clone)]
pub struct SyscallResponse {
    pub syscall_id: u64,
    pub step_id: String,
    pub output: String,
    pub cost_units: u64,
}

pub trait SyscallQueue: Send + Sync {
    fn enqueue(&self, req: SyscallRequest) -> Result<()>;
    fn dequeue(&self) -> Option<SyscallRequest>;
    fn is_empty(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct InMemorySyscallQueue {
    queue: Mutex<VecDeque<SyscallRequest>>,
}

impl SyscallQueue for InMemorySyscallQueue {
    fn enqueue(&self, req: SyscallRequest) -> Result<()> {
        let mut q = self
            .queue
            .lock()
            .map_err(|_| anyhow!("syscall queue lock poisoned"))?;
        q.push_back(req);
        Ok(())
    }

    fn dequeue(&self) -> Option<SyscallRequest> {
        let mut q = self.queue.lock().ok()?;
        if q.is_empty() {
            return None;
        }
        let mut best_idx = 0usize;
        for (i, item) in q.iter().enumerate() {
            let best = &q[best_idx];
            let higher = item.priority > best.priority;
            let equal_older = item.priority == best.priority && item.seq < best.seq;
            if higher || equal_older {
                best_idx = i;
            }
        }
        q.remove(best_idx)
    }

    fn is_empty(&self) -> bool {
        self.queue.lock().map(|q| q.is_empty()).unwrap_or(true)
    }
}

pub struct SyscallScheduler {
    queue: Arc<dyn SyscallQueue>,
    counter: Mutex<u64>,
}

impl SyscallScheduler {
    pub fn new(queue: Arc<dyn SyscallQueue>) -> Self {
        Self {
            queue,
            counter: Mutex::new(0),
        }
    }

    pub fn submit(
        &self,
        step_id: String,
        capability: String,
        input: String,
        priority: u8,
    ) -> Result<()> {
        let mut c = self
            .counter
            .lock()
            .map_err(|_| anyhow!("counter lock poisoned"))?;
        *c += 1;
        let req = SyscallRequest {
            syscall_id: *c,
            step_id,
            kind: SyscallKind::CapabilityCall,
            capability,
            input,
            priority,
            seq: *c,
        };
        self.queue.enqueue(req)
    }

    pub fn drain<F>(&self, mut executor: F) -> Result<Vec<SyscallResponse>>
    where
        F: FnMut(&SyscallRequest) -> Result<(String, u64)>,
    {
        let mut out = Vec::new();
        while let Some(req) = self.queue.dequeue() {
            let (output, cost_units) = executor(&req)?;
            out.push(SyscallResponse {
                syscall_id: req.syscall_id,
                step_id: req.step_id,
                output,
                cost_units,
            });
        }
        Ok(out)
    }
}
