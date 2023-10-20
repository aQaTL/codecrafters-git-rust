#[allow(unused_imports)]
use std::env;
#[allow(unused_imports)]
use std::fs;
use std::io::{BufReader, Read};

use clap::{Parser, Subcommand};
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    #[command(name = "cat-file")]
    CatFile {
        #[arg(short, long)]
        pretty_print: bool,

        #[arg(required = true)]
        object: String,
    },
}

fn main() {
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    let args = Args::parse();

    let result: Result<(), Box<dyn std::error::Error>> = match args.command {
        Command::Init => init().map_err(Into::into),
        Command::CatFile {
            pretty_print,
            object,
        } => cat_file(pretty_print, object).map_err(Into::into),
    };

    if let Err(err) = result {
        println!("{err}");
        std::process::exit(1);
    }
}

#[derive(Debug, Error)]
enum InitError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn init() -> Result<(), InitError> {
    fs::create_dir(".git")?;
    fs::create_dir(".git/objects")?;
    fs::create_dir(".git/refs")?;
    fs::write(".git/HEAD", "ref: refs/heads/master\n")?;
    eprintln!("Initialized git directory");

    Ok(())
}

#[derive(Debug, Error)]
enum CatFileError {
    #[error("Not a valid object name {0}")]
    InvalidObjectName(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn cat_file(pretty_print: bool, object: String) -> Result<(), CatFileError> {
    eprintln!("pretty_print: {pretty_print}");
    if object.len() != 40 {
        return Err(CatFileError::InvalidObjectName(object));
    }

    let dirname = &object[0..2];
    eprintln!("dirname: {dirname}");

    let filename = &object[2..];
    let file = fs::File::open(format!(".git/objects/{dirname}/{filename}"))?;
    let file_buffered = BufReader::new(file);

    let mut decoder = flate2::bufread::ZlibDecoder::new(file_buffered);
    let mut file_contents = String::new();
    decoder.read_to_string(&mut file_contents)?;

    print!("{file_contents}");

    Ok(())
}
