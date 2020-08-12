use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, fs};
use std::os::unix::fs as unix_fs;
use std::ffi::OsStr;

mod mime_info;
use mime_info::{Mime, MimeInfoDb};

use magic::Cookie;
use walkdir::WalkDir;

#[derive(Debug)]
pub struct Config {
    pub mime_info_db_root: PathBuf,
    pub libmagic_db_file: PathBuf,
    pub libmagic_used_for: Vec<String>,
}

#[derive(Debug)]
pub struct Params {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
}

trait Contains<T> where {
    fn contains_ref(&self, val: T) -> bool;
}

impl<T, U> Contains<U> for [T] where T: PartialEq<U> {
    fn contains_ref(&self, val: U) -> bool {
        self.iter().any(|x| x == &val)
    }
}

fn guess_extension<'a>(mime_info_db: &'a mut MimeInfoDb, mime_type: &str) -> Option<&'a str> {
    let mime = mime_info_db.get(mime_type);
    match mime {
        Mime::WithExt(ext) => Some(ext),
        _ => None,
    }
}

fn get_magic_cookie(libmagic_db_file: &Path, flags: magic::flags::CookieFlags) -> Result<Cookie, Box<dyn Error>> {
    let cookie = Cookie::open(flags)?;
    let databases = [libmagic_db_file];

    match cookie.load(&databases) {
        Ok(()) => Ok(cookie),
        Err(e) => Err(Box::new(e)),
    }
}

fn get_magic_cookie_opt(libmagic_db_file: &Path, flags: magic::flags::CookieFlags) -> Option<Cookie> {
    match get_magic_cookie(libmagic_db_file, flags) {
        Ok(cookie) => Some(cookie),
        Err(e) => {
            eprintln!("Warning: could not load magic cookie from {}: {}", libmagic_db_file.display(), e);
            None
        },
    }
}

#[derive(Debug)]
struct ClassifierError(String);

impl fmt::Display for ClassifierError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for ClassifierError {}

struct Classifier {
    config: Config,
    cookie_mime_opt: Option<Cookie>,
    cookie_ext_opt: Option<Cookie>,
    mime_info_db: MimeInfoDb,
}

#[derive(Debug)]
struct FileType {
    mime: Option<String>,
    ext: Option<String>,
}

impl FileType {
    fn unknown() -> Self {
        Self{mime: None, ext: None}
    }
}

impl Classifier {
    fn new(config: Config) -> Self {
        let mime_info_db = MimeInfoDb::new(&config.mime_info_db_root);
        let cookie_mime_opt = get_magic_cookie_opt(&config.libmagic_db_file, magic::flags::MIME_TYPE);
        let cookie_ext_opt = get_magic_cookie_opt(&config.libmagic_db_file, magic::flags::EXTENSION);

        Classifier{config, cookie_mime_opt, cookie_ext_opt, mime_info_db}
    }

    fn process_file(&mut self, input_path: &Path) -> FileType {
        if let Some(mime_type) = tree_magic_mini::from_filepath(input_path) {
            let input_path_str = input_path.display();
            let mut libmagic_used = false;

            let mime_type_final = if self.config.libmagic_used_for.contains_ref(mime_type) {
                match &self.cookie_mime_opt {
                    Some(cookie) => {
                        println!("{}: Match {} can be further refined", input_path_str, mime_type);
                        match cookie.file(input_path) {
                            Ok(mime_type2) => {
                                libmagic_used = true;
                                mime_type2
                            },
                            Err(_) => mime_type.to_owned(),
                        }
                    },
                    None => mime_type.to_owned(),
                }
            } else {
                mime_type.to_owned()
            };
            println!("{}: File matches {}", input_path_str, mime_type_final);

            if let Some(ext) = guess_extension(&mut self.mime_info_db, &mime_type_final).map(str::to_owned).or_else(|| {
                match &self.cookie_ext_opt {
                    Some(cookie) if libmagic_used =>
                        match cookie.file(input_path) {
                            Ok(exts) if exts.len() > 0 && exts != "???" => {
                                let ext = exts.split('/').next().unwrap().to_owned();
                                // libmagic cannot return both mime and extension in one operation
                                // but we can cache the mapping to avoid matching each file twice
                                self.mime_info_db.set(&mime_type_final, &ext);
                                Some(ext)
                            },
                            _ => None,
                        }
                    _ => None,
                }
            }) {
                println!("{}: Guessed extension: {}", input_path_str, ext);
                return FileType{mime: Some(mime_type_final), ext: Some(ext)};
            }

            return FileType{mime: Some(mime_type_final), ext: None};
        }

        FileType::unknown()
    }
}

