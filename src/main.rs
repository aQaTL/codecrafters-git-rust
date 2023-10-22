#![allow(dead_code)]

use std::borrow::Cow;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

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

	WriteTree,
}

fn main() {
	let args = Args::parse();

	let result: Result<(), Box<dyn std::error::Error>> = match args.command {
		Command::Init => init().map_err(Into::into),
		Command::CatFile {
			pretty_print,
			object,
		} => cat_file(object, pretty_print).map_err(Into::into),
		Command::HashObject { write, file } => hash_object_cmd(file, write).map_err(Into::into),
		Command::LsTree { name_only, object } => ls_tree(object, name_only).map_err(Into::into),
		Command::WriteTree => write_tree().map_err(Into::into),
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

	let file = decode_object(object)?;

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
	#[error("Could not open {path} for reading: {err}")]
	InputIo {
		#[source]
		err: std::io::Error,

		path: PathBuf,
	},

	#[error("Could not open {path}: {err}")]
	OutputIo {
		#[source]
		err: std::io::Error,

		path: PathBuf,
	},

	#[error("Failed to encode: {0}")]
	EncodeObject(std::io::Error),
}

fn hash_object_cmd(path: PathBuf, write: bool) -> Result<(), HashObjectError> {
	let sha1_str = hash_object(&path, write)?.hash_str;
	println!("{sha1_str}");
	Ok(())
}

fn hash_object(path: &Path, write: bool) -> Result<HashedObject, HashObjectError> {
	let file_contents = fs::read(path).map_err(|err| HashObjectError::InputIo {
		path: path.to_owned(),
		err,
	})?;

	let hashed_object = hash_git_object(GitObject::Blob(Cow::Borrowed(&file_contents)), write)?;
	Ok(hashed_object)
}

/// Encodes and hashes given [GitObject]. Returns the SHA1 hash of that object.
fn hash_git_object(object: GitObject, write: bool) -> Result<HashedObject, HashObjectError> {
	let mut encoded_file_content = Vec::new();
	encode_object(object, &mut encoded_file_content).map_err(HashObjectError::EncodeObject)?;

	let sha1_hash = sha1::sha1(&encoded_file_content);
	let sha1_str = hex::encode(sha1_hash);

	if write {
		let dirname = &sha1_str[0..2];
		let dirpath = PathBuf::from(format!(".git/objects/{dirname}"));
		if !dirpath
			.try_exists()
			.map_err(|err| HashObjectError::OutputIo {
				err,
				path: dirpath.clone(),
			})? {
			fs::create_dir(&dirpath).map_err(|err| HashObjectError::OutputIo {
				err,
				path: dirpath.clone(),
			})?;
		}

		let filename = dirpath.join(&sha1_str[2..]);
		if filename.exists() {
			return Ok(HashedObject {
				hash: sha1_hash,
				hash_str: sha1_str,
			});
		}
		let mut file = fs::File::create(&filename).map_err(|err| HashObjectError::OutputIo {
			err,
			path: filename.clone(),
		})?;

		let mut zlibencoder = ZlibEncoder::new(&mut file, flate2::Compression::default());
		zlibencoder
			.write_all(&encoded_file_content)
			.map_err(|err| HashObjectError::OutputIo {
				err,
				path: filename,
			})?;
	}

	Ok(HashedObject {
		hash: sha1_hash,
		hash_str: sha1_str,
	})
}

struct HashedObject {
	hash: [u8; 20],
	hash_str: String,
}

enum GitObject<'a> {
	Blob(Cow<'a, [u8]>),
	Commit,
	Tag,
	Tree(Cow<'a, [TreeEntry<'a>]>),
}

#[derive(Clone)]
struct TreeEntry<'a> {
	mode: u32,
	name: Cow<'a, str>,
	object_hash: Cow<'a, [u8; 20]>,
}

impl From<IndexEntry> for TreeEntry<'static> {
	fn from(entry: IndexEntry) -> Self {
		TreeEntry {
			mode: entry.mode,
			name: entry.path.into(),
			object_hash: Cow::Owned(entry.sha1),
		}
	}
}

