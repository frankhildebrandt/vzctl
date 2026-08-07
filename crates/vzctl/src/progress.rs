use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::Print,
    terminal::{self, Clear, ClearType},
    QueueableCommand,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const APPLY_STEPS: &[&str] = &[
    "validate",
    "acquire_lease",
    "ensure_nets",
    "ensure_dns",
    "ensure_ca",
    "ensure_images",
    "attach_nets",
    "ensure_vms",
    "start_helpers",
    "await_agents",
    "await_cloud_init",
    "ensure_guest_utils",
    "ensure_docker_project_mount",
    "ensure_oidc",
    "ensure_ingress",
    "ensure_ca_rollout",
    "ensure_oidc_inject",
    "ensure_docker_context",
    "ensure_containers",
    "ensure_ports",
    "apply_routes_policies",
    "release_lease",
];

pub const DOWN_STEPS: &[&str] = &[
    "purge_ingress",
    "purge_dns_records",
    "purge_oidc",
    "stop_helpers",
    "detach_nets",
    "destroy_managed",
    "purge_docker_context",
    "purge_ports",
    "dns_cleanup",
    "release_lease",
];

const MAX_LOG_LINES: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressMode {
    Off,
    Plain,
    Ui,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobSpec {
    pub id: String,
    pub parent: Option<String>,
    pub label: String,
    pub units: u32,
}

impl JobSpec {
    fn group(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            parent: None,
            label: label.to_string(),
            units: 0,
        }
    }

    fn leaf(
        id: impl Into<String>,
        parent: impl Into<String>,
        label: impl Into<String>,
        units: u32,
    ) -> Self {
        Self {
            id: id.into(),
            parent: Some(parent.into()),
            label: label.into(),
            units,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ProgressMessage {
    Plan(Vec<JobSpec>),
    JobStart {
        id: String,
    },
    JobProgress {
        id: String,
        completed: u32,
        total: u32,
        detail: Option<String>,
    },
    JobDetail {
        id: String,
        key: String,
        value: String,
    },
    JobDone {
        id: String,
        elapsed: Duration,
    },
    JobSkipped {
        id: String,
        detail: String,
    },
    JobFailed {
        id: String,
        message: String,
    },
    Log {
        job_id: Option<String>,
        line: String,
    },
    VmState {
        vm_id: String,
        state: String,
    },
    Finished {
        ok: bool,
        message: String,
    },
}

#[derive(Clone, Debug)]
struct JobState {
    spec: JobSpec,
    status: JobStatus,
    progress: Option<(u32, u32)>,
    detail: Option<String>,
    details: Vec<(String, String)>,
    started: Option<Instant>,
    elapsed: Option<Duration>,
}

impl JobState {
    fn new(spec: JobSpec) -> Self {
        Self {
            spec,
            status: JobStatus::Pending,
            progress: None,
            detail: None,
            details: Vec::new(),
            started: None,
            elapsed: None,
        }
    }

    fn fraction(&self) -> f64 {
        match self.status {
            JobStatus::Done | JobStatus::Skipped => 1.0,
            JobStatus::Running => self
                .progress
                .filter(|(_, total)| *total > 0)
                .map(|(completed, total)| (completed.min(total) as f64) / (total as f64))
                .unwrap_or(0.0),
            JobStatus::Failed => self
                .progress
                .filter(|(_, total)| *total > 0)
                .map(|(completed, total)| (completed.min(total) as f64) / (total as f64))
                .unwrap_or(0.0),
            JobStatus::Pending => 0.0,
        }
    }
}

#[derive(Clone, Debug)]
struct ProgressState {
    jobs: BTreeMap<String, JobState>,
    order: Vec<String>,
    logs: Vec<(Option<String>, String)>,
    current: Option<String>,
    finished: Option<(bool, String)>,
    started: Instant,
}

impl ProgressState {
    fn new(specs: Vec<JobSpec>) -> Self {
        let mut state = Self {
            jobs: BTreeMap::new(),
            order: Vec::new(),
            logs: Vec::new(),
            current: None,
            finished: None,
            started: Instant::now(),
        };
        state.install_specs(specs);
        state
    }

    fn install_specs(&mut self, specs: Vec<JobSpec>) {
        for spec in specs {
            if !self.jobs.contains_key(&spec.id) {
                self.order.push(spec.id.clone());
            }
            self.jobs
                .entry(spec.id.clone())
                .or_insert_with(|| JobState::new(spec));
        }
    }

    fn apply(&mut self, message: ProgressMessage) {
        match message {
            ProgressMessage::Plan(specs) => self.install_specs(specs),
            ProgressMessage::JobStart { id } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.status = JobStatus::Running;
                    job.started.get_or_insert_with(Instant::now);
                    self.current = Some(id);
                }
            }
            ProgressMessage::JobProgress {
                id,
                completed,
                total,
                detail,
            } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.status = JobStatus::Running;
                    job.started.get_or_insert_with(Instant::now);
                    job.progress = (total > 0).then_some((completed.min(total), total));
                    if detail.is_some() {
                        job.detail = detail;
                    }
                    self.current = Some(id);
                }
            }
            ProgressMessage::JobDetail { id, key, value } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    if let Some(existing) = job.details.iter_mut().find(|(name, _)| name == &key) {
                        existing.1 = value;
                    } else {
                        job.details.push((key, value));
                    }
                }
            }
            ProgressMessage::JobDone { id, elapsed } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.status = JobStatus::Done;
                    job.progress = Some((1, 1));
                    job.elapsed = Some(elapsed);
                }
                if self.current.as_deref() == Some(id.as_str()) {
                    self.current = self.last_running_job();
                }
            }
            ProgressMessage::JobSkipped { id, detail } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.status = JobStatus::Skipped;
                    job.progress = Some((1, 1));
                    job.detail = Some(detail);
                }
                if self.current.as_deref() == Some(id.as_str()) {
                    self.current = self.last_running_job();
                }
            }
            ProgressMessage::JobFailed { id, message } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.status = JobStatus::Failed;
                    job.detail = Some(message);
                    job.elapsed = job.started.map(|started| started.elapsed());
                }
                self.current = Some(id);
            }
            ProgressMessage::Log { job_id, line } => self.push_log(job_id, line),
            ProgressMessage::VmState { vm_id, state } => {
                self.push_log(None, format!("vm {vm_id} → {state}"));
            }
            ProgressMessage::Finished { ok, message } => self.finished = Some((ok, message)),
        }
    }

    fn push_log(&mut self, job_id: Option<String>, line: String) {
        if line.is_empty() {
            return;
        }
        self.logs.push((job_id, line));
        if self.logs.len() > MAX_LOG_LINES {
            self.logs.drain(0..self.logs.len() - MAX_LOG_LINES);
        }
    }

    fn percent(&self) -> u8 {
        let total = self
            .jobs
            .values()
            .filter(|job| job.spec.units > 0)
            .map(|job| job.spec.units as f64)
            .sum::<f64>();
        if total == 0.0 {
            return 0;
        }
        let done = self
            .jobs
            .values()
            .filter(|job| job.spec.units > 0)
            .map(|job| (job.spec.units as f64) * job.fraction())
            .sum::<f64>();
        ((done / total) * 100.0).floor().clamp(0.0, 100.0) as u8
    }

    fn job_counts(&self) -> (usize, usize) {
        let jobs = self.jobs.values().filter(|job| job.spec.units > 0);
        let total = jobs.clone().count();
        let done = jobs
            .filter(|job| matches!(job.status, JobStatus::Done | JobStatus::Skipped))
            .count();
        (done, total)
    }

    fn group_status(&self, group_id: &str) -> JobStatus {
        let children = self
            .jobs
            .iter()
            .filter(|(_, job)| job.spec.parent.as_deref() == Some(group_id))
            .map(|(id, _)| self.effective_status(id))
            .collect::<Vec<_>>();
        if children.contains(&JobStatus::Failed) {
            JobStatus::Failed
        } else if children.contains(&JobStatus::Running) {
            JobStatus::Running
        } else if !children.is_empty()
            && children
                .iter()
                .all(|status| matches!(status, JobStatus::Done | JobStatus::Skipped))
        {
            JobStatus::Done
        } else {
            JobStatus::Pending
        }
    }

    fn effective_status(&self, id: &str) -> JobStatus {
        self.jobs.get(id).map_or(JobStatus::Pending, |job| {
            if job.spec.units == 0 {
                self.group_status(id)
            } else {
                job.status
            }
        })
    }

    fn last_running_job(&self) -> Option<String> {
        self.order.iter().rev().find_map(|id| {
            self.jobs
                .get(id)
                .is_some_and(|job| job.spec.units > 0 && job.status == JobStatus::Running)
                .then(|| id.clone())
        })
    }
}

