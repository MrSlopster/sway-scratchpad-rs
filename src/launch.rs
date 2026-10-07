//! Launching the command and identifying the window it opens.
//!
//! The original script subscribed to window events only *after* launching,
//! accepted any `focus` event as "the new window", and then marked whatever
//! was focused. That is how the focused app ended up in the scratchpad.
//! Here we subscribe first, only consider `new` events, pick the window by
//! PID ancestry (or explicit app_id/class), and return its container id so
//! the caller can mark exactly that container.

use std::collections::{HashSet, VecDeque};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rustix::process::{Pid, Signal, kill_process_group};
use swayipc::{Connection, Event, EventType, Fallible, Node, WindowChange};

use crate::proc;

/// Optional explicit identification of the window to capture.
#[derive(Debug, Default, Clone)]
pub struct Criteria {
    pub app_id: Option<String>,
    pub class: Option<String>,
}

impl Criteria {
    fn is_set(&self) -> bool {
        self.app_id.is_some() || self.class.is_some()
    }

    fn matches(&self, w: &Candidate) -> bool {
        let field = |want: &Option<String>, have: &Option<String>| {
            want.as_ref().is_none_or(|want| have.as_ref() == Some(want))
        };
        field(&self.app_id, &w.app_id) && field(&self.class, &w.class)
    }
}

/// The parts of a new window that matter for picking it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: i64,
    pub pid: Option<i32>,
    pub app_id: Option<String>,
    pub class: Option<String>,
}

impl From<&Node> for Candidate {
    fn from(n: &Node) -> Self {
        Self {
            id: n.id,
            pid: n.pid,
            app_id: n.app_id.clone(),
            class: n.window_properties.as_ref().and_then(|p| p.class.clone()),
        }
    }
}

/// Decides which new window belongs to the launched command.
pub struct Selector<F> {
    criteria: Criteria,
    child_pid: i32,
    /// The child exited successfully and the handoff grace period is running.
    handoff_open: bool,
    /// Every window created since the launch.
    created: HashSet<i64>,
    /// Foreign new windows seen while the child was alive, oldest first.
    /// A handed-off window opens around the time the launcher exits, so the
    /// newest of these is the best guess.
    queued: VecDeque<i64>,
    is_descendant: F,
}

impl<F: Fn(i32, i32) -> bool> Selector<F> {
    pub fn new(criteria: Criteria, child_pid: i32, is_descendant: F) -> Self {
        Self {
            criteria,
            child_pid,
            handoff_open: false,
            created: HashSet::new(),
            queued: VecDeque::new(),
            is_descendant,
        }
    }

    /// A `new` window event. Returns the container id if it is ours.
    pub fn on_new(&mut self, w: &Candidate) -> Option<i64> {
        self.created.insert(w.id);
        if self.criteria.is_set() {
            return self.criteria.matches(w).then_some(w.id);
        }
        match w.pid {
            Some(pid) if pid > 0 && (self.is_descendant)(pid, self.child_pid) => {
                return Some(w.id);
            }
            // i3 does not report PIDs; fall back to the first new window.
            None => return Some(w.id),
            Some(_) => {}
        }
        if self.handoff_open {
            // The command handed off to an already running instance
            // (single-instance apps); the next new window is the result.
            return Some(w.id);
        }
        self.queued.push_back(w.id);
        None
    }

    /// Any other change to a window (title, marks, ...). Some X11 apps set
    /// their class only after mapping, so explicit criteria are re-checked,
    /// but only for windows created since the launch.
    pub fn on_update(&mut self, w: &Candidate) -> Option<i64> {
        (self.criteria.is_set() && self.created.contains(&w.id) && self.criteria.matches(w))
            .then_some(w.id)
    }

    /// A `close` window event, or a candidate that vanished before it could
    /// be claimed.
    pub fn on_close(&mut self, id: i64) {
        self.created.remove(&id);
        self.queued.retain(|&q| q != id);
    }

    /// The launched process exited. Pending events must be handled first.
    pub fn on_child_exit(&mut self, success: bool) -> Result<()> {
        if self.criteria.is_set() {
            // Explicit criteria decide on their own; keep waiting.
            return Ok(());
        }
        if !success {
            bail!("command exited unsuccessfully before opening a window");
        }
        self.handoff_open = true;
        Ok(())
    }

    /// The handoff grace period is over. Windows opened after the exit were
    /// preferred; failing those, the newest window that opened while the
    /// command was running is the best remaining guess.
    pub fn close_handoff(&mut self) -> Option<i64> {
        if !self.handoff_open {
            return None;
        }
        self.handoff_open = false;
        self.queued.pop_back()
    }

