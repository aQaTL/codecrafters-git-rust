#[allow(unused_imports)]
use std::env;
#[allow(unused_imports)]
use std::fs;

use thiserror::Error;

fn main() {
	// You can use print statements as follows for debugging, they'll be visible when running tests.
	println!("Logs from your program will appear here!");

	// Uncomment this block to pass the first stage
	let args: Vec<String> = env::args().collect();
	if args.len() <= 1 {
		print_usage();
		std::process::exit(1);
	}

	let result: Result<(), Box<dyn std::error::Error>> = match args[1].as_str() {
		"init" => init().map_err(Into::into),
		cmd => Err(anyhow::anyhow!("unknown command: {cmd}").into()),
	};

	if let Err(err) = result {
		println!("{err}");
		std::process::exit(1);
	}
}

fn print_usage() {
	println!(
		"\
Usage: git-starter-rust [COMMAND]

COMMAND:
	init
\
		"
	);
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
	println!("Initialized git directory");

	Ok(())
}