pub struct ProgressReporter {
    mode: ProgressMode,
    tx: Option<Sender<ProgressMessage>>,
    state: ProgressState,
    job_started: BTreeMap<String, Instant>,
    current: Option<String>,
    plain_percent: Arc<AtomicU8>,
}

impl ProgressReporter {
    pub fn new(mode: ProgressMode, tx: Option<Sender<ProgressMessage>>, steps: &[&str]) -> Self {
        let specs = default_job_specs(steps);
        if let Some(sender) = &tx {
            let _ = sender.send(ProgressMessage::Plan(specs.clone()));
        }
        Self {
            mode,
            tx,
            state: ProgressState::new(specs),
            job_started: BTreeMap::new(),
            current: None,
            plain_percent: Arc::new(AtomicU8::new(0)),
        }
    }

    pub fn enabled(&self) -> bool {
        self.mode != ProgressMode::Off
    }

    pub fn pipe_subprocess_stderr(&self) -> bool {
        self.mode != ProgressMode::Off
    }

    pub fn plain_percent(&self) -> Option<u8> {
        (self.mode == ProgressMode::Plain).then(|| self.state.percent())
    }

    pub fn add_jobs(&mut self, specs: Vec<JobSpec>) {
        if specs.is_empty() {
            return;
        }
        self.state.install_specs(specs.clone());
        self.sync_plain_percent();
        self.emit(ProgressMessage::Plan(specs));
    }

    pub fn add_vm_jobs(&mut self, vm_ids: &[String]) {
        let mut specs = Vec::new();
        for vm_id in vm_ids {
            let parent = format!("vm:{vm_id}");
            specs.push(JobSpec {
                id: parent.clone(),
                parent: Some("phase-vms".to_string()),
                label: vm_id.clone(),
                units: 0,
            });
            for (suffix, label, units) in [
                ("create", "VM erstellen", 2),
                ("start", "Helper starten", 1),
                ("agent", "Guest Agent", 1),
                ("cloud-init", "Cloud-init provisioning", 4),
            ] {
                specs.push(JobSpec::leaf(
                    format!("{parent}:{suffix}"),
                    parent.clone(),
                    label,
                    units,
                ));
            }
        }
        self.add_jobs(specs);
    }

    pub fn step_start(&mut self, step: &str) {
        self.job_start(&format!("step:{step}"));
    }

    pub fn step_done(&mut self, step: &str) {
        self.job_done(&format!("step:{step}"));
    }

