use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use perf_harness::{DEFAULT_BUFFER_SIZE, receive_file, send_file};

#[derive(Debug, Parser)]
#[command(name = "perf_harness", about = "LAN Chat raw TCP performance harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Receive {
        #[arg(long)]
        listen: SocketAddr,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = DEFAULT_BUFFER_SIZE)]
        buffer_size: usize,
    },
    Send {
        #[arg(long)]
        connect: SocketAddr,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value_t = DEFAULT_BUFFER_SIZE)]
        buffer_size: usize,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Receive {
            listen,
            output,
            buffer_size,
        } => receive_file(listen, &output, buffer_size),
        Command::Send {
            connect,
            file,
            buffer_size,
        } => send_file(connect, &file, buffer_size),
    };

    match result {
        Ok(metrics) => {
            println!(
                "{}",
                serde_json::to_string(&metrics).expect("metrics serialize")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
