use std::env;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use swayipc::Connection;

static VERBOSE: AtomicBool = AtomicBool::new(false);

macro_rules! debug {
    ($($arg:tt)*) => {
        if $crate::VERBOSE.load(::std::sync::atomic::Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
}

mod cli;
mod geometry;
mod launch;
mod proc;
mod tree;

use cli::{Cli, Wm};
use geometry::{Edge, Rect};

const FRAME_DELAY: Duration = Duration::from_millis(10);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sway-scratchpad: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.version {
        println!("Version {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    VERBOSE.store(cli.verbose, Ordering::Relaxed);
    if let Some(wm) = cli.wm {
        select_wm(wm);
    }

    let command_line = cli.command_line();
    let mark = cli::mark_for(&command_line);
    debug!("Determined mark as {mark}");
    // Serialise invocations for the same command, so a double keypress
    // cannot launch two instances.
    let _lock = lock(&mark)?;

    let mut conn = Connection::new().context("connecting to the window manager")?;
    let outputs = conn.get_outputs().context("querying outputs")?;
    let output =
        tree::pick_output(&outputs, cli.screen.as_deref()).context("no active output found")?;
    debug!("Using output {}", output.name);
    let output = Rect::from(output.rect);
    let target = geometry::resolve(output, cli.size, cli.pos, cli.anchor);
    debug!("Calculated window geometry {target:?}");

    let mut win = Scratchpad {
        conn,
        criteria: format!("[con_mark=\"{mark}\"]"),
        output,
        target,
        edge: cli.edge,
    };

    let tree = win.layout_tree()?;
    match tree::find_marked(&tree, &mark) {
        Some(found) if cli.toggle => {
            let focused = tree::focused_workspace(&tree);
            if found.workspace.is_some() && found.workspace.as_deref() == focused {
                debug!("Window is on the focused workspace, hiding it");
                win.hide(found.rect)
            } else {
                debug!("Window is not on the focused workspace, showing it");
                win.show()
            }
        }
        Some(_) => win.show(),
        None => {
            let criteria = launch::Criteria {
                app_id: cli.app_id,
                class: cli.class,
            };
            let timeout = Duration::try_from_secs_f64(cli.timeout).context("invalid --timeout")?;
            let id = launch::spawn_and_capture(&command_line, criteria, timeout, |id| {
                win.claim(id, &mark)
            })?;
            debug!("Marked window {id} with {mark}");
            win.show()
        }
    }
}

/// Makes swayipc pick the requested window manager's socket.
fn select_wm(wm: Wm) {
    // SAFETY: called at startup, before any other thread exists.
    unsafe {
        match wm {
            Wm::Sway => {
                if let Ok(sock) = env::var("SWAYSOCK") {
                    env::set_var("I3SOCK", sock);
                }
            }
            Wm::I3 => env::remove_var("SWAYSOCK"),
        }
    }
}

fn lock(mark: &str) -> Result<File> {
    let dir = env::var_os("XDG_RUNTIME_DIR").map_or_else(env::temp_dir, PathBuf::from);
    let path = dir.join(format!("sway-scratchpad-{mark}.lock"));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening lock file {}", path.display()))?;
    file.lock().context("locking")?;
    Ok(file)
}

struct Scratchpad {
    conn: Connection,
    /// `[con_mark="…"]`
    criteria: String,
    output: Rect,
    target: Rect,
    edge: Option<Edge>,
}

impl Scratchpad {
    fn layout_tree(&mut self) -> Result<swayipc::Node> {
        self.conn.get_tree().context("querying the layout tree")
    }

    /// Marks exactly the captured container, never "whatever is focused".
    /// `Ok(false)` if the window closed before it could be marked.
    fn claim(&mut self, id: i64, mark: &str) -> Result<bool> {
        let cmd = format!("[con_id={id}] mark --add \"{mark}\"");
        debug!("> {cmd}");
        let failed = self
            .conn
            .run_command(&cmd)?
            .into_iter()
            .find_map(Result::err);
        match failed {
            None => Ok(true),
            Some(_) if !tree::contains(&self.layout_tree()?, id) => Ok(false),
            Some(e) => Err(e).with_context(|| format!("command failed: {cmd}")),
        }
    }

    /// Runs a command, failing if any part of it is rejected.
    fn command(&mut self, cmd: &str) -> Result<()> {
        debug!("> {cmd}");
        for outcome in self.conn.run_command(cmd)? {
            outcome.with_context(|| format!("command failed: {cmd}"))?;
        }
        Ok(())
    }

    /// Runs an animation step at size `w`×`h`; failures only get logged.
    fn frame(&mut self, (x, y): (i32, i32), (w, h): (i32, i32)) {
        let cmd = format!(
            "{} resize set {w} px {h} px, move absolute position {x} px {y} px",
            self.criteria
        );
        match self.conn.run_command(&cmd) {
            Ok(outcomes) => {
                for e in outcomes.into_iter().filter_map(Result::err) {
                    debug!("Animation step failed: {e}");
                }
            }
            Err(e) => debug!("Animation step failed: {e}"),
        }
        thread::sleep(FRAME_DELAY);
    }

    fn show(mut self) -> Result<()> {
        let Rect { x, y, w, h } = self.target;
        let (sx, sy) = match self.edge {
            Some(edge) => geometry::show_start(edge, self.target, self.output),
            None => (x, y),
        };
        self.command(&format!(
            "{} move to scratchpad, scratchpad show, resize set {w} px {h} px, move absolute position {sx} px {sy} px",
            self.criteria
        ))?;
        if self.edge.is_some() {
            debug!("Starting show animation");
            for step in geometry::frames((sx, sy), (x, y), true) {
                self.frame(step, (w, h));
            }
            // Settle on the exact geometry, in case a frame was dropped.
            self.command(&format!(
                "{} resize set {w} px {h} px, move absolute position {x} px {y} px",
                self.criteria
            ))?;
        }
        Ok(())
    }

    /// Hides the window, sliding it out from where it currently is.
    fn hide(mut self, current: Rect) -> Result<()> {
        if let Some(edge) = self.edge {
            debug!("Starting hide animation from {current:?}");
            let end = geometry::hide_end(edge, current, self.output);
            for step in geometry::frames((current.x, current.y), end, false) {
                self.frame(step, (current.w, current.h));
            }
        }
        self.command(&format!("{} move to scratchpad", self.criteria))
    }
}
