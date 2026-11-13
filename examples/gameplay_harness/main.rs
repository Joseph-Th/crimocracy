//! Line-oriented player client for Crimocracy.
//!
//! Run with `cargo text-play` (an alias for `--play`). The client renders only
//! player-held or deliberately public information, and every action runs through
//! the canonical production validation, commit, and minute-tick owners.

mod campaign;
mod scenario;
mod text_play;

use crimocracy::build_registry;
use std::error::Error;

const HELP: &str = "\
Crimocracy text play

Usage:
  cargo text-play                    Start a new campaign from the default opening
  cargo text-play --seed <n>         Start from an explicit world seed
  cargo text-play --policy-seed <n>  Vary authored opening timing independently
  cargo text-play --opening <name>   night-trap | late-patrol | fleeting-window |
                                     veteran-crew | thin-crew | green-scout
  cargo text-play --script <file>    Read commands from a file instead of a terminal
  cargo text-play --load <slot>      Resume a saved campaign

Inside a session, `help` lists player commands and `brief` states the situation.
";

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    if arguments
        .first()
        .is_some_and(|argument| argument == "--help" || argument == "-h")
    {
        print!("{HELP}");
        return Ok(());
    }

    let registry = build_registry();

    let play_arguments: Vec<String> = if arguments
        .first()
        .is_some_and(|argument| argument == "--play")
    {
        arguments[1..].to_vec()
    } else {
        arguments
    };

    text_play::run(&registry, &play_arguments)?;
    Ok(())
}