fn encode_object<W: Write>(kind: GitObject, w: &mut W) -> Result<(), std::io::Error> {
	match kind {
		GitObject::Blob(blob) => encode_blob(blob, w),
		GitObject::Tree(entries) => encode_tree(&entries, w),
		_ => unimplemented!(),
	}
}

fn encode_blob<W: Write>(blob: Cow<[u8]>, w: &mut W) -> Result<(), std::io::Error> {
	let header = format!("blob {}", blob.len());
	w.write_all(header.as_bytes())?;
	w.write_all(&[0_u8])?;
	w.write_all(&blob)?;
	Ok(())
}

fn encode_tree<W: Write>(entries: &[TreeEntry], w: &mut W) -> Result<(), std::io::Error> {
	w.write_all(b"tree ")?;

	let mut size = 21 * entries.len();
	for entry in entries {
		size += format!("{:o} {}", entry.mode, entry.name).len();
	}

	w.write_all(size.to_string().as_bytes())?;
	w.write_all(&[0_u8])?;

	for entry in entries {
		w.write_all(format!("{:o} {}", entry.mode, entry.name).as_bytes())?;
		w.write_all(&[0])?;
		w.write_all(entry.object_hash.as_slice())?;
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

fn decode_object(mut sha1: String) -> Result<GitObject<'static>, ReadObjectError> {
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
						u32::from_str_radix(mode_str, 8).map_err(|_| ReadObjectError::TreeEntryMode)
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
					object_hash: Cow::Borrowed(object_hash),
				});

				if rest.len() > 20 {
					rest = &rest[20..];
				} else {
					break;
				}
			}

			Ok(GitObject::Tree(Cow::Owned(tree_entries)))
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

	let object = decode_object(object)?;

	let GitObject::Tree(tree_entries) = object else {
		return Err(LsTreeError::NotATree);
	};
	for entry in tree_entries.iter() {
		println!("{}", entry.name);
	}

	Ok(())
}