    pub fn job_start(&mut self, id: &str) {
        self.job_started.insert(id.to_string(), Instant::now());
        self.current = Some(id.to_string());
        self.publish(ProgressMessage::JobStart { id: id.to_string() });
        self.plain_line("→", id, None);
    }

    pub fn job_progress(&mut self, id: &str, completed: u32, total: u32, detail: Option<String>) {
        self.current = Some(id.to_string());
        let plain_detail = detail.clone();
        self.publish(ProgressMessage::JobProgress {
            id: id.to_string(),
            completed,
            total,
            detail,
        });
        if self.mode == ProgressMode::Plain {
            if let Some(detail) = plain_detail {
                let label = job_path(&self.state.jobs, id);
                eprintln!(
                    "[{}] {:>3}%   {label}: {detail}",
                    wall_clock(),
                    self.state.percent()
                );
            }
        }
    }

    pub fn job_detail(&mut self, id: &str, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        self.publish(ProgressMessage::JobDetail {
            id: id.to_string(),
            key: key.clone(),
            value: value.clone(),
        });
        if self.mode == ProgressMode::Plain {
            eprintln!(
                "[{}] {:>3}%   {} · {key}: {value}",
                wall_clock(),
                self.state.percent(),
                job_path(&self.state.jobs, id)
            );
        }
    }

    pub fn job_done(&mut self, id: &str) {
        let elapsed = self
            .job_started
            .remove(id)
            .map(|started| started.elapsed())
            .unwrap_or_default();
        self.publish(ProgressMessage::JobDone {
            id: id.to_string(),
            elapsed,
        });
        self.plain_line("✓", id, Some(format_elapsed(elapsed)));
        if self.current.as_deref() == Some(id) {
            self.current = None;
        }
    }

    pub fn job_skip(&mut self, id: &str, detail: impl Into<String>) {
        let detail = detail.into();
        self.publish(ProgressMessage::JobSkipped {
            id: id.to_string(),
            detail: detail.clone(),
        });
        self.plain_line("–", id, Some(detail));
    }

    pub fn job_fail(&mut self, id: &str, message: impl Into<String>) {
        let message = message.into();
        let elapsed = self
            .job_started
            .remove(id)
            .map(|started| started.elapsed())
            .unwrap_or_default();
        self.publish(ProgressMessage::JobFailed {
            id: id.to_string(),
            message: message.clone(),
        });
        self.plain_line(
            "✗",
            id,
            Some(format!("{message} · {}", format_elapsed(elapsed))),
        );
    }

    pub fn log(&mut self, line: impl Into<String>) {
        let line = line.into();
        if line.is_empty() {
            return;
        }
        let job_id = self.current.clone();
        self.publish(ProgressMessage::Log {
            job_id,
            line: line.clone(),
        });
        if self.mode == ProgressMode::Plain {
            let hierarchy = self
                .current
                .as_deref()
                .map(|id| format!("{} · ", job_path(&self.state.jobs, id)))
                .unwrap_or_default();
            eprintln!(
                "[{}] {:>3}%   {hierarchy}{line}",
                wall_clock(),
                self.state.percent()
            );
        }
    }

    pub fn log_sender(&self) -> Option<Sender<ProgressMessage>> {
        self.tx.clone()
    }

    pub fn current_job_id(&self) -> Option<String> {
        self.current.clone()
    }

    pub fn current_job_path(&self) -> Option<String> {
        self.current
            .as_deref()
            .map(|id| job_path(&self.state.jobs, id))
    }

    pub fn percent_handle(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.plain_percent)
    }

    pub fn finished(&mut self, ok: bool, message: impl Into<String>) {
        let message = message.into();
        self.publish(ProgressMessage::Finished {
            ok,
            message: message.clone(),
        });
        if self.mode == ProgressMode::Plain {
            let mark = if ok { "✓" } else { "✗" };
            eprintln!(
                "[{}] {:>3}% {mark} {message}",
                wall_clock(),
                self.state.percent()
            );
        }
    }

    fn publish(&mut self, message: ProgressMessage) {
        self.state.apply(message.clone());
        self.sync_plain_percent();
        self.emit(message);
    }

    fn sync_plain_percent(&self) {
        self.plain_percent
            .store(self.state.percent(), Ordering::Relaxed);
    }

    fn emit(&self, message: ProgressMessage) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(message);
        }
    }

    fn plain_line(&self, mark: &str, id: &str, suffix: Option<String>) {
        if self.mode != ProgressMode::Plain {
            return;
        }
        let label = job_path(&self.state.jobs, id);
        let suffix = suffix
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        eprintln!(
            "[{}] {:>3}% {mark} {label}{suffix}",
            wall_clock(),
            self.state.percent()
        );
    }
}

pub fn resolve_progress_mode(
    explicit: Option<ProgressMode>,
    human_format: bool,
    dashboard_by_default: bool,
) -> ProgressMode {
    resolve_progress_mode_for(
        explicit,
        human_format,
        dashboard_by_default,
        io::stderr().is_terminal(),
    )
}

fn resolve_progress_mode_for(
    explicit: Option<ProgressMode>,
    human_format: bool,
    dashboard_by_default: bool,
    interactive: bool,
) -> ProgressMode {
    if let Some(mode) = explicit {
        return mode;
    }
    if !human_format {
        return ProgressMode::Off;
    }
    if dashboard_by_default && interactive {
        ProgressMode::Ui
    } else {
        ProgressMode::Plain
    }
}

pub fn ui_available() -> bool {
    io::stderr().is_terminal() && io::stdin().is_terminal()
}

