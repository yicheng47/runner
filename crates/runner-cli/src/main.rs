mod command;
mod env;
mod help;
mod msg;
mod output;
mod roster;
mod signal;

use clap::Parser;

fn main() {
    std::process::exit(command::run(command::Cli::parse()));
}
