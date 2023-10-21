use std::borrow::Cow;
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

	CatFile {
		#[arg(short, long)]
		pretty_print: bool,

		#[arg(required = true)]
		object: String,
	},

	HashObject {
		#[arg(short)]
		write: bool,

		#[arg(required = true)]
		file: PathBuf,
	},

	LsTree {
		#[arg(short, long)]
		name_only: bool,

		#[arg(required = true)]
		object: String,
	},
}

fn main() {
	let args = Args::parse();

	let result: Result<(), Box<dyn std::error::Error>> = match args.command {
		Command::Init => init().map_err(Into::into),
		Command::CatFile {
			pretty_print,
			object,
		} => cat_file(object, pretty_print).map_err(Into::into),
		Command::HashObject { write, file } => hash_object(file, write).map_err(Into::into),
		Command::LsTree { name_only, object } => ls_tree(object, name_only).map_err(Into::into),
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

	#[error(transparent)]
	ReadObject(#[from] ReadObjectError),
}

fn cat_file(object: String, pretty_print: bool) -> Result<(), CatFileError> {
	if object.len() != 40 {
		return Err(CatFileError::InvalidObjectName(object));
	}

	if !pretty_print {
		return Err(CatFileError::MustUsePrettyPrint);
	}

	let file = read_object(object)?;

	let file_content_bytes: &[u8] = match file {
		GitObject::Blob(ref file_content) => file_content,
		_ => unimplemented!(),
	};

	let file_content = String::from_utf8_lossy(file_content_bytes);
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

	let mut encoded_file_content = Vec::new();
	write_object(
		GitObject::Blob(Cow::Borrowed(&file_contents)),
		&mut encoded_file_content,
	)
	.map_err(|err| HashObjectError::OutputIo {
		err,
		path: path.clone(),
	})?;

	let sha1_hash = sha1::sha1(&encoded_file_content);
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

		let mut zlibencoder = ZlibEncoder::new(&mut file, flate2::Compression::default());
		zlibencoder
			.write_all(&encoded_file_content)
			.map_err(|err| HashObjectError::OutputIo {
				err,
				path: path.clone(),
			})?;
	}

	Ok(())
}

#[allow(dead_code)]
enum GitObject<'a> {
	Blob(Cow<'a, [u8]>),
	Commit,
	Tag,
	Tree(Vec<TreeEntry<'a>>),
}

#[allow(dead_code)]
struct TreeEntry<'a> {
	mode: u32,
	name: Cow<'a, str>,
	object_hash: &'a [u8; 20],
}

fn write_object<W: Write>(kind: GitObject, w: &mut W) -> Result<(), std::io::Error> {
	match kind {
		GitObject::Blob(blob) => {
			let header = format!("blob {}", blob.len());
			w.write_all(header.as_bytes())?;
			w.write_all(&[0_u8])?;
			w.write_all(&blob)?;
		}
		_ => unimplemented!(),
	}

	Ok(())
}

#[derive(Debug, Error)]
enum ReadObjectError {
	#[error(transparent)]
	Io(#[from] std::io::Error),

	#[error("Invalid object: {0}")]
	InvalidHash(#[from] hex::FromHexError),

	#[error("Corrupted object {context}")]
	CorruptedObject { context: &'static str },

	#[error("Unknown object kind")]
	UnknownObjectKind,

	#[error("Invalid object size")]
	InvalidObjectSize,

	#[error("Corrupted tree entry")]
	CorruptedTreeEntry,

	#[error("Corrupted tree entry mode")]
	TreeEntryMode,

	#[error("Invalid tree entry name: {0}")]
	TreeEntryName(std::str::Utf8Error),

	#[error("Corrupted tree entry SHA1")]
	CorruptedTreeEntrySha1,
}

fn read_object(mut sha1: String) -> Result<GitObject<'static>, ReadObjectError> {
	sha1.make_ascii_lowercase();
	// Just a check that a given sha1 is correct
	let _ = hex::decode(&sha1)?;

	let dirname = &sha1[0..2];
	let filename = &sha1[2..];

	let file = fs::File::open(format!(".git/objects/{dirname}/{filename}"))?;
	let file_buffered = BufReader::new(file);
	let mut decoder = flate2::bufread::ZlibDecoder::new(file_buffered);

	let mut file_content_bytes = Vec::new();
	decoder.read_to_end(&mut file_content_bytes)?;

	if file_content_bytes.len() <= 1 {
		return Err(ReadObjectError::CorruptedObject {
			context: "too short",
		});
	}
	let space_idx = file_content_bytes.iter().position(|x| *x == b' ').ok_or(
		ReadObjectError::CorruptedObject {
			context: "no space",
		},
	)?;

	let (object_type, mut rest) = file_content_bytes.split_at(space_idx);
	// Skip space
	rest = &rest[1..];

	let null_byte_idx =
		rest.iter()
			.position(|x| !x.is_ascii_digit())
			.ok_or(ReadObjectError::CorruptedObject {
				context: "only ascii digits???",
			})?;
	if rest[null_byte_idx] != 0 {
		return Err(ReadObjectError::CorruptedObject {
			context: "byte after digits isn't null",
		});
	}

	// Safety iterator to find the null byte checked that all bytes are ascii digits
	let size: u64 = unsafe {
		std::str::from_utf8_unchecked(&rest[..null_byte_idx])
			.parse()
			.expect("all bytes were checked to be ascii digits")
	};

	rest = &rest[(null_byte_idx + 1)..];
	rest = rest
		.get(..(size as usize))
		.ok_or(ReadObjectError::InvalidObjectSize)?;

	match object_type {
		b"blob" => Ok(GitObject::Blob(Cow::Owned(rest.to_vec()))),
		b"commit" => {
			unimplemented!()
		}
		b"tag" => {
			unimplemented!()
		}
		b"tree" => {
			let mut tree_entries = Vec::new();
			loop {
				let space_idx = rest
					.iter()
					.position(|x| *x == b' ')
					.ok_or(ReadObjectError::CorruptedTreeEntry)?;

				let mode: u32 = std::str::from_utf8(&rest[..space_idx])
					.map_err(|_| ReadObjectError::TreeEntryMode)
					.and_then(|mode_str| {
						mode_str.parse().map_err(|_| ReadObjectError::TreeEntryMode)
					})?;

				rest = rest
					.get((space_idx + 1)..)
					.ok_or(ReadObjectError::CorruptedTreeEntry)?;

				let null_byte_idx = rest
					.iter()
					.position(|x| *x == 0)
					.ok_or(ReadObjectError::CorruptedTreeEntry)?;

				let name = std::str::from_utf8(&rest[..null_byte_idx])
					.map_err(ReadObjectError::TreeEntryName)?;
				let name = Cow::Owned(name.to_string());

				rest = rest
					.get((null_byte_idx + 1)..)
					.ok_or(ReadObjectError::CorruptedTreeEntry)?;

				if rest.len() < 20 {
					return Err(ReadObjectError::CorruptedTreeEntrySha1);
				}

				let object_hash = unsafe { &*(rest[..20].as_ptr() as *const [u8; 20]) };

				tree_entries.push(TreeEntry {
					mode,
					name,
					object_hash,
				});

				if rest.len() > 20 {
					rest = &rest[20..];
				} else {
					break;
				}
			}

			Ok(GitObject::Tree(tree_entries))
		}
		_ => Err(ReadObjectError::UnknownObjectKind),
	}
}

#[derive(Debug, Error)]
enum LsTreeError {
	#[error("You must use --name-only option right now :/")]
	MustUseNameOnly,

	#[error(transparent)]
	ReadObject(#[from] ReadObjectError),

	#[error("Not a tree object")]
	NotATree,
}

fn ls_tree(object: String, name_only: bool) -> Result<(), LsTreeError> {
	if !name_only {
		return Err(LsTreeError::MustUseNameOnly);
	}

	let object = read_object(object)?;

	let GitObject::Tree(tree_entries) = object else {
		return Err(LsTreeError::NotATree);
	};
	for entry in tree_entries {
		println!("{}", entry.name);
	}

	Ok(())
}
