use super::*;

/// Keeps ownership alive while a single apply step waits on a long-running child.
/// A failed renewal is sticky: no subsequent checkpoint or step may proceed.
pub(super) struct LeaseHeartbeat {
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    failure: Arc<Mutex<Option<Failure>>>,
}

impl LeaseHeartbeat {
    pub(super) fn start(
        socket: &Path,
        id: &str,
        stack: &str,
        holder: &str,
        interval: Duration,
    ) -> Result<Self, Failure> {
        let params = json!({"id": id, "stack_id": stack, "holder": holder});
        // Fail before doing work when the installed supervisor lacks heartbeat support.
        rpc(socket, "stack.heartbeat", params.clone())?;
        let socket = socket.to_path_buf();
        let (stop, receiver) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let reported_failure = failure.clone();
        let worker = thread::spawn(move || {
            while matches!(
                receiver.recv_timeout(interval),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                if let Err(error) = rpc(&socket, "stack.heartbeat", params.clone()) {
                    *reported_failure.lock().unwrap() = Some(Failure::new(
                        EXIT_LEASE,
                        format!("apply lease renewal failed: {}", error.message),
                    ));
                    break;
                }
            }
        });
        Ok(Self {
            stop,
            worker: Some(worker),
            failure,
        })
    }

    pub(super) fn check(&self) -> Result<(), Failure> {
        match self.failure.lock().unwrap().clone() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    pub(super) fn stop(mut self) -> Result<(), Failure> {
        self.join();
        self.check()
    }

    fn join(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for LeaseHeartbeat {
    fn drop(&mut self) {
        self.join();
    }
}
