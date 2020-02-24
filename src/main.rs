extern crate chrono;
extern crate clap;
extern crate config;
extern crate linefeed;

use chrono::Local;
use clap::{App, AppSettings, Arg};
use linefeed::{Interface, ReadResult};
use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use std::{error, fmt, io};

fn main() -> io::Result<()> {
    // Load Settings
    let mut settings = config::Config::default();

    if let Err(e) = settings.merge(config::File::with_name("cli.yaml")) {
        eprintln!("Failed to merge config file: {:?}", e);
        settings = config::Config::default()
    }

    // Not clear how this (merge no environment) fails or why. I'm guessing we have a problem
    // though if it does. As we'll not reset settings and don't want to
    // because of a potentially succesful merge with the file.
    // Should we panic? Clone the settings first then try to merge?
    if let Err(e) = settings.merge(config::Environment::with_prefix("CLI")) {
        eprintln!("Failed to merge environment into config: {:?}", e);
    }
    match settings.try_into::<HashMap<String, String>>() {
        Ok(hm) => println!("{:?}", hm),
        Err(e) => eprintln!("Config not a hashmap: {:?}", e),
    }

    //
    // Prompt & Readline loop.
    let (tx, rx) = mpsc::channel();
    prompt_start_up(tx);

    // Set up read loop.
    let rl = Arc::new(Interface::new("cli")?);
    if let Err(e) = rl.set_prompt("cli> ") {
        eprintln!("Couldn't set prompt: {}", e)
    }

    loop {
        match rl.read_line_step(Some(Duration::from_millis(1000))) {
            Ok(Some(ReadResult::Input(line))) => match parse_exec(line) {
                Ok(ParseResult::Complete) => continue,
                Ok(ParseResult::Exit) => break,
                Err(e) => eprintln!("Parse error: {}", e),
            },
            // Check for a prompt update.
            Ok(None) => {
                let mut p = None;
                // Eat all that have come in but that last.
                for pm in rx.try_iter() {
                    p = Some(pm);
                }
                // If something new, then do the update.
                if let Some(p) = p {
                    if let Err(e) = rl.set_prompt(&p.new_prompt) {
                        eprintln!("Failed to set prompt: {:?}", e)
                    }
                }
                continue;
            }
            Ok(Some(ReadResult::Eof)) => {
                println!("Use the \"quit\" command to exit the applicaiton.");
                continue;
            }
            Ok(Some(ReadResult::Signal(s))) => {
                println!("Caught signal: {:?}", s);
                continue;
            }
            Err(e) => eprintln!("Failed to readline: {:?}", e),
            v => {
                eprintln!("Readline step: {:?}", v);
                continue;
            }
        }
    }
    Ok(())
}

struct PromptUpdate {
    new_prompt: String,
}

const TIME_FMT: &str = "%a %b %e %Y %T";
fn prompt_start_up(tx: mpsc::Sender<PromptUpdate>) {
    thread::spawn(move || {
        let mut i = 0;
        loop {
            thread::sleep(Duration::from_millis(1000));
            if let Err(e) = tx.send(PromptUpdate {
                new_prompt: String::from(format!(
                    "cli <{}> ",
                    Local::now().format(TIME_FMT).to_string()
                )),
            }) {
                eprintln!("Failed to send a new prompt: {:?}", e)
            }
            i = i + 1;
        }
    });
}

type Result<T> = std::result::Result<T, ParseError>;
fn parse_exec(l: String) -> Result<ParseResult> {
    //
    // Build out command tree
    // We're rebuilding this each time.
    // It would be great not to.
    let cmds = App::new("cli")
        .setting(AppSettings::NoBinaryName)
        .about("CLI - dmeo cli app in rust")
        .version("0.0.1")
        .subcommand(App::new("http").about("use http commands"))
        .subcommand(App::new("quit").about("exit"));

    let words: Vec<&str> = l.split_whitespace().collect();
    let matches = cmds.get_matches_from_safe(words);
    match matches {
        Ok(m) => match m.subcommand() {
            ("http", Some(http_matches)) => {
                println!("http command: {:?}", http_matches);
                Ok(ParseResult::Complete)
            }
            ("quit", Some(_)) => Ok(ParseResult::Exit),
            ("", None) => {
                println!("empty command.");
                Ok(ParseResult::Complete)
            }
            _ => unreachable!(),
        },
        Err(e) => match e.kind {
            clap::ErrorKind::VersionDisplayed => {
                eprintln!("Version displayed: {:?}", e);
                Ok(ParseResult::Complete)
            }
            clap::ErrorKind::HelpDisplayed => {
                println!("{}", e.message);
                Ok(ParseResult::Complete)
            }
            _ => {
                eprintln!("{}", e);
                Ok(ParseResult::Complete)
            }
        },
    }
}

enum ParseResult {
    Complete,
    Exit,
}

#[derive(Debug, Clone)]
struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Couldn't parse input.")
    }
}

impl error::Error for ParseError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        None
    }
}