#[derive(Debug, Error)]
enum WriteTreeError {
	#[error("index: {0}")]
	ReadIndex(#[from] ReadIndexError),

	#[error(transparent)]
	Io(#[from] std::io::Error),

	#[error(transparent)]
	HashObject(#[from] HashObjectError),
}

fn write_tree() -> Result<(), WriteTreeError> {
	// let index = read_index()?;
	//
	// let tree_entries = index
	// 	.entries
	// 	.into_iter()
	// 	.map(TreeEntry::from)
	// 	.collect::<Vec<_>>();
	// let sha1_str = hash_object(GitObject::Tree(tree_entries), true)?;
	// println!("{sha1_str}");

	let tree = read_tree_from_dir(".".as_ref())?;
	let hash_str = hex::encode(tree.hash.as_slice());
	println!("{hash_str}",);

	Ok(())
}

struct Tree<'a> {
	hash: Cow<'a, [u8; 20]>,
	mode: u32,
	name: Cow<'a, str>,
	entries: Vec<TreeEntry<'a>>,
}

fn read_tree_from_dir(path: &Path) -> Result<Tree<'static>, WriteTreeError> {
	let mut entries = Vec::new();

	let read_dir = fs::read_dir(path)?;
	for entry in read_dir {
		let entry = match entry {
			Ok(v) => v,
			Err(err) => {
				eprintln!("WARN cannot read: {err}");
				continue;
			}
		};

		let path = entry.path();
		let path = path.strip_prefix(".").unwrap_or(&path);
		let Some(file_name) = path.file_name() else {
			continue;
		};

		let file_name = file_name.to_str().unwrap().to_string();
		if file_name.starts_with('.') {
			continue;
		}

		if path.is_file() {
			let hashed_object = hash_object(path, true)?;
			entries.push(TreeEntry {
				mode: path.metadata().unwrap().mode(),
				name: Cow::Owned(file_name),
				object_hash: Cow::Owned(hashed_object.hash),
			});
		} else {
			let tree = read_tree_from_dir(path)?;
			entries.push(TreeEntry {
				mode: tree.mode,
				name: Cow::Owned(file_name),
				object_hash: tree.hash,
			});
		}
	}

	entries.sort_by_key(|e| e.name.clone());
	let hashed_object = hash_git_object(GitObject::Tree(Cow::Borrowed(&entries)), true)?;

	Ok(Tree {
		hash: Cow::Owned(hashed_object.hash),
		mode: path.metadata().unwrap().mode(),
		name: Cow::Owned(path.display().to_string()),
		entries,
	})
}

#[derive(Debug, Error)]
enum ReadIndexError {
	#[error(transparent)]
	Io(#[from] std::io::Error),

	#[error("Failed to read index SHA1 hash")]
	NoIndexHash,

	#[error("Failed to read index header")]
	NoIndexHeader,

	#[error("Invalid index signature {0}")]
	InvalidSignature(String),

	#[error("Failed to read index entries")]
	NoIndexEntries,

	#[error("Missing index entries. Expected {expected}, got {got}.")]
	MissingEntries { expected: usize, got: usize },

	#[error("Path missing from an index entry")]
	NoIndexEntryPath,

	#[error("Path is not a valid string: {0}")]
	CorruptedPath(std::str::Utf8Error),
}

#[derive(Debug)]
#[allow(dead_code)]
struct Index {
	sha1: [u8; 20],
	version: u32,
	entries: Vec<IndexEntry>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct IndexEntry {
	ctime_s: u32,
	ctime_n: u32,
	mtime_s: u32,
	mtime_n: u32,
	dev: u32,
	ino: u32,
	mode: u32,
	uid: u32,
	gid: u32,
	size: u32,
	sha1: [u8; 20],
	flags: u16,
	path: String,
}

fn read_index() -> Result<Index, ReadIndexError> {
	let index = fs::read(".git/index")?;

	let sha1 = index
		.get((index.len() - 20)..)
		.ok_or(ReadIndexError::NoIndexHash)?;
	let sha1 = unsafe { *(sha1.as_ptr() as *const [u8; 20]) };

	let header = index.get(..12).ok_or(ReadIndexError::NoIndexHeader)?;
	let signature = &header[0..4];
	let version = unsafe { (*header.as_ptr().add(4).cast::<u32>()).to_be() };
	let num_entries = unsafe { (*header.as_ptr().add(8).cast::<u32>()).to_be() };

	if signature != b"DIRC" {
		return Err(ReadIndexError::InvalidSignature(
			String::from_utf8_lossy(signature).to_string(),
		));
	}

	let mut entries_bytes = index
		.get(12..(index.len() - 20))
		.ok_or(ReadIndexError::NoIndexEntries)?;

	let mut entries = Vec::new();

	let idx = 0;
	while idx + 62 < entries_bytes.len() {
		let fields = &entries_bytes[..62];

		let null_byte_idx = entries_bytes
			.iter()
			.skip(idx + 62)
			.position(|x| *x == 0)
			.ok_or(ReadIndexError::NoIndexEntryPath)?;

		let path = &entries_bytes[(idx + 62)..(idx + 62 + null_byte_idx)];
		let path = std::str::from_utf8(path)
			.map_err(ReadIndexError::CorruptedPath)?
			.to_string();
		let path_len = path.len();

		let fields_u32_ptr = fields.as_ptr().cast::<u32>();

		entries.push(unsafe {
			IndexEntry {
				ctime_s: (*fields_u32_ptr).to_be(),
				ctime_n: (*fields_u32_ptr.add(1)).to_be(),
				mtime_s: (*fields_u32_ptr.add(2)).to_be(),
				mtime_n: (*fields_u32_ptr.add(3)).to_be(),
				dev: (*fields_u32_ptr.add(4)).to_be(),
				ino: (*fields_u32_ptr.add(5)).to_be(),
				mode: (*fields_u32_ptr.add(6)).to_be(),
				uid: (*fields_u32_ptr.add(7)).to_be(),
				gid: (*fields_u32_ptr.add(8)).to_be(),
				size: (*fields_u32_ptr.add(9)).to_be(),
				sha1: *(fields[40..60].as_ptr() as *const [u8; 20]),
				flags: (*fields.as_ptr().add(60).cast::<u16>()).to_be(),
				path,
			}
		});

		entries_bytes = &entries_bytes[(((62 + path_len + 8) / 8) * 8)..];
	}

	if entries.len() != num_entries as usize {
		return Err(ReadIndexError::MissingEntries {
			expected: num_entries as usize,
			got: entries.len(),
		});
	}

	Ok(Index {
		sha1,
		version,
		entries,
	})
}