pub fn parse_progress_flag(value: Option<&str>) -> Result<ProgressMode, String> {
    match value {
        None => Err("--progress requires plain, ui, or off".to_string()),
        Some("plain") => Ok(ProgressMode::Plain),
        Some("ui") | Some("tui") => Ok(ProgressMode::Ui),
        Some("off") | Some("false") | Some("0") => Ok(ProgressMode::Off),
        Some(other) => Err(format!(
            "unsupported --progress value: {other} (expected plain, ui, or off)"
        )),
    }
}

pub struct EventListener {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for EventListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn spawn_event_listener(
    socket_path: &Path,
    tx: Option<Sender<ProgressMessage>>,
    plain_percent: Option<Arc<AtomicU8>>,
) -> EventListener {
    let path = socket_path.to_path_buf();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        if let Ok(mut stream) = UnixStream::connect(&path) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "events.subscribe",
                "params": {"filter": "vm.state"},
                "id": 1
            });
            if writeln!(stream, "{request}").is_err() {
                return;
            }
            let mut reader = BufReader::new(stream);
            let mut ack = String::new();
            if reader.read_line(&mut ack).is_err() {
                return;
            }
            while !worker_stop.load(Ordering::Relaxed) {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let Ok(event) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        if event["type"] != "vm.state" {
                            continue;
                        }
                        let data = &event["data"];
                        let vm_id = data["vm_id"].as_str().unwrap_or_default();
                        let state = data["state"].as_str().unwrap_or_default();
                        if vm_id.is_empty() || state.is_empty() {
                            continue;
                        }
                        if let Some(tx) = &tx {
                            let _ = tx.send(ProgressMessage::VmState {
                                vm_id: vm_id.to_string(),
                                state: state.to_string(),
                            });
                        } else {
                            let percent = plain_percent
                                .as_ref()
                                .map(|value| value.load(Ordering::Relaxed))
                                .unwrap_or(0);
                            eprintln!(
                                "[{}] {:>3}%   VM-Provisioning / {vm_id} / Zustand: {state}",
                                wall_clock(),
                                percent
                            );
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock
                                | io::ErrorKind::TimedOut
                                | io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => break,
                }
            }
        }
    });
    EventListener {
        stop,
        handle: Some(handle),
    }
}

