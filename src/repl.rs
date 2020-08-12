use std::io::{self, Write}
use mime_db::{extension, extensions, lookup};

trait CollectCmd : Iterator {
    fn collect_cmd(&mut self) -> (Option<Self::Item>, Option<Self::Item>) {
        (self.next(), self.next())
    }
}

impl<T: Iterator> CollectCmd for T {}

fn _print_result<T: std::fmt::Debug>(res: Option<T>) {
    println!("{:?}", res);
}

fn _repl() -> Result<(), io::Error> {
    loop {
        print!("> ");
        io::stdout().flush().unwrap();
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;

        let cmd = line.split(' ').map(str::trim).collect_cmd();
        match cmd {
            (Some("extensions"), Some(arg)) => {
                _print_result(extensions(arg));
            }
            (Some("extension"), Some(arg)) => {
                _print_result(extension(arg));
            }
            (Some("lookup"), Some(arg)) => {
                _print_result(lookup(arg));
            }
            (Some("exit"), None) => {
                break
            }
            _ => ()
        }
    }
    Ok(())
}