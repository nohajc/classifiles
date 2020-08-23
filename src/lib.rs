use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, fs};
use std::os::unix::fs as unix_fs;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

mod mime_info;
use mime_info::{Mime, MimeInfoDb};

use magic::Cookie;
use walkdir::WalkDir;

use slog::{Logger, o, info};

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

trait Contains<T> {
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

    fn process_file(&mut self, input_path: &Path, log: &Logger) -> FileType {
        if let Some(mime_type) = tree_magic_mini::from_filepath(input_path) {
            let mut libmagic_used = false;

            let mime_type_final = if self.config.libmagic_used_for.contains_ref(mime_type) {
                match &self.cookie_mime_opt {
                    Some(cookie) => {
                        info!(log, "Match {} can be further refined", mime_type);
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
            info!(log, "File matches {}", mime_type_final);

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
                info!(log, "Guessed extension: {}", ext);
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

fn link_to_output(input: &Path, input_root: &Path, output_root: &Path, file_type: &FileType) -> Result<(), Box<dyn Error>> {
    let mut output_name = input.file_name()
        .map(|s| append_ext_if_needed(s, &file_type.ext))
        .unwrap_or(random_name(&file_type.ext));

    let mut output_link_dir = match &file_type.mime {
        Some(mime_str) => output_root.join(mime_str),
        None => output_root.join(OUTPUT_UNKNOWN),
    };
    if let Ok(input_rel) = input.strip_prefix(input_root) {
        if let Some(input_rel_dir) = input_rel.parent() {
            output_link_dir = output_link_dir.join(input_rel_dir);
        }
    }

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

struct BackupProcessor {
    params: Params
}

impl BackupProcessor {
    fn new(params: Params) -> Self {
        Self{params}
    }

    fn input_root(&self) -> &Path {
        &self.params.input_path
    }

    fn output_root(&self) -> &Path {
        &self.params.output_path
    }

    fn backup_item<F>(&self, src_path: &Path, creator: F) -> Result<(), Box<dyn Error>>
        where F: Fn(&Path) -> std::io::Result<()> {

        let src_rel_path = src_path.strip_prefix(self.input_root())?;
        let dst_path = self.output_root().to_owned().join(src_rel_path);
        creator(&dst_path)?;

        Ok(())
    }

    fn backup_dir(&self, src_path: &Path, log: &Logger)  -> Result<(), Box<dyn Error>> {
        self.backup_item(src_path, |dst| {
            // println!("read dir: {}, write to: {}", src_path.display(), dst.display());
            info!(log, "{} -> {}", src_path.display(), dst.display());
            fs::create_dir_all(dst)?;
            Ok(())
        })
    }

    fn backup_symlink(&self, src_path: &Path, log: &Logger)  -> Result<(), Box<dyn Error>> {
        self.backup_item(src_path, |dst| {
            let link_target = fs::read_link(src_path)?;

            let mut dst_str = dst.as_os_str().to_owned();
            dst_str.push(".lns");
            let dst_file = PathBuf::from(dst_str);

            // println!("read link from: {}, with target: {}, write to: {}",
            //     src_path.display(), link_target.display(), dst_file.display());
            info!(log, "{} -> {}", src_path.display(), dst_file.display());
            let link_target_bytes = link_target.as_os_str().as_bytes();
            fs::write(dst_file, [link_target_bytes, &[b'\n']].concat())?;
            Ok(())
        })
    }
}

fn get_entry_log(log: &Logger, item: &Path, i: usize, item_count: usize) -> Logger {
    let percent = (((i + 1) as f64 / item_count as f64) * 100.0) as u32;
    log.new(o!("progress" => format!("{} % ({}/{})", percent, i + 1, item_count)))
        .new(o!("item" => format!("{}", item.display())))
}

pub fn run_backup(params: Params, log: &Logger) -> Result<(), Box<dyn Error>> {
    if !params.output_path.is_dir() {
        return Err(Box::new(ClassifierError(
            format!("{} is not a directory", params.output_path.display())
        )));
    }

    let b_proc = BackupProcessor::new(params);
    let get_walker = || WalkDir::new(b_proc.input_root()).into_iter().filter_map(|e| e.ok());

    let item_count = get_walker().count();
    let walker = get_walker();

    for (i, entry) in walker.enumerate() {
        let entry_log = get_entry_log(log, entry.path(), i, item_count);

        if let Ok(entry_info) = fs::symlink_metadata(entry.path()) {
            if entry_info.is_dir() {
                // println!("Visiting {}", entry.path().display());
                b_proc.backup_dir(entry.path(), &entry_log)?;
            } else if entry_info.file_type().is_symlink() {
                b_proc.backup_symlink(entry.path(), &entry_log)?;
            }
        }
    }

    Ok(())
}

struct RestoreProcessor {
    params: Params
}

impl RestoreProcessor {
    fn new(params: Params) -> Self {
        Self{params}
    }

    fn input_root(&self) -> &Path {
        &self.params.input_path
    }

    fn output_root(&self) -> &Path {
        &self.params.output_path
    }

    fn restore_item<F>(&self, src_path: &Path, creator: F) -> Result<(), Box<dyn Error>>
        where F: Fn(&Path) -> Result<(), Box<dyn Error>> {

        let src_rel_path = src_path.strip_prefix(self.input_root())?;
        let dst_path = self.output_root().to_owned().join(src_rel_path);
        creator(&dst_path)?;

        Ok(())
    }

    fn restore_dir(&self, src_path: &Path, log: &Logger)  -> Result<(), Box<dyn Error>> {
        self.restore_item(src_path, |dst| {
            // println!("read dir: {}, write to: {}", src_path.display(), dst.display());
            info!(log, "{} -> {}", src_path.display(), dst.display());
            fs::create_dir_all(dst)?;
            Ok(())
        })
    }

    fn restore_symlink(&self, src_path: &Path, log: &Logger)  -> Result<(), Box<dyn Error>> {
        self.restore_item(src_path, |dst| {
            if let Some(ext) = src_path.extension() {
                if ext == OsStr::new("lns") {
                    let src_bytes = fs::read(src_path)?;
                    let link_bytes = if src_bytes[src_bytes.len() - 1] == b'\n' {
                        &src_bytes[0..src_bytes.len()-1]
                    } else {
                        &src_bytes[..]
                    };
                    let link_target = Path::new(OsStr::from_bytes(link_bytes));

                    let dst_file = match dst.file_stem() {
                        Some(file_stem) => {
                            let parent_path = dst.parent().ok_or("could not extract parent path")?;
                            parent_path.join(file_stem)
                        }
                        None => dst.to_owned()
                    };
                    info!(log, "{} -> {}", src_path.display(), dst_file.display());
                    unix_fs::symlink(link_target, dst_file)?;
                }
            }
            Ok(())
        })
    }
}

pub fn run_restore(params: Params, log: &Logger) -> Result<(), Box<dyn Error>> {
    if !params.output_path.is_dir() {
        return Err(Box::new(ClassifierError(
            format!("{} is not a directory", params.output_path.display())
        )));
    }

    let r_proc = RestoreProcessor::new(params);
    let get_walker = || WalkDir::new(r_proc.input_root()).into_iter().filter_map(|e| e.ok());

    let item_count = get_walker().count();
    let walker = get_walker();

    for (i, entry) in walker.enumerate() {
        let entry_log = get_entry_log(log, entry.path(), i, item_count);

        if let Ok(entry_info) = fs::symlink_metadata(entry.path()) {
            if entry_info.is_dir() {
                // println!("Visiting {}", entry.path().display());
                r_proc.restore_dir(entry.path(), &entry_log)?;
            } else if entry_info.is_file() {
                r_proc.restore_symlink(entry.path(), &entry_log)?;
            }
        }
    }

    Ok(())
}

pub fn run_scan(config: Config, params: Params, log: &Logger) -> Result<(), Box<dyn Error>> {
    let mut classifier = Classifier::new(config);

    if !params.output_path.is_dir() {
        return Err(Box::new(ClassifierError(
            format!("{} is not a directory", params.output_path.display())
        )));
    }

    let get_walker = || WalkDir::new(&params.input_path).into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file());

    let file_count = get_walker().count();
    let walker = get_walker();

    for (i, entry) in walker.enumerate() {
        let entry_log = get_entry_log(log, entry.path(), i, file_count);

        let file_type = classifier.process_file(entry.path(), &entry_log);
        link_to_output(entry.path(), &params.input_path, &params.output_path, &file_type)?;
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