pub fn run_ui_session(
    title: &str,
    rx: Receiver<ProgressMessage>,
    apply: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let mut guard = UiGuard::enter().map_err(|error| format!("terminal setup: {error}"))?;
    let (done_tx, done_rx) = mpsc::channel();
    let apply_thread = thread::spawn(move || {
        let result = apply();
        let _ = done_tx.send(result);
    });

    let mut ui = UiSession::new(title);
    let mut plain_follow = false;
    let result = loop {
        match done_rx.try_recv() {
            Ok(result) => {
                while let Ok(message) = rx.try_recv() {
                    if plain_follow {
                        print_plain_message(&mut ui.state, message);
                    } else {
                        ui.apply(message);
                    }
                }
                break result;
            }
            Err(mpsc::TryRecvError::Disconnected) => break Err("apply worker stopped".to_string()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(message) => {
                if plain_follow {
                    print_plain_message(&mut ui.state, message);
                } else {
                    ui.apply(message);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !plain_follow {
            ui.draw().map_err(|error| error.to_string())?;
            if event::poll(Duration::from_millis(1)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if is_ctrl_c(&key) {
                        guard.leave()?;
                        plain_follow = true;
                        eprintln!(
                            "[{}] Dashboard verlassen; Apply läuft im Vordergrund weiter. Ctrl-C beendet.",
                            wall_clock()
                        );
                    } else {
                        ui.handle_key(key);
                    }
                }
            }
        }
    };
    let _ = apply_thread.join();
    if !plain_follow {
        ui.draw_final(&result).map_err(|error| error.to_string())?;
    } else {
        let mark = if result.is_ok() { "✓" } else { "✗" };
        let message = result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("Fertig");
        eprintln!("[{}] {mark} {message}", wall_clock());
    }
    result
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

struct UiSession {
    state: ProgressState,
    title: String,
    selected: usize,
    expanded: BTreeSet<String>,
    show_log: bool,
}

impl UiSession {
    fn new(title: &str) -> Self {
        Self {
            state: ProgressState::new(Vec::new()),
            title: title.to_string(),
            selected: 0,
            expanded: BTreeSet::from(["phase-vms".to_string()]),
            show_log: false,
        }
    }

    fn apply(&mut self, message: ProgressMessage) {
        let started_id = if let ProgressMessage::JobStart { id } = &message {
            if let Some(parent) = self
                .state
                .jobs
                .get(id)
                .and_then(|job| job.spec.parent.clone())
            {
                self.expanded.insert(parent);
            }
            Some(id.clone())
        } else {
            None
        };
        self.state.apply(message);
        let visible = self.visible_jobs();
        if let Some(position) = started_id.and_then(|id| visible.iter().position(|job| job == &id))
        {
            self.selected = position;
        } else if self.selected >= visible.len() {
            self.selected = visible.len().saturating_sub(1);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let len = self.visible_jobs().len();
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if len > 0 => self.selected = (self.selected + 1).min(len - 1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(id) = self.visible_jobs().get(self.selected).cloned() {
                    if self.expanded.contains(&id) {
                        self.expanded.remove(&id);
                    } else {
                        self.expanded.insert(id);
                    }
                }
            }
            KeyCode::Char('l') | KeyCode::Char('L') => self.show_log = !self.show_log,
            _ => {}
        }
    }

    fn visible_jobs(&self) -> Vec<String> {
        let mut result = Vec::new();
        fn append(
            state: &ProgressState,
            expanded: &BTreeSet<String>,
            id: &str,
            result: &mut Vec<String>,
        ) {
            result.push(id.to_string());
            if !expanded.contains(id) {
                return;
            }
            let children = state
                .order
                .iter()
                .filter(|child_id| {
                    state
                        .jobs
                        .get(*child_id)
                        .and_then(|job| job.spec.parent.as_deref())
                        == Some(id)
                })
                .cloned()
                .collect::<Vec<_>>();
            for child in children {
                append(state, expanded, &child, result);
            }
        }
        for id in &self.state.order {
            if self
                .state
                .jobs
                .get(id)
                .is_some_and(|job| job.spec.parent.is_none())
            {
                append(&self.state, &self.expanded, id, &mut result);
            }
        }
        result
    }

    fn draw(&self) -> io::Result<()> {
        let (width, height) = terminal::size().unwrap_or((100, 30));
        let lines = self.render_lines(width as usize, height as usize);
        let mut out = io::stderr();
        execute!(out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        for line in lines {
            out.queue(Print(line))?.queue(Print("\r\n"))?;
        }
        out.flush()
    }

    fn render_lines(&self, width: usize, height: usize) -> Vec<String> {
        let width = width.max(32);
        let compact = width < 80 || height < 24;
        let percent = self.state.percent();
        let (done, total) = self.state.job_counts();
        let mut lines = vec![truncate(
            &format!(
                "◆ vzctl {}  {}  seit {}",
                self.title,
                wall_clock(),
                format_clock(self.state.started.elapsed())
            ),
            width,
        )];
        let bar_width = width.saturating_sub(30).clamp(10, 50);
        lines.push(format!(
            "{} {:>3}%  {done}/{total} Jobs",
            progress_bar(percent, bar_width),
            percent
        ));
        lines.push(String::new());

        if compact {
            if let Some(current) = self
                .state
                .current
                .as_ref()
                .and_then(|id| self.state.jobs.get(id))
            {
                lines.push(truncate(
                    &format!("{} {}", status_mark(current.status), current.spec.label),
                    width,
                ));
                if let Some(detail) = &current.detail {
                    lines.push(truncate(&format!("  {detail}"), width));
                }
            }
            lines.push(String::new());
            lines.push("Log".to_string());
            let available = height.saturating_sub(lines.len() + 2).min(8);
            for (_, line) in self.state.logs.iter().rev().take(available).rev() {
                lines.push(truncate(&format!("  {line}"), width));
            }
            lines.push("Ctrl-C: zu Plain wechseln".to_string());
            return lines.into_iter().take(height).collect();
        }

        let visible = self.visible_jobs();
        let split = (width * 52 / 100).clamp(38, width.saturating_sub(28));
        let detail_width = width.saturating_sub(split + 3);
        let selected_id = visible.get(self.selected);
        let detail_lines = self.detail_lines(selected_id, detail_width, height.saturating_sub(6));
        let max_rows = height.saturating_sub(6);
        let start_row = self.selected.saturating_sub(max_rows.saturating_sub(1));
        for row in 0..max_rows {
            let visible_index = start_row + row;
            let left = visible
                .get(visible_index)
                .and_then(|id| self.state.jobs.get(id).map(|job| (id, job)))
                .map(|(id, job)| {
                    let depth = job_depth(&self.state.jobs, id);
                    let status = if job.spec.units == 0 {
                        self.state.group_status(id)
                    } else {
                        job.status
                    };
                    let cursor = if visible_index == self.selected {
                        "›"
                    } else {
                        " "
                    };
                    let progress = job
                        .progress
                        .filter(|(_, total)| *total > 0 && job.status == JobStatus::Running)
                        .map(|(done, total)| format!(" {:>3}%", done.saturating_mul(100) / total))
                        .unwrap_or_default();
                    format!(
                        "{cursor} {}{} {}{progress}",
                        "  ".repeat(depth),
                        status_mark(status),
                        job.spec.label
                    )
                })
                .unwrap_or_default();
            let right = detail_lines.get(row).cloned().unwrap_or_default();
            lines.push(format!(
                "{:<left_width$} │ {}",
                truncate(&left, split),
                truncate(&right, detail_width),
                left_width = split
            ));
        }
        lines.push(truncate(
            "↑↓ wählen · Enter auf-/zuklappen · L Job-Log · Ctrl-C Plain",
            width,
        ));
        lines.into_iter().take(height).collect()
    }

    fn detail_lines(&self, selected_id: Option<&String>, width: usize, max: usize) -> Vec<String> {
        let Some(id) = selected_id else {
            return vec!["Details".to_string()];
        };
        let Some(job) = self.state.jobs.get(id) else {
            return Vec::new();
        };
        let status = if job.spec.units == 0 {
            self.state.group_status(id)
        } else {
            job.status
        };
        let mut lines = vec![
            job.spec.label.clone(),
            format!("Status: {}", status_label(status)),
        ];
        if let Some(elapsed) = job
            .elapsed
            .or_else(|| job.started.map(|start| start.elapsed()))
        {
            lines.push(format!("Dauer: {}", format_clock(elapsed)));
        }
        if let Some(detail) = &job.detail {
            lines.push(detail.clone());
        }
        for (key, value) in &job.details {
            lines.push(format!("{key}: {value}"));
        }
        if self.show_log {
            lines.push(String::new());
            lines.push("Job-Log".to_string());
            let job_logs = self
                .state
                .logs
                .iter()
                .filter(|(job_id, _)| job_id.as_deref() == Some(id.as_str()))
                .rev()
                .take(max.saturating_sub(lines.len()))
                .collect::<Vec<_>>();
            for (_, line) in job_logs.into_iter().rev() {
                lines.push(line.clone());
            }
        }
        lines
            .into_iter()
            .take(max)
            .map(|line| truncate(&line, width))
            .collect()
    }

    fn draw_final(&mut self, result: &Result<(), String>) -> io::Result<()> {
        if let Err(message) = result {
            if let Some(id) = self.state.current.clone() {
                self.state.apply(ProgressMessage::JobFailed {
                    id,
                    message: message.clone(),
                });
            }
        }
        self.draw()?;
        thread::sleep(Duration::from_millis(250));
        Ok(())
    }
}

pub struct UiGuard {
    active: bool,
}

impl UiGuard {
    pub fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut out = io::stderr();
        execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
        out.flush()?;
        Ok(Self { active: true })
    }

    fn leave(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let mut out = io::stderr();
        execute!(out, cursor::Show, terminal::LeaveAlternateScreen)
            .map_err(|error| error.to_string())?;
        terminal::disable_raw_mode().map_err(|error| error.to_string())?;
        out.flush().map_err(|error| error.to_string())?;
        self.active = false;
        Ok(())
    }
}

impl Drop for UiGuard {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

fn print_plain_message(state: &mut ProgressState, message: ProgressMessage) {
    state.apply(message.clone());
    match message {
        ProgressMessage::JobStart { id } => {
            if state.jobs.contains_key(&id) {
                eprintln!(
                    "[{}] {:>3}% → {}",
                    wall_clock(),
                    state.percent(),
                    job_path(&state.jobs, &id)
                );
            }
        }
        ProgressMessage::JobProgress { id, detail, .. } => {
            if state.jobs.contains_key(&id) {
                let Some(detail) = detail else { return };
                eprintln!(
                    "[{}] {:>3}%   {}: {detail}",
                    wall_clock(),
                    state.percent(),
                    job_path(&state.jobs, &id)
                );
            }
        }
        ProgressMessage::JobDone { id, elapsed } => {
            if state.jobs.contains_key(&id) {
                eprintln!(
                    "[{}] {:>3}% ✓ {} ({})",
                    wall_clock(),
                    state.percent(),
                    job_path(&state.jobs, &id),
                    format_elapsed(elapsed)
                );
            }
        }
        ProgressMessage::JobSkipped { id, detail } => {
            if state.jobs.contains_key(&id) {
                eprintln!(
                    "[{}] {:>3}% – {} ({detail})",
                    wall_clock(),
                    state.percent(),
                    job_path(&state.jobs, &id)
                );
            }
        }
        ProgressMessage::JobFailed { id, message } => {
            if let Some(job) = state.jobs.get(&id) {
                let elapsed = job.elapsed.unwrap_or_default();
                eprintln!(
                    "[{}] {:>3}% ✗ {} ({message} · {})",
                    wall_clock(),
                    state.percent(),
                    job_path(&state.jobs, &id),
                    format_elapsed(elapsed)
                );
            }
        }
        ProgressMessage::JobDetail { id, key, value } => {
            if state.jobs.contains_key(&id) {
                eprintln!(
                    "[{}] {:>3}%   {} · {key}: {value}",
                    wall_clock(),
                    state.percent(),
                    job_path(&state.jobs, &id)
                );
            }
        }
        ProgressMessage::Log { job_id, line } => {
            let hierarchy = job_id
                .as_deref()
                .map(|id| format!("{} · ", job_path(&state.jobs, id)))
                .unwrap_or_default();
            eprintln!(
                "[{}] {:>3}%   {hierarchy}{line}",
                wall_clock(),
                state.percent()
            );
        }
        ProgressMessage::VmState {
            vm_id,
            state: vm_state,
        } => {
            eprintln!(
                "[{}] {:>3}%   vm {vm_id} → {vm_state}",
                wall_clock(),
                state.percent()
            );
        }
        ProgressMessage::Finished { ok, message } => {
            eprintln!(
                "[{}] {:>3}% {} {message}",
                wall_clock(),
                state.percent(),
                if ok { "✓" } else { "✗" }
            );
        }
        ProgressMessage::Plan(_) => {}
    }
}

fn default_job_specs(steps: &[&str]) -> Vec<JobSpec> {
    let groups = [
        JobSpec::group("phase-plan", "Plan & Lease"),
        JobSpec::group("phase-infra", "Infrastruktur"),
        JobSpec::group("phase-images", "Images"),
        JobSpec::group("phase-vms", "VM-Provisioning"),
        JobSpec::group("phase-services", "Dienste & Policies"),
        JobSpec::group("phase-finish", "Abschluss"),
    ];
    let mut result = groups.to_vec();
    for step in steps {
        result.push(JobSpec::leaf(
            format!("step:{step}"),
            step_group(step),
            step_label(step),
            step_units(step),
        ));
    }
    result
}

fn step_group(step: &str) -> &'static str {
    match step {
        "validate" | "acquire_lease" => "phase-plan",
        "ensure_nets" | "ensure_dns" | "ensure_ca" | "attach_nets" => "phase-infra",
        "ensure_images" => "phase-images",
        "ensure_vms" | "start_helpers" | "await_agents" | "await_cloud_init" => "phase-vms",
        "release_lease" => "phase-finish",
        _ => "phase-services",
    }
}

fn step_units(step: &str) -> u32 {
    match step {
        "ensure_images" | "ensure_vms" | "await_agents" | "await_cloud_init" => 2,
        _ => 1,
    }
}

fn step_label(step: &str) -> &'static str {
    match step {
        "validate" => "Konfiguration prüfen",
        "acquire_lease" => "Apply-Lease übernehmen",
        "ensure_nets" => "Netze bereitstellen",
        "ensure_dns" => "DNS bereitstellen",
        "ensure_ca" => "CA bereitstellen",
        "ensure_images" => "Images bereitstellen",
        "attach_nets" => "Netze verbinden",
        "ensure_vms" => "VMs erstellen/aktualisieren",
        "start_helpers" => "VM-Helper starten",
        "await_agents" => "Guest Agents abwarten",
        "await_cloud_init" => "Cloud-init provisioning",
        "ensure_guest_utils" => "Guest Utils aktualisieren",
        "ensure_docker_project_mount" => "Projekt-Mount verbinden",
        "ensure_oidc" => "OIDC bereitstellen",
        "ensure_ingress" => "Ingress bereitstellen",
        "ensure_ca_rollout" => "CA in Gäste ausrollen",
        "ensure_oidc_inject" => "OIDC-Konfiguration injizieren",
        "ensure_docker_context" => "Docker-Kontext bereitstellen",
        "ensure_containers" => "Container bereitstellen",
        "ensure_ports" => "Port-Forwards bereitstellen",
        "apply_routes_policies" => "Routen und Policies anwenden",
        "release_lease" => "Apply abschließen",
        "purge_ingress" => "Ingress entfernen",
        "purge_dns_records" => "DNS-Einträge entfernen",
        "purge_oidc" => "OIDC entfernen",
        "stop_helpers" => "VM-Helper stoppen",
        "detach_nets" => "Netze trennen",
        "destroy_managed" => "Verwaltete Ressourcen löschen",
        "purge_docker_context" => "Docker-Kontext entfernen",
        "purge_ports" => "Port-Forwards entfernen",
        "dns_cleanup" => "DNS bereinigen",
        _ => "Apply-Schritt",
    }
}

fn job_depth(jobs: &BTreeMap<String, JobState>, id: &str) -> usize {
    let mut depth = 0;
    let mut current = jobs.get(id).and_then(|job| job.spec.parent.as_deref());
    while let Some(parent) = current {
        depth += 1;
        current = jobs.get(parent).and_then(|job| job.spec.parent.as_deref());
    }
    depth
}

fn job_path(jobs: &BTreeMap<String, JobState>, id: &str) -> String {
    let mut labels = Vec::new();
    let mut current = Some(id);
    while let Some(current_id) = current {
        let Some(job) = jobs.get(current_id) else {
            if labels.is_empty() {
                labels.push(current_id.to_string());
            }
            break;
        };
        labels.push(job.spec.label.clone());
        current = job.spec.parent.as_deref();
    }
    labels.reverse();
    labels.join(" / ")
}

fn status_mark(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "○",
        JobStatus::Running => "◆",
        JobStatus::Done => "✓",
        JobStatus::Skipped => "–",
        JobStatus::Failed => "✗",
    }
}

fn status_label(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "wartet",
        JobStatus::Running => "läuft",
        JobStatus::Done => "fertig",
        JobStatus::Skipped => "übersprungen",
        JobStatus::Failed => "fehlgeschlagen",
    }
}

fn progress_bar(percent: u8, width: usize) -> String {
    let filled = width.saturating_mul(percent as usize) / 100;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(width - filled))
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let mut result = value.chars().take(width - 1).collect::<String>();
    result.push('…');
    result
}

