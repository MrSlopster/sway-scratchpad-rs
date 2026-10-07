//! Command line interface, compatible with the original Bash script.

use clap::Parser;
use md5::{Digest, Md5};

use crate::geometry::{Align, Anchor, Dim, Edge, Size};

const EXAMPLE: &str = "\
Example:
 # Popdown seethrough terminal (kitty)
 bindsym $mod+Ctrl+Return exec --no-startup-id sway-scratchpad -t -mt -atc -d 60%x60% -- kitty --override background_opacity=0.7";

/// Executes a program in a positioned scratchpad window.
///
/// Marks windows for the command arguments, so executing the same command
/// will re-use the existing window, if it still exists.
#[derive(Parser, Debug)]
#[command(name = "sway-scratchpad", disable_version_flag = true, after_help = EXAMPLE)]
pub struct Cli {
    /// Sets where to calculate position from. Valid values are combinations of
    /// {top,centre,bottom} and {left,centre,right}, separated by "-" (example:
    /// centre-right). Can be shortened as: tl, tr, etc. Position is then
    /// calculated from the anchor point on the screen to that same anchor
    /// point on the window.
    #[arg(short = 'a', value_name = "anchor", default_value = "cc", value_parser = parse_anchor)]
    pub anchor: Anchor,

    /// Dimensions of window in pixels, in WIDTHxHEIGHT format. Percentages of
    /// the screen dimensions can be used as well.
    #[arg(short = 'd', value_name = "size", default_value = "50%x50%", value_parser = parse_size)]
    pub size: Size,

    /// Animates the movement to target position from specified edge. Valid
    /// values are top, left, bottom, right / t, l, b, r.
    #[arg(short = 'm', value_name = "edge", value_parser = parse_edge)]
    pub edge: Option<Edge>,

    /// Position of window in pixels, in X,Y format. Negative values can be
    /// used as well.
    #[arg(short = 'p', value_name = "pos", default_value = "0,0", value_parser = parse_pos,
          allow_hyphen_values = true)]
    pub pos: (i32, i32),

    /// Output name (as in `swaymsg -t get_outputs`). Falls back to the focused
    /// output (sway) or the primary output (i3).
    #[arg(short = 's', value_name = "screen")]
    pub screen: Option<String>,

    /// Toggles the window.
    #[arg(short = 't')]
    pub toggle: bool,

    /// Verbose, for debugging.
    #[arg(short = 'v')]
    pub verbose: bool,

    /// Print version information.
    #[arg(short = 'V')]
    pub version: bool,

    /// Which window manager to use.
    #[arg(short = 'w', value_name = "wm", value_parser = parse_wm)]
    pub wm: Option<Wm>,

    /// Seconds to wait for the command to open its window.
    #[arg(long, value_name = "secs", default_value_t = 5.0)]
    pub timeout: f64,

    /// Only accept a new window with this app_id (Wayland apps on sway; i3
    /// has no app_id, use --class there).
    #[arg(long, value_name = "id")]
    pub app_id: Option<String>,

    /// Only accept a new window with this X11 class.
    #[arg(long, value_name = "class")]
    pub class: Option<String>,

    /// Command to execute in order to open window, if it doesn't exist.
    #[arg(
        value_name = "command",
        trailing_var_arg = true,
        required_unless_present = "version"
    )]
    pub command: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wm {
    Sway,
    I3,
}

impl Cli {
    /// The command line as the original script saw it (`$*`).
    pub fn command_line(&self) -> String {
        self.command.join(" ")
    }
}

/// The identifying mark for a command. Matches the Bash version
/// (`md5sum <<< "$command"`, which appends a newline), so windows opened by
/// it are recognised.
pub fn mark_for(command_line: &str) -> String {
    let digest = Md5::new()
        .chain_update(command_line.as_bytes())
        .chain_update(b"\n")
        .finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("_sway_scratchpad_{hex}")
}

fn parse_align(s: &str, start: &str, end: &str) -> Option<Align> {
    match s {
        _ if s == start || s == &start[..1] => Some(Align::Start),
        "centre" | "center" | "c" => Some(Align::Centre),
        _ if s == end || s == &end[..1] => Some(Align::End),
        _ => None,
    }
}

fn parse_anchor(s: &str) -> Result<Anchor, String> {
    let (v, h) = match s.split_once('-') {
        Some(parts) => parts,
        None if s.len() == 2 && s.is_ascii() => s.split_at(1),
        None => ("", ""),
    };
    match (
        parse_align(v, "top", "bottom"),
        parse_align(h, "left", "right"),
    ) {
        (Some(v), Some(h)) => Ok(Anchor { v, h }),
        _ => Err(format!(
            "anchor \"{s}\" is invalid (examples: tc, br, top-left, centre-right)"
        )),
    }
}

fn parse_edge(s: &str) -> Result<Edge, String> {
    match s {
        "t" | "top" => Ok(Edge::Top),
        "b" | "bottom" => Ok(Edge::Bottom),
        "l" | "left" => Ok(Edge::Left),
        "r" | "right" => Ok(Edge::Right),
        _ => Err(format!(
            "edge \"{s}\" is invalid (expected top, bottom, left or right)"
        )),
    }
}

