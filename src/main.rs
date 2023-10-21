use std::borrow::Cow;
#[allow(unused_imports)]
use std::env;
#[allow(unused_imports)]
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use flate2::write::ZlibEncoder;
use thiserror::Error;

mod sha1;

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

	#[command(name = "hash-object")]
	HashObject {
		#[arg(short)]
		write: bool,

		#[arg(required = true)]
		file: PathBuf,
	},
}

fn main() {
	let args = Args::parse();

	let result: Result<(), Box<dyn std::error::Error>> = match args.command {
		Command::Init => init().map_err(Into::into),
		Command::CatFile {
			pretty_print,
			object,
		} => cat_file(pretty_print, object).map_err(Into::into),
		Command::HashObject { write, file } => hash_object(file, write).map_err(Into::into),
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

	#[error("You must use -p option right now :/")]
	MustUsePrettyPrint,
}

fn cat_file(pretty_print: bool, object: String) -> Result<(), CatFileError> {
	if object.len() != 40 {
		return Err(CatFileError::InvalidObjectName(object));
	}

	if !pretty_print {
		return Err(CatFileError::MustUsePrettyPrint);
	}

	let dirname = &object[0..2];

	let filename = &object[2..];
	let file = fs::File::open(format!(".git/objects/{dirname}/{filename}"))?;
	let file_buffered = BufReader::new(file);

	let mut decoder = flate2::bufread::ZlibDecoder::new(file_buffered);
	let mut file_content_bytes = Vec::new();
	decoder.read_to_end(&mut file_content_bytes)?;
	let null_pos = file_content_bytes.iter().position(|x| *x == 0).unwrap();

	let file_content = std::str::from_utf8(&file_content_bytes[(null_pos + 1)..]).unwrap();

	print!("{file_content}");

	Ok(())
}

#[derive(Debug, Error)]
enum HashObjectError {
	#[error("could not open {path} for reading: {err}")]
	InputIo {
		#[source]
		err: std::io::Error,

		path: PathBuf,
	},

	#[error("could not open {path}: {err}")]
	OutputIo {
		#[source]
		err: std::io::Error,

		path: PathBuf,
	},
}

fn hash_object(path: PathBuf, write: bool) -> Result<(), HashObjectError> {
	let file_contents = fs::read(&path).map_err(|err| HashObjectError::InputIo {
		path: path.clone(),
		err,
	})?;
	let sha1_hash = sha1::sha1(&file_contents);
	let sha1_str = hex::encode(sha1_hash);
	println!("{sha1_str}");

	if write {
		let dirname = &sha1_str[0..2];
		let dirpath = PathBuf::from(format!(".git/objects/{dirname}"));
		if !dirpath
			.try_exists()
			.map_err(|err| HashObjectError::OutputIo {
				err,
				path: path.clone(),
			})? {
			fs::create_dir(&dirpath).map_err(|err| HashObjectError::OutputIo {
				err,
				path: path.clone(),
			})?;
		}

		let filename = dirpath.join(&sha1_str[2..]);
		let mut file = fs::File::create(filename).map_err(|err| HashObjectError::OutputIo {
			err,
			path: path.clone(),
		})?;

		write_object(ObjectKind::Blob(Cow::Borrowed(&file_contents)), &mut file).map_err(
			|err| HashObjectError::OutputIo {
				err,
				path: path.clone(),
			},
		)?;
	}

	Ok(())
}

enum ObjectKind<'a> {
	Blob(Cow<'a, [u8]>),
	Commit,
	Tag,
	Tree,
}

fn write_object<W: Write>(kind: ObjectKind, w: &mut W) -> Result<(), std::io::Error> {
	let mut zlibencoder = ZlibEncoder::new(w, flate2::Compression::default());
	match kind {
		ObjectKind::Blob(blob) => {
			let header = format!("blob {}\0", blob.len());
			zlibencoder.write_all(header.as_bytes())?;
			zlibencoder.write_all(&blob)?;
		}
		_ => unimplemented!(),
	}

	Ok(())
}
