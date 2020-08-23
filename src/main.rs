use std::{env, error::Error, fs, path::{Path, PathBuf}, process};
use classifiles::{Config, Params};

use slog::{o, Drain};

mod yaml_conf {
    use serde::{Serialize, Deserialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    pub struct Config {
        pub mime_info_db: InfoDbConfig,
        pub libmagic: LibMagicConfig,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    pub struct InfoDbConfig {
        pub root: String,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    pub struct LibMagicConfig {
        pub db_file: String,
        pub used_for: Vec<String>,
    }
}

fn config_from_yaml(cfg_path: impl AsRef<Path>) -> Result<yaml_conf::Config, Box<dyn Error>> {
    let conf_str = fs::read_to_string(cfg_path)?;
    let conf: yaml_conf::Config = serde_yaml::from_str(&conf_str)?;
    Ok(conf)
}

fn main() {
    let mut args = env::args();
    // skip program name
    args.next();

    let verb = args.next().unwrap_or("".to_owned());

    let decorator = slog_term::TermDecorator::new().stdout().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let async_drain = slog_async::Async::new(drain).build().fuse();

    let root_log = slog::Logger::root(async_drain, o!());

    match verb.as_str() {
        "scan" => {
            let input_path = PathBuf::from(args.next().unwrap_or_else(|| {
                eprintln!("Error: missing input path argument");
                process::exit(1)
            }));

            let output_path = PathBuf::from(args.next().unwrap_or_else(|| {
                eprintln!("Error: missing output path argument");
                process::exit(1)
            }));

            let params = Params{input_path, output_path};

            let config = match config_from_yaml("config.yaml") {
                Ok(conf) => {
                    eprintln!("Using configuration from config.yaml");
                    Config{
                        mime_info_db_root: PathBuf::from(conf.mime_info_db.root),
                        libmagic_db_file: PathBuf::from(conf.libmagic.db_file),
                        libmagic_used_for: conf.libmagic.used_for,
                    }
                }
                Err(_) => {
                    eprintln!("Using default configuration");
                    Config{
                        mime_info_db_root: PathBuf::from("/usr/share/mime"),
                        libmagic_db_file: PathBuf::from("/usr/share/file/misc/magic.mgc"),
                        libmagic_used_for: vec![
                            "application/zip".to_owned(),
                            //"application/x-sharedlib".to_owned()
                        ],
                    }
                }
            };

            if let Err(e) = classifiles::run_scan(config, params, &root_log) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        "backup" => {
            let input_path = PathBuf::from(args.next().unwrap_or_else(|| {
                eprintln!("Error: missing input path argument");
                process::exit(1)
            }));

            let output_path = PathBuf::from(args.next().unwrap_or_else(|| {
                eprintln!("Error: missing output path argument");
                process::exit(1)
            }));

            let params = Params{input_path, output_path};

            if let Err(e) = classifiles::run_backup(params, &root_log) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        "restore" => {
            let input_path = PathBuf::from(args.next().unwrap_or_else(|| {
                eprintln!("Error: missing input path argument");
                process::exit(1)
            }));

            let output_path = PathBuf::from(args.next().unwrap_or_else(|| {
                eprintln!("Error: missing output path argument");
                process::exit(1)
            }));

            let params = Params{input_path, output_path};

            if let Err(e) = classifiles::run_restore(params, &root_log) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        _ => eprintln!("Error: invalid verb. Valid verbs are: scan, backup, restore"),
    }
}
