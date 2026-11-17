//! The run manager: device runs take minutes, so they go on in the background, one per target at a time.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::{Core, android, cmd, ios};

pub const TARGETS: [&str; 4] = ["phone", "emulator", "iphone", "simulator"];

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Running,
    /// Installed and started, on the device with this serial or UDID.
    Started(String),
    Failed(String),
    /// A newer run on the target, the tray menu or quitting stopped it.
    Cancelled,
}

/// One device run. It writes what it does to its log, for people to read; its state lives here.
pub struct Run {
    /// What status polls ask for.
    pub id: u64,
    pub target: &'static str,
    pub pr: u64,
    pub branch: String,
    /// The worktree it builds.
    pub path: PathBuf,
    pub log: PathBuf,
    pub started: SystemTime,
    log_file: File,
    /// What the tray menu calls the device; a run that picks its device fills it in.
    device: Mutex<String>,
    /// The state, and when the run got into it.
    state: Mutex<(State, SystemTime)>,
    cancelled: AtomicBool,
    /// The process group of the command it is running now.
    group: Mutex<Option<libc::pid_t>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Run {
    /// A run that isn't going yet; its log is `<branch>-<target>.log` in `log_dir`, started empty.
    pub fn new(log_dir: &Path, id: u64, target: &'static str, pr: u64, branch: &str, path: &Path, device: &str) -> io::Result<Self> {
        fs::create_dir_all(log_dir)?;
        let log = log_dir.join(format!("{}-{target}.log", branch.replace('/', "-")));
        let now = SystemTime::now();
        Ok(Self {
            id,
            target,
            pr,
            branch: branch.to_owned(),
            path: path.to_owned(),
            log_file: File::create(&log)?,
            log,
            started: now,
            device: Mutex::new(device.to_owned()),
            state: Mutex::new((State::Running, now)),
            cancelled: AtomicBool::new(false),
            group: Mutex::default(),
            thread: Mutex::default(),
        })
    }

    pub fn state(&self) -> (State, SystemTime) {
        self.state.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.state().0 == State::Running
    }

    pub fn device(&self) -> String {
        self.device.lock().unwrap().clone()
    }

    pub fn set_device(&self, device: &str) {
        *self.device.lock().unwrap() = device.to_owned();
    }

    /// What a status poll gets.
    pub fn status(&self) -> Value {
        let (state, message) = match self.state().0 {
            State::Running => ("running", None),
            State::Started(serial) => ("started", Some(format!("started on {serial}"))),
            State::Failed(message) => ("failed", Some(message)),
            State::Cancelled => ("cancelled", None),
        };
        let mut status = json!({ "ok": true, "state": state, "log": self.log });
        if let Some(message) = message {
            status["message"] = message.into();
        }
        status
    }

    /// Stops the command the run is in, and with it what that started: SIGTERM to the process group also ends the
    /// Gradle client, whose daemon then cancels the build.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(group) = *self.group.lock().unwrap() {
            // SAFETY: plain syscall. The group's leader is still our unreaped child, so the id can't be someone else's.
            unsafe { libc::killpg(group, libc::SIGTERM) };
        }
    }

    /// Adds a line to the log.
    pub fn say(&self, line: &str) {
        let _ = writeln!(&self.log_file, "{line}");
    }

    /// Runs a command of the run and returns its trimmed stdout.
    pub fn capture(&self, cmd: &mut Command) -> Result<String, String> {
        let output = self.output(cmd.stdout(Stdio::piped()).stderr(Stdio::piped()));
        cmd::checked(cmd, output)
    }

    /// Runs a command of the run with its output going to the log. Returns whether it succeeded and what it printed.
    pub fn stream(&self, cmd: &mut Command) -> Result<(bool, String), String> {
        let io_error = |e: io::Error| format!("can't write {}: {e}", self.log.display());
        let from = self.log_file.metadata().map_err(io_error)?.len();
        cmd.stdout(self.log_file.try_clone().map_err(io_error)?).stderr(self.log_file.try_clone().map_err(io_error)?);
        let output = self.output(cmd).map_err(|e| format!("{} failed: {e}", cmd.get_program().to_string_lossy()))?;
        let mut printed = vec![];
        let mut log = File::open(&self.log).map_err(io_error)?;
        log.seek(SeekFrom::Start(from)).and_then(|_| log.read_to_end(&mut printed)).map_err(io_error)?;
        Ok((output.status.success(), String::from_utf8_lossy(&printed).into_owned()))
    }

    /// Waits, unless the run is cancelled meanwhile.
    pub fn pause(&self, time: Duration) -> Result<(), String> {
        for _ in 0..time.as_millis() / 100 {
            self.check_cancelled().map_err(|e| e.to_string())?;
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    fn check_cancelled(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::SeqCst) { Err(io::Error::other("cancelled")) } else { Ok(()) }
    }

    /// Each command leads its own process group, which `cancel` signals.
    fn output(&self, cmd: &mut Command) -> io::Result<Output> {
        self.check_cancelled()?;
        let child = cmd.process_group(0).spawn()?;
        *self.group.lock().unwrap() = Some(child.id() as libc::pid_t);
        if self.check_cancelled().is_err() {
            self.cancel(); // cancelled while it was starting
        }
        let output = child.wait_with_output();
        *self.group.lock().unwrap() = None;
        output
    }

    fn finish(&self, result: Result<String, String>) {
        let state = match result {
            _ if self.cancelled.load(Ordering::SeqCst) => State::Cancelled,
            Ok(serial) => State::Started(serial),
            Err(error) => State::Failed(error),
        };
        match &state {
            State::Started(serial) => self.say(&format!("started on {serial}")),
            State::Failed(error) => self.say(&format!("FAILED: {error}")),
            _ => self.say("CANCELLED"),
        }
        *self.state.lock().unwrap() = (state, SystemTime::now());
    }
}