fn format_elapsed(elapsed: Duration) -> String {
    if elapsed.as_secs() == 0 {
        format!("{}ms", elapsed.as_millis())
    } else {
        format_clock(elapsed)
    }
}

fn format_clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn wall_clock() -> String {
    // SAFETY: `localtime_r` writes only into the provided `tm` and receives a
    // valid pointer to a `time_t` produced by libc.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut local).is_null() {
            return "--:--:--".to_string();
        }
        format!(
            "{:02}:{:02}:{:02}",
            local.tm_hour, local.tm_min, local.tm_sec
        )
    }
}

pub fn print_plain_subprocess_line(percent: u8, job_path: Option<&str>, line: &str) {
    if !line.is_empty() {
        if let Some(job_path) = job_path {
            eprintln!("[{}] {:>3}%   {job_path} · {line}", wall_clock(), percent);
        } else {
            eprintln!("[{}] {:>3}%   {line}", wall_clock(), percent);
        }
    }
}

pub fn cloud_init_stage_progress(stage: Option<&str>) -> Option<(u32, u32)> {
    let completed = match stage? {
        "init-local" => 0,
        "init" => 10,
        "modules-config" => 30,
        "modules-final" => 60,
        _ => return None,
    };
    Some((completed, 100))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress_flag_accepts_aliases() {
        assert!(parse_progress_flag(None).is_err());
        assert_eq!(parse_progress_flag(Some("ui")).unwrap(), ProgressMode::Ui);
        assert_eq!(parse_progress_flag(Some("tui")).unwrap(), ProgressMode::Ui);
        assert_eq!(parse_progress_flag(Some("off")).unwrap(), ProgressMode::Off);
        assert!(parse_progress_flag(Some("fancy")).is_err());
    }

    #[test]
    fn json_defaults_to_off_and_human_pipe_to_plain() {
        assert_eq!(
            resolve_progress_mode_for(None, false, true, true),
            ProgressMode::Off
        );
        assert_eq!(
            resolve_progress_mode_for(None, true, true, false),
            ProgressMode::Plain
        );
        assert_eq!(
            resolve_progress_mode_for(None, true, true, true),
            ProgressMode::Ui
        );
        assert_eq!(
            resolve_progress_mode_for(None, true, false, true),
            ProgressMode::Plain
        );
    }

    #[test]
    fn percentages_are_weighted_monotone_and_skips_complete_work() {
        let specs = vec![
            JobSpec::leaf("a", "group", "A", 1),
            JobSpec::leaf("b", "group", "B", 3),
        ];
        let mut state = ProgressState::new(specs);
        assert_eq!(state.percent(), 0);
        state.apply(ProgressMessage::JobDone {
            id: "a".into(),
            elapsed: Duration::ZERO,
        });
        assert_eq!(state.percent(), 25);
        state.apply(ProgressMessage::JobProgress {
            id: "b".into(),
            completed: 60,
            total: 100,
            detail: None,
        });
        assert_eq!(state.percent(), 70);
        state.apply(ProgressMessage::JobSkipped {
            id: "b".into(),
            detail: "unchanged".into(),
        });
        assert_eq!(state.percent(), 100);
    }

    #[test]
    fn failure_keeps_last_measured_progress() {
        let mut state = ProgressState::new(vec![JobSpec::leaf("a", "group", "A", 1)]);
        state.apply(ProgressMessage::JobProgress {
            id: "a".into(),
            completed: 60,
            total: 100,
            detail: None,
        });
        assert_eq!(state.percent(), 60);
        state.apply(ProgressMessage::JobFailed {
            id: "a".into(),
            message: "kaputt".into(),
        });
        assert_eq!(state.percent(), 60);
        assert!(state.jobs["a"].elapsed.is_some());
    }

    #[test]
    fn skipped_resume_jobs_reach_completion() {
        let mut state = ProgressState::new(vec![
            JobSpec::leaf("a", "group", "A", 1),
            JobSpec::leaf("b", "group", "B", 2),
        ]);
        for id in ["a", "b"] {
            state.apply(ProgressMessage::JobSkipped {
                id: id.into(),
                detail: "aus Journal".into(),
            });
        }
        assert_eq!(state.percent(), 100);
    }

    #[test]
    fn renderer_stays_within_narrow_terminal_width() {
        let mut ui = UiSession::new("apply edge-dmz");
        ui.apply(ProgressMessage::Plan(default_job_specs(APPLY_STEPS)));
        ui.apply(ProgressMessage::JobStart {
            id: "step:ensure_images".into(),
        });
        for width in [32, 70, 120] {
            let lines = ui.render_lines(width, 20);
            assert!(lines.iter().all(|line| line.chars().count() <= width));
            assert!(lines.len() <= 20);
        }
    }

    #[test]
    fn renderer_scrolls_selected_job_into_view() {
        let specs = (0..30)
            .map(|index| JobSpec {
                id: format!("job-{index}"),
                parent: None,
                label: format!("Job {index}"),
                units: 1,
            })
            .collect();
        let mut ui = UiSession::new("apply");
        ui.apply(ProgressMessage::Plan(specs));
        ui.selected = 29;
        let lines = ui.render_lines(100, 24);
        assert!(lines.iter().any(|line| line.contains("› ○ Job 29")));
    }

    #[test]
    fn ctrl_c_is_distinct_from_regular_c() {
        assert!(is_ctrl_c(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_ctrl_c(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn job_paths_include_phase_and_vm() {
        let mut state = ProgressState::new(vec![JobSpec::group("phase", "VM-Provisioning")]);
        state.install_specs(vec![
            JobSpec {
                id: "vm:demo/web".into(),
                parent: Some("phase".into()),
                label: "demo/web".into(),
                units: 0,
            },
            JobSpec::leaf("vm:demo/web:cloud", "vm:demo/web", "Cloud-init", 1),
        ]);
        assert_eq!(
            job_path(&state.jobs, "vm:demo/web:cloud"),
            "VM-Provisioning / demo/web / Cloud-init"
        );
    }

    #[test]
    fn nested_groups_aggregate_leaf_status_recursively() {
        let mut state = ProgressState::new(vec![
            JobSpec::group("phase", "VM-Provisioning"),
            JobSpec {
                id: "vm".into(),
                parent: Some("phase".into()),
                label: "demo/web".into(),
                units: 0,
            },
            JobSpec::leaf("cloud", "vm", "Cloud-init", 1),
        ]);
        assert_eq!(state.group_status("phase"), JobStatus::Pending);
        state.apply(ProgressMessage::JobDone {
            id: "cloud".into(),
            elapsed: Duration::ZERO,
        });
        assert_eq!(state.group_status("vm"), JobStatus::Done);
        assert_eq!(state.group_status("phase"), JobStatus::Done);
    }

    #[test]
    fn completing_parallel_job_keeps_another_running_job_current() {
        let mut state = ProgressState::new(vec![
            JobSpec::leaf("a", "group", "A", 1),
            JobSpec::leaf("b", "group", "B", 1),
        ]);
        state.apply(ProgressMessage::JobStart { id: "a".into() });
        state.apply(ProgressMessage::JobStart { id: "b".into() });
        state.apply(ProgressMessage::JobDone {
            id: "b".into(),
            elapsed: Duration::ZERO,
        });
        assert_eq!(state.current.as_deref(), Some("a"));
    }

    #[test]
    fn log_buffer_is_capped() {
        let mut state = ProgressState::new(Vec::new());
        for index in 0..(MAX_LOG_LINES + 20) {
            state.apply(ProgressMessage::Log {
                job_id: None,
                line: format!("line {index}"),
            });
        }
        assert_eq!(state.logs.len(), MAX_LOG_LINES);
        assert_eq!(state.logs[0].1, "line 20");
    }

    #[test]
    fn cloud_init_stage_weights_are_stable() {
        assert_eq!(
            cloud_init_stage_progress(Some("init-local")),
            Some((0, 100))
        );
        assert_eq!(cloud_init_stage_progress(Some("init")), Some((10, 100)));
        assert_eq!(
            cloud_init_stage_progress(Some("modules-config")),
            Some((30, 100))
        );
        assert_eq!(
            cloud_init_stage_progress(Some("modules-final")),
            Some((60, 100))
        );
        assert_eq!(cloud_init_stage_progress(None), None);
    }
}
