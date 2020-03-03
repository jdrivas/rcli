extern crate attohttpc;
extern crate chrono;
extern crate clap;
extern crate config;
extern crate linefeed;
extern crate structopt;

use chrono::Local;
// use clap::Clap;
// use clap::{App, AppSettings, Arg};
use clap::AppSettings;
use linefeed::{Interface, ReadResult};
use std::collections::HashMap;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use std::{error, fmt, io};
use structopt::StructOpt;

fn main() -> Result<()> {
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

    // First time through.
    let opt = AppCmds::from_args();
    // eprintln!("Options are: {:?}", opt);
    match parse_exec(RootCommands::App(opt)).unwrap() {
        ParseResult::Interactive => readloop(),
        _ => Ok(()),
    };

    Ok(())
}

fn readloop() -> Result<()> {
    //
    // Prompt & Readline loop.
    let (tx, rx) = mpsc::channel();
    prompt_start_up(tx);

    // Set up read loop.
    let rl = Arc::new(Interface::new("cli").unwrap());
    if let Err(e) = rl.set_prompt("cli> ") {
        eprintln!("Couldn't set prompt: {}", e)
    }

    loop {
        match rl.read_line_step(Some(Duration::from_millis(1000))) {
            Ok(Some(ReadResult::Input(line))) => {
                let words: Vec<&str> = line.split_whitespace().collect();
                match ICmds::from_iter_safe(words) {
                    Ok(opt) => match parse_exec(RootCommands::Interactive(opt)) {
                        Ok(ParseResult::Interactive) => continue, // should not get here.
                        Ok(ParseResult::Complete) => continue,
                        Ok(ParseResult::Exit) => break,
                        Err(e) => eprintln!("{}", e),
                    },
                    Err(e) => eprintln!("{}", e),
                }
            }
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
            Err(e) => eprintln!("Failed on readline: {:?}", e),
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

#[derive(Debug)]
enum RootCommands {
    App(AppCmds),
    Interactive(ICmds),
}

#[derive(StructOpt, Debug)]
#[structopt(name = "cli", version = "0.0.1", setting(AppSettings::NoBinaryName))]
struct ICmds {
    /// File name for configuration.
    #[structopt(short = "c", long = "config", default_value = "cli.yaml")]
    config: String,

    #[structopt(subcommand)]
    subcmd: InteractiveCommands,
}

#[derive(Debug, StructOpt)]
#[structopt(name = "cli-a", version = "0.0.1", setting(AppSettings::NoBinaryName))]
struct AppCmds {
    /// File name for configuration.
    #[structopt(short = "c", long = "config", default_value = "cli.yaml")]
    config: String,

    #[structopt(subcommand)]
    subcmd: RootSubcommand,
}

#[derive(StructOpt, Debug)]
enum RootSubcommand {
    Interactive,
    #[structopt(flatten)]
    InteractiveSubCommand(InteractiveCommands),
}

#[derive(StructOpt, Debug)]
enum InteractiveCommands {
    /// End the program.
    #[structopt(name = "quit")]
    Quit,

    /// Make an http call.
    HTTP(HTTPCmd),
}

#[derive(StructOpt, Debug)]
struct HTTPCmd {
    #[structopt(subcommand)]
    subcmd: HTTPVerb,
}

#[derive(StructOpt, Debug)]
enum HTTPVerb {
    /// Make an http get call.
    Get(HTTPArg),
    /// Make an http put call.
    Put(HTTPArg),
}

#[derive(StructOpt, Debug)]
struct HTTPArg {
    uri: String,
    content: Vec<String>,
    #[structopt(skip)]
    method: attohttpc::Method,
}

fn parse_exec(opt: RootCommands) -> Result<ParseResult> {
    match opt {
        RootCommands::Interactive(c) => parse_interactive(c),
        RootCommands::App(c) => parse_app(c),
    }
}

fn parse_app(opt: AppCmds) -> Result<ParseResult> {
    Ok(ParseResult::Interactive)
}

fn parse_interactive(opt: ICmds) -> Result<ParseResult> {
    match opt.subcmd {
        InteractiveCommands::HTTP(v) => {
            let mut args: HTTPArg;
            match v.subcmd {
                HTTPVerb::Get(a) => {
                    args = a;
                    args.method = attohttpc::Method::GET;
                }
                HTTPVerb::Put(a) => {
                    args = a;
                    args.method = attohttpc::Method::PUT;
                }
            }

            if args.content.len() > 0 {
                println!("{} {} <{}>", args.method, args.uri, args.content.join(" "))
            } else {
                println!("{} {}", args.method, args.uri)
            }
            let resp = attohttpc::RequestBuilder::try_new(args.method, &args.uri)?.send()?;
            println!("{:?}", resp);
            Ok(ParseResult::Complete)
        }
        InteractiveCommands::Quit => Ok(ParseResult::Exit),
    }
}

type Result<T> = std::result::Result<T, ParseError>;

enum ParseResult {
    Interactive,
    Complete,
    Exit,
}

#[derive(Debug)]
enum ParseError {
    Clap(clap::Error),
    HTTP(attohttpc::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // write!(f, "{:?}", self)
        match self {
            ParseError::Clap(e) => match e.kind {
                clap::ErrorKind::VersionDisplayed => write!(f, ""),
                clap::ErrorKind::HelpDisplayed => write!(f, "{}", e.message),
                _ => write!(f, "Parse error => {}", e),
            },
            ParseError::HTTP(e) => write!(f, "{}", e),
        }
    }
}

impl error::Error for ParseError {
    fn cause(&self) -> Option<&dyn error::Error> {
        None
    }

    fn description(&self) -> &str {
        "parse error!"
    }
}

impl From<clap::Error> for ParseError {
    fn from(err: clap::Error) -> ParseError {
        ParseError::Clap(err)
    }
}
impl From<attohttpc::Error> for ParseError {
    fn from(err: attohttpc::Error) -> Self {
        ParseError::HTTP(err)
    }
}
