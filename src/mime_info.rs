use std::{error::Error, fmt, fs::{self, File}, str};
use fnv::FnvHashMap;
use std::path::{Path, PathBuf};
use std::io::Read;

#[derive(Debug)]
struct MimeInfoDbError(String);

impl fmt::Display for MimeInfoDbError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for MimeInfoDbError {}

#[derive(Debug, PartialEq, Eq)]
pub enum Mime {
    Generic,
    WithExt(String),
    Unknown,
}

pub struct MimeInfoDb {
    db_root_path: Option<PathBuf>,
    mime_map: FnvHashMap<String, Mime>,
}

impl MimeInfoDb {
    pub fn new(db_root_path: &Path) -> Self {
        let path_info_result = fs::metadata(db_root_path);
        let db_root_opt = match path_info_result {
            Ok(path_info) => if path_info.is_dir() {
                Some(db_root_path)
            } else {
                eprintln!("Warning: ignoring db_root_path, it is not a directory: {}", db_root_path.display());
                None
            }
            _ => {
                eprintln!("Warning: ignoring non-existing db_root_path {}", db_root_path.display());
                None
            }
        };

        Self{
            db_root_path: db_root_opt.map(PathBuf::from),
            mime_map: FnvHashMap::default(),
        }
    }

    pub fn get(&mut self, mime: &str) -> &Mime {
        let Self { db_root_path, mime_map } = self;

        let entry = mime_map.entry(mime.to_owned());
        entry.or_insert_with(|| {
            let mime_info = match db_root_path {
                Some(db_root) => Self::load_mime_info(db_root, mime),
                None => Mime::Unknown,
            };
            if mime_info == Mime::Unknown {
                // eprintln!("using secondary extension db");
                match mime_db::extensions(mime) {
                    Some(exts) => if exts.len() > 0 {
                        Mime::WithExt(exts[0].to_owned())
                    } else {
                        Mime::Generic
                    },
                    None => Mime::Unknown,
                }
            } else {
                mime_info
            }
        })
    }

    pub fn set(&mut self, mime: &str, ext: &str) {
        self.mime_map.insert(mime.to_owned(), Mime::WithExt(ext.to_owned()));
    }

    fn load_mime_info(root_path: &Path, mime: &str) -> Mime {
        let mime_path = root_path.join(format!("{}.xml", mime));
        // eprintln!("loading {} from {}", mime, mime_path.display());

        let mime_info_file = File::open(mime_path);
        match mime_info_file {
            Ok(mut file) => Self::parse_mime_info(&mut file),
            Err(_) => Mime::Unknown,
        }
    }

    fn extract_glob(doc: &roxmltree::Document) -> Mime {
        match doc.descendants().find(|n| n.tag_name().name() == "glob") {
            Some(node) => match node.attribute("pattern") {
                Some(pattern) => Mime::WithExt(pattern.trim_start_matches("*.").to_owned()),
                None => Mime::Generic,
            },
            None => Mime::Generic,
        }
    }

    fn parse_mime_info(f: &mut File) -> Mime {
        use roxmltree::Document;
        let mut xml_str = String::new();
        if let Err(_) = f.read_to_string(&mut xml_str) {
            return Mime::Unknown;
        }
        match Document::parse(&xml_str) {
            Ok(doc) => Self::extract_glob(&doc),
            Err(e) => panic!("Error: {}", e),
        }
    }
}