static OUTPUT_UNKNOWN: &str = "unknown";

fn random_name(ext: &Option<String>) -> PathBuf {
    use rand::Rng;
    use rand::distributions::Alphanumeric;

    let mut name = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .collect::<String>();

    if let Some(e) = ext {
        name += &e
    }
    PathBuf::from(name)
}

fn append_ext_if_needed(file_name: &OsStr, ext: &Option<String>) -> PathBuf {
    if let Some(ext) = ext {
        let file_ext = Path::new(file_name).extension().unwrap_or(OsStr::new(""));

        if file_ext != OsStr::new(ext) {
            // if the file does not already have the guessed extension, append it
            let mut new_file_name = file_name.to_owned();
            new_file_name.push(".");
            new_file_name.push(ext);
            return PathBuf::from(new_file_name);
        }
    }

    PathBuf::from(file_name)
}

fn link_to_output(input: &Path, output_root: &Path, file_type: &FileType) -> Result<(), Box<dyn Error>> {
    let mut output_name = input.file_name()
        .map(|s| append_ext_if_needed(s, &file_type.ext))
        .unwrap_or(random_name(&file_type.ext));

    let output_link_dir = match &file_type.mime {
        Some(mime_str) => output_root.join(mime_str),
        None => output_root.join(OUTPUT_UNKNOWN),
    };
    fs::create_dir_all(&output_link_dir)?;

    while fs::symlink_metadata(output_link_dir.join(&output_name)).is_ok() {
        // path already exists so we have to use a different name
        output_name = match output_name.file_stem() {
            Some(stem) => {
                let mut output_name_str = stem.to_owned(); // output_name without extension
                output_name_str.push("-");
                output_name_str.push(&random_name(&None)); // random string
                match output_name.extension() {
                    Some(out_ext) => {
                        output_name_str.push(".");
                        output_name_str.push(out_ext); // output_name extension
                    }
                    None => ()
                }

                PathBuf::from(output_name_str)
            }
            None => random_name(&file_type.ext)
        }
    }

    unix_fs::symlink(input, output_link_dir.join(&output_name))?;
    Ok(())
}

pub fn run_scan(config: Config, params: Params) -> Result<(), Box<dyn Error>> {
    let mut classifier = Classifier::new(config);

    if !params.output_path.is_dir() {
        return Err(Box::new(ClassifierError(
            format!("{} is not a directory", params.output_path.display())
        )));
    }

    let input_info = fs::metadata(&params.input_path)?;
    if input_info.is_file() {
        let file_type = classifier.process_file(&params.input_path);
        link_to_output(&params.input_path, &params.output_path, &file_type)?;
        // println!("{:?}", file_type);
    } else {
        let walker = WalkDir::new(&params.input_path).into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file());

        for entry in walker {
            let file_type = classifier.process_file(entry.path());
            link_to_output(entry.path(), &params.output_path, &file_type)?;
        }
    }

    // let mime = mime_info_db.get("application/zip");
    // println!("{:?}", mime);
    // let mime = mime_info_db.get("application/zip");
    // println!("{:?}", mime);

    // let mime = mime_info_db.get("application/vnd.rar");
    // println!("{:?}", mime);
    // let mime = mime_info_db.get("application/vnd.rar");
    // println!("{:?}", mime);

    // let mime = mime_info_db.get("application/octet-stream");
    // println!("{:?}", mime);
    // let mime = mime_info_db.get("application/octet-stream");
    // println!("{:?}", mime);

    // let mime = mime_info_db.get("application/zip");
    // println!("{:?}", mime);

    // let mime = mime_info_db.get("get/schwifty");
    // println!("{:?}", mime);
    // let mime = mime_info_db.get("get/schwifty");
    // println!("{:?}", mime);
    // let mime = mime_info_db.get("get/schwifty");
    // println!("{:?}", mime);

    Ok(())
}