    fn handle(&mut self, message: Fallible<Event>) -> Result<Option<i64>> {
        let Event::Window(event) = message.context("reading window events")? else {
            return Ok(None);
        };
        let w = Candidate::from(&event.container);
        Ok(match event.change {
            WindowChange::New => {
                debug!("New window {w:?}");
                self.on_new(&w)
            }
            WindowChange::Close => {
                self.on_close(w.id);
                None
            }
            _ => self.on_update(&w),
        })
    }
}

/// The launched command's process group, killed on drop unless released.
struct Launched {
    child: Child,
    armed: bool,
}

impl Drop for Launched {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // The group outlives `sh` while the app runs, so signal the group
        // rather than the child. ESRCH (already gone) is fine.
        if let Some(pgid) = Pid::from_raw(self.child.id() as i32) {
            debug!("Killing process group {pgid:?}");
            let _ = kill_process_group(pgid, Signal::TERM);
        }
        let _ = self.child.try_wait();
    }
}

/// How long to block on the event channel before re-checking the child.
const POLL: Duration = Duration::from_millis(50);

/// How long after a successful exit of the command a window from another
/// process is still accepted as the handed-off result.
const HANDOFF_GRACE: Duration = Duration::from_secs(2);

/// Runs `command_line` via `sh -c` and returns the container id of the
/// window it opens, once `claim` accepted it.
///
/// `claim` should mark the window. It returns `Ok(false)` when the window has
/// vanished in the meantime, and the search continues. On failure or timeout
/// the launched command's process group is killed, so no stray window shows
/// up unmarked later.
pub fn spawn_and_capture(
    command_line: &str,
    criteria: Criteria,
    timeout: Duration,
    mut claim: impl FnMut(i64) -> Result<bool>,
) -> Result<i64> {
    // Subscribe before spawning so the `new` event cannot be missed.
    let events = Connection::new()
        .context("connecting to the window manager")?
        .subscribe([EventType::Window])
        .context("subscribing to window events")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for event in events {
            if tx.send(event).is_err() {
                break;
            }
        }
    });

    let child = Command::new("sh")
        .arg("-c")
        .arg(command_line)
        .stdin(Stdio::null())
        // Own process group: not tied to the caller's terminal, unaffected
        // when we exit, and killable as a whole.
        .process_group(0)
        .spawn()
        .with_context(|| format!("spawning `{command_line}`"))?;
    let mut launched = Launched { child, armed: true };
    let child_pid = launched.child.id() as i32;
    debug!("Spawned `{command_line}` as pid {child_pid}");

    let mut selector = Selector::new(criteria, child_pid, |pid, ancestor| {
        proc::is_descendant(pid, ancestor, proc::parent_of)
    });
    let deadline = Instant::now() + timeout;
    let mut exited_at = None;

    // Claims a candidate; Some(id) when done.
    let mut take = |selector: &mut Selector<_>, id: Option<i64>| -> Result<Option<i64>> {
        let Some(id) = id else { return Ok(None) };
        if claim(id)? {
            return Ok(Some(id));
        }
        debug!("Window {id} vanished before it could be marked, waiting for another");
        selector.on_close(id);
        Ok(None)
    };

    let id = 'capture: loop {
        let exit = match exited_at {
            None => launched.child.try_wait()?,
            Some(_) => None,
        };
        // Handle everything that arrived before deciding on the exit, so a
        // queued foreign window cannot win over the real one.
        while let Ok(message) = rx.try_recv() {
            let candidate = selector.handle(message)?;
            if let Some(id) = take(&mut selector, candidate)? {
                break 'capture id;
            }
        }
        if let Some(status) = exit {
            exited_at = Some(Instant::now());
            debug!("Command exited with {status}");
            selector.on_child_exit(status.success())?;
        }
        if exited_at.is_some_and(|t: Instant| t.elapsed() >= HANDOFF_GRACE) {
            let candidate = selector.close_handoff();
            if let Some(id) = take(&mut selector, candidate)? {
                debug!("Taking window {id} that opened while the command ran");
                break id;
            }
        }

        let now = Instant::now();
        if now >= deadline {
            bail!("timed out waiting for `{command_line}` to open a window");
        }
        match rx.recv_timeout((deadline - now).min(POLL)) {
            Ok(message) => {
                let candidate = selector.handle(message)?;
                if let Some(id) = take(&mut selector, candidate)? {
                    break id;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => bail!("window event stream closed"),
        }
    };
    launched.armed = false;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD: i32 = 100;

    fn win(id: i64, pid: i32) -> Candidate {
        Candidate {
            id,
            pid: Some(pid),
            app_id: None,
            class: None,
        }
    }

    /// PIDs 100..200 are the child's descendants.
    fn selector(criteria: Criteria) -> Selector<impl Fn(i32, i32) -> bool> {
        Selector::new(criteria, CHILD, |pid, anc| {
            anc == CHILD && (100..200).contains(&pid)
        })
    }

    #[test]
    fn takes_descendant_window() {
        let mut s = selector(Criteria::default());
        assert_eq!(s.on_new(&win(1, 150)), Some(1));
    }

    #[test]
    fn foreign_window_is_not_taken_while_child_runs() {
        let mut s = selector(Criteria::default());
        assert_eq!(s.on_new(&win(1, 999)), None);
        assert_eq!(s.on_new(&win(2, 150)), Some(2));
    }

    #[test]
    fn handoff_takes_next_window_after_successful_exit() {
        let mut s = selector(Criteria::default());
        s.on_child_exit(true).unwrap();
        assert_eq!(s.on_new(&win(7, 999)), Some(7));
    }

    #[test]
    fn queued_window_only_after_grace() {
        let mut s = selector(Criteria::default());
        assert_eq!(s.on_new(&win(3, 999)), None);
        assert_eq!(s.on_new(&win(4, 998)), None);
        assert_eq!(s.on_new(&win(5, 997)), None);
        s.on_close(5);
        s.on_child_exit(true).unwrap();
        // The newest surviving one: closest to the launcher's exit.
        assert_eq!(s.close_handoff(), Some(4));
    }

    #[test]
    fn window_after_exit_beats_window_from_before() {
        let mut s = selector(Criteria::default());
        // Something unrelated opens while the launcher still runs...
        assert_eq!(s.on_new(&win(3, 999)), None);
        s.on_child_exit(true).unwrap();
        // ...the handed-off window opens after it exited.
        assert_eq!(s.on_new(&win(4, 998)), Some(4));
    }

    #[test]
    fn handoff_window_only_within_grace() {
        let mut s = selector(Criteria::default());
        s.on_child_exit(true).unwrap();
        assert_eq!(s.close_handoff(), None);
        assert_eq!(s.on_new(&win(8, 999)), None);
        assert_eq!(s.close_handoff(), None);
    }

    #[test]
    fn window_without_pid_is_taken() {
        // i3 reports no PIDs.
        let mut s = selector(Criteria::default());
        let w = Candidate {
            pid: None,
            ..win(9, 0)
        };
        assert_eq!(s.on_new(&w), Some(9));
    }

    #[test]
    fn late_class_matches_only_windows_created_since_launch() {
        let criteria = Criteria {
            app_id: None,
            class: Some("Spotify".into()),
        };
        let mut s = selector(criteria);
        let spotify = |id| Candidate {
            class: Some("Spotify".into()),
            ..win(id, 999)
        };
        // An existing window gaining the class (e.g. a title event) is ignored.
        assert_eq!(s.on_update(&spotify(1)), None);
        // A new window without a class yet, which sets it later, is taken.
        assert_eq!(s.on_new(&win(2, 999)), None);
        assert_eq!(s.on_update(&spotify(2)), Some(2));
    }

    #[test]
    fn failed_child_is_an_error() {
        let mut s = selector(Criteria::default());
        assert!(s.on_child_exit(false).is_err());
    }

    #[test]
    fn explicit_criteria_win() {
        let criteria = Criteria {
            app_id: Some("foot".into()),
            class: None,
        };
        let mut s = selector(criteria);
        // A descendant with the wrong app_id is rejected...
        assert_eq!(s.on_new(&win(1, 150)), None);
        // ...exit does not trigger the fallback...
        s.on_child_exit(false).unwrap();
        assert_eq!(s.close_handoff(), None);
        assert_eq!(s.on_new(&win(2, 999)), None);
        // ...only the matching window is taken.
        let foot = Candidate {
            app_id: Some("foot".into()),
            ..win(3, 999)
        };
        assert_eq!(s.on_new(&foot), Some(3));
    }

    #[test]
    fn class_criteria() {
        let criteria = Criteria {
            app_id: None,
            class: Some("Brave".into()),
        };
        let mut s = selector(criteria);
        let brave = Candidate {
            class: Some("Brave".into()),
            ..win(5, 999)
        };
        assert_eq!(s.on_new(&brave), Some(5));
    }
}