/// Every run since the app started. Nothing is stored: quitting cancels the runs still going.
pub struct Runs {
    // shortcut: finished runs are never dropped (a few hundred bytes each); prune if the app ever runs for months.
    all: Mutex<Vec<Arc<Run>>>,
    events: broadcast::Sender<Arc<Run>>,
}

impl Default for Runs {
    fn default() -> Self {
        Self { all: Mutex::default(), events: broadcast::channel(16).0 }
    }
}

impl Runs {
    /// A run's state changes arrive here: when it starts and when it ends.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Run>> {
        self.events.subscribe()
    }

    /// The latest run of each target that has had one.
    pub fn latest(&self) -> Vec<Arc<Run>> {
        let all = self.all.lock().unwrap();
        TARGETS.iter().filter_map(|target| all.iter().rev().find(|run| run.target == *target).cloned()).collect()
    }

    /// The run with this ID. Only runs since the app started are known.
    pub fn get(&self, id: u64) -> Option<Arc<Run>> {
        self.all.lock().unwrap().get(usize::try_from(id).ok()?).cloned()
    }

    pub fn cancel_all(&self) {
        self.latest().iter().for_each(|run| run.cancel());
    }
}

/// What a run does.
pub struct Launch<'a> {
    pub target: &'static str,
    pub pr: u64,
    pub branch: &'a str,
    /// The worktree to build.
    pub path: &'a Path,
    /// The AVD name or the simulator's UDID.
    pub device: &'a str,
    /// What the tray menu calls the device.
    pub label: &'a str,
    /// The repo's app ID (Android) or scheme (iOS).
    pub app: &'a str,
}

/// Starts a run on the target, after stopping the one still going there (any PR), so a new click starts fresh.
pub fn start(core: &Arc<Core>, launch: Launch) -> Result<Arc<Run>, String> {
    let Launch { target, pr, branch, path, device, label, app } = launch;
    if let Some(previous) = core.runs.latest().into_iter().find(|run| run.target == target) {
        previous.cancel();
        // Wait until it is gone: it may write to the same log.
        if let Some(thread) = previous.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
    let run = {
        let mut all = core.runs.all.lock().unwrap();
        let run = Run::new(&core.paths.log_dir, all.len() as u64, target, pr, branch, path, label);
        let run = Arc::new(run.map_err(|e| format!("can't write the log in {}: {e}", core.paths.log_dir.display()))?);
        all.push(run.clone());
        run
    };
    let _ = core.runs.events.send(run.clone());
    let (core, device, app, job) = (core.clone(), device.to_owned(), app.to_owned(), run.clone());
    *run.thread.lock().unwrap() = Some(thread::spawn(move || {
        job.finish(match job.target {
            "phone" | "emulator" => android::launch(&core, &job, &device, &app),
            _ => ios::launch(&core, &job, &device, &app),
        });
        let _ = core.runs.events.send(job);
    }));
    Ok(run)
}