fn parse_dim(s: &str) -> Option<Dim> {
    fn number(s: &str) -> Option<i32> {
        let digits = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| s.parse().ok()).flatten()
    }
    match s.strip_suffix('%') {
        Some(p) => number(p).map(Dim::Percent),
        None => number(s).map(Dim::Px),
    }
}

fn parse_size(s: &str) -> Result<Size, String> {
    s.split_once('x')
        .and_then(|(w, h)| {
            Some(Size {
                w: parse_dim(w)?,
                h: parse_dim(h)?,
            })
        })
        .ok_or_else(|| {
            format!("size \"{s}\" is invalid (expected WIDTHxHEIGHT, e.g. 800x600 or 60%x40%)")
        })
}

fn parse_pos(s: &str) -> Result<(i32, i32), String> {
    s.split_once(',')
        .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
        .ok_or_else(|| format!("position \"{s}\" is invalid (expected X,Y, e.g. 0,-20)"))
}

fn parse_wm(s: &str) -> Result<Wm, String> {
    let lower = s.to_lowercase();
    if lower.contains("sway") {
        Ok(Wm::Sway)
    } else if lower.contains("i3") {
        Ok(Wm::I3)
    } else {
        Err(format!(
            "couldn't recognise wm value \"{s}\"; expected sway or i3"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("sway-scratchpad").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn bundled_flags_like_getopts() {
        let cli = parse(&["-tmt", "-atc", "-d", "60%x60%", "--", "kitty", "-o", "x"]);
        assert!(cli.toggle);
        assert_eq!(cli.edge, Some(Edge::Top));
        assert_eq!(
            cli.anchor,
            Anchor {
                v: Align::Start,
                h: Align::Centre
            }
        );
        assert_eq!(
            cli.size,
            Size {
                w: Dim::Percent(60),
                h: Dim::Percent(60)
            }
        );
        assert_eq!(cli.command, ["kitty", "-o", "x"]);
    }

    #[test]
    fn readme_browser_example() {
        let cli = parse(&[
            "-d900x500",
            "-p0,0",
            "-mr",
            "-tatr",
            "--",
            "brave",
            "--app=https://x",
        ]);
        assert_eq!(
            cli.size,
            Size {
                w: Dim::Px(900),
                h: Dim::Px(500)
            }
        );
        assert_eq!(cli.pos, (0, 0));
        assert_eq!(cli.edge, Some(Edge::Right));
        assert!(cli.toggle);
        assert_eq!(
            cli.anchor,
            Anchor {
                v: Align::Start,
                h: Align::End
            }
        );
        assert_eq!(cli.command, ["brave", "--app=https://x"]);
    }

    #[test]
    fn command_options_without_double_dash() {
        let cli = parse(&["-tmt", "-atc", "kitty", "-o", "background_opacity=0.7"]);
        assert_eq!(cli.command, ["kitty", "-o", "background_opacity=0.7"]);
        assert!(!cli.verbose);
    }

    #[test]
    fn defaults() {
        let cli = parse(&["foot"]);
        assert_eq!(
            cli.anchor,
            Anchor {
                v: Align::Centre,
                h: Align::Centre
            }
        );
        assert_eq!(
            cli.size,
            Size {
                w: Dim::Percent(50),
                h: Dim::Percent(50)
            }
        );
        assert_eq!(cli.pos, (0, 0));
        assert_eq!(cli.edge, None);
        assert!(!cli.toggle);
    }

    #[test]
    fn negative_position() {
        assert_eq!(parse(&["-p", "-10,-20", "foot"]).pos, (-10, -20));
        assert_eq!(parse(&["-p-10,20", "foot"]).pos, (-10, 20));
    }

    #[test]
    fn missing_command_is_an_error() {
        assert!(Cli::try_parse_from(["sway-scratchpad", "-t"]).is_err());
        assert!(Cli::try_parse_from(["sway-scratchpad", "-V"]).is_ok());
    }

    #[test]
    fn anchors() {
        let a = |s| parse_anchor(s).unwrap();
        assert_eq!(
            a("br"),
            Anchor {
                v: Align::End,
                h: Align::End
            }
        );
        assert_eq!(
            a("top-left"),
            Anchor {
                v: Align::Start,
                h: Align::Start
            }
        );
        assert_eq!(
            a("centre-right"),
            Anchor {
                v: Align::Centre,
                h: Align::End
            }
        );
        assert_eq!(
            a("center-center"),
            Anchor {
                v: Align::Centre,
                h: Align::Centre
            }
        );
        for bad in ["", "x", "lt", "top", "top-top", "ccc"] {
            assert!(parse_anchor(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sizes_and_positions() {
        assert!(parse_size("50%x").is_err());
        assert!(parse_size("-5x10").is_err());
        assert!(parse_pos("1,").is_err());
        assert_eq!(parse_wm("Sway").unwrap(), Wm::Sway);
        assert!(parse_wm("hyprland").is_err());
    }

    #[test]
    fn mark_matches_bash_md5sum() {
        // printf '%s\n' "kitty -o background_opacity=0.7" | md5sum
        assert_eq!(
            mark_for("kitty -o background_opacity=0.7"),
            "_sway_scratchpad_31d238615331492b90b9dde5a6e63fd2"
        );
    }
}
